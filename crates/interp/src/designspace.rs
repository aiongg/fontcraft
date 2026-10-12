//! The designspace of a font or of one variable font setting (M9.2).
//!
//! Rows: `axes.definition`, `axes.tagRules`, `axes.coordinatesAreFreeNumbers`,
//! `axes.linearSpread`, `axes.internalVersusExternal`, `axes.unusedAxesDropped`,
//! `masters.mustCoverDesignspace`, `masters.minimalSetup`, `masters.cornerAndIntermediateSetups`,
//! `variable.originMaster`, `variable.originIsFallback`, `variable.originNeedsMinimalSetup`,
//! `axisLocation.*`, `axisMappings.*`, `virtualMaster.*`, `variableFontSetting.disableMasters`,
//! `variableFontSetting.defaultWhenNone`, `instances.extrapolatedExcludedFromVariable`,
//! `font.axes`, `master.axesValues`, `instance.axesValues`, `instance.type`.
//!
//! Contract (`docs/plans/M9.md` § 3 › Design decisions):
//! - Coordinates are **internal** (design) everywhere in the model and in commands. External
//!   (user) coordinates appear only at the table boundary (`fvar`, `avar`, `STAT`, feature
//!   variations) and when a command asks for them. Normalized coordinates (−1…1, 0 at the
//!   origin) are what the variation model works in.
//! - The map from external to internal coordinates is the font's Axis Mappings parameter, else
//!   the masters' Axis Location parameters, else identity (`fontcraft_model::axes::mapping`
//!   derives it as the oracles do); with both present the oracles' precedence applies. A
//!   variable font setting's own Axis Mappings replaces the font's for that setting. A map that
//!   neither rises nor falls throughout is ignored with a warning ([`Designspace::warnings`]).
//! - The origin is the first master unless the Variable Font Origin parameter (or its older name)
//!   names another (`docs/plans/M9.md` § 7 question 3, decided: the handbook's rule). The
//!   difference from the oracles is recorded in `docs/spec/40-compile.md` § 4a.
//! - [`Designspace::for_setting`] applies a variable font setting: its Disable Masters list
//!   removes masters, and when it removes any, an axis on which every remaining master and
//!   virtual master sits at the same value is dropped.
//! - Axis ranges cover the masters and the virtual masters; an explicit Axis Mappings parameter
//!   (or a range the source states) widens them to its ends, as the oracles do. Instances outside
//!   the ranges are [`Placement::Outside`] (extrapolated: allowed for static instances, left out
//!   of variable fonts).
//! - Normalization follows the oracles: internal values are normalized against the internal
//!   values at the external minimum, default and maximum, and the sign follows the external axis
//!   even where internal values fall as external ones rise.
//! - Building a designspace checks every count against [`crate::error`]'s limits and refuses
//!   coordinates that are not finite. Pure; no I/O.
//!
//! The axis ranges and normalization follow fontc's `glyphs2fontir/src/toir.rs` (`to_ir_axis`)
//! and fontdrasil's `CoordConverter` (`fontdrasil/src/coords.rs`), Apache-2.0.

use std::sync::Arc;

use fontcraft_geom::variation::normalize_value;
use fontcraft_model::axes::{self, Mapping};
use fontcraft_model::{AxisRange, Font, Master};

use crate::params::{self, ORIGIN, ORIGIN_BY_NAME};
use crate::{InterpError, MAX_AXES, MAX_COORDINATE, MAX_INSTANCES, MAX_MAPPING_POINTS, MAX_SOURCES};

/// Two coordinates closer than this (relative to their size) are the same.
const SAME: f64 = 1e-9;

/// A location in internal (design) coordinates: one value per axis of the designspace, in its
/// axis order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Location(pub Vec<f64>);

/// A location in external (user) coordinates, one value per axis.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UserLocation(pub Vec<f64>);

/// A location in normalized coordinates: −1 at an axis minimum, 0 at the origin, 1 at the
/// maximum; beyond ±1 for an extrapolated location.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NormLocation(pub Vec<f64>);

/// The coordinate space a value is given in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Space {
    Internal,
    External,
}

/// One axis of a designspace.
#[derive(Clone, Debug, PartialEq)]
pub struct AxisInfo {
    /// The four-character OpenType tag.
    pub tag: String,
    pub name: String,
    pub hidden: bool,
    /// The internal coordinates at the external minimum, at the origin and at the external
    /// maximum. `minimum` is greater than `maximum` on an axis whose internal values fall as its
    /// external values rise.
    pub internal: AxisRange,
    /// Minimum, origin and maximum in external coordinates.
    pub external: AxisRange,
    /// External to internal map of the axis (identity when the font sets none).
    pub map: Mapping,
    /// The axis's index in `Font::axes` (differs from its index here when a setting dropped an
    /// axis).
    pub font_axis: usize,
}

impl AxisInfo {
    /// +1 when internal values rise with external ones, −1 when they fall.
    fn sign(&self) -> f64 {
        if self.internal.minimum > self.internal.maximum { -1.0 } else { 1.0 }
    }

    /// The internal range turned to rise with the external one: (minimum, default, maximum).
    fn oriented(&self) -> (f64, f64, f64) {
        let s = self.sign();
        (s * self.internal.minimum, s * self.internal.default, s * self.internal.maximum)
    }

    /// `v` (internal) normalized; extrapolated beyond the range.
    fn normalize(&self, v: f64, extrapolate: bool) -> Option<f64> {
        let (lo, def, hi) = self.oriented();
        normalize_value(self.sign() * v, lo, def, hi, extrapolate)
    }

    /// Whether `v` (internal) is within the range, both ends included.
    fn contains(&self, v: f64) -> bool {
        let (lo, _, hi) = self.oriented();
        let v = self.sign() * v;
        let slack = SAME * (1.0 + v.abs());
        v.is_finite() && (lo - slack..=hi + slack).contains(&v)
    }
}

