//! The instancer (M12.20, M12.21; rows `instance`, `font.instantiate`). A port of fontTools'
//! `varLib.instancer` (MIT, Copyright (c) 2019 Google LLC and the fontTools authors): full
//! instances ([`full`]), partial pinning and axis ranges ([`partial`]) with its region solver
//! ([`solver`]). `CFF2` outlines, variable `BASE` tables and names from `STAT` (M12.20b) are
//! the private modules `cff2`, `base` and `names`. Instances from sources (M12.22) are the
//! engine's job.
//!
//! # API for the packages that build on this one
//!
//! - [`instantiate`] is the entry point: every `fvar` axis pinned (by [`AxisLimit::Pin`],
//!   [`AxisLimit::Drop`] or a range whose ends meet) gives a static font through
//!   [`full::full_instance_with`]; anything else goes to [`partial::partial_instance`].
//! - [`pin_location`] turns a user location into every axis's clamped user value and its
//!   normalised value (through `avar`, quantised to F2Dot14), as fontTools does before it
//!   instances.
//! - [`axis_scalar`] and [`tuple_scalar`] give how much a region contributes at a location, with
//!   the instancer's rule that a malformed region contributes nothing.
//! - [`iup::iup_delta`] infers the deltas of points a `gvar` tuple leaves out.
//! - [`ivs::StoreDeltas`] evaluates an item variation store at a location (used for `MVAR`,
//!   `GDEF`/`GPOS` and `HVAR`).
//! - [`named_instances`] lists the `fvar` named instances with their names.

mod base;
pub(crate) mod cff2;
pub mod full;
mod glyf;
pub mod iup;
pub mod ivs;
mod names;
mod otl;
pub mod partial;
pub mod solver;
mod static_tables;
#[cfg(test)]
mod tests;

use read_fonts::tables::fvar::Fvar;
use read_fonts::tables::name::Name;
use read_fonts::{FontData, FontRead};

use crate::limits::MAX_AXES;
use crate::sfnt::{SfntFont, Tag, tag_str};
use crate::variations::{self, FvarAxis, Space};
use crate::{Result, TablesError};

/// What happens to one axis, in user coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AxisLimit {
    /// Pin the axis at this value and drop it.
    Pin(f64),
    /// Limit the axis to a range; `default` moves the default when given.
    Range { min: f64, default: Option<f64>, max: f64 },
    /// Pin the axis at its default and drop it.
    Drop,
}

/// What [`instantiate`] does. Axes not named keep their full range.
#[derive(Clone, Debug, PartialEq)]
pub struct InstanceOptions {
    /// At most [`crate::limits::MAX_AXES`] entries; values finite.
    pub limits: Vec<(Tag, AxisLimit)>,
    /// Rewrite the style names (IDs 1, 2, 3, 4, 6, 16, 17) from the `STAT` axis values at the
    /// new default location, and the style bits of `head` and `OS/2` from them; refused when
    /// `STAT` lacks a value there.
    pub update_names: bool,
    /// Set the overlap flags on simple glyphs of a static instance.
    pub overlap_flags: bool,
    /// Write `CFF ` instead of `CFF2` when nothing in the outlines varies any more (every axis
    /// pinned, or none the outlines use left).
    pub downgrade_cff2: bool,
    /// Recompute the font-wide bounds and extents (`head`, `hhea`, `vhea`) of the result, as
    /// fontTools does when it saves with `recalcBBoxes`. Glyph bounds are always recomputed.
    pub recalc_bounds: bool,
}

impl Default for InstanceOptions {
    fn default() -> Self {
        Self { limits: Vec::new(), update_names: false, overlap_flags: true, downgrade_cff2: false, recalc_bounds: true }
    }
}

/// Reads a `CFF2` table for instancing whose variation store has `axis_count` axes, and drops
/// it; answers its glyph count. For tests that measure memory.
#[cfg(test)]
pub(crate) fn read_cff2_for_test(data: &[u8], axis_count: usize) -> Result<usize> {
    cff2::Table::read(data, axis_count).map(|t| t.glyph_count())
}

