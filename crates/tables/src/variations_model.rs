//! Variation models: HVAR from phantom points, model description, plots and `avar` planning
//! (M12.34; rows `variations build-hvar`, `model`, `plot`, `plan-avar` and their commands).
//! Oracles: fontTools `varLib.hvar`, `varLib.models`, `varLib.plot`, `varLib.avar.plan`.
//!
//! Ported from fontTools 4.66 (MIT, Copyright (c) 2017 Just van Rossum, Behdad Esfahbod and the
//! fontTools authors; see `NOTICE`):
//!
//! - [`build_hvar`] follows `varLib.hvar`: each glyph's advance deltas are read straight from
//!   the left and right phantom points of its `gvar` tuples, one delta per tuple region, and
//!   stored through the compiler's own `HVAR` builder ([`fontcraft_compile::var::hvar`]).
//! - [`describe_model`] follows `varLib.models.main`: the master order and supports of
//!   `VariationModel`, computed by [`fontcraft_geom::variation::VariationModel`].
//! - [`plot`] draws what `varLib.plot` draws, as SVG instead of a window.
//! - [`plan_avar`] follows `varLib.avar.plan`: measurements of the font at sampled locations
//!   (`measureWeight`, `measureWidth`, `measureSlant`) and the planner's interpolation.

mod designspace;
mod glyphset;
mod hvar;
mod plan;
mod plot;
#[cfg(test)]
mod tests;

use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

pub use designspace::{DesignspaceLocations, designspace_locations};
pub use hvar::{HvarBuild, HvarOptions, TableAction, build_hvar_with};
pub use plan::{AxisPlan, AxisPlanResult, DEFAULT_SAMPLES, MAX_PLAN_VALUES, MAX_SAMPLES, PlanOptions, PlanResult, plan_avar_with};
pub use plot::{PlotPanel, font_panels, plan_plot, plot_font, plot_panels, plot_with};

/// Most master locations one model takes ([`fontcraft_geom::variation::MAX_MASTERS`]).
pub const MAX_MODEL_MASTERS: usize = fontcraft_geom::variation::MAX_MASTERS;

/// The font with an `HVAR` table built from the `gvar` phantom points, replacing an old one, and
/// a `VVAR` table too when the font has `vmtx`.
pub fn build_hvar(font: &SfntFont) -> Result<SfntFont> {
    Ok(build_hvar_with(font, &HvarOptions { vvar: true, replace: true })?.font)
}

/// One master's region in a variation model.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelSupport {
    /// The master location, normalised, by axis tag; axes at 0 are left out.
    pub location: Vec<(Tag, f64)>,
    /// Per axis, `(start, peak, end)`; only the axes the master is away from 0 on.
    pub region: Vec<(Tag, [f64; 3])>,
    /// The master's index in the caller's list.
    pub master: usize,
}

/// The supports of the model for `locations` (normalised), in fontTools' master order.
///
/// Axes are numbered in the order they first appear in `locations`; that order breaks ties
/// between masters, as fontTools' sorted axis names do, so callers list the axes in that order.
pub fn describe_model(locations: &[Vec<(Tag, f64)>]) -> Result<Vec<ModelSupport>> {
    use fontcraft_geom::variation::{VariationError, VariationModel};
    if locations.len() > MAX_MODEL_MASTERS {
        return Err(TablesError::TooLarge { what: "the number of master locations", limit: MAX_MODEL_MASTERS });
    }
    let axes = model_axes(locations)?;
    let dense: Vec<Vec<f64>> = locations.iter().map(|loc| dense_location(loc, &axes)).collect();
    let model = VariationModel::new(&dense, false).map_err(|e| match e {
        VariationError::DuplicateLocation => TablesError::BadOptions("the master locations must be unique".into()),
        VariationError::NoDefault => TablesError::BadOptions("no master is at the default location (all zeros)".into()),
        VariationError::Size => TablesError::BadOptions("give between 1 and 1,024 master locations".into()),
        other => TablesError::BadOptions(format!("the master locations: {other}")),
    })?;
    let mut out = Vec::with_capacity(dense.len());
    for region in model.regions() {
        let peaks: Vec<f64> = region.0.iter().map(|a| a.peak).collect();
        let master = dense
            .iter()
            .position(|l| l.len() == peaks.len() && l.iter().zip(&peaks).all(|(a, b)| a == b))
            .ok_or_else(|| TablesError::invalid("a support of the model matches no master"))?;
        let location = axes.iter().zip(dense.get(master).into_iter().flatten()).filter(|(_, v)| **v != 0.0).map(|(t, v)| (*t, *v)).collect();
        let region = axes.iter().zip(&region.0).filter(|(_, a)| a.peak != 0.0).map(|(t, a)| (*t, [a.start, a.peak, a.end])).collect();
        out.push(ModelSupport { location, region, master });
    }
    Ok(out)
}