/// A master of the designspace.
#[derive(Clone, Debug, PartialEq)]
pub struct MasterInfo {
    pub id: String,
    pub name: String,
    /// Its index in `Font::masters`.
    pub index: usize,
    pub location: Location,
}

/// A virtual master: a location the interpolation treats as a master that has no layers of its
/// own, so intermediate layers on an axis with a single master have something to interpolate
/// from (`virtualMaster.*`).
#[derive(Clone, Debug, PartialEq)]
pub struct VirtualMasterInfo {
    pub name: String,
    pub location: Location,
}

/// Where a location lies relative to the axis ranges.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// On or within every axis range.
    Inside,
    /// Beyond the range of at least one axis: an extrapolated location.
    Outside,
}

/// The axes, masters and origin a font interpolates with.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Designspace {
    axes: Vec<AxisInfo>,
    masters: Vec<MasterInfo>,
    virtual_masters: Vec<VirtualMasterInfo>,
    /// Index into `masters`.
    origin: Option<usize>,
    setting: Option<usize>,
    warnings: Vec<String>,
}

/// A coordinate checked to be finite and within [`MAX_COORDINATE`].
fn usable(v: f64, what: impl FnOnce() -> String) -> Result<f64, InterpError> {
    if v.is_finite() && v.abs() <= MAX_COORDINATE { Ok(v) } else { Err(InterpError::BadNumber { what: what() }) }
}

/// The internal coordinate on font axis `i` of a master or instance location; a location shorter
/// than the axes has the reference's defaults.
fn coordinate(location: &[f64], i: usize, what: impl FnOnce() -> String) -> Result<f64, InterpError> {
    match location.get(i) {
        Some(v) => usable(*v, what),
        None => Ok(axes::design_value(location, i)),
    }
}

/// The OpenType tag of an axis: printable ASCII, not starting with a space; a longer tag
/// exports its first four characters and a shorter one is padded with spaces.
pub fn export_tag(raw: &str) -> Result<String, InterpError> {
    let bad = || InterpError::BadAxisTag(raw.to_string());
    let tag: String = raw.chars().take(4).collect();
    if tag.is_empty() || tag.starts_with(' ') || !tag.chars().all(|c| (' '..='~').contains(&c)) {
        return Err(bad());
    }
    Ok(format!("{tag:<4}"))
}

/// The index in `masters` of the origin: the master the Variable Font Origin parameter names
/// (by id, or by name as older files do), else the master the older Variation Font Origin
/// parameter names, else the first.
fn origin_index(font: &Font, masters: &[&Master]) -> Result<Option<usize>, InterpError> {
    for (param, by_id) in [(ORIGIN, true), (ORIGIN_BY_NAME, false)] {
        let Some(value) = font.info.params.get(param) else { continue };
        let named = value.as_str().ok_or_else(|| InterpError::BadParameter { name: param.to_string(), reason: "it is not a master name".into() })?;
        let by_name = || masters.iter().position(|m| m.name == named);
        let found = if by_id { masters.iter().position(|m| m.id == named).or_else(by_name) } else { by_name() };
        return match found {
            Some(i) => Ok(Some(i)),
            None => Err(InterpError::BadMasterSetup(format!("the {param} parameter names `{named}`, which is not among the masters"))),
        };
    }
    Ok((!masters.is_empty()).then_some(0))
}

/// Whether the internal values of `pairs` (sorted by external value) never fall, or never rise.
fn monotonic(pairs: &[(f64, f64)]) -> bool {
    let rises = pairs.windows(2).all(|w| matches!(w, [a, b] if b.1 >= a.1));
    let falls = pairs.windows(2).all(|w| matches!(w, [a, b] if b.1 <= a.1));
    rises || falls
}

impl Designspace {
    /// The designspace of the whole font: every axis, every master, the virtual masters and the
    /// origin. Refuses bad axis tags, duplicate tags, unusable mappings, coordinates that are not
    /// finite and counts beyond the limits.
    pub fn new(font: &Font) -> Result<Designspace, InterpError> {
        let all: Vec<usize> = (0..font.masters.len()).collect();
        build(font, &all)
    }

    /// The designspace of the variable font setting at `setting` (an index into
    /// `Font::instances` whose kind is `variable`): the masters its Disable Masters parameter
    /// names are removed, and when any is, axes on which the remaining masters and the virtual
    /// masters all sit at one value are dropped. The setting's Axis Mappings, when it has one,
    /// replaces the font's.
    pub fn for_setting(font: &Font, setting: usize) -> Result<Designspace, InterpError> {
        let instance = font.instances.get(setting).ok_or(InterpError::UnknownInstance(setting))?;
        if instance.kind.as_deref() != Some("variable") {
            return Err(InterpError::NotAVariableSetting(setting));
        }
        let disabled = params::disabled_masters(instance);
        let mut warnings: Vec<String> = disabled
            .iter()
            .filter(|n| !font.masters.iter().any(|m| &m.name == *n))
            .map(|n| format!("{} names `{n}`, which is not a master", params::DISABLE_MASTERS))
            .collect();
        let keep: Vec<usize> = font.masters.iter().enumerate().filter(|(_, m)| !disabled.contains(&m.name)).map(|(i, _)| i).collect();
        if keep.is_empty() && !font.masters.is_empty() {
            return Err(InterpError::BadMasterSetup(format!("{} leaves no master", params::DISABLE_MASTERS)));
        }
        let subset = keep.len() != font.masters.len();
        let mut lite = font.clone();
        if subset {
            lite.masters = Arc::new(keep.iter().filter_map(|&i| font.masters.get(i).cloned()).collect());
        }
        if let Some(value) = instance.params.get(params::AXIS_MAPPINGS) {
            Arc::make_mut(&mut lite.info).params.set(params::AXIS_MAPPINGS, value.clone());
        }
        let mut ds = build(&lite, &keep)?;
        ds.setting = Some(setting);
        warnings.append(&mut ds.warnings);
        ds.warnings = warnings;
        if subset {
            ds.drop_unused_axes();
        }
        Ok(ds)
    }

