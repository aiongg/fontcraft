//! Variable kerning and mark positions: per-master values as a default plus deltas (M9.9).
//!
//! Rows: `compile.instanceLayoutInterpolation` (variable half), `masters.metricsInterpolate`
//! (kerning and anchors in variable fonts).
//!
//! Contract (`docs/plans/M9.md` § 3 › Design decisions):
//! - [`VarLayout`] holds the variable font's axes and masters (the origin first) and their
//!   variation model. [`VarLayout::resolve`] turns one value per master into the origin's value
//!   plus one rounded delta per region, as fontTools' `getDeltas(round=otRound)` does.
//! - The writers keep the static writers' lookups, features and limits; only the values gain
//!   deltas. Kerning: [`crate::KernRequest::variable`] (a pair a master lacks takes the value its
//!   groups give it there, else 0, as fontc does). Marks: [`crate::implicit::from_master_marks`]
//!   zips the mark writer's output for each master.
//! - The deltas travel as write-fonts device deltas inside the kern writer and as
//!   [`fontcraft_fea::Metric`] deltas in the neutral form; fea-rs builds them into the `GDEF`
//!   variation store and replaces the pending variation indices once the store is built.

use fontcraft_geom::variation::{Region, VariationModel};
use write_fonts::tables::layout::builders::DeviceOrDeltas;
use write_fonts::tables::variations::{RegionAxisCoordinates, VariationRegion};
use write_fonts::types::{F2Dot14, Tag};

use crate::FeatureError;

/// Most values times masters one [`VarLayout::resolve`] call works on.
pub const MAX_VARIABLE_CELLS: usize = 16_000_000;

/// A value that varies: the origin's value and one delta per region, in font units.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VarValue {
    pub default: i32,
    pub deltas: Vec<(Region, i32)>,
}

/// A master of the variable font and its normalized location (one value per axis, in `fvar`
/// order).
#[derive(Clone, Debug, PartialEq)]
pub struct VarMaster {
    pub id: String,
    pub location: Vec<f64>,
}

/// The variable font's axes and masters, with the model that turns per-master values into
/// deltas.
#[derive(Clone, Debug)]
pub struct VarLayout {
    axes: Vec<Tag>,
    masters: Vec<VarMaster>,
    model: VariationModel,
}

impl VarLayout {
    /// `masters` with the origin first (at all zeros). A master at the location of an earlier one
    /// is left out: the earlier one gives the value there. Refuses locations that are not finite,
    /// lie outside -1 to 1 or do not have one value per axis, and more masters than the model
    /// takes.
    pub fn new(axes: Vec<Tag>, masters: Vec<VarMaster>) -> Result<Self, FeatureError> {
        let bad = |m: &str| FeatureError::Unsupported(format!("the variable layout cannot be built: {m}"));
        if axes.is_empty() || axes.len() > fontcraft_geom::variation::MAX_AXES {
            return Err(bad("the font has no axes or too many"));
        }
        if masters.len() > fontcraft_geom::variation::MAX_MASTERS {
            return Err(FeatureError::TooMany { what: "masters", limit: fontcraft_geom::variation::MAX_MASTERS });
        }
        if !masters.first().is_some_and(|m| m.location.iter().all(|v| *v == 0.0)) {
            return Err(bad("the origin is not the first master"));
        }
        let mut kept: Vec<VarMaster> = Vec::with_capacity(masters.len());
        for m in masters {
            if m.location.len() != axes.len() || m.location.iter().any(|v| !v.is_finite() || !(-1.0..=1.0).contains(v)) {
                return Err(bad("a master's location is outside the axes"));
            }
            if !kept.iter().any(|k| k.location == m.location) {
                kept.push(m);
            }
        }
        let locations: Vec<Vec<f64>> = kept.iter().map(|m| m.location.clone()).collect();
        let model = VariationModel::new(&locations, false).map_err(|e| bad(&e.to_string()))?;
        Ok(Self { axes, masters: kept, model })
    }

    /// The layout of the masters at `indices` (into [`Self::masters`], the origin's 0 first), for
    /// values some masters do not have.
    pub fn subset(&self, indices: &[usize]) -> Result<Self, FeatureError> {
        let masters = indices.iter().filter_map(|&i| self.masters.get(i).cloned()).collect();
        Self::new(self.axes.clone(), masters)
    }