/// The instance.
pub fn instantiate(font: &SfntFont, options: &InstanceOptions) -> Result<SfntFont> {
    if options.limits.len() > MAX_AXES {
        return Err(TablesError::TooLarge { what: "the number of axis limits", limit: MAX_AXES });
    }
    let axes = variations::fvar_axes(font)?;
    let mut pinned: Vec<(Tag, f64)> = Vec::with_capacity(axes.len());
    for (tag, limit) in &options.limits {
        let Some(axis) = axes.iter().find(|a| a.tag == *tag) else {
            return Err(TablesError::BadOptions(format!("the font has no `{}` axis", tag_str(*tag).trim_end())));
        };
        if options.limits.iter().filter(|(t, _)| t == tag).count() > 1 {
            return Err(TablesError::BadOptions(format!("the `{}` axis is limited twice", tag_str(*tag).trim_end())));
        }
        let value = match *limit {
            AxisLimit::Pin(v) => Some(v),
            AxisLimit::Drop => Some(axis.default),
            AxisLimit::Range { min, max, .. } if min == max => Some(min),
            AxisLimit::Range { .. } => None,
        };
        if let Some(v) = value {
            if !v.is_finite() {
                return Err(TablesError::BadOptions(format!("the `{}` value is not a finite number", tag_str(*tag).trim_end())));
            }
            pinned.push((*tag, v));
        }
    }
    if axes.iter().all(|a| pinned.iter().any(|(t, _)| *t == a.tag)) {
        full::full_instance_with(font, &pinned, options)
    } else {
        partial::partial_instance_with(font, options)
    }
}

/// A location with every `fvar` axis, in table order.
#[derive(Clone, Debug, PartialEq)]
pub struct PinnedLocation {
    pub axes: Vec<FvarAxis>,
    /// User values clamped to each axis's range; axes not given sit at their default.
    pub user: Vec<(Tag, f64)>,
    /// Normalised values after `avar`, quantised to F2Dot14 (a multiple of 1/16384).
    pub normalized: Vec<f64>,
}

/// The normalised location of `location` (axis tag → user value). Unknown axes, repeated axes,
/// values that are not finite and more than [`MAX_AXES`] entries are refused.
pub fn pin_location(font: &SfntFont, location: &[(Tag, f64)]) -> Result<PinnedLocation> {
    if location.len() > MAX_AXES {
        return Err(TablesError::TooLarge { what: "the number of axes in the location", limit: MAX_AXES });
    }
    let axes = variations::fvar_axes(font)?;
    for (i, (tag, v)) in location.iter().enumerate() {
        let name = tag_str(*tag);
        if !axes.iter().any(|a| a.tag == *tag) {
            return Err(TablesError::BadOptions(format!("the font has no `{}` axis", name.trim_end())));
        }
        if location.iter().take(i).any(|(t, _)| t == tag) {
            return Err(TablesError::BadOptions(format!("the `{}` axis is given twice", name.trim_end())));
        }
        if !v.is_finite() {
            return Err(TablesError::BadOptions(format!("the `{}` value is not a finite number", name.trim_end())));
        }
    }
    let user: Vec<(Tag, f64)> = axes
        .iter()
        .map(|a| {
            let v = location.iter().find(|(t, _)| *t == a.tag).map_or(a.default, |(_, v)| *v);
            (a.tag, v.max(a.minimum).min(a.maximum))
        })
        .collect();
    let mapped = variations::map_coordinates(font, std::slice::from_ref(&user), Space::User)?;
    let mapped = mapped.into_iter().next().ok_or_else(|| TablesError::invalid("the location could not be normalised"))?;
    let normalized = axes.iter().map(|a| mapped.mapped.iter().find(|(t, _)| *t == a.tag).map_or(0.0, |(_, v)| quantize(*v))).collect();
    Ok(PinnedLocation { axes, user, normalized })
}

/// A value rounded to the nearest multiple of 1/16384, halves away from negative infinity as
/// fontTools' `otRound` does.
pub(crate) fn quantize(v: f64) -> f64 {
    let v = v.clamp(-2.0, 2.0);
    (v * 16384.0 + 0.5).floor() / 16384.0
}

/// fontTools' `otRound`: halves round towards positive infinity.
pub(crate) fn ot_round(v: f64) -> f64 {
    (v + 0.5).floor()
}