    /// Removes the axes on which every master and virtual master sits at the origin's value.
    fn drop_unused_axes(&mut self) {
        let Some(origin) = self.origin().map(|m| m.location.0.clone()) else { return };
        let used: Vec<bool> = (0..self.axes.len())
            .map(|k| {
                let Some(&o) = origin.get(k) else { return true };
                let differs = |l: &Location| l.0.get(k).is_some_and(|v| !same(*v, o));
                self.masters.iter().map(|m| &m.location).chain(self.virtual_masters.iter().map(|v| &v.location)).any(differs)
            })
            .collect();
        let keep = |values: &mut Vec<f64>| {
            let mut flags = used.iter();
            values.retain(|_| flags.next().copied().unwrap_or(true));
        };
        for m in &mut self.masters {
            keep(&mut m.location.0);
        }
        for v in &mut self.virtual_masters {
            keep(&mut v.location.0);
        }
        let mut flags = used.iter();
        self.axes.retain(|_| flags.next().copied().unwrap_or(true));
    }

    /// The axes, in the font's axis order (less any a setting dropped).
    pub fn axes(&self) -> &[AxisInfo] {
        &self.axes
    }

    /// The axis with tag `tag` and its index here.
    pub fn axis(&self, tag: &str) -> Option<(usize, &AxisInfo)> {
        self.axes.iter().enumerate().find(|(_, a)| a.tag == tag)
    }

    /// The masters, in the font's order (less any a setting disabled).
    pub fn masters(&self) -> &[MasterInfo] {
        &self.masters
    }

    /// The master with id `id`.
    pub fn master(&self, id: &str) -> Option<&MasterInfo> {
        self.masters.iter().find(|m| m.id == id)
    }

    pub fn virtual_masters(&self) -> &[VirtualMasterInfo] {
        &self.virtual_masters
    }

    /// The origin master: the default of every axis, whose outlines a variable font stores.
    /// `None` only for a font without masters.
    pub fn origin(&self) -> Option<&MasterInfo> {
        self.origin.and_then(|i| self.masters.get(i))
    }

    /// The variable font setting this designspace was built for; `None` for the whole font.
    pub fn setting(&self) -> Option<usize> {
        self.setting
    }

    /// What building the designspace ignored: a mapping that neither rises nor falls, a
    /// disabled master name that is not a master. For the export report.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// The internal location of the instance at `instance` in `Font::instances`, its
    /// coordinates matched to this designspace's axes.
    pub fn instance_location(&self, font: &Font, instance: usize) -> Result<Location, InterpError> {
        let inst = font.instances.get(instance).ok_or(InterpError::UnknownInstance(instance))?;
        let values = self
            .axes
            .iter()
            .map(|a| coordinate(&inst.location, a.font_axis, || format!("the {} coordinate of instance `{}`", a.name, inst.name)))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Location(values))
    }

    /// The static instances a variable font names (`instances.extrapolatedExcludedFromVariable`):
    /// the exported ones inside the axis ranges, as indices into `Font::instances`.
    pub fn named_instances(&self, font: &Font) -> Vec<usize> {
        (0..font.instances.len().min(crate::MAX_INSTANCES))
            .filter(|&i| font.instances.get(i).is_some_and(|inst| inst.exports && inst.kind.as_deref() != Some("variable")))
            .filter(|&i| self.instance_location(font, i).is_ok_and(|l| self.classify(&l) == Placement::Inside))
            .collect()
    }

    /// Checks that a location has one usable value per axis.
    fn check_location(&self, values: &[f64]) -> Result<(), InterpError> {
        if values.len() != self.axes.len() {
            return Err(InterpError::BadNumber { what: format!("a location with {} values for {} axes", values.len(), self.axes.len()) });
        }
        for (v, a) in values.iter().zip(&self.axes) {
            usable(*v, || format!("the {} coordinate {v}", a.name))?;
        }
        Ok(())
    }

    /// `location` normalized for the variation model. Values beyond an axis range extrapolate.
    /// Refuses a location of the wrong length or with a value that is not finite.
    pub fn to_normalized(&self, location: &Location) -> Result<NormLocation, InterpError> {
        self.check_location(&location.0)?;
        let values = location
            .0
            .iter()
            .zip(&self.axes)
            .map(|(v, a)| a.normalize(*v, true).ok_or_else(|| InterpError::BadNumber { what: format!("the range of axis {}", a.name) }))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(NormLocation(values))
    }

    /// `location` in external coordinates, through each axis's map read backwards.
    pub fn to_external(&self, location: &Location) -> Result<UserLocation, InterpError> {
        self.check_location(&location.0)?;
        Ok(UserLocation(location.0.iter().zip(&self.axes).map(|(v, a)| a.map.to_user(*v)).collect()))
    }

    /// `location` in internal coordinates, through each axis's map.
    pub fn to_internal(&self, location: &UserLocation) -> Result<Location, InterpError> {
        self.check_location(&location.0)?;
        Ok(Location(location.0.iter().zip(&self.axes).map(|(v, a)| a.map.to_design(*v)).collect()))
    }

    /// Whether `location` is within every axis range (both ends included). A location of the
    /// wrong length or with a value that is not finite is outside.
    pub fn classify(&self, location: &Location) -> Placement {
        let inside = location.0.len() == self.axes.len() && location.0.iter().zip(&self.axes).all(|(v, a)| a.contains(*v));
        if inside { Placement::Inside } else { Placement::Outside }
    }

    /// The `avar` segment map of the axis at `axis`: `(external, internal)` pairs in normalized
    /// coordinates, sorted, with −1, 0 and 1 present. `None` for an unknown axis and for a map
    /// that is the identity once normalized (no `avar` entry needed).
    pub fn avar_map(&self, axis: usize) -> Option<Vec<(f64, f64)>> {
        let a = self.axes.get(axis)?;
        let ext = a.external;
        let mut points: Vec<(f64, f64)> = Vec::new();
        for &(user, design) in a.map.pairs() {
            let key = normalize_value(user, ext.minimum, ext.default, ext.maximum, false)?;
            let value = a.normalize(design, false)?;
            match points.iter_mut().find(|p| p.0 == key) {
                Some(p) => p.1 = value,
                None => points.push((key, value)),
            }
        }
        for end in [-1.0, 0.0, 1.0] {
            if !points.iter().any(|p| p.0 == end) {
                points.push((end, end));
            }
        }
        points.sort_by(|p, q| p.0.total_cmp(&q.0));
        if points.iter().all(|(k, v)| (k - v).abs() <= SAME) { None } else { Some(points) }
    }

    /// Checks that the origin can span the designspace in a variable font
    /// (`masters.minimalSetup`, `variable.originNeedsMinimalSetup`): on every axis, on each side
    /// of the origin the range reaches, some master or virtual master must differ from the origin
    /// on that axis alone. The refusal names the first axis and side that has none.
    pub fn check_variable_origin(&self) -> Result<(), InterpError> {
        let Some(origin) = self.origin() else { return Err(InterpError::BadMasterSetup("the font has no masters".into())) };
        let o = &origin.location.0;
        let sources: Vec<&Location> = self.masters.iter().map(|m| &m.location).chain(self.virtual_masters.iter().map(|v| &v.location)).collect();
        for (k, a) in self.axes.iter().enumerate() {
            let Some(&ok) = o.get(k) else { continue };
            let (lo, def, hi) = a.oriented();
            for (needed, side, label) in [(lo < def, -1.0, "below"), (hi > def, 1.0, "above")] {
                if !needed {
                    continue;
                }
                let reached = sources.iter().any(|l| {
                    let only_here = l.0.iter().zip(o).enumerate().all(|(j, (v, w))| j == k || same(*v, *w));
                    let here = l.0.get(k).copied().unwrap_or(ok);
                    only_here && (a.sign() * (here - ok)) * side > 0.0
                });
                if !reached {
                    return Err(InterpError::BadMasterSetup(format!(
                        "no master lies {label} the origin `{}` on the {} axis alone; add one, or choose another origin",
                        origin.name, a.name
                    )));
                }
            }
        }
        Ok(())
    }
}