    /// The axis tags, in `fvar` order.
    pub fn axes(&self) -> &[Tag] {
        &self.axes
    }

    /// The masters values are given for, the origin first.
    pub fn masters(&self) -> &[VarMaster] {
        &self.masters
    }

    /// `values[m][i]`, value `i` at master `m` (in [`Self::masters`] order), as one [`VarValue`]
    /// per `i`: each master's value rounded half up, then the deltas rounded the same way.
    pub fn resolve(&self, values: &[Vec<f64>]) -> Result<Vec<VarValue>, FeatureError> {
        let len = values.first().map_or(0, Vec::len);
        if len.saturating_mul(values.len()) > MAX_VARIABLE_CELLS {
            return Err(FeatureError::TooMany { what: "variable values", limit: MAX_VARIABLE_CELLS });
        }
        let round = |v: f64| if v.is_finite() { (v + 0.5).floor() } else { 0.0 };
        let rounded: Vec<Vec<f64>> = values.iter().map(|m| m.iter().map(|v| round(*v)).collect()).collect();
        let deltas = self.model.deltas_with(&rounded, round).map_err(|e| FeatureError::Unsupported(e.to_string()))?;
        let regions = self.model.regions();
        let mut out: Vec<VarValue> =
            rounded.first().map(|d| d.iter().map(|v| VarValue { default: *v as i32, deltas: Vec::new() }).collect()).unwrap_or_default();
        for (region, column) in regions.iter().zip(&deltas).skip(1) {
            for (value, d) in out.iter_mut().zip(column) {
                if *d != 0.0 {
                    value.deltas.push((region.clone(), *d as i32));
                }
            }
        }
        Ok(out)
    }

    /// `v`'s deltas as write-fonts device deltas over the font's axes, each clamped to 16 bits;
    /// `None` when it has none.
    pub fn device(&self, v: &VarValue) -> Option<DeviceOrDeltas> {
        if v.deltas.is_empty() {
            return None;
        }
        let f2 = |x: f64| F2Dot14::from_f32(x as f32);
        let deltas = v
            .deltas
            .iter()
            .map(|(region, d)| {
                let axes = region.0.iter().map(|a| RegionAxisCoordinates::new(f2(a.start), f2(a.peak), f2(a.end))).collect();
                (VariationRegion::new(axes), clamp16(*d))
            })
            .collect();
        Some(DeviceOrDeltas::Deltas(deltas))
    }

    /// `v` in the compiler's neutral form, each number clamped to 16 bits.
    pub fn metric(&self, v: &VarValue) -> fontcraft_fea::Metric {
        let deltas = v.deltas.iter().map(|(region, d)| (neutral_region(&self.axes, region), clamp16(*d))).collect();
        fontcraft_fea::Metric { default: clamp16(v.default), deltas }
    }
}

fn clamp16(v: i32) -> i16 {
    v.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

/// `region` (one tent per axis of `axes`) with its axes named, unconstrained axes left out.
fn neutral_region(axes: &[Tag], region: &Region) -> fontcraft_fea::Region {
    fontcraft_fea::Region(
        axes.iter()
            .zip(&region.0)
            .filter(|(_, a)| a.peak != 0.0)
            .map(|(tag, a)| fontcraft_fea::RegionAxis { axis: *tag, start: a.start, peak: a.peak, end: a.end })
            .collect(),
    )
}

/// The write-fonts deltas of `device` in the neutral form, with `axes` naming the region axes.
pub(crate) fn neutral_deltas(axes: &[Tag], device: &DeviceOrDeltas) -> Vec<(fontcraft_fea::Region, i16)> {
    let DeviceOrDeltas::Deltas(deltas) = device else { return Vec::new() };
    deltas
        .iter()
        .map(|(region, d)| {
            let axes = axes
                .iter()
                .zip(&region.region_axes)
                .filter(|(_, a)| a.peak_coord != F2Dot14::ZERO)
                .map(|(tag, a)| fontcraft_fea::RegionAxis {
                    axis: *tag,
                    start: f64::from(a.start_coord.to_f32()),
                    peak: f64::from(a.peak_coord.to_f32()),
                    end: f64::from(a.end_coord.to_f32()),
                })
                .collect();
            (fontcraft_fea::Region(axes), *d)
        })
        .collect()
}
