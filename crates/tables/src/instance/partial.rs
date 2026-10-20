//! Partial instances (M12.21): some axes pinned, others limited to a range, possibly with a new
//! default. A port of the partial path of fontTools' `varLib.instancer` (MIT, Copyright (c) 2019
//! Google LLC and the fontTools authors).
//!
//! The steps, in fontTools' order:
//!
//! 1. Each limit becomes a user range (pins and drops are ranges of zero width), clamped to the
//!    `fvar` range, then a normalised range through the `avar` segment maps, quantised to
//!    F2Dot14.
//! 2. Every tuple variation (`gvar`, `cvar`) and every item-variation-store region (`MVAR`,
//!    `HVAR`, `VVAR`, `GDEF`) is rebased axis by axis, in tag order, by the region solver
//!    ([`super::solver`]). Parts that no longer depend on any axis are summed into the new
//!    default; the rest are merged by region and rounded.
//! 3. `glyf`, `hmtx` and `vmtx` take the default parts of `gvar` (as a full instance does); the
//!    remaining `gvar` deltas are optimised against the new default outline (IUP, tolerance
//!    0.5). `cvt ` and the `MVAR` fields take theirs; `GDEF` carets and `GPOS` values theirs,
//!    keeping their variation-index devices while the store still has regions.
//! 4. Feature-variation conditions are renormalised, conditions on pinned axes are settled, and
//!    a catch-all record keeps the old features where a record now applies at the default.
//! 5. `avar` maps are renormalised, `STAT` keeps the axis values inside the ranges, `fvar` loses
//!    the pinned axes and the named instances outside the new space, and the `name` records only
//!    they used go.
//! 6. `OS/2.xAvgCharWidth`, `usWeightClass`, `usWidthClass` and `post.italicAngle` follow the
//!    new defaults; bounds and `maxp` are recomputed when `glyf` was rewritten.
//! 7. `CFF2` (see `cff2.rs`): the blends of each variation-store subtable are rebased as the
//!    items of one item variation store subtable; `HVAR` and `VVAR` move the advances of a font
//!    without `glyf` by their default parts. `BASE` coordinates take theirs, keeping their
//!    devices while its store has regions. With `update_names`, the style names come from the
//!    `STAT` axis values at the new default location (see `names.rs`).
//! 8. `COLR` (see `colr.rs`): its store is rebased and its varied fields take their default
//!    parts, as `GDEF` does; fontTools leaves `COLR` as it is.
//! 9. With `avar` version 2 (see `avar2.rs`), the variation tables stay in the input's final
//!    coordinates and the `avar` table makes up the difference; only self-contained pinned
//!    axes are instanced in them.
//! 10. A `VARC` table (see `varc.rs`) has its axis indexes renumbered.
//!
//! The item variation stores keep their subtables and item counts, so every variation index
//! stays valid; fontTools reorders and packs them afterwards, which changes no value.
//!
//! Refused: limits on an axis a `VARC` table uses, feature-variation conditions other than axis
//! ranges, an `avar` map that is not valid.

use std::collections::{HashMap, HashSet};

use fontcraft_geom::{Point, Vec2};
use read_fonts::tables::cvar::Cvar as ReadCvar;
use read_fonts::tables::gvar::Gvar as ReadGvar;
use read_fonts::tables::variations::{ItemVariationStore as ReadStore, Tuple};
use read_fonts::types::GlyphId;
use read_fonts::{FontData, FontRead};
use write_fonts::from_obj::ToOwnedTable;
use write_fonts::tables::cvar::{Cvar as WriteCvar, CvtDeltas};
use write_fonts::tables::gdef::{CaretValue, Gdef};
use write_fonts::tables::gpos::Gpos;
use write_fonts::tables::gsub::Gsub;
use write_fonts::tables::gvar::iup::iup_delta_optimize;
use write_fonts::tables::gvar::{GlyphDelta, GlyphDeltas, GlyphVariations, Gvar as WriteGvar};
use write_fonts::tables::hvar::Hvar;
use write_fonts::tables::layout::{
    Condition, ConditionFormat1, ConditionSet, DeviceOrVariationIndex, FeatureList, FeatureTableSubstitution, FeatureVariationRecord,
    FeatureVariations,
};
use write_fonts::tables::mvar::Mvar;
use write_fonts::tables::variations::{ItemVariationStore as WriteStore, Tent as WriteTent};
use write_fonts::tables::vvar::Vvar;
use write_fonts::types::F2Dot14;
use write_fonts::{NullableOffsetMarker, OffsetMarker};

use super::glyf::{self, Glyph, Metrics};
use super::iup::{Pair, iup_delta};
use super::ivs::StoreDeltas;
use super::otl::{self, Fold};
use super::solver::{AxisRange, Tent, rebase_tent};
use super::static_tables as st;
use super::{AxisLimit, InstanceOptions, base, cff2, full, names, ot_round, quantize};
use crate::limits::{MAX_SFNT, MAX_TUPLE_VARIATIONS};
use crate::sfnt::{SfntFont, Tag, tag_str};
use crate::variations::{self, FvarAxis};
use crate::{Result, TablesError};

/// Most delta values copied, scaled or added while rebasing one font, over all its tables.
const MAX_REBASE_WORK: usize = 256 << 20;
/// Most bytes of deltas one item (a glyph's or `cvar`'s tuple variations) holds at once, read
/// and then rebased: each delta is an `f64`. Lower where the address space is 32-bit.
const MAX_ITEM_DELTA_BYTES: usize = if usize::BITS >= 64 { 512 << 20 } else { 128 << 20 };
const DELTA_BYTES: usize = std::mem::size_of::<f64>();
/// Most regions in one item variation store, and region indexes in one of its subtables.
const MAX_REGIONS: usize = 65_535;
/// Most bytes one `gvar` tuple variation's data may take (its size field has 16 bits).
const MAX_TUPLE_DATA: usize = 65_535;
/// The IUP tolerance fontTools optimises the remaining `gvar` deltas with.
const IUP_TOLERANCE: f64 = 0.5;

fn bad(what: impl Into<String>) -> TablesError {
    TablesError::invalid(what)
}

/// The variable font with `limits` applied; at least one axis keeps a range.
pub fn partial_instance(font: &SfntFont, limits: &[(Tag, AxisLimit)]) -> Result<SfntFont> {
    partial_instance_with(font, &InstanceOptions { limits: limits.to_vec(), ..InstanceOptions::default() })
}

/// [`partial_instance`] with the options of [`InstanceOptions`]. When every axis ends up pinned
/// (a range clamped to one value), the result is the full instance.
pub fn partial_instance_with(font: &SfntFont, options: &InstanceOptions) -> Result<SfntFont> {
    if font.get(*b"fvar").is_none() {
        return Err(TablesError::MissingTable("fvar".into()));
    }
    if font.get(*b"gvar").is_some() && font.get(*b"glyf").is_none() {
        return Err(bad("the font has a gvar table but no glyf table"));
    }
    let limits = resolve_limits(font, &options.limits)?;
    let tags: Vec<Tag> = limits.axes.iter().map(|a| a.tag).collect();
    let limited: Vec<bool> = limits.user.iter().map(Option::is_some).collect();
    super::varc::check_limits(font, &tags, &limited)?;
    if limits.new_count() == 0 {
        let pinned: Vec<(Tag, f64)> = limits.axes.iter().zip(&limits.user).filter_map(|(a, u)| u.map(|u| (a.tag, u.default))).collect();
        return full::full_instance_with(font, &pinned, options);
    }

    let mut out = font.clone();
    out.remove(*b"DSIG");
    if options.update_names {
        let defaults: Vec<(Tag, f64)> = limits.axes.iter().zip(&limits.user).map(|(a, u)| (a.tag, u.map_or(a.default, |u| u.default))).collect();
        names::update_name_table(&mut out, &defaults)?;
    }
    let mut budget = MAX_REBASE_WORK;
    let names_before = st::variation_name_ids(&out);
    let glyph_count = out.get(*b"maxp").and_then(|m| st::u16_at(m, 4)).map(usize::from);

    let avar2 = out.get(*b"avar").and_then(|d| st::u16_at(d, 0)).is_some_and(|v| v >= 2);
    let mut removed: Vec<bool> = (0..limits.axes.len()).map(|i| limits.is_pinned(i)).collect();
    let gvar_result = if avar2 {
        // The variation tables stay in the input's final coordinates; only self-contained
        // pinned axes are instanced in them. The result is still variable, so CFF2 stays.
        let rebased = avar2::instantiate_avar2(&mut out, &limits, &mut budget)?;
        removed = (0..limits.axes.len()).map(|i| rebased.self_contained.iter().any(|(j, _)| *j == i)).collect();
        if rebased.self_contained.is_empty() {
            None
        } else {
            let sc = avar2::self_contained_limits(&limits, &rebased.self_contained);
            let sc_options = InstanceOptions { downgrade_cff2: false, ..options.clone() };
            instance_tables(&mut out, &sc, &sc_options, &mut budget)?
        }
    } else {
        let metrics = instance_tables(&mut out, &limits, options, &mut budget)?;
        partial_avar(&mut out, &limits)?;
        metrics
    };

    let ranges: Vec<(Tag, f64, f64)> = limits.axes.iter().zip(&limits.user).filter_map(|(a, u)| u.map(|u| (a.tag, u.min, u.max))).collect();
    st::reduce_stat_ranges(&mut out, &ranges)?;
    partial_fvar(&mut out, &limits, &removed)?;
    super::varc::renumber(&mut out, &removed)?;
    let names_after = st::variation_name_ids(&out);
    let unused: Vec<u16> = names_before.into_iter().filter(|id| !names_after.contains(id)).collect();
    st::prune_names(&mut out, &unused)?;

    if out.get(*b"OS/2").is_some() {
        let hmtx = match gvar_result {
            Some(ref h) => Some(h.clone()),
            None => match (glyph_count, out.get(*b"hmtx"), out.get(*b"hhea")) {
                (Some(n), Some(m), Some(h)) => Some(glyf::read_metrics(m, h, n)?),
                _ => None,
            },
        };
        if let Some(h) = &hmtx {
            st::recalc_avg_char_width(&mut out, h);
        }
    }
    let defaults: Vec<(Tag, f64)> = limits.axes.iter().zip(&limits.user).filter_map(|(a, u)| u.map(|u| (a.tag, u.default))).collect();
    st::set_weight_width_slant(&mut out, &defaults);
    // The instancer reads the glyph order of every font, which loads `post`; saving writes it
    // again (`transform::post::recompile`).
    if let (Some(post), Some(n)) = (out.get(*b"post"), glyph_count) {
        let post = crate::transform::post::recompile(post, n)?;
        out.insert(*b"post", post);
    }
    if gvar_result.is_some() && options.recalc_bounds {
        crate::ttx::read_core::recalc_compiled_maxp(&mut out)?;
    }
    if options.update_names {
        names::set_ribbi_bits(&mut out)?;
    }
    Ok(out)
}