fn same(a: f64, b: f64) -> bool {
    (a - b).abs() <= SAME * (1.0 + a.abs().max(b.abs()))
}

/// The designspace of `font`, whose masters are the font's masters at `indices` (in order).
fn build(font: &Font, indices: &[usize]) -> Result<Designspace, InterpError> {
    if font.axes.len() > MAX_AXES {
        return Err(InterpError::LimitExceeded { what: "axes", limit: MAX_AXES });
    }
    if font.instances.len() > MAX_INSTANCES {
        return Err(InterpError::LimitExceeded { what: "instances", limit: MAX_INSTANCES });
    }
    let virtual_entries = params::virtual_masters(font)?;
    if font.masters.len().saturating_add(virtual_entries.len()) > MAX_SOURCES {
        return Err(InterpError::LimitExceeded { what: "masters and virtual masters", limit: MAX_SOURCES });
    }
    let master_refs: Vec<&Master> = font.masters.iter().collect();
    let origin = origin_index(font, &master_refs)?;

    let mut masters = Vec::with_capacity(master_refs.len());
    for (n, m) in master_refs.iter().enumerate() {
        let values = (0..font.axes.len())
            .map(|i| coordinate(&m.location, i, || format!("coordinate {} of master `{}`", i + 1, m.name)))
            .collect::<Result<Vec<_>, _>>()?;
        masters.push(MasterInfo { id: m.id.clone(), name: m.name.clone(), index: indices.get(n).copied().unwrap_or(n), location: Location(values) });
    }
    let origin_location = origin.and_then(|i| masters.get(i)).map(|m| m.location.0.clone());
    let default_at = |i: usize| origin_location.as_ref().and_then(|l| l.get(i).copied()).unwrap_or_else(|| axes::design_value(&[], i));

    let mut virtual_masters = Vec::with_capacity(virtual_entries.len());
    for (n, entry) in virtual_entries.iter().enumerate() {
        let mut values: Vec<f64> = (0..font.axes.len()).map(default_at).collect();
        for (name, v) in &entry.coordinates {
            // Coordinates of axes the font does not have are ignored, as the oracles do.
            if let Some(slot) = font.axes.iter().position(|a| &a.name == name).and_then(|i| values.get_mut(i)) {
                *slot = usable(*v, || format!("the {name} coordinate of virtual master {}", n + 1))?;
            }
        }
        virtual_masters.push(VirtualMasterInfo { name: format!("Virtual Master {}", n + 1), location: Location(values) });
    }

    let mut warnings = Vec::new();
    let mut tags: Vec<String> = Vec::with_capacity(font.axes.len());
    let mut axis_infos = Vec::with_capacity(font.axes.len());
    for (i, axis) in font.axes.iter().enumerate() {
        let tag = export_tag(&axis.tag)?;
        if tags.contains(&tag) {
            return Err(InterpError::DuplicateAxisTag(tag));
        }
        tags.push(tag.clone());
        let designs: Vec<f64> =
            masters.iter().map(|m| &m.location).chain(virtual_masters.iter().map(|v| &v.location)).filter_map(|l| l.0.get(i).copied()).collect();
        let info = axis_info(font, i, &tag, default_at(i), &designs, &mut warnings)?;
        axis_infos.push(info);
    }
    Ok(Designspace { axes: axis_infos, masters, virtual_masters, origin, setting: None, warnings })
}

