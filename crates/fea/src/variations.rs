//! What a variable-font compile needs to know beyond the code ([`FeaVariations`], which
//! `fontcraft_features::var_info` implements), and its adapter to fea-rs's `VariationInfo`.
//!
//! The adapter follows fontc's `FeaVariationInfo` (`fontbe/src/features.rs`, Apache-2.0, see
//! `NOTICE`): axes with a user-to-design converter, so fea-rs normalises condition sets and
//! variable values through the axis mapping; values resolved into a default plus deltas by the
//! implementation; number values asked for at every master location.

use std::collections::HashMap;

use fontdrasil::coords::{CoordConverter, DesignCoord, NormalizedCoord, NormalizedLocation, UserCoord};
use write_fonts_052::tables::variations::VariationRegion;

use crate::{Diagnostic, MAX_AXES, MAX_DELTAS, MAX_LOCATIONS, Metric, Severity, Tag};

/// One axis of the variable font, in user coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct Axis {
    pub tag: Tag,
    pub name: String,
    pub min: f64,
    pub default: f64,
    pub max: f64,
    pub hidden: bool,
    /// The axis mapping as (user, design) pairs in increasing order; empty when design
    /// coordinates equal user coordinates.
    pub mapping: Vec<(f64, f64)>,
}

/// A point in the design space: a normalised coordinate (-1 to 1) per axis tag. Axes not listed
/// are at their default (0).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Location(pub Vec<(Tag, f64)>);

/// The variable font's axes, masters and number values, and the step that turns per-location
/// values into a default plus deltas. Implementations must give finite numbers and keep within
/// [`crate::MAX_AXES`] and [`crate::MAX_LOCATIONS`]; the compile checks both and refuses more.
/// `Sync`: the compile runs on its own thread.
pub trait FeaVariations: Sync {
    /// The axes in `fvar` order.
    fn axes(&self) -> &[Axis];

    /// The masters' locations, normalised.
    fn master_locations(&self) -> &[Location];

    /// The number value `name` at `location` (a master's location), or `None` when the font
    /// has no number of that name. fea-rs asks for each master location in turn.
    fn number_value(&self, name: &str, location: &Location) -> Option<f64>;

    /// One value per location (the masters', or the locations a `(axis=coord:value …)` scalar
    /// names) turned into the default location's value plus deltas, rounded to font units. An
    /// error is a message for the user (values that cannot be interpolated, a location outside
    /// the axes).
    fn resolve(&self, values: &[(Location, f64)]) -> Result<Metric, String>;
}

/// Why fea-rs could not resolve a variable value; the text reaches the user.
#[derive(Debug)]
pub(crate) struct VariationError(String);

impl std::fmt::Display for VariationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for VariationError {}

/// fea-rs's view of the variable font: the axes and masters of a [`FeaVariations`], or, for a
/// variable font compiled without one, the generated values' axis tags alone (fea-rs then
/// builds their deltas into the `GDEF` variation store, and any variable syntax in the code is
/// refused as unknown axes).
pub(crate) struct Info<'a> {
    tags: Vec<Tag>,
    axes: Vec<fontdrasil::types::Axis>,
    masters: Vec<NormalizedLocation>,
    source: Option<&'a dyn FeaVariations>,
}

impl<'a> Info<'a> {
    /// `None` for a static font. Refuses axes and masters beyond the limits, coordinates that are
    /// not finite and mappings that cannot be inverted.
    pub(crate) fn new(source: Option<&'a dyn FeaVariations>, generated_axes: &[Tag]) -> Result<Option<Self>, Diagnostic> {
        let refuse = |m: String| Diagnostic::request(Severity::Error, m);
        let Some(source) = source else {
            if generated_axes.is_empty() {
                return Ok(None);
            }
            if generated_axes.len() > MAX_AXES {
                return Err(refuse(format!("a variable font may have at most {MAX_AXES} axes")));
            }
            return Ok(Some(Self { tags: generated_axes.to_vec(), axes: Vec::new(), masters: Vec::new(), source: None }));
        };
        let given = source.axes();
        if given.is_empty() || given.len() > MAX_AXES {
            return Err(refuse(format!("a variable font needs between 1 and {MAX_AXES} axes")));
        }
        let mut tags = Vec::with_capacity(given.len());
        let mut axes = Vec::with_capacity(given.len());
        for axis in given {
            if tags.contains(&axis.tag) {
                return Err(refuse(format!("the axis `{}` is listed twice", axis.tag)));
            }
            tags.push(axis.tag);
            axes.push(fea_axis(axis).map_err(|m| refuse(format!("the axis `{}` cannot be used: {m}", axis.tag)))?);
        }
        let locations = source.master_locations();
        if locations.len() > MAX_LOCATIONS {
            return Err(refuse(format!("a variable font may have at most {MAX_LOCATIONS} masters")));
        }
        let masters = locations.iter().map(|l| normalized(&tags, l)).collect::<Result<Vec<_>, _>>().map_err(refuse)?;
        Ok(Some(Self { tags, axes, masters, source: Some(source) }))
    }

    /// The axis tags in `fvar` order.
    pub(crate) fn tags(&self) -> &[Tag] {
        &self.tags
    }

    fn source(&self) -> Result<&'a dyn FeaVariations, VariationError> {
        self.source.ok_or_else(|| VariationError("variable values need the font's axes".into()))
    }
}