/// Every variation table rebased by `limits`; answers the new metrics when `gvar` was applied.
fn instance_tables(out: &mut SfntFont, limits: &Limits, options: &InstanceOptions, budget: &mut usize) -> Result<Option<Metrics>> {
    let metrics = if out.get(*b"gvar").is_some() { Some(partial_gvar(out, limits, options.recalc_bounds, budget)?) } else { None };
    partial_cvar(out, limits, budget)?;
    partial_mvar(out, limits, budget)?;
    partial_metrics_store(out, *b"HVAR", limits, budget)?;
    partial_metrics_store(out, *b"VVAR", limits, budget)?;
    partial_cff2(out, limits, options, budget)?;
    partial_base(out, limits, budget)?;
    partial_layout(out, limits, budget)?;
    partial_colr(out, limits, budget)?;
    Ok(metrics)
}

/// `COLR`: the store is rebased and the varied fields take their default parts (see `colr.rs`);
/// with no region left, the variation data goes.
fn partial_colr(out: &mut SfntFont, limits: &Limits, budget: &mut usize) -> Result<()> {
    let Some(offset) = super::colr::store_offset(out) else { return Ok(()) };
    let data = out.get(*b"COLR").unwrap_or_default().to_vec();
    let rebased = rebase_store(&data, offset, limits, budget)?;
    let store = if rebased.regions == 0 { None } else { Some(rebased.owned()?) };
    super::colr::apply(out, &StoreDeltas::from_rows(rebased.defaults), store)
}

// ---------------------------------------------------------------------------------------------
// Limits

/// A resolved user range: minimum, default, maximum, within the `fvar` range.
#[derive(Clone, Copy, Debug, PartialEq)]
struct UserRange {
    min: f64,
    default: f64,
    max: f64,
}

impl UserRange {
    fn pinned(&self) -> bool {
        self.min == self.max
    }
}

/// The limits of one partial instance, per `fvar` axis of the input.
#[derive(Clone, Debug)]
struct Limits {
    axes: Vec<FvarAxis>,
    /// `None` for an axis that keeps its full range.
    user: Vec<Option<UserRange>>,
    /// The range in the space of the variation tables: normalised, through `avar`, quantised.
    norm: Vec<Option<AxisRange>>,
    /// The range normalised without `avar`, quantised: what the `avar` maps are rebased with.
    plain: Vec<Option<AxisRange>>,
    /// Each axis's index in the new `fvar`; `None` for a pinned axis.
    new_index: Vec<Option<usize>>,
    /// The limited axes in tag order, the order fontTools rebases them in.
    order: Vec<usize>,
    /// The `avar` segment maps, one per axis (empty for none), as a dictionary keeps them.
    avar: Option<Vec<SegmentMap>>,
}

impl Limits {
    fn new_count(&self) -> usize {
        self.new_index.iter().filter(|i| i.is_some()).count()
    }

    fn is_pinned(&self, axis: usize) -> bool {
        self.user.get(axis).copied().flatten().is_some_and(|u| u.pinned())
    }
}

fn resolve_limits(font: &SfntFont, given: &[(Tag, AxisLimit)]) -> Result<Limits> {
    let axes = variations::fvar_axes(font)?;
    let avar = read_avar(font, axes.len())?;
    let mut user: Vec<Option<UserRange>> = vec![None; axes.len()];
    for (tag, limit) in given {
        let name = tag_str(*tag);
        let name = name.trim_end();
        let Some(i) = axes.iter().position(|a| a.tag == *tag) else {
            return Err(TablesError::BadOptions(format!("the font has no `{name}` axis")));
        };
        let (Some(slot), Some(a)) = (user.get_mut(i), axes.get(i)) else { continue };
        if slot.is_some() {
            return Err(TablesError::BadOptions(format!("the `{name}` axis is limited twice")));
        }
        if !(a.minimum <= a.default && a.default <= a.maximum) {
            return Err(bad(format!("the fvar range of the `{name}` axis is out of order")));
        }
        let (min, default, max) = match *limit {
            AxisLimit::Pin(v) => {
                if !v.is_finite() {
                    return Err(TablesError::BadOptions(format!("the `{name}` value is not a finite number")));
                }
                (v, Some(v), v)
            }
            AxisLimit::Drop => (a.default, Some(a.default), a.default),
            AxisLimit::Range { min, default, max } => (min, default, max),
        };
        if min.is_nan() || max.is_nan() || default.is_some_and(|d| !d.is_finite()) {
            return Err(TablesError::BadOptions(format!("the `{name}` range holds a value that is not a number")));
        }
        let default = default.or((min == max).then_some(min));
        let sorted = min <= max && default.is_none_or(|d| min <= d && d <= max);
        if !sorted {
            return Err(TablesError::BadOptions(format!("the `{name}` range must be in order: minimum, default, maximum")));
        }
        // fontTools' `limitRangeAndPopulateDefaults`; an open end is the fvar end.
        let min = min.max(a.minimum).min(a.maximum);
        let max = max.max(a.minimum).min(a.maximum);
        let default = default.unwrap_or(a.default).min(max).max(min);
        *slot = Some(UserRange { min, default, max });
    }
    let mut norm = Vec::with_capacity(axes.len());
    let mut plain = Vec::with_capacity(axes.len());
    for (i, (a, u)) in axes.iter().zip(&user).enumerate() {
        let Some(u) = u else {
            norm.push(None);
            plain.push(None);
            continue;
        };
        let n = |v: f64| variations::normalize_value(v, a);
        let segments = avar.as_ref().and_then(|s| s.get(i));
        let mapped = |v: f64| quantize(segments.map_or(v, |s| piecewise(v, s)));
        let (dn, dp) = (a.default - a.minimum, a.maximum - a.default);
        let r =
            AxisRange { min: mapped(n(u.min)), default: mapped(n(u.default)), max: mapped(n(u.max)), distance_negative: dn, distance_positive: dp };
        if !(-1.0 <= r.min && r.min <= r.default && r.default <= r.max && r.max <= 1.0) {
            return Err(bad(format!("the avar map of the `{}` axis puts the range out of order", tag_str(a.tag).trim_end())));
        }
        norm.push(Some(r));
        plain.push(Some(AxisRange {
            min: quantize(n(u.min)),
            default: quantize(n(u.default)),
            max: quantize(n(u.max)),
            distance_negative: dn,
            distance_positive: dp,
        }));
    }
    let mut next = 0;
    let new_index = user
        .iter()
        .map(|u| {
            if u.is_some_and(|u| u.pinned()) {
                None
            } else {
                next += 1;
                Some(next - 1)
            }
        })
        .collect();
    let mut order: Vec<usize> = (0..axes.len()).filter(|i| user.get(*i).is_some_and(Option::is_some)).collect();
    order.sort_by_key(|i| axes.get(*i).map(|a| a.tag));
    Ok(Limits { axes, user, norm, plain, new_index, order, avar })
}

/// One axis's `avar` segment map: (from, to) pairs in normalised coordinates.
type SegmentMap = Vec<(f64, f64)>;

/// The `avar` segment maps, one per `fvar` axis (of a version 2 table too).
fn read_avar(font: &SfntFont, axis_count: usize) -> Result<Option<Vec<SegmentMap>>> {
    let Some(data) = font.get(*b"avar") else { return Ok(None) };
    let major = st::u16_at(data, 0).ok_or_else(|| bad("the avar table is truncated"))?;
    if major > 2 {
        return Err(TablesError::Unsupported(format!("instancing an avar table of version {major}")));
    }
    let count = usize::from(st::u16_at(data, 6).ok_or_else(|| bad("the avar table is truncated"))?);
    let mut maps = Vec::with_capacity(axis_count);
    let mut at = 8usize;
    for _ in 0..count.min(axis_count) {
        let n = usize::from(st::u16_at(data, at).ok_or_else(|| bad("the avar axis maps are cut short"))?);
        let mut pairs = Vec::with_capacity(n);
        for k in 0..n {
            let (Some(from), Some(to)) = (st::u16_at(data, at + 2 + 4 * k), st::u16_at(data, at + 4 + 4 * k)) else {
                return Err(bad("the avar axis maps are cut short"));
            };
            pairs.push((f2dot14(from), f2dot14(to)));
        }
        maps.push(as_map(pairs));
        at += 2 + 4 * n;
    }
    maps.resize(axis_count, Vec::new());
    Ok(Some(maps))
}

fn f2dot14(bits: u16) -> f64 {
    f64::from(bits as i16) / 16384.0
}