/// Axis `i` of `font`: its map, internal and external ranges. `origin` is the origin's internal
/// coordinate and `designs` the coordinates of every master and virtual master.
fn axis_info(font: &Font, i: usize, tag: &str, origin: f64, designs: &[f64], warnings: &mut Vec<String>) -> Result<AxisInfo, InterpError> {
    let axis = font.axes.get(i).ok_or_else(|| InterpError::UnknownAxis(tag.to_string()))?;
    // The parameter's own size first: reading it into a map costs more than counting it.
    let raw_points = font
        .info
        .params
        .get(params::AXIS_MAPPINGS)
        .and_then(fontcraft_model::plist::Value::as_dict)
        .and_then(|d| d.get(&axis.tag))
        .and_then(fontcraft_model::plist::Value::as_dict)
        .map_or(0, |d| d.entries.len());
    if raw_points > MAX_MAPPING_POINTS {
        return Err(InterpError::LimitExceeded { what: "points in an axis mapping", limit: MAX_MAPPING_POINTS });
    }
    let mut map = axes::mapping(font, i);
    if map.pairs().len() > MAX_MAPPING_POINTS {
        return Err(InterpError::LimitExceeded { what: "points in an axis mapping", limit: MAX_MAPPING_POINTS });
    }
    for (u, d) in map.pairs() {
        usable(*u, || format!("the mapping of axis {}", axis.name))?;
        usable(*d, || format!("the mapping of axis {}", axis.name))?;
    }
    if !monotonic(map.pairs()) {
        warnings.push(format!("the mapping of axis {} neither rises nor falls throughout; it is ignored", axis.name));
        map = Mapping::new(vec![(origin, origin)], map.default);
    }
    // An explicit mapping, or a range the source states, sets the external range by its ends.
    let explicit_ends = match axis.range {
        Some(r) => Some((r.minimum, r.maximum)),
        None => axes::mapping_parameter(font, &axis.tag).filter(|p| !p.is_empty()).map(|_| (map.minimum(), map.maximum())),
    };
    let (first, last) = (map.pairs().first().map_or(origin, |p| p.1), map.pairs().last().map_or(origin, |p| p.1));
    let sign = if last < first { -1.0 } else { 1.0 };
    let mut oriented: Vec<f64> = designs.iter().map(|d| sign * d).collect();
    oriented.push(sign * origin);
    let lowest = |v: &[f64]| v.iter().copied().fold(f64::INFINITY, f64::min);
    let highest = |v: &[f64]| v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let degenerate = same(lowest(&oriented), highest(&oriented));
    if let Some((umin, umax)) = explicit_ends {
        oriented.push(sign * map.to_design(umin));
        oriented.push(sign * map.to_design(umax));
    }
    let (lo, hi) = (lowest(&oriented), highest(&oriented));
    let (imin, imax) = (sign * lo, sign * hi);
    if degenerate {
        // Every master at one value: no extent, and user and internal coordinates are the same,
        // as the oracles treat it.
        let range = AxisRange { minimum: origin, default: origin, maximum: origin };
        let map = Mapping::new(vec![(origin, origin)], origin);
        return Ok(AxisInfo {
            tag: tag.to_string(),
            name: axis.name.clone(),
            hidden: axis.hidden,
            internal: range,
            external: range,
            map,
            font_axis: i,
        });
    }
    let mut emin = map.to_user(imin);
    let mut emax = map.to_user(imax);
    if let Some((umin, umax)) = explicit_ends {
        // Taken from the mapping itself: reading a many-to-one map backwards loses its ends.
        emin = emin.min(umin);
        emax = emax.max(umax);
    }
    usable(emin, || format!("the external minimum of axis {}", axis.name))?;
    usable(emax, || format!("the external maximum of axis {}", axis.name))?;
    let edefault = map.to_user(origin).max(emin).min(emax);
    map.default = edefault;
    Ok(AxisInfo {
        tag: tag.to_string(),
        name: axis.name.clone(),
        hidden: axis.hidden,
        internal: AxisRange { minimum: imin, default: origin, maximum: imax },
        external: AxisRange { minimum: emin, default: edefault, maximum: emax },
        map,
        font_axis: i,
    })
}

/// The indices in `Font::instances` of the variable font settings, in order. A font without any
/// exports one variable font from the whole designspace (`variableFontSetting.defaultWhenNone`).
pub fn variable_settings(font: &Font) -> Vec<usize> {
    font.instances.iter().enumerate().filter(|(_, i)| i.kind.as_deref() == Some("variable")).map(|(n, _)| n).collect()
}

#[cfg(test)]
pub(crate) mod tests {
    //! Expected values: the `fvar` axes and `avar` maps fontc 1.0.0 and fontmake 3.12.1 write
    //! for fontc's test sources of the same names, rebuilt here in code. Where FontCraft's origin
    //! (the first master) differs from theirs, the test also checks the values with the origin
    //! named, which are the oracles' own.
    use super::*;
    use crate::params::{self, AxisLocationEntry, VirtualMasterEntry};
    use fontcraft_model::plist::{Dict, Str, Value};
    use fontcraft_model::{Axis, Instance, Params, StyleClass};

    pub(crate) fn font(axes: &[(&str, &str)], masters: &[(&str, &[f64])]) -> Font {
        let mut f = Font::new("Interp");
        f.axes = Arc::new(axes.iter().map(|(n, t)| Axis { name: (*n).into(), tag: (*t).into(), ..Default::default() }).collect());
        let template = f.masters[0].clone();
        f.masters = Arc::new(
            masters
                .iter()
                .enumerate()
                .map(|(k, (name, loc))| Master { id: format!("m{k}"), name: (*name).into(), location: loc.to_vec(), ..template.clone() })
                .collect(),
        );
        f
    }

    pub(crate) fn edit_params(f: &mut Font, edit: impl FnOnce(&Params) -> Params) {
        let info = Arc::make_mut(&mut f.info);
        info.params = edit(&info.params);
    }

    fn set_origin(f: &mut Font, id: &str) {
        Arc::make_mut(&mut f.info).params.set(ORIGIN, Value::String(Str::new(id)));
    }

    fn range(a: &AxisInfo) -> [(f64, f64, f64); 2] {
        [(a.internal.minimum, a.internal.default, a.internal.maximum), (a.external.minimum, a.external.default, a.external.maximum)]
    }