/// The axes of `locations` in the order they first appear, checked: finite values, no axis twice
/// in one location, at most [`crate::limits::MAX_AXES`] axes.
fn model_axes(locations: &[Vec<(Tag, f64)>]) -> Result<Vec<Tag>> {
    let mut axes: Vec<Tag> = Vec::new();
    for loc in locations {
        for (i, (tag, v)) in loc.iter().enumerate() {
            if !v.is_finite() {
                return Err(TablesError::BadOptions(format!(
                    "the `{}` value of a location is not a finite number",
                    crate::sfnt::tag_str(*tag).trim_end()
                )));
            }
            if loc.iter().take(i).any(|(t, _)| t == tag) {
                return Err(TablesError::BadOptions(format!("a location gives the `{}` axis twice", crate::sfnt::tag_str(*tag).trim_end())));
            }
            if !axes.contains(tag) {
                if axes.len() >= crate::limits::MAX_AXES {
                    return Err(TablesError::TooLarge { what: "the number of axes", limit: crate::limits::MAX_AXES });
                }
                axes.push(*tag);
            }
        }
    }
    Ok(axes)
}

/// `loc` with one value per axis of `axes`, 0 where it gives none (`-0.0` read as 0).
fn dense_location(loc: &[(Tag, f64)], axes: &[Tag]) -> Vec<f64> {
    axes.iter().map(|a| loc.iter().find(|(t, _)| t == a).map_or(0.0, |(_, v)| if *v == 0.0 { 0.0 } else { *v })).collect()
}

/// An SVG plot of the model's supports.
pub fn plot(locations: &[Vec<(Tag, f64)>]) -> Result<String> {
    plot_with(locations, &[], None)
}

/// One axis's `avar` segment map: the axis tag and its `(from, to)` points.
pub type AvarSegments = (Tag, Vec<(f64, f64)>);

/// The `avar` segment maps fontTools' planner proposes for the weight, width, slant and optical
/// size axes of `font`, per axis tag.
pub fn plan_avar(font: &SfntFont) -> Result<Vec<AvarSegments>> {
    let plan = plan_avar_with(font, &PlanOptions::default())?;
    Ok(plan.axes.into_iter().filter(|a| !a.normalized.is_empty()).map(|a| (a.tag, a.normalized)).collect())
}

/// fontTools' `models.supportScalar` without axis ranges: how much a delta with `support` counts
/// at `location`. An axis whose peak is 0, or whose tent is malformed or crosses 0, does not take
/// part.
pub(crate) fn support_scalar(location: &[(Tag, f64)], support: &[(Tag, [f64; 3])]) -> f64 {
    let mut scalar = 1.0;
    for (axis, [lower, peak, upper]) in support {
        let (lower, peak, upper) = (*lower, *peak, *upper);
        if peak == 0.0 || lower > peak || peak > upper || (lower < 0.0 && upper > 0.0) {
            continue;
        }
        let v = location.iter().find(|(t, _)| t == axis).map_or(0.0, |(_, v)| *v);
        if v == peak {
            continue;
        }
        if v <= lower || upper <= v {
            return 0.0;
        }
        scalar *= if v < peak { (v - lower) / (peak - lower) } else { (v - upper) / (peak - upper) };
    }
    scalar
}

/// fontTools' `models.piecewiseLinearMap`: `v` through the `(from, to)` points, shifted past
/// the ends; `v` itself when there are none.
pub(crate) fn piecewise_linear_map(v: f64, mapping: &[(f64, f64)]) -> f64 {
    if let Some((_, to)) = mapping.iter().find(|(k, _)| *k == v) {
        return *to;
    }
    let Some(lowest) = mapping.iter().min_by(|a, b| a.0.total_cmp(&b.0)) else { return v };
    if v < lowest.0 {
        return v + lowest.1 - lowest.0;
    }
    let Some(highest) = mapping.iter().max_by(|a, b| a.0.total_cmp(&b.0)) else { return v };
    if v > highest.0 {
        return v + highest.1 - highest.0;
    }
    let below = mapping.iter().filter(|(k, _)| *k < v).max_by(|a, b| a.0.total_cmp(&b.0));
    let above = mapping.iter().filter(|(k, _)| *k > v).min_by(|a, b| a.0.total_cmp(&b.0));
    match (below, above) {
        (Some(&(a, va)), Some(&(b, vb))) if b != a => va + (vb - va) * (v - a) / (b - a),
        _ => v,
    }
}

/// C's `%g` (six significant digits), as fontTools prints numbers.
pub(crate) fn fmt_g(v: f64) -> String {
    if v == 0.0 {
        return "0".into();
    }
    fontcraft_decompile::type1::fmt_g(v, 6)
}