/// fontTools' `floatToFixed(v, 14)`, saturated.
fn to_f2(v: f64) -> F2Dot14 {
    F2Dot14::from_bits(ot_round(v * 16384.0).clamp(-32768.0, 32767.0) as i16)
}

fn f2_value(v: F2Dot14) -> f64 {
    f64::from(v.to_bits()) / 16384.0
}

/// Sorted by the first value; of pairs with the same first value the last one wins, as a
/// dictionary built from them keeps it.
fn as_map(mut pairs: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out: Vec<(f64, f64)> = Vec::with_capacity(pairs.len());
    for (k, v) in pairs {
        match out.last_mut() {
            Some(last) if last.0 == k => last.1 = v,
            _ => out.push((k, v)),
        }
    }
    out
}

/// fontTools' `piecewiseLinearMap` over a map sorted by its first values with no repeats.
fn piecewise(v: f64, map: &[(f64, f64)]) -> f64 {
    let (Some(&(first, first_to)), Some(&(last, last_to))) = (map.first(), map.last()) else { return v };
    if let Some(&(_, to)) = map.iter().find(|(k, _)| *k == v) {
        return to;
    }
    if v < first {
        return v + first_to - first;
    }
    if v > last {
        return v + last_to - last;
    }
    let (Some(&(a, va)), Some(&(b, vb))) = (map.iter().rev().find(|(k, _)| *k < v), map.iter().find(|(k, _)| *k > v)) else { return v };
    va + (vb - va) * (v - a) / (b - a)
}

// ---------------------------------------------------------------------------------------------
// Tuple variations and their rebasing

/// One tuple variation (or one store region with its column of deltas): the tent of each axis
/// that takes part, by `fvar` index of the input, and its deltas (x and y interleaved for `gvar`).
#[derive(Clone, Debug, PartialEq)]
struct TupleVar {
    axes: Vec<(usize, Tent)>,
    deltas: Vec<f64>,
}

/// The tuple variations once rebased: the deltas folded into the default (`None` when nothing
/// was), and the rest, merged by region and rounded.
#[derive(Debug, Default)]
struct Rebased {
    default: Option<Vec<f64>>,
    kept: Vec<TupleVar>,
}

/// Refuses an item whose deltas would take more than [`MAX_ITEM_DELTA_BYTES`].
fn check_held(deltas: usize) -> Result<()> {
    if deltas.saturating_mul(DELTA_BYTES) > MAX_ITEM_DELTA_BYTES {
        return Err(TablesError::TooLarge { what: "the bytes of deltas one item holds while rebasing", limit: MAX_ITEM_DELTA_BYTES });
    }
    Ok(())
}

fn charge(budget: &mut usize, work: usize) -> Result<()> {
    if work > *budget {
        return Err(TablesError::TooLarge { what: "the work of rebasing the variation data", limit: MAX_REBASE_WORK });
    }
    *budget -= work;
    Ok(())
}

/// fontTools' `changeTupleVariationAxisLimit` for one variation and one axis.
fn change_axis(mut var: TupleVar, axis: usize, range: AxisRange, out: &mut Vec<TupleVar>) {
    let Some(pos) = var.axes.iter().position(|(i, _)| *i == axis) else {
        out.push(var);
        return;
    };
    let Some(&(_, tent)) = var.axes.get(pos) else { return };
    if tent.peak == 0.0 {
        var.axes.remove(pos);
        out.push(var);
        return;
    }
    // A tent out of order, or one that crosses 0, is dropped.
    if !(tent.start <= tent.peak && tent.peak <= tent.end) || (tent.start < 0.0 && tent.end > 0.0) {
        return;
    }
    for (scalar, solved) in rebase_tent(tent, range) {
        let mut v = var.clone();
        match solved {
            None => {
                v.axes.remove(pos);
            }
            Some(t) => {
                if let Some(slot) = v.axes.get_mut(pos) {
                    slot.1 = t;
                }
            }
        }
        if scalar != 1.0 {
            for d in &mut v.deltas {
                *d *= scalar;
            }
        }
        out.push(v);
    }
}

/// fontTools' `instantiateTupleVariationStore`, with inferred deltas already filled in.
fn rebase_vars(mut vars: Vec<TupleVar>, limits: &Limits, budget: &mut usize) -> Result<Rebased> {
    for &axis in &limits.order {
        let Some(range) = limits.norm.get(axis).copied().flatten() else { continue };
        let mut next: Vec<TupleVar> = Vec::with_capacity(vars.len());
        // Deltas in `next`; those of `vars` were checked when they were made.
        let mut held = 0usize;
        for v in vars {
            // The solver gives at most five regions for one.
            charge(budget, v.deltas.len().saturating_add(1).saturating_mul(5))?;
            let before = next.len();
            change_axis(v, axis, range, &mut next);
            if next.len() > MAX_TUPLE_VARIATIONS {
                return Err(TablesError::TooLarge { what: "the tuple variations of one item once rebased", limit: MAX_TUPLE_VARIATIONS });
            }
            held = next.get(before..).unwrap_or_default().iter().fold(held, |n, v| n.saturating_add(v.deltas.len()));
            check_held(held)?;
        }
        vars = next;
    }
    let mut index: HashMap<Vec<(usize, [u64; 3])>, usize> = HashMap::new();
    let mut merged: Vec<TupleVar> = Vec::new();
    for mut v in vars {
        v.axes.sort_by_key(|(i, _)| *i);
        // Adding 0.0 turns −0 into +0, which a dictionary key treats as equal.
        let key: Vec<(usize, [u64; 3])> = v.axes.iter().map(|(i, t)| (*i, [t.start, t.peak, t.end].map(|x| (x + 0.0).to_bits()))).collect();
        match index.get(&key).and_then(|k| merged.get_mut(*k)) {
            Some(m) => {
                charge(budget, v.deltas.len())?;
                for (a, b) in m.deltas.iter_mut().zip(&v.deltas) {
                    *a += *b;
                }
            }
            None => {
                index.insert(key, merged.len());
                merged.push(v);
            }
        }
    }
    let default = merged.iter().position(|v| v.axes.is_empty()).map(|k| merged.remove(k).deltas);
    for v in &mut merged {
        for d in &mut v.deltas {
            *d = ot_round(*d);
        }
    }
    Ok(Rebased { default, kept: merged })
}

/// The axes of a tuple read from `gvar` or `cvar` that take part (peak not 0), with the
/// intermediate region implied by the peak when none is given.
fn tuple_axes(peak: &Tuple<'_>, start: Option<&Tuple<'_>>, end: Option<&Tuple<'_>>, axis_count: usize) -> Vec<(usize, Tent)> {
    (0..axis_count)
        .filter_map(|i| {
            let p = peak.get(i).map_or(0.0, f2_value);
            if p == 0.0 {
                return None;
            }
            let s = start.and_then(|t| t.get(i)).map_or(p.min(0.0), f2_value);
            let e = end.and_then(|t| t.get(i)).map_or(p.max(0.0), f2_value);
            Some((i, Tent { start: s, peak: p, end: e }))
        })
        .collect()
}

/// The tents of a kept variation for the new axes, in new `fvar` order.
fn write_tents(var: &TupleVar, limits: &Limits) -> Vec<WriteTent> {
    let mut tents = vec![WriteTent::new(F2Dot14::ZERO, None); limits.new_count()];
    for (old, t) in &var.axes {
        if let Some(Some(j)) = limits.new_index.get(*old)
            && let Some(slot) = tents.get_mut(*j)
        {
            *slot = WriteTent::new(to_f2(t.peak), Some((to_f2(t.start), to_f2(t.end))));
        }
    }
    tents
}

fn new_axis_count(limits: &Limits) -> Result<u16> {
    u16::try_from(limits.new_count()).map_err(|_| TablesError::TooLarge { what: "the number of axes", limit: usize::from(u16::MAX) })
}

// ---------------------------------------------------------------------------------------------
// glyf, gvar, hmtx, vmtx