    fn assert_close(got: &[(f64, f64)], want: &[(f64, f64)]) {
        assert_eq!(got.len(), want.len(), "{got:?} against {want:?}");
        for (g, w) in got.iter().zip(want) {
            assert!((g.0 - w.0).abs() < 1e-9 && (g.1 - w.1).abs() < 1e-9, "{got:?} against {want:?}");
        }
    }

    fn named(name: &str, weight: f64, class: StyleClass) -> Instance {
        Instance { name: name.into(), location: vec![weight], weight_class: Some(class), ..Default::default() }
    }

    #[test]
    fn the_instances_map_the_axis_and_the_first_master_is_the_origin() {
        // WghtVar_Avar: masters at 300, 400, 700; the Medium instance (500) sits at 600.
        let mut f = font(&[("Weight", "wght")], &[("Light", &[300.0]), ("Regular", &[400.0]), ("Bold", &[700.0])]);
        let class = |n: &str| StyleClass::Name(n.into());
        f.instances = Arc::new(vec![
            named("Light", 300.0, class("Light")),
            named("Regular", 400.0, class("Regular")),
            named("Medium", 600.0, class("Medium")),
            named("Bold", 700.0, class("Bold")),
        ]);
        let ds = Designspace::new(&f).unwrap();
        assert_eq!(ds.origin().unwrap().name, "Light", "the handbook's origin: the first master");
        let wght = &ds.axes()[0];
        assert_eq!(range(wght), [(300.0, 300.0, 700.0), (300.0, 300.0, 700.0)]);
        assert_close(&ds.avar_map(0).unwrap(), &[(-1.0, -1.0), (0.0, 0.0), (0.25, 0.25), (0.5, 0.75), (1.0, 1.0)]);
        assert_eq!(ds.to_normalized(&Location(vec![700.0])).unwrap(), NormLocation(vec![1.0]));

        // With Regular named as the origin: the oracles' fvar and avar.
        set_origin(&mut f, "m1");
        let ds = Designspace::new(&f).unwrap();
        assert_eq!(range(&ds.axes()[0]), [(300.0, 400.0, 700.0), (300.0, 400.0, 700.0)]);
        assert_close(&ds.avar_map(0).unwrap(), &[(-1.0, -1.0), (0.0, 0.0), (1.0 / 3.0, 2.0 / 3.0), (1.0, 1.0)]);
        let medium = ds.instance_location(&f, 2).unwrap();
        assert_eq!(medium, Location(vec![600.0]));
        assert_eq!(ds.to_external(&medium).unwrap(), UserLocation(vec![500.0]));
        assert_eq!(ds.to_internal(&UserLocation(vec![500.0])).unwrap(), medium);
        let n = ds.to_normalized(&medium).unwrap().0[0];
        assert!((n - 2.0 / 3.0).abs() < 1e-12);
    }

    #[test]
    fn axis_location_gives_the_external_coordinates() {
        // WghtVar_AxisLocation: masters at 0, 8 and 10 exposed as 400, 500 and 700; the SemiBold
        // instance at 8.5 exposed as 600.
        let mut f = font(&[("Weight", "wght")], &[("Regular", &[0.0]), ("Medium", &[8.0]), ("Bold", &[10.0])]);
        for (m, user) in Arc::make_mut(&mut f.masters).iter_mut().zip([400.0, 500.0, 700.0]) {
            m.params = params::set_axis_location(&m.params, &[AxisLocationEntry { axis: "Weight".into(), location: user }]).unwrap();
        }
        let mut semibold = named("SemiBold", 8.5, StyleClass::Number(600.0));
        semibold.params = params::set_axis_location(&semibold.params, &[AxisLocationEntry { axis: "Weight".into(), location: 600.0 }]).unwrap();
        f.instances = Arc::new(vec![named("SemiMedium", 4.0, StyleClass::Number(450.0)), semibold]);
        let ds = Designspace::new(&f).unwrap();
        assert_eq!(range(&ds.axes()[0]), [(0.0, 0.0, 10.0), (400.0, 400.0, 700.0)]);
        assert_close(&ds.avar_map(0).unwrap(), &[(-1.0, -1.0), (0.0, 0.0), (1.0 / 3.0, 0.8), (2.0 / 3.0, 0.85), (1.0, 1.0)]);
        assert_eq!(crate::mapping::convert(&ds, 0, 4.0, Space::Internal).unwrap(), 450.0);
        assert_eq!(crate::mapping::convert(&ds, 0, 600.0, Space::External).unwrap(), 8.5);
        assert!(crate::mapping::convert(&ds, 1, 600.0, Space::External).is_err());
        assert!(crate::mapping::convert(&ds, 0, f64::NAN, Space::External).is_err());
    }

    #[test]
    fn explicit_axis_mappings_reach_beyond_the_masters() {
        // AxisMappingsBeyondMasters, as fontmake builds it: the axis spans the whole mapping.
        let mut f = font(&[("Weight", "wght")], &[("Regular", &[80.0]), ("Bold", &[200.0])]);
        edit_params(&mut f, |p| params::set_axis_mappings(p, "wght", &[(300.0, 30.0), (400.0, 80.0), (700.0, 200.0), (900.0, 250.0)]).unwrap());
        let inst = |name: &str, w: f64| Instance { name: name.into(), location: vec![w], ..Default::default() };
        f.instances = Arc::new(vec![inst("Light", 30.0), inst("Black", 250.0), inst("Beyond", 260.0)]);
        let ds = Designspace::new(&f).unwrap();
        assert_eq!(range(&ds.axes()[0]), [(30.0, 80.0, 250.0), (300.0, 400.0, 900.0)]);
        assert_close(&ds.avar_map(0).unwrap(), &[(-1.0, -1.0), (0.0, 0.0), (0.6, 120.0 / 170.0), (1.0, 1.0)]);
        let bold = &ds.masters()[1].location;
        assert!((ds.to_normalized(bold).unwrap().0[0] - 120.0 / 170.0).abs() < 1e-12);
        assert_eq!(ds.classify(&Location(vec![250.0])), Placement::Inside);
        assert_eq!(ds.classify(&Location(vec![260.0])), Placement::Outside);
        assert_eq!(ds.classify(&Location(vec![250.0, 1.0])), Placement::Outside, "wrong length");
        assert_eq!(ds.named_instances(&f), vec![0, 1], "an extrapolated instance is not named");
        // Extrapolation continues the slope of the nearest side.
        assert!((ds.to_normalized(&Location(vec![-120.0])).unwrap().0[0] + 4.0).abs() < 1e-12);
        assert!(ds.to_normalized(&Location(vec![f64::INFINITY])).is_err());
        assert!(ds.to_normalized(&Location(vec![])).is_err());
    }