/// The fea-rs axis of `axis`: its user range and a converter through its mapping.
fn fea_axis(axis: &Axis) -> Result<fontdrasil::types::Axis, String> {
    let (min, default, max) = (axis.min, axis.default, axis.max);
    if ![min, default, max].iter().all(|v| v.is_finite()) || !(min <= default && default <= max) {
        return Err("its minimum, default and maximum are not in order".into());
    }
    let converter = if axis.mapping.is_empty() {
        CoordConverter::unmapped(UserCoord::new(min), UserCoord::new(default), UserCoord::new(max))
    } else {
        if axis.mapping.len() > MAX_LOCATIONS || axis.mapping.iter().any(|(u, d)| !u.is_finite() || !d.is_finite()) {
            return Err("its mapping is not a list of finite points".into());
        }
        // The ends and the default are mapping points, so the design range is the mapped range.
        let mut points = axis.mapping.clone();
        for user in [min, default, max] {
            if !points.iter().any(|(u, _)| *u == user) {
                points.push((user, piecewise(&axis.mapping, user)));
            }
        }
        points.sort_by(|a, b| a.0.total_cmp(&b.0));
        points.dedup_by(|a, b| a.0 == b.0);
        let default_idx = points.iter().position(|(u, _)| *u == default).unwrap_or(0);
        let pairs = points.into_iter().map(|(u, d)| (UserCoord::new(u), DesignCoord::new(d))).collect();
        CoordConverter::new(pairs, default_idx).map_err(|e| e.to_string())?
    };
    Ok(fontdrasil::types::Axis {
        name: axis.name.clone(),
        tag: axis.tag,
        min: UserCoord::new(min),
        default: UserCoord::new(default),
        max: UserCoord::new(max),
        hidden: axis.hidden,
        converter,
        localized_names: HashMap::new(),
    })
}

/// `x` through the piecewise linear map `points` (sorted by input), held flat past the ends.
fn piecewise(points: &[(f64, f64)], x: f64) -> f64 {
    let mut previous: Option<(f64, f64)> = None;
    for &(px, py) in points {
        if x <= px {
            return match previous {
                Some((qx, qy)) if px > qx => qy + (py - qy) * (x - qx) / (px - qx),
                _ => py,
            };
        }
        previous = Some((px, py));
    }
    previous.map_or(x, |(_, y)| y)
}

/// `location` with every axis of `tags` listed, in `tags` order.
fn normalized(tags: &[Tag], location: &Location) -> Result<NormalizedLocation, String> {
    let mut out = NormalizedLocation::new();
    for tag in tags {
        let value = location.0.iter().find(|(t, _)| t == tag).map_or(0.0, |(_, v)| *v);
        if !value.is_finite() || !(-1.0..=1.0).contains(&value) {
            return Err(format!("a master's location on `{tag}` is outside -1 to 1"));
        }
        out.insert(*tag, NormalizedCoord::new(value));
    }
    Ok(out)
}

impl fea_rs::compile::VariationInfo for Info<'_> {
    type Error = VariationError;

    fn axis_count(&self) -> u16 {
        u16::try_from(self.tags.len()).unwrap_or(u16::MAX)
    }

    fn axis(&self, tag: Tag) -> Option<(usize, &fontdrasil::types::Axis)> {
        self.axes.iter().enumerate().find(|(_, a)| a.tag == tag)
    }

    fn resolve_variable_metric(&self, locations: &HashMap<NormalizedLocation, i16>) -> Result<(i16, Vec<(VariationRegion, i16)>), VariationError> {
        let source = self.source()?;
        if locations.len() > MAX_LOCATIONS {
            return Err(VariationError(format!("a variable value may list at most {MAX_LOCATIONS} locations")));
        }
        let mut values: Vec<(Location, f64)> = locations
            .iter()
            .map(|(loc, v)| {
                let coords = self.tags.iter().filter_map(|t| Some((*t, loc.get(*t)?.to_f64()))).filter(|(_, c)| *c != 0.0).collect();
                (Location(coords), f64::from(*v))
            })
            .collect();
        // The map's order is random; the model's result must not depend on it.
        values.sort_by(|a, b| {
            let key = |l: &Location| self.tags.iter().map(|t| l.0.iter().find(|(x, _)| x == t).map_or(0.0, |(_, v)| *v)).collect::<Vec<_>>();
            key(&a.0).partial_cmp(&key(&b.0)).unwrap_or(std::cmp::Ordering::Equal)
        });
        let metric = source.resolve(&values).map_err(VariationError)?;
        if metric.deltas.len() > MAX_DELTAS {
            return Err(VariationError(format!("a variable value has more than {MAX_DELTAS} deltas")));
        }
        let deltas = match crate::provider::deltas(&metric, &self.tags) {
            Some(write_fonts_052::tables::layout::builders::DeviceOrDeltas::Deltas(d)) => d,
            _ => Vec::new(),
        };
        Ok((metric.default, deltas))
    }

    fn resolve_glyphs_number_value(&self, name: &str) -> Result<HashMap<NormalizedLocation, f64>, VariationError> {
        let source = self.source()?;
        let mut out = HashMap::with_capacity(self.masters.len());
        let mut known = false;
        for (location, master) in source.master_locations().iter().zip(&self.masters) {
            let value = source.number_value(name, location);
            known |= value.is_some();
            out.insert(master.clone(), value.filter(|v| v.is_finite()).unwrap_or(0.0));
        }
        if !known {
            return Err(VariationError(format!("the font has no number value `{name}`")));
        }
        Ok(out)
    }
}