/// Applies the default part of `gvar` to the outlines and metrics, writes the rest as the new
/// `gvar` (or removes it), and gives the new horizontal metrics.
fn partial_gvar(out: &mut SfntFont, limits: &Limits, recalc_bounds: bool, budget: &mut usize) -> Result<Metrics> {
    let glyphs_n = out.get(*b"maxp").and_then(|m| st::u16_at(m, 4)).map(usize::from).ok_or_else(|| TablesError::MissingTable("maxp".into()))?;
    let head = out.get(*b"head").ok_or_else(|| TablesError::MissingTable("head".into()))?;
    let long_loca = st::u16_at(head, 50).ok_or_else(|| bad("the head table is truncated"))? != 0;
    let loca = out.get(*b"loca").ok_or_else(|| TablesError::MissingTable("loca".into()))?;
    let mut glyphs = glyf::read_glyf(out.get(*b"glyf").unwrap_or_default(), loca, long_loca, glyphs_n)?;
    let mut hmtx = {
        let m = out.get(*b"hmtx").ok_or_else(|| TablesError::MissingTable("hmtx".into()))?;
        let h = out.get(*b"hhea").ok_or_else(|| TablesError::MissingTable("hhea".into()))?;
        glyf::read_metrics(m, h, glyphs_n)?
    };
    let mut vmtx = match (out.get(*b"vmtx"), out.get(*b"vhea")) {
        (Some(m), Some(h)) => Some(glyf::read_metrics(m, h, glyphs_n)?),
        _ => None,
    };
    let axis_count = limits.axes.len();
    let new_axes = new_axis_count(limits)?;
    let gvar_data = out.get(*b"gvar").unwrap_or_default().to_vec();
    let table = ReadGvar::read(FontData::new(&gvar_data)).map_err(|e| bad(format!("the gvar table cannot be read: {e}")))?;
    if usize::from(table.axis_count()) != axis_count {
        return Err(bad(format!("the gvar table has {} axes and the fvar table {axis_count}", table.axis_count())));
    }
    let mut order: Vec<(usize, usize)> = glyf::depths(&glyphs)?.into_iter().enumerate().map(|(g, d)| (d, g)).collect();
    order.sort_unstable();
    let mut flatten_budget = glyf::MAX_FLATTEN_WORK;
    let mut variations: Vec<Vec<GlyphDeltas>> = vec![Vec::new(); glyphs_n];
    for (_, g) in order {
        let Some(glyph) = glyphs.get(g) else { continue };
        let [x_min, _, _, y_max] = glyph.bbox().map(f64::from);
        let adv = f64::from(hmtx.advances.get(g).copied().unwrap_or(0));
        let lsb = f64::from(hmtx.bearings.get(g).copied().unwrap_or(0));
        let left = x_min - lsb;
        let (top, bottom) = match vmtx.as_ref() {
            Some(v) => {
                let top = f64::from(v.bearings.get(g).copied().unwrap_or(0)) + y_max;
                (top, top - f64::from(v.advances.get(g).copied().unwrap_or(0)))
            }
            None => (0.0, 0.0),
        };
        let simple = matches!(glyph, Glyph::Simple(_) | Glyph::Empty);
        let (mut coords0, ends): (Vec<Pair>, Vec<usize>) = match glyph {
            Glyph::Simple(s) => (s.points.clone(), s.end_pts.iter().map(|e| usize::from(*e)).collect()),
            Glyph::Composite(c) => (c.components.iter().map(|k| (k.dx, k.dy)).collect(), (0..c.components.len()).collect()),
            Glyph::Empty => (Vec::new(), Vec::new()),
        };
        coords0.extend([(left, 0.0), (left + adv, 0.0), (0.0, top), (0.0, bottom)]);
        let vars = glyph_vars(&table, g, axis_count, &coords0, &ends, budget)?;
        let rebased = rebase_vars(vars, limits, budget)?;
        let mut moved = coords0.clone();
        if let Some(d) = &rebased.default {
            for (k, p) in moved.iter_mut().enumerate() {
                p.0 += d.get(2 * k).copied().unwrap_or(0.0);
                p.1 += d.get(2 * k + 1).copied().unwrap_or(0.0);
            }
        }
        let n = moved.len();
        let phantom = |k: usize| moved.get(n.saturating_sub(k)).copied().unwrap_or((0.0, 0.0));
        let (left, right, top, bottom) = (phantom(4).0, phantom(3).0, phantom(2).1, phantom(1).1);
        match glyphs.get_mut(g) {
            Some(Glyph::Simple(s)) => {
                for (p, m) in s.points.iter_mut().zip(&moved) {
                    *p = (ot_round(m.0), ot_round(m.1));
                }
            }
            Some(Glyph::Composite(c)) => {
                for (k, m) in c.components.iter_mut().zip(&moved) {
                    if k.flags & 0x0002 != 0 {
                        (k.dx, k.dy) = *m;
                    }
                }
            }
            _ => {}
        }
        let bbox = glyf::recalc_bounds(&glyphs, g, &mut flatten_budget)?;
        match glyphs.get_mut(g) {
            Some(Glyph::Simple(s)) => s.bbox = bbox,
            Some(Glyph::Composite(c)) => c.bbox = bbox,
            _ => {}
        }
        let [x_min, _, _, y_max] = bbox.map(f64::from);
        if let (Some(a), Some(b)) = (hmtx.advances.get_mut(g), hmtx.bearings.get_mut(g)) {
            *a = ot_round(right - left).clamp(0.0, 65535.0) as u16;
            *b = clamp_i16(ot_round(x_min - left));
        }
        if let Some(v) = vmtx.as_mut()
            && let (Some(a), Some(b)) = (v.advances.get_mut(g), v.bearings.get_mut(g))
        {
            *a = ot_round(top - bottom).clamp(0.0, 65535.0) as u16;
            *b = clamp_i16(ot_round(top - y_max));
        }
        // The kept deltas are optimised against the new default outline, rounded.
        let origin: Vec<Point> = moved.iter().map(|p| Point::new(ot_round(p.0), ot_round(p.1))).collect();
        let slot = variations.get_mut(g).ok_or_else(|| bad("the glyph count changed"))?;
        for var in &rebased.kept {
            if var.deltas.iter().all(|d| *d == 0.0) {
                continue;
            }
            let deltas = glyph_deltas(var, &origin, &ends, simple).map_err(|e| bad(format!("glyph {g}: {e}")))?;
            slot.push(GlyphDeltas::new(write_tents(var, limits), deltas));
        }
    }
    if variations.iter().all(Vec::is_empty) {
        out.remove(*b"gvar");
    } else {
        let list: Vec<GlyphVariations> =
            variations.into_iter().enumerate().map(|(g, v)| GlyphVariations::new(GlyphId::new(u32::try_from(g).unwrap_or(u32::MAX)), v)).collect();
        let gvar = WriteGvar::new(list, new_axes).map_err(|e| bad(format!("the gvar table cannot be built: {e:?}")))?;
        let bytes = write_fonts::dump_table(&gvar).map_err(|e| bad(format!("the gvar table could not be written: {e}")))?;
        if bytes.len() > MAX_SFNT {
            return Err(TablesError::TooLarge { what: "the gvar table", limit: MAX_SFNT });
        }
        out.insert(*b"gvar", bytes);
    }
    let (glyf_bytes, loca_bytes, long) = glyf::write_glyf(&glyphs)?;
    out.insert(*b"glyf", glyf_bytes);
    out.insert(*b"loca", loca_bytes);
    st::put_u16(out, *b"head", 50, u16::from(long));
    let (bytes, long_metrics) = glyf::write_metrics(&hmtx);
    out.insert(*b"hmtx", bytes);
    st::put_u16(out, *b"hhea", 34, long_metrics);
    if let Some(v) = &vmtx {
        let (bytes, long_metrics) = glyf::write_metrics(v);
        out.insert(*b"vmtx", bytes);
        st::put_u16(out, *b"vhea", 34, long_metrics);
    }
    if recalc_bounds {
        st::recalc_bounds(out, &glyphs, &hmtx, vmtx.as_ref());
    }
    Ok(hmtx)
}

fn clamp_i16(v: f64) -> i16 {
    v.clamp(-32768.0, 32767.0) as i16
}

/// The tuple variations of glyph `g`, every point's delta given (left-out points inferred
/// against the default outline `coords0`).
fn glyph_vars(table: &ReadGvar<'_>, g: usize, axis_count: usize, coords0: &[Pair], ends: &[usize], budget: &mut usize) -> Result<Vec<TupleVar>> {
    let n = coords0.len();
    let gid = GlyphId::new(u32::try_from(g).unwrap_or(u32::MAX));
    let Some(data) = table.glyph_variation_data(gid).map_err(|e| bad(format!("the gvar data of glyph {g} cannot be read: {e}")))? else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for (count, tuple) in data.tuples().enumerate() {
        if count >= MAX_TUPLE_VARIATIONS {
            return Err(TablesError::TooLarge { what: "the tuple variations of one glyph", limit: MAX_TUPLE_VARIATIONS });
        }
        charge(budget, n.saturating_mul(2))?;
        check_held(count.saturating_add(1).saturating_mul(n).saturating_mul(2))?;
        let start = tuple.intermediate_start();
        let end = tuple.intermediate_end();
        let axes = tuple_axes(&tuple.peak(), start.as_ref(), end.as_ref(), axis_count);
        let mut explicit: Vec<Option<Pair>> = vec![None; n];
        for d in tuple.deltas() {
            if let Some(slot) = explicit.get_mut(usize::from(d.position)) {
                *slot = Some((f64::from(d.x_delta), f64::from(d.y_delta)));
            }
        }
        let full: Vec<Pair> = if explicit.iter().all(Option::is_some) {
            explicit.into_iter().map(|d| d.unwrap_or((0.0, 0.0))).collect()
        } else {
            iup_delta(&explicit, coords0, ends)
        };
        out.push(TupleVar { axes, deltas: full.into_iter().flat_map(|(x, y)| [x, y]).collect() });
    }
    Ok(out)
}

/// The `gvar` deltas of one kept variation: optimised for simple and empty glyphs, every one
/// given for composites (whose left-out points do not move).
fn glyph_deltas(var: &TupleVar, origin: &[Point], ends: &[usize], simple: bool) -> std::result::Result<Vec<GlyphDelta>, String> {
    let mut pairs = Vec::with_capacity(origin.len());
    for c in var.deltas.chunks(2) {
        let (Some(&x), Some(&y)) = (c.first(), c.get(1)) else { return Err("odd delta count".into()) };
        if !(-32768.0..=32767.0).contains(&x) || !(-32768.0..=32767.0).contains(&y) {
            return Err("a delta of the partial instance lies outside −32,768 to 32,767".into());
        }
        pairs.push((x as i16, y as i16));
    }
    // The optimiser slices the points by contour, so it only sees well-formed end points.
    let ends_ok = ends.windows(2).all(|w| matches!(w, [a, b] if a < b)) && ends.last().map_or(0, |e| e + 1) + 4 == origin.len();
    let deltas: Vec<GlyphDelta> = if simple && ends_ok {
        let vecs: Vec<Vec2> = pairs.iter().map(|&(x, y)| Vec2::new(f64::from(x), f64::from(y))).collect();
        iup_delta_optimize(vecs, origin.to_vec(), IUP_TOLERANCE, ends).map_err(|e| format!("the deltas cannot be optimised: {e:?}"))?
    } else {
        pairs.iter().map(|&(x, y)| GlyphDelta::required(x, y)).collect()
    };
    if tuple_size(&deltas) > MAX_TUPLE_DATA {
        return Err(format!("one tuple variation would need more than {MAX_TUPLE_DATA} bytes"));
    }
    Ok(deltas)
}