    #[test]
    fn the_origin_parameter_names_a_master() {
        // CustomOrigin.
        let mut f = font(&[("Weight", "wght")], &[("Master 1", &[100.0]), ("Master 2", &[400.0])]);
        assert_eq!(range(&Designspace::new(&f).unwrap().axes()[0])[1], (100.0, 100.0, 400.0));
        set_origin(&mut f, "m1");
        let ds = Designspace::new(&f).unwrap();
        assert_eq!(ds.origin().unwrap().id, "m1");
        assert_eq!(range(&ds.axes()[0])[1], (100.0, 400.0, 400.0));
        assert_eq!(ds.avar_map(0), None, "an identity map needs no avar");
        set_origin(&mut f, "gone");
        assert!(matches!(Designspace::new(&f), Err(InterpError::BadMasterSetup(_))));
        // The older parameter names the master by name.
        Arc::make_mut(&mut f.info).params.remove(ORIGIN);
        Arc::make_mut(&mut f.info).params.set(ORIGIN_BY_NAME, Value::String(Str::new("Master 2")));
        assert_eq!(Designspace::new(&f).unwrap().origin().unwrap().id, "m1");
    }

    fn intermediate_layer_font() -> Font {
        let mut f = font(&[("Cap Height", "CPHT"), ("Weight", "wght")], &[("Regular", &[700.0, 400.0]), ("Black", &[700.0, 900.0])]);
        let vm = |cap: f64| VirtualMasterEntry { coordinates: vec![("Cap Height".into(), cap), ("Weight".into(), 400.0)] };
        edit_params(&mut f, |p| params::set_virtual_masters(p, &[vm(600.0), vm(800.0)]).unwrap());
        f
    }

    #[test]
    fn virtual_masters_extend_an_axis() {
        // IntermediateLayer: both masters at Cap Height 700, virtual masters at 600 and 800.
        let f = intermediate_layer_font();
        let ds = Designspace::new(&f).unwrap();
        assert_eq!(range(&ds.axes()[0]), [(600.0, 700.0, 800.0), (600.0, 700.0, 800.0)]);
        assert_eq!(range(&ds.axes()[1]), [(400.0, 400.0, 900.0), (400.0, 400.0, 900.0)]);
        assert_eq!((ds.avar_map(0), ds.avar_map(1)), (None, None));
        let locations: Vec<_> = ds.virtual_masters().iter().map(|v| v.location.0.clone()).collect();
        assert_eq!(locations, [[600.0, 400.0], [800.0, 400.0]]);
        assert_eq!(ds.check_variable_origin(), Ok(()));
    }

    #[test]
    fn disable_masters_drops_an_unused_axis() {
        let mut f =
            font(&[("Weight", "wght"), ("Width", "wdth")], &[("Thin", &[200.0, 100.0]), ("Black", &[900.0, 100.0]), ("Thin Narrow", &[200.0, 60.0])]);
        let setting = |names: &str| {
            let mut s = Instance { name: "VF".into(), kind: Some("variable".into()), ..Default::default() };
            s.params.set(params::DISABLE_MASTERS, Value::String(Str::new(names)));
            s
        };
        let fixed =
            Instance { name: "Medium".into(), location: vec![500.0, 100.0], weight_class: Some(StyleClass::Number(500.0)), ..Default::default() };
        f.instances = Arc::new(vec![setting("Thin Narrow"), fixed, setting("Thin, Black, Thin Narrow"), setting("Thin Narrow, Light")]);
        let ds = Designspace::for_setting(&f, 0).unwrap();
        assert_eq!(ds.setting(), Some(0));
        assert_eq!(ds.axes().iter().map(|a| a.tag.as_str()).collect::<Vec<_>>(), ["wght"]);
        assert_eq!(ds.axes()[0].font_axis, 0);
        assert_eq!(ds.masters().iter().map(|m| (m.index, m.location.0.clone())).collect::<Vec<_>>(), [(0, vec![200.0]), (1, vec![900.0])]);
        assert_eq!(ds.instance_location(&f, 1).unwrap(), Location(vec![500.0]));
        assert!(ds.warnings().is_empty());
        // The whole font keeps both axes.
        assert_eq!(Designspace::new(&f).unwrap().axes().len(), 2);
        assert_eq!(Designspace::for_setting(&f, 1), Err(InterpError::NotAVariableSetting(1)));
        assert_eq!(Designspace::for_setting(&f, 9), Err(InterpError::UnknownInstance(9)));
        assert!(matches!(Designspace::for_setting(&f, 2), Err(InterpError::BadMasterSetup(_))));
        assert_eq!(Designspace::for_setting(&f, 3).unwrap().warnings().len(), 1, "Light is not a master");

        // Virtual masters keep their axis.
        let mut g = intermediate_layer_font();
        g.instances = Arc::new(vec![setting("Black")]);
        let ds = Designspace::for_setting(&g, 0).unwrap();
        assert_eq!(ds.axes().iter().map(|a| a.tag.as_str()).collect::<Vec<_>>(), ["CPHT"]);
        assert_eq!(ds.virtual_masters().iter().map(|v| v.location.0.clone()).collect::<Vec<_>>(), [[600.0], [800.0]]);
    }