/// How much a region spanning `lower..peak..upper` on one axis contributes at `v`. A peak of 0
/// means the axis does not take part (1). A malformed region (out of order, or crossing 0)
/// contributes nothing, as the instancer drops such tuples.
pub fn axis_scalar(v: f64, lower: f64, peak: f64, upper: f64) -> f64 {
    if peak == 0.0 {
        return 1.0;
    }
    if lower > peak || peak > upper || (lower < 0.0 && upper > 0.0) {
        return 0.0;
    }
    if v == peak {
        return 1.0;
    }
    if v <= lower || v >= upper {
        return 0.0;
    }
    if v < peak { (v - lower) / (peak - lower) } else { (v - upper) / (peak - upper) }
}

/// The product of [`axis_scalar`] over the axes; each entry of `region` is (start, peak, end)
/// for the axis of the same index in `coords`. Missing axes do not take part.
pub fn tuple_scalar(coords: &[f64], region: impl IntoIterator<Item = (f64, f64, f64)>) -> f64 {
    let mut scalar = 1.0;
    for (i, (lower, peak, upper)) in region.into_iter().enumerate() {
        let v = coords.get(i).copied().unwrap_or(0.0);
        scalar *= axis_scalar(v, lower, peak, upper);
        if scalar == 0.0 {
            break;
        }
    }
    scalar
}

/// One named instance of the `fvar` table.
#[derive(Clone, Debug, PartialEq)]
pub struct NamedInstance {
    /// Zero-based, in table order.
    pub index: usize,
    /// User coordinates, one per axis in table order.
    pub coordinates: Vec<(Tag, f64)>,
    pub subfamily_name_id: u16,
    /// `None` when the instance has no PostScript name ID (absent or 0xFFFF).
    pub postscript_name_id: Option<u16>,
    /// The English (or first) Windows name of `subfamily_name_id`.
    pub subfamily_name: Option<String>,
    pub postscript_name: Option<String>,
}

/// The named instances of the font's `fvar` table; more than
/// [`MAX_NAMED_INSTANCES`](crate::limits::MAX_NAMED_INSTANCES) are refused.
pub fn named_instances(font: &SfntFont) -> Result<Vec<NamedInstance>> {
    let data = font.get(*b"fvar").ok_or_else(|| TablesError::MissingTable("fvar".into()))?;
    let fvar = Fvar::read(FontData::new(data)).map_err(|e| TablesError::invalid(format!("the fvar table cannot be read: {e}")))?;
    let axes = variations::fvar_axes(font)?;
    let instances = fvar.instances().map_err(|e| TablesError::invalid(format!("the fvar instances cannot be read: {e}")))?;
    if instances.len() > crate::limits::MAX_NAMED_INSTANCES {
        return Err(TablesError::TooLarge { what: "the named instances of the fvar table", limit: crate::limits::MAX_NAMED_INSTANCES });
    }
    let name = font.get(*b"name").and_then(|d| Name::read(FontData::new(d)).ok());
    let mut out = Vec::with_capacity(instances.len());
    for (index, inst) in instances.iter().enumerate() {
        let inst = inst.map_err(|e| TablesError::invalid(format!("fvar instance {index} cannot be read: {e}")))?;
        let coordinates = axes.iter().zip(inst.coordinates.iter()).map(|(a, c)| (a.tag, c.get().to_f64())).collect();
        let subfamily_name_id = inst.subfamily_name_id.to_u16();
        let postscript_name_id = inst.post_script_name_id.map(|n| n.to_u16()).filter(|n| *n != 0xFFFF);
        out.push(NamedInstance {
            index,
            coordinates,
            subfamily_name_id,
            postscript_name_id,
            subfamily_name: name.as_ref().and_then(|n| english_name(n, subfamily_name_id)),
            postscript_name: postscript_name_id.and_then(|id| name.as_ref().and_then(|n| english_name(n, id))),
        });
    }
    Ok(out)
}

/// The Windows English name of `id`, else the first Windows or Unicode one.
fn english_name(name: &Name<'_>, id: u16) -> Option<String> {
    let records = name.name_record();
    let pick = |want: &dyn Fn(u16, u16) -> bool| {
        records
            .iter()
            .filter(|r| r.name_id().to_u16() == id && want(r.platform_id(), r.language_id()))
            .find_map(|r| r.string(name.string_data()).ok().map(|s| s.chars().collect::<String>()))
    };
    pick(&|p, l| p == 3 && l == 0x409).or_else(|| pick(&|p, _| p == 3)).or_else(|| pick(&|p, _| p == 0))
}