/// An upper bound of the bytes a tuple variation's data takes, with every point and with only
/// the required ones (the writer may pick either).
fn tuple_size(deltas: &[GlyphDelta]) -> usize {
    let packed = |values: &mut dyn Iterator<Item = i16>| -> usize {
        let v: Vec<i16> = values.collect();
        let n = v.len();
        let width = if v.iter().all(|d| i8::try_from(*d).is_ok()) { 1 } else { 2 };
        width * n + n.div_ceil(64)
    };
    let all = 1 + packed(&mut deltas.iter().map(|d| d.x)) + packed(&mut deltas.iter().map(|d| d.y));
    let required: Vec<&GlyphDelta> = deltas.iter().filter(|d| d.required).collect();
    let k = required.len();
    let sparse = 3 + 2 * k + k.div_ceil(128) + packed(&mut required.iter().map(|d| d.x)) + packed(&mut required.iter().map(|d| d.y));
    all.max(sparse)
}

// ---------------------------------------------------------------------------------------------
// cvar

fn partial_cvar(out: &mut SfntFont, limits: &Limits, budget: &mut usize) -> Result<()> {
    let (Some(data), Some(cvt)) = (out.get(*b"cvar"), out.get(*b"cvt ")) else { return Ok(()) };
    let count = cvt.len() / 2;
    let axis_count = limits.axes.len();
    let cvar = ReadCvar::read(FontData::new(data)).map_err(|e| bad(format!("the cvar table cannot be read: {e}")))?;
    let axes = u16::try_from(axis_count).map_err(|_| bad("too many axes"))?;
    let Some(variation_data) = cvar.variation_data(axes) else { return Err(bad("the cvar variation data cannot be read")) };
    // Every tuple's deltas are held together, so the whole set is checked before any is made.
    check_held(usize::from(cvar.tuple_variation_count().count()).saturating_mul(count))?;
    let mut vars = Vec::new();
    for (n, tuple) in variation_data.tuples().enumerate() {
        if n >= MAX_TUPLE_VARIATIONS {
            return Err(TablesError::TooLarge { what: "the tuple variations of cvar", limit: MAX_TUPLE_VARIATIONS });
        }
        charge(budget, count)?;
        let start = tuple.intermediate_start();
        let end = tuple.intermediate_end();
        let tuple_axes = tuple_axes(&tuple.peak(), start.as_ref(), end.as_ref(), axis_count);
        // A value the tuple leaves out does not move.
        let mut deltas = vec![0.0; count];
        for d in tuple.deltas() {
            if let Some(s) = deltas.get_mut(usize::from(d.position)) {
                *s = f64::from(d.value);
            }
        }
        vars.push(TupleVar { axes: tuple_axes, deltas });
    }
    let rebased = rebase_vars(vars, limits, budget)?;
    let mut new_cvt = cvt.to_vec();
    if let Some(d) = &rebased.default {
        for (i, delta) in d.iter().enumerate() {
            if *delta != 0.0
                && let Some(slot) = new_cvt.get_mut(2 * i..2 * i + 2)
            {
                let v = i16::from_be_bytes([slot.first().copied().unwrap_or(0), slot.get(1).copied().unwrap_or(0)]);
                slot.copy_from_slice(&clamp_i16(f64::from(v) + ot_round(*delta)).to_be_bytes());
            }
        }
    }
    let mut kept = Vec::with_capacity(rebased.kept.len());
    for var in &rebased.kept {
        if var.deltas.iter().all(|d| *d == 0.0) {
            continue;
        }
        if var.deltas.iter().any(|d| !(-32768.0..=32767.0).contains(d)) {
            return Err(bad("a cvar delta of the partial instance lies outside −32,768 to 32,767"));
        }
        kept.push(CvtDeltas::new(write_tents(var, limits), var.deltas.iter().map(|d| *d as i32).collect()));
    }
    out.insert(*b"cvt ", new_cvt);
    if kept.is_empty() {
        out.remove(*b"cvar");
    } else {
        let table = WriteCvar::new(kept, new_axis_count(limits)?).map_err(|e| bad(format!("the cvar table cannot be built: {e:?}")))?;
        let bytes = write_fonts::dump_table(&table).map_err(|e| bad(format!("the cvar table could not be written: {e}")))?;
        out.insert(*b"cvar", bytes);
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Item variation stores: MVAR, HVAR, VVAR, GDEF

/// An item variation store once rebased: the new store's bytes, the deltas folded into the
/// default (by outer then inner index) and the number of regions left.
struct RebasedStore {
    bytes: Vec<u8>,
    defaults: Vec<Vec<f64>>,
    regions: usize,
}

impl RebasedStore {
    fn owned(&self) -> Result<WriteStore> {
        let store = ReadStore::read(FontData::new(&self.bytes)).map_err(|e| bad(format!("the rebased item variation store cannot be read: {e}")))?;
        Ok(store.to_owned_table())
    }
}

/// fontTools' `instantiateItemVariationStore`: every subtable keeps its item count, so the
/// variation indexes stay valid.
fn rebase_store(table: &[u8], offset: usize, limits: &Limits, budget: &mut usize) -> Result<RebasedStore> {
    let mut defaults = Vec::new();
    let mut subtables: Vec<(usize, Vec<TupleVar>)> = Vec::new();
    for (count, vars) in read_store(table, offset, budget)? {
        let rebased = rebase_vars(vars, limits, budget)?;
        defaults.push(rebased.default.unwrap_or_else(|| vec![0.0; count]));
        subtables.push((count, rebased.kept));
    }
    write_store(&subtables, &limits.new_index, limits.new_count()).map(|(bytes, regions)| RebasedStore { bytes, defaults, regions })
}

/// The subtables of the item variation store at `offset` in `table`, each as its item count
/// and one variation per region column.
fn read_store(table: &[u8], offset: usize, budget: &mut usize) -> Result<Vec<(usize, Vec<TupleVar>)>> {
    let data = table.get(offset..).ok_or_else(|| bad("an item variation store offset points past the end of its table"))?;
    let store = ReadStore::read(FontData::new(data)).map_err(|e| bad(format!("an item variation store cannot be read: {e}")))?;
    let list = store.variation_region_list().map_err(|e| bad(format!("the variation region list cannot be read: {e}")))?;
    let mut regions: Vec<Vec<(usize, Tent)>> = Vec::new();
    for r in list.variation_regions().iter() {
        let r = r.map_err(|e| bad(format!("a variation region cannot be read: {e}")))?;
        let axes: Vec<(usize, Tent)> = r
            .region_axes()
            .iter()
            .enumerate()
            .filter_map(|(i, a)| {
                let peak = f2_value(a.peak_coord());
                (peak != 0.0).then(|| (i, Tent { start: f2_value(a.start_coord()), peak, end: f2_value(a.end_coord()) }))
            })
            .collect();
        charge(budget, axes.len().saturating_add(1))?;
        regions.push(axes);
    }
    let mut subtables: Vec<(usize, Vec<TupleVar>)> = Vec::new();
    for i in 0..store.item_variation_data_count() {
        let sub = match store.item_variation_data().get(usize::from(i)) {
            None => {
                subtables.push((0, Vec::new()));
                continue;
            }
            Some(sub) => sub.map_err(|e| bad(format!("item variation data {i} cannot be read: {e}")))?,
        };
        let count = usize::from(sub.item_count());
        let indexes: Vec<usize> = sub.region_indexes().iter().map(|r| usize::from(r.get())).collect();
        charge(budget, count.saturating_mul(indexes.len()).saturating_add(count))?;
        let mut columns: Vec<Vec<f64>> = vec![Vec::with_capacity(count); indexes.len()];
        for inner in 0..count {
            let Ok(inner) = u16::try_from(inner) else { break };
            for (column, delta) in columns.iter_mut().zip(sub.delta_set(inner)) {
                column.push(f64::from(delta));
            }
        }
        let mut vars = Vec::with_capacity(indexes.len());
        for (r, mut deltas) in indexes.iter().zip(columns) {
            deltas.resize(count, 0.0);
            let axes = regions.get(*r).cloned().ok_or_else(|| bad(format!("item variation data {i} names region {r}, which the store lacks")))?;
            vars.push(TupleVar { axes, deltas });
        }
        subtables.push((count, vars));
    }
    Ok(subtables)
}

/// Serialises a store: the regions in first-use order, each subtable with the regions its
/// variations use and deltas of one width (8, 16 or 32 bits) for every column. `new_index` gives
/// each input axis's index among the `new_axes` axes written, `None` for an axis that goes.
fn write_store(subtables: &[(usize, Vec<TupleVar>)], new_index: &[Option<usize>], new_axes: usize) -> Result<(Vec<u8>, usize)> {
    let mut region_index: HashMap<Vec<(usize, [i16; 3])>, u16> = HashMap::new();
    let mut regions: Vec<Vec<(usize, [i16; 3])>> = Vec::new();
    let mut datas: Vec<Vec<u8>> = Vec::with_capacity(subtables.len());
    for (count, vars) in subtables {
        let mut indexes: Vec<u16> = Vec::with_capacity(vars.len());
        for var in vars {
            let mut key: Vec<(usize, [i16; 3])> = var
                .axes
                .iter()
                .filter_map(|(old, t)| new_index.get(*old).copied().flatten().map(|j| (j, [t.start, t.peak, t.end].map(|v| to_f2(v).to_bits()))))
                .collect();
            key.sort_unstable();
            let idx = match region_index.get(&key) {
                Some(i) => *i,
                None => {
                    let i = u16::try_from(regions.len())
                        .ok()
                        .filter(|i| usize::from(*i) < MAX_REGIONS)
                        .ok_or(TablesError::TooLarge { what: "the regions of an item variation store", limit: MAX_REGIONS })?;
                    region_index.insert(key.clone(), i);
                    regions.push(key);
                    i
                }
            };
            indexes.push(idx);
        }
        let item_count = u16::try_from(*count).map_err(|_| bad("an item variation subtable holds more than 65,535 items"))?;
        // The word-count field keeps its top bit for the long-words flag.
        let region_count = u16::try_from(indexes.len())
            .ok()
            .filter(|n| *n < 0x8000)
            .ok_or(TablesError::TooLarge { what: "the regions of one item variation subtable", limit: 0x7FFF })?;
        let fits = |lo: f64, hi: f64| vars.iter().all(|v| v.deltas.iter().all(|d| (lo..=hi).contains(d)));
        if !fits(f64::from(i32::MIN), f64::from(i32::MAX)) {
            return Err(bad("an item variation delta of the partial instance does not fit 32 bits"));
        }
        let (width, word_count) = if fits(-128.0, 127.0) {
            (1usize, 0u16)
        } else if fits(-32768.0, 32767.0) {
            (2, region_count)
        } else {
            (4, region_count | 0x8000)
        };
        let mut d = Vec::with_capacity(6 + 2 * indexes.len() + count.saturating_mul(indexes.len()).saturating_mul(width));
        for v in [item_count, word_count, region_count] {
            d.extend_from_slice(&v.to_be_bytes());
        }
        for i in &indexes {
            d.extend_from_slice(&i.to_be_bytes());
        }
        for item in 0..*count {
            for var in vars {
                let v = var.deltas.get(item).copied().unwrap_or(0.0) as i32;
                match width {
                    1 => d.extend_from_slice(&(v as i8).to_be_bytes()),
                    2 => d.extend_from_slice(&(v as i16).to_be_bytes()),
                    _ => d.extend_from_slice(&v.to_be_bytes()),
                }
            }
        }
        if d.len() > MAX_SFNT {
            return Err(TablesError::TooLarge { what: "an item variation subtable", limit: MAX_SFNT });
        }
        datas.push(d);
    }
    let axis_count = u16::try_from(new_axes).map_err(|_| bad("too many axes"))?;
    let mut list = Vec::with_capacity(4 + regions.len() * new_axes * 6);
    list.extend_from_slice(&axis_count.to_be_bytes());
    list.extend_from_slice(&u16::try_from(regions.len()).unwrap_or(u16::MAX).to_be_bytes());
    for region in &regions {
        for j in 0..new_axes {
            let coords = region.iter().find(|(a, _)| *a == j).map_or([0i16; 3], |(_, c)| *c);
            for c in coords {
                list.extend_from_slice(&c.to_be_bytes());
            }
        }
    }
    let data_count = u16::try_from(datas.len()).map_err(|_| bad("too many item variation subtables"))?;
    let header = 8 + 4 * datas.len();
    let total = header + list.len() + datas.iter().map(Vec::len).sum::<usize>();
    if total > MAX_SFNT {
        return Err(TablesError::TooLarge { what: "an item variation store", limit: MAX_SFNT });
    }
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&(header as u32).to_be_bytes());
    out.extend_from_slice(&data_count.to_be_bytes());
    let mut at = header + list.len();
    for d in &datas {
        out.extend_from_slice(&(at as u32).to_be_bytes());
        at += d.len();
    }
    out.extend_from_slice(&list);
    for d in &datas {
        out.extend_from_slice(d);
    }
    Ok((out, regions.len()))
}

/// `MVAR`: the default deltas go into the fields; the store is rebased, or the table goes when
/// no region is left.
fn partial_mvar(out: &mut SfntFont, limits: &Limits, budget: &mut usize) -> Result<()> {
    let Some(data) = out.get(*b"MVAR") else { return Ok(()) };
    let offset = usize::from(st::u16_at(data, 10).ok_or_else(|| bad("the MVAR table is truncated"))?);
    if offset == 0 {
        return Ok(());
    }
    let data = data.to_vec();
    let rebased = rebase_store(&data, offset, limits, budget)?;
    st::apply_mvar_deltas(out, &StoreDeltas::from_rows(rebased.defaults.clone()))?;
    if rebased.regions == 0 {
        out.remove(*b"MVAR");
        return Ok(());
    }
    let read = read_fonts::tables::mvar::Mvar::read(FontData::new(&data)).map_err(|e| bad(format!("the MVAR table cannot be read: {e}")))?;
    let mut mvar: Mvar = read.to_owned_table();
    mvar.item_variation_store = NullableOffsetMarker::new(Some(rebased.owned()?));
    let bytes = write_fonts::dump_table(&mvar).map_err(|e| bad(format!("the MVAR table could not be written again: {e}")))?;
    out.insert(*b"MVAR", bytes);
    Ok(())
}

/// `HVAR` or `VVAR`: the store is rebased. In a font with `glyf` outlines the metrics already
/// moved with the phantom points; in one without (CFF2), the default parts move the advances.
fn partial_metrics_store(out: &mut SfntFont, tag: Tag, limits: &Limits, budget: &mut usize) -> Result<()> {
    let Some(data) = out.get(tag) else { return Ok(()) };
    let data = data.to_vec();
    let offset = st::u16_at(&data, 4).zip(st::u16_at(&data, 6)).map(|(h, l)| (usize::from(h) << 16) | usize::from(l));
    let offset = offset.ok_or_else(|| bad(format!("the {} table is truncated", tag_str(tag))))?;
    let rebased = rebase_store(&data, offset, limits, budget)?;
    if out.get(*b"glyf").is_none() {
        cff2::shift_advances(out, tag, &StoreDeltas::from_rows(rebased.defaults.clone()))?;
    }
    let bytes = if tag == *b"HVAR" {
        let read = read_fonts::tables::hvar::Hvar::read(FontData::new(&data)).map_err(|e| bad(format!("the HVAR table cannot be read: {e}")))?;
        let mut t: Hvar = read.to_owned_table();
        t.item_variation_store = OffsetMarker::new(rebased.owned()?);
        write_fonts::dump_table(&t)
    } else {
        let read = read_fonts::tables::vvar::Vvar::read(FontData::new(&data)).map_err(|e| bad(format!("the VVAR table cannot be read: {e}")))?;
        let mut t: Vvar = read.to_owned_table();
        t.item_variation_store = OffsetMarker::new(rebased.owned()?);
        write_fonts::dump_table(&t)
    };
    let bytes = bytes.map_err(|e| bad(format!("the {} table could not be written again: {e}", tag_str(tag))))?;
    out.insert(tag, bytes);
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// CFF2 and BASE

/// `CFF2`: the items of each variation-store subtable (the values its blends make) rebased as
/// the items of one item variation store subtable. A subtable left without regions goes and its
/// values stop varying; the store goes when none is left, and with `downgrade_cff2` the table
/// becomes `CFF `, as fontTools does when nothing in the outlines varies any more.
fn partial_cff2(out: &mut SfntFont, limits: &Limits, options: &InstanceOptions, budget: &mut usize) -> Result<()> {
    let Some(data) = out.get(*b"CFF2") else { return Ok(()) };
    let mut table = cff2::Table::read(data, limits.axes.len())?;
    if table.datas.is_empty() {
        return cff2::instance_full(out, &vec![0.0; limits.axes.len()], options.downgrade_cff2, options.recalc_bounds);
    }
    let n_items = table.items.len();
    let mut by_major: Vec<Vec<usize>> = vec![Vec::new(); table.datas.len()];
    for (i, it) in table.items.iter().enumerate() {
        if let Some(v) = by_major.get_mut(it.major) {
            v.push(i);
        }
    }
    let mut add = vec![0.0; n_items];
    let mut new_deltas: Vec<Vec<f64>> = vec![Vec::new(); n_items];
    let mut majors: Vec<Option<usize>> = Vec::with_capacity(table.datas.len());
    let mut kept_subtables: Vec<(usize, Vec<TupleVar>)> = Vec::new();
    for (major, region_idx) in table.datas.iter().enumerate() {
        let members = by_major.get(major).map(Vec::as_slice).unwrap_or_default();
        let count = members.len();
        charge(budget, count.saturating_mul(region_idx.len()).saturating_add(count).saturating_add(1))?;
        let mut vars = Vec::with_capacity(region_idx.len());
        for (j, r) in region_idx.iter().enumerate() {
            let region = table
                .regions
                .get(usize::from(*r))
                .ok_or_else(|| bad(format!("CFF2 variation subtable {major} names region {r}, which the store lacks")))?;
            let axes: Vec<(usize, Tent)> = region
                .iter()
                .enumerate()
                .filter(|(_, (_, p, _))| *p != 0.0)
                .map(|(i, (s, p, e))| (i, Tent { start: *s, peak: *p, end: *e }))
                .collect();
            let deltas: Vec<f64> = members.iter().map(|i| table.items.get(*i).and_then(|it| it.deltas.get(j)).copied().unwrap_or(0.0)).collect();
            vars.push(TupleVar { axes, deltas });
        }
        let rebased = rebase_vars(vars, limits, budget)?;
        if let Some(d) = &rebased.default {
            for (pos, i) in members.iter().enumerate() {
                if let Some(slot) = add.get_mut(*i) {
                    *slot = d.get(pos).copied().unwrap_or(0.0);
                }
            }
        }
        if rebased.kept.is_empty() {
            majors.push(None);
            continue;
        }
        for (pos, i) in members.iter().enumerate() {
            if let Some(slot) = new_deltas.get_mut(*i) {
                *slot = rebased.kept.iter().map(|v| v.deltas.get(pos).copied().unwrap_or(0.0)).collect();
            }
        }
        majors.push(Some(kept_subtables.len()));
        kept_subtables.push((0, rebased.kept));
    }
    table.resolve(&cff2::Resolved { add: &add, deltas: Some(&new_deltas), majors: &majors })?;
    let store = if kept_subtables.is_empty() { None } else { Some(write_store(&kept_subtables, &limits.new_index, limits.new_count())?.0) };
    cff2::finish(out, &table, store.as_deref(), options.downgrade_cff2, options.recalc_bounds)
}

/// `BASE`: the store rebased, its default parts added to the coordinates; the devices stay
/// while regions are left.
fn partial_base(out: &mut SfntFont, limits: &Limits, budget: &mut usize) -> Result<()> {
    let Some(offset) = base::store_offset(out)? else { return Ok(()) };
    let data = out.get(*b"BASE").unwrap_or_default().to_vec();
    let rebased = rebase_store(&data, offset, limits, budget)?;
    let deltas = StoreDeltas::from_rows(rebased.defaults.clone());
    let store = if rebased.regions > 0 { Some(rebased.owned()?) } else { None };
    base::apply(out, &deltas, store)
}

// ---------------------------------------------------------------------------------------------
// GDEF, GPOS, GSUB

/// `GDEF`'s store rebased and its default deltas added to the carets and `GPOS` values (their
/// devices stay while regions are left, as the store still varies them); then the feature
/// variations of `GSUB` and `GPOS`.
fn partial_layout(out: &mut SfntFont, limits: &Limits, budget: &mut usize) -> Result<()> {
    let mut fold: Option<(StoreDeltas, bool)> = None;
    if let Some(data) = out.get(*b"GDEF") {
        let version = st::u16_at(data, 0).zip(st::u16_at(data, 2)).map(|(a, b)| (u32::from(a) << 16) | u32::from(b)).unwrap_or(0);
        let offset = if version >= 0x0001_0003 {
            st::u16_at(data, 14).zip(st::u16_at(data, 16)).map_or(0, |(h, l)| (usize::from(h) << 16) | usize::from(l))
        } else {
            0
        };
        if offset != 0 {
            crate::layout_budget::check_gdef(data)?;
            let data = data.to_vec();
            let rebased = rebase_store(&data, offset, limits, budget)?;
            let deltas = StoreDeltas::from_rows(rebased.defaults.clone());
            let keep = rebased.regions > 0;
            let read = read_fonts::tables::gdef::Gdef::read(FontData::new(&data)).map_err(|e| bad(format!("the GDEF table cannot be read: {e}")))?;
            let mut gdef: Gdef = read.to_owned_table();
            if let Some(list) = gdef.lig_caret_list.as_mut() {
                for lig in list.lig_glyphs.iter_mut() {
                    for caret in lig.caret_values.iter_mut() {
                        if let CaretValue::Format3(t) = &mut **caret
                            && let DeviceOrVariationIndex::VariationIndex(v) = &*t.device
                        {
                            let d = ot_round(deltas.get(v.delta_set_outer_index, v.delta_set_inner_index));
                            let value = clamp_i16(f64::from(t.coordinate) + d);
                            if keep {
                                t.coordinate = value;
                            } else {
                                **caret = CaretValue::format_1(value);
                            }
                        }
                    }
                }
            }
            if keep {
                gdef.item_var_store = NullableOffsetMarker::new(Some(rebased.owned()?));
            } else {
                gdef.item_var_store = NullableOffsetMarker::new(None);
            }
            let empty = !keep
                && gdef.glyph_class_def.is_none()
                && gdef.attach_list.is_none()
                && gdef.lig_caret_list.is_none()
                && gdef.mark_attach_class_def.is_none()
                && gdef.mark_glyph_sets_def.is_none();
            if empty {
                out.remove(*b"GDEF");
            } else {
                let bytes = write_fonts::dump_table(&gdef).map_err(|e| bad(format!("the GDEF table could not be written again: {e}")))?;
                out.insert(*b"GDEF", bytes);
            }
            fold = Some((deltas, keep));
        }
    }
    if let Some(data) = out.get(*b"GPOS") {
        let read = read_fonts::tables::gpos::Gpos::read(FontData::new(data)).map_err(|e| bad(format!("the GPOS table cannot be read: {e}")))?;
        let has_variations = read.feature_variations_offset().is_some_and(|o| !o.is_null());
        if fold.is_some() || has_variations {
            crate::layout_budget::check_gpos(data)?;
            let mut gpos: Gpos = read.to_owned_table();
            crate::layout_budget::restore_empty_pair_records(&mut gpos)?;
            if let Some((deltas, keep)) = &fold {
                otl::apply_gpos(&mut gpos, Fold { deltas, keep_devices: *keep });
            }
            partial_feature_variations(&mut gpos.feature_list, &mut gpos.feature_variations, limits)?;
            let bytes = write_fonts::dump_table(&gpos).map_err(|e| bad(format!("the GPOS table could not be written again: {e}")))?;
            out.insert(*b"GPOS", bytes);
        }
    }
    if let Some(data) = out.get(*b"GSUB") {
        let read = read_fonts::tables::gsub::Gsub::read(FontData::new(data)).map_err(|e| bad(format!("the GSUB table cannot be read: {e}")))?;
        if read.feature_variations_offset().is_some_and(|o| !o.is_null()) {
            crate::layout_budget::check_gsub(data)?;
            let mut gsub: Gsub = read.to_owned_table();
            partial_feature_variations(&mut gsub.feature_list, &mut gsub.feature_variations, limits)?;
            let bytes = write_fonts::dump_table(&gsub).map_err(|e| bad(format!("the GSUB table could not be written again: {e}")))?;
            out.insert(*b"GSUB", bytes);
        }
    }
    Ok(())
}

/// fontTools' `_instantiateFeatureVariations`: conditions renormalised to the new ranges,
/// conditions on pinned axes settled, records that can no longer hold dropped, the first record
/// that holds at the new default applied to the features, and a catch-all record (no
/// conditions) that restores the old features where no kept record holds.
fn partial_feature_variations(
    features: &mut OffsetMarker<FeatureList>,
    variations: &mut NullableOffsetMarker<FeatureVariations, 4>,
    limits: &Limits,
) -> Result<()> {
    let Some(fv) = variations.as_ref() else { return Ok(()) };
    let full_range = AxisRange::new(-1.0, 0.0, 1.0);
    let mut applied = false;
    let mut universal = false;
    let mut seen: HashSet<Vec<(u16, i16, i16)>> = HashSet::new();
    let mut records: Vec<FeatureVariationRecord> = Vec::new();
    let mut defaults: Option<FeatureTableSubstitution> = None;
    for rec in &fv.feature_variation_records {
        let mut applies = true;
        let mut keep = false;
        let mut conditions: Option<Vec<ConditionFormat1>> = Some(Vec::new());
        let given: Vec<&Condition> = rec.condition_set.as_ref().map(|s| s.conditions.iter().map(|c| &**c).collect()).unwrap_or_default();
        for c in given {
            let Condition::Format1AxisRange(c) = c else {
                return Err(TablesError::Unsupported("instancing feature variations whose conditions are not axis ranges".into()));
            };
            let axis = usize::from(c.axis_index);
            if axis >= limits.axes.len() {
                return Err(bad(format!("a feature variation condition names axis {axis}, which the font does not have")));
            }
            let (min, max) = (f2_value(c.filter_range_min_value), f2_value(c.filter_range_max_value));
            let triple = limits.norm.get(axis).copied().flatten().unwrap_or(full_range);
            if !(min <= triple.default && triple.default <= max) {
                applies = false;
            }
            if triple.min > max || triple.max < min {
                conditions = None;
                break;
            }
            if let Some(Some(j)) = limits.new_index.get(axis) {
                if min > max || min > triple.max || max < triple.min {
                    conditions = None;
                    break;
                }
                let (lo, hi) = (triple.renormalize(min, false), triple.renormalize(max, false));
                keep = true;
                if (lo != -1.0 || hi != 1.0)
                    && let Some(list) = conditions.as_mut()
                {
                    let j = u16::try_from(*j).map_err(|_| bad("too many axes"))?;
                    list.push(ConditionFormat1::new(j, to_f2(lo), to_f2(hi)));
                }
            }
        }
        let kept = conditions.filter(|_| keep);
        let is_universal = kept.as_ref().is_some_and(Vec::is_empty);
        if let Some(list) = &kept {
            let mut key: Vec<(u16, i16, i16)> =
                list.iter().map(|c| (c.axis_index, c.filter_range_min_value.to_bits(), c.filter_range_max_value.to_bits())).collect();
            key.sort_unstable();
            key.dedup();
            if seen.insert(key) {
                let set = (!list.is_empty()).then(|| ConditionSet::new(list.iter().cloned().map(Condition::Format1AxisRange).collect()));
                records.push(FeatureVariationRecord::new(set, rec.feature_table_substitution.as_ref().cloned()));
            }
        }
        if applies && !applied {
            if let Some(subst) = rec.feature_table_substitution.as_ref() {
                let mut restore = subst.clone();
                for (back, s) in restore.substitutions.iter_mut().zip(&subst.substitutions) {
                    let record = features
                        .feature_records
                        .get_mut(usize::from(s.feature_index))
                        .ok_or_else(|| bad(format!("a feature variation substitutes feature {}, which the feature list lacks", s.feature_index)))?;
                    back.alternate_feature = OffsetMarker::new((*record.feature).clone());
                    record.feature = OffsetMarker::new((*s.alternate_feature).clone());
                }
                defaults = Some(restore);
            }
            applied = true;
        }
        universal = is_universal;
        if universal {
            break;
        }
    }
    if applied
        && !records.is_empty()
        && !universal
        && let Some(restore) = defaults
    {
        records.push(FeatureVariationRecord::new(Some(ConditionSet::new(Vec::new())), Some(restore)));
    }
    *variations = if records.is_empty() { NullableOffsetMarker::new(None) } else { NullableOffsetMarker::new(Some(FeatureVariations::new(records))) };
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// avar, fvar

/// fontTools' `_isValidAvarSegmentMap`: empty, or holding −1→−1, 0→0 and 1→1 with targets that
/// never decrease.
fn valid_segment_map(map: &[(f64, f64)]) -> bool {
    if map.is_empty() {
        return true;
    }
    let has = |k: f64| map.iter().any(|&(f, t)| f == k && t == k);
    has(-1.0) && has(0.0) && has(1.0) && map.windows(2).all(|w| matches!(w, [(_, a), (_, b)] if a <= b))
}

/// fontTools' rebasing of the `avar` segment maps, one entry per input axis: a limited axis keeps
/// the points inside its range, renormalised on both sides, with −1, 0 and 1 mapping to
/// themselves; an axis left alone keeps its map. A pinned axis's map goes (`None`), or, for
/// `avar` version 2 (`identity_pins`), becomes the identity map.
fn rebased_segment_maps(limits: &Limits, identity_pins: bool) -> Result<Vec<Option<SegmentMap>>> {
    let Some(maps) = &limits.avar else { return Ok(Vec::new()) };
    let mut out: Vec<Option<SegmentMap>> = Vec::with_capacity(maps.len());
    for (i, map) in maps.iter().enumerate() {
        if limits.is_pinned(i) {
            out.push(identity_pins.then(|| vec![(-1.0, -1.0), (0.0, 0.0), (1.0, 1.0)]));
            continue;
        }
        let tag = limits.axes.get(i).map(|a| tag_str(a.tag)).unwrap_or_default();
        if !valid_segment_map(map) {
            return Err(TablesError::Unsupported(format!("rebasing the avar map of the `{}` axis, which is not a valid map", tag.trim_end())));
        }
        let Some(range) = limits.plain.get(i).copied().flatten().filter(|_| !map.is_empty()) else {
            out.push(Some(map.clone()));
            continue;
        };
        let mapped = AxisRange {
            min: quantize(piecewise(range.min, map)),
            default: quantize(piecewise(range.default, map)),
            max: quantize(piecewise(range.max, map)),
            ..range
        };
        let mut new_map: Vec<(f64, f64)> = Vec::new();
        let mut set = |k: f64, v: f64| match new_map.iter_mut().find(|(f, _)| *f == k) {
            Some(slot) => slot.1 = v,
            None => new_map.push((k, v)),
        };
        for &(from, to) in map {
            if from < range.min || from > range.max {
                continue;
            }
            if !(mapped.min <= to && to <= mapped.max) {
                return Err(bad(format!("the avar map of the `{}` axis does not keep its order", tag.trim_end())));
            }
            set(quantize(range.renormalize(from, true)), quantize(mapped.renormalize(to, true)));
        }
        for k in [-1.0, 0.0, 1.0] {
            set(k, k);
        }
        new_map.sort_by(|a, b| a.0.total_cmp(&b.0));
        out.push(Some(new_map));
    }
    Ok(out)
}

/// `avar` (version 1): pinned axes' maps go; limited axes' maps are rebased.
fn partial_avar(out: &mut SfntFont, limits: &Limits) -> Result<()> {
    if limits.avar.is_none() {
        return Ok(());
    }
    let kept: Vec<SegmentMap> = rebased_segment_maps(limits, false)?.into_iter().flatten().collect();
    out.insert(*b"avar", write_avar(&kept, None)?);
    Ok(())
}

/// An `avar` table: version 1 with the segment maps alone, version 2 when `v2` gives the bytes
/// of its delta-set index map (empty for none) and item variation store.
fn write_avar(maps: &[SegmentMap], v2: Option<(&[u8], &[u8])>) -> Result<Vec<u8>> {
    let mut avar = Vec::new();
    let major = if v2.is_some() { 2u16 } else { 1 };
    for v in [major, 0, 0, u16::try_from(maps.len()).map_err(|_| bad("too many axes"))?] {
        avar.extend_from_slice(&v.to_be_bytes());
    }
    for map in maps {
        avar.extend_from_slice(&u16::try_from(map.len()).map_err(|_| bad("an avar map is too long"))?.to_be_bytes());
        for (from, to) in map {
            avar.extend_from_slice(&to_f2(*from).to_bits().to_be_bytes());
            avar.extend_from_slice(&to_f2(*to).to_bits().to_be_bytes());
        }
    }
    if let Some((index_map, store)) = v2 {
        let start = avar.len() + 8;
        let offset = |at: usize| u32::try_from(at).map_err(|_| TablesError::TooLarge { what: "the avar table", limit: MAX_SFNT });
        let map_at = if index_map.is_empty() { 0 } else { offset(start)? };
        let store_at = if store.is_empty() { 0 } else { offset(start + index_map.len())? };
        avar.extend_from_slice(&map_at.to_be_bytes());
        avar.extend_from_slice(&store_at.to_be_bytes());
        avar.extend_from_slice(index_map);
        avar.extend_from_slice(store);
        if avar.len() > MAX_SFNT {
            return Err(TablesError::TooLarge { what: "the avar table", limit: MAX_SFNT });
        }
    }
    Ok(avar)
}

fn fixed(v: f64) -> [u8; 4] {
    (ot_round(v * 65536.0).clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32).to_be_bytes()
}

/// `fvar` without the `removed` axes, with the limited axes' new ranges, keeping the named
/// instances that sit at the pinned values and inside the new ranges. A pinned axis that stays
/// (`avar` version 2) is hidden, its range the pinned value.
fn partial_fvar(out: &mut SfntFont, limits: &Limits, removed: &[bool]) -> Result<()> {
    let removed = |i: usize| removed.get(i).copied().unwrap_or(false);
    let data = out.get(*b"fvar").ok_or_else(|| TablesError::MissingTable("fvar".into()))?;
    let field = |at: usize| st::u16_at(data, at).map(usize::from).ok_or_else(|| bad("the fvar table is truncated"));
    let (axes_at, axis_count, axis_size, instance_count, instance_size) = (field(4)?, field(8)?, field(10)?, field(12)?, field(14)?);
    if axis_count != limits.axes.len() || axis_size < 20 || instance_size < 4 + 4 * axis_count {
        return Err(bad("the fvar table does not match its axes"));
    }
    let fixed_at = |at: usize| -> Option<f64> {
        data.get(at..at + 4).and_then(|b| <[u8; 4]>::try_from(b).ok()).map(|b| f64::from(i32::from_be_bytes(b)) / 65536.0)
    };
    let new_count = (0..axis_count).filter(|i| !removed(*i)).count();
    let mut axes = Vec::with_capacity(20 * new_count);
    for i in 0..axis_count {
        if removed(i) {
            continue;
        }
        let at = axes_at + i * axis_size;
        let record = data.get(at..at + 20).ok_or_else(|| bad("the fvar axes are truncated"))?;
        match limits.user.get(i).copied().flatten() {
            Some(u) => {
                axes.extend_from_slice(record.get(0..4).unwrap_or_default());
                for v in [u.min, u.default, u.max] {
                    axes.extend_from_slice(&fixed(v));
                }
                let flags = st::u16_at(record, 16).unwrap_or(0) | u16::from(u.pinned());
                axes.extend_from_slice(&flags.to_be_bytes());
                axes.extend_from_slice(record.get(18..20).unwrap_or_default());
            }
            None => axes.extend_from_slice(record),
        }
    }
    let instances_at = axes_at + axis_count * axis_size;
    let has_ps = instance_size >= 6 + 4 * axis_count;
    let mut kept: Vec<(u16, u16, Vec<f64>, u16)> = Vec::new();
    for k in 0..instance_count {
        let at = instances_at + k * instance_size;
        let (Some(name), Some(flags)) = (st::u16_at(data, at), st::u16_at(data, at + 2)) else {
            return Err(bad("the fvar instances are truncated"));
        };
        let coords: Vec<f64> =
            (0..axis_count).map(|i| fixed_at(at + 4 + 4 * i)).collect::<Option<_>>().ok_or_else(|| bad("the fvar instances are truncated"))?;
        let ps = if has_ps { st::u16_at(data, at + 4 + 4 * axis_count).unwrap_or(0xFFFF) } else { 0xFFFF };
        let fits = coords.iter().zip(&limits.user).all(|(c, u)| match u {
            Some(u) if u.pinned() => *c == u.default,
            Some(u) => u.min <= *c && *c <= u.max,
            None => true,
        });
        if fits {
            let rest = coords.iter().enumerate().filter(|(i, _)| !removed(*i)).map(|(_, c)| *c).collect();
            kept.push((name, flags, rest, ps));
        }
    }
    let with_ps = kept.iter().any(|k| k.3 != 0xFFFF);
    let size = 4 + 4 * new_count + if with_ps { 2 } else { 0 };
    let mut table = Vec::with_capacity(16 + axes.len() + kept.len() * size);
    table.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    let count = |n: usize| u16::try_from(n).map_err(|_| bad("the fvar table is too large"));
    for v in [16u16, 2, count(new_count)?, 20, count(kept.len())?, count(size)?] {
        table.extend_from_slice(&v.to_be_bytes());
    }
    table.extend_from_slice(&axes);
    for (name, flags, coords, ps) in &kept {
        table.extend_from_slice(&name.to_be_bytes());
        table.extend_from_slice(&flags.to_be_bytes());
        for c in coords {
            table.extend_from_slice(&fixed(*c));
        }
        if with_ps {
            table.extend_from_slice(&ps.to_be_bytes());
        }
    }
    out.insert(*b"fvar", table);
    Ok(())
}

#[path = "avar2.rs"]
mod avar2;

#[cfg(test)]
#[path = "partial_tests.rs"]
mod tests;