    #[test]
    fn a_setting_axis_mappings_replace_the_font_ones() {
        let mut f = font(&[("Weight", "wght")], &[("Regular", &[80.0]), ("Bold", &[200.0])]);
        edit_params(&mut f, |p| params::set_axis_mappings(p, "wght", &[(400.0, 80.0), (700.0, 200.0)]).unwrap());
        let mut s = Instance { name: "VF".into(), kind: Some("variable".into()), ..Default::default() };
        s.params = params::set_axis_mappings(&s.params, "wght", &[(300.0, 80.0), (900.0, 200.0)]).unwrap();
        f.instances = Arc::new(vec![s]);
        assert_eq!(range(&Designspace::new(&f).unwrap().axes()[0])[1], (400.0, 400.0, 700.0));
        assert_eq!(range(&Designspace::for_setting(&f, 0).unwrap().axes()[0])[1], (300.0, 300.0, 900.0));
    }

    #[test]
    fn the_minimal_setup_is_checked_for_the_variable_origin() {
        // The handbook's example: Thin can be the origin, Black cannot.
        let mut f =
            font(&[("Weight", "wght"), ("Width", "wdth")], &[("Thin Narrow", &[200.0, 60.0]), ("Thin", &[200.0, 100.0]), ("Black", &[900.0, 100.0])]);
        set_origin(&mut f, "m1");
        assert_eq!(Designspace::new(&f).unwrap().check_variable_origin(), Ok(()));
        set_origin(&mut f, "m2");
        let refused = Designspace::new(&f).unwrap().check_variable_origin();
        assert!(matches!(&refused, Err(InterpError::BadMasterSetup(m)) if m.contains("Width") && m.contains("Black")), "{refused:?}");
        Arc::make_mut(&mut f.info).params.remove(ORIGIN);
        let refused = Designspace::new(&f).unwrap().check_variable_origin();
        assert!(matches!(&refused, Err(InterpError::BadMasterSetup(m)) if m.contains("Weight")), "{refused:?}");
    }

    #[test]
    fn a_falling_map_normalizes_with_the_external_axis() {
        let mut f = font(&[("Weight", "wght")], &[("Light", &[100.0]), ("Bold", &[50.0])]);
        let mut inner = Dict::new();
        inner.insert("300", Value::Number(fontcraft_model::plist::Number::int(100)));
        inner.insert("700", Value::Number(fontcraft_model::plist::Number::int(50)));
        let mut outer = Dict::new();
        outer.insert("wght", Value::Dict(inner));
        Arc::make_mut(&mut f.info).params.set(params::AXIS_MAPPINGS, Value::Dict(outer));
        let ds = Designspace::new(&f).unwrap();
        assert_eq!(range(&ds.axes()[0]), [(100.0, 100.0, 50.0), (300.0, 300.0, 700.0)]);
        assert_eq!(ds.to_normalized(&Location(vec![75.0])).unwrap(), NormLocation(vec![0.5]));
        assert_eq!(ds.classify(&Location(vec![60.0])), Placement::Inside);
        assert_eq!(ds.classify(&Location(vec![40.0])), Placement::Outside);
        assert_eq!(ds.avar_map(0), None);
    }

    #[test]
    fn an_oversized_axis_mapping_is_refused_before_it_is_read() {
        // Half a million points: reading them into a map first would take pairwise work.
        let mut f = font(&[("Weight", "wght")], &[("Light", &[100.0]), ("Bold", &[900.0])]);
        let entries = (0..500_000i64).map(|k| (Str::new(k.to_string()), Value::Number(fontcraft_model::plist::Number::int(k)))).collect();
        let mut outer = Dict::new();
        outer.insert("wght", Value::Dict(Dict { entries, ..Default::default() }));
        Arc::make_mut(&mut f.info).params.set(params::AXIS_MAPPINGS, Value::Dict(outer));
        assert_eq!(Designspace::new(&f), Err(InterpError::LimitExceeded { what: "points in an axis mapping", limit: MAX_MAPPING_POINTS }));
    }

    #[test]
    fn a_map_that_turns_back_is_ignored_with_a_warning() {
        let mut f = font(&[("Weight", "wght")], &[("A", &[0.0]), ("B", &[20.0]), ("C", &[10.0])]);
        for (m, user) in Arc::make_mut(&mut f.masters).iter_mut().zip([300.0, 400.0, 500.0]) {
            m.params = params::set_axis_location(&m.params, &[AxisLocationEntry { axis: "Weight".into(), location: user }]).unwrap();
        }
        let ds = Designspace::new(&f).unwrap();
        assert_eq!(ds.warnings().len(), 1);
        assert_eq!(range(&ds.axes()[0]), [(0.0, 0.0, 20.0), (0.0, 0.0, 20.0)]);
    }

    #[test]
    fn tags_and_coordinates_are_checked() {
        assert_eq!(export_tag("wght").unwrap(), "wght");
        assert_eq!(export_tag("ab").unwrap(), "ab  ");
        assert_eq!(export_tag("wghtx").unwrap(), "wght");
        for bad in ["", " abc", "w\u{e9}ht", "a\tb"] {
            assert_eq!(export_tag(bad), Err(InterpError::BadAxisTag(bad.into())));
        }
        let f = font(&[("Weight", "wght"), ("Weight 2", "wght")], &[("A", &[1.0, 2.0])]);
        assert_eq!(Designspace::new(&f), Err(InterpError::DuplicateAxisTag("wght".into())));
        let f = font(&[("Weight", "wght")], &[("A", &[f64::NAN])]);
        assert!(matches!(Designspace::new(&f), Err(InterpError::BadNumber { .. })));
        let f = font(&[("Weight", "wght")], &[("A", &[2.0 * MAX_COORDINATE])]);
        assert!(matches!(Designspace::new(&f), Err(InterpError::BadNumber { .. })));
        let mut f = font(&[], &[("A", &[])]);
        f.axes = Arc::new((0..=MAX_AXES).map(|i| Axis { name: format!("A{i}"), tag: format!("A{i:03}"), ..Default::default() }).collect());
        assert!(matches!(Designspace::new(&f), Err(InterpError::LimitExceeded { .. })));
    }
}
