//! Partial instances of fonts with an `avar` version 2 table. A port of `_instantiateAvarV2`,
//! `_computeOldIntermediates`, `_estimateAvar2OffsetError`, `_effectiveAvar2VarIdx` and
//! `_instantiateFvarForAvar2` of fontTools' `varLib.instancer` (MIT, Copyright (c) 2019 Google
//! LLC and the fontTools authors), with the one-axis case of `varLib.models.VariationModel`.
//!
//! The variation tables stay in the coordinates of the input's `avar` output ("old final
//! space"). The `avar` table makes up the difference:
//!
//! 1. Its segment maps are rebased as version 1 maps are; a pinned axis's map becomes the
//!    identity map.
//! 2. Its item variation store is rebased by the limits in the space between the segment maps
//!    and the store ("intermediate space").
//! 3. A pinned axis whose final coordinate no longer depends on any other axis is
//!    self-contained: it leaves `fvar` and `avar`, and the variation tables are instanced at its
//!    final coordinate. Any other pinned axis stays as a hidden axis whose range is the pinned
//!    value.
//! 4. Each other limited axis gets deltas on its own row that map its new intermediate
//!    coordinate back to the old one, so the final coordinates stay what they were.
//!
//! fontTools then removes regions of the other variation tables that the new ranges can no
//! longer reach, and packs the store; neither changes a value, and FontCraft does neither.

use super::{
    Limits, SegmentMap, TupleVar, UserRange, bad, charge, piecewise, read_store, rebase_vars, rebased_segment_maps, write_avar, write_store,
};
use crate::Result;
use crate::instance::solver::{AxisRange, Tent};
use crate::instance::{ot_round, quantize};
use crate::sfnt::SfntFont;
use std::collections::HashMap;

/// The variation index that means "no variation".
const NO_VARIATION: u32 = 0xFFFF_FFFF;
/// fontTools samples the offset function at this many evenly spaced user values.
const ERROR_SAMPLES: usize = 257;

/// One subtable of the `avar` store while it is rebased.
struct Sub {
    count: usize,
    vars: Vec<TupleVar>,
    /// The deltas the limits folded into the default, per item (not rounded).
    defaults: Vec<f64>,
}

/// What the `avar` step gives the rest of the partial instance.
pub(super) struct Avar2 {
    /// Self-contained pinned axes (input `fvar` index) and their final normalised coordinate.
    pub self_contained: Vec<(usize, f64)>,
}

/// Rebases the `avar` version 2 table of `out` by `limits` and writes it, without the
/// self-contained axes.
pub(super) fn instantiate_avar2(out: &mut SfntFont, limits: &Limits, budget: &mut usize) -> Result<Avar2> {
    let data = out.get(*b"avar").ok_or_else(|| bad("the avar table is missing"))?.to_vec();
    let axis_count = limits.axes.len();
    let (map_offset, store_offset) = v2_offsets(&data)?;
    let had_map = map_offset != 0;
    let raw_map = if had_map { read_index_map(&data, map_offset, axis_count)? } else { (0..axis_count).map(|i| i as u32).collect() };

    let mut subs: Vec<Sub> = Vec::new();
    if store_offset != 0 {
        for (count, vars) in read_store(&data, store_offset, budget)? {
            let rebased = rebase_vars(vars, limits, budget)?;
            let defaults = rebased.default.unwrap_or_else(|| vec![0.0; count]);
            subs.push(Sub { count, vars: rebased.kept, defaults });
        }
    }
    // Entries that point outside the store vary nothing, as at run time.
    let mut mapping: Vec<u32> = raw_map
        .iter()
        .map(|&idx| {
            let (outer, inner) = split(idx);
            let inside = idx != NO_VARIATION && subs.get(outer).is_some_and(|s| inner < s.count);
            if inside { idx } else { NO_VARIATION }
        })
        .collect();

    let segments = rebased_segment_maps(limits, true)?;
    let old_segments: Vec<SegmentMap> = limits.avar.clone().unwrap_or_default();
    let old = |i: usize| limits.norm.get(i).copied().flatten();

    // Self-contained pinned axes.
    let mut self_contained: Vec<(usize, f64)> = Vec::new();
    for i in 0..axis_count {
        if !limits.is_pinned(i) {
            continue;
        }
        let Some(range) = old(i) else { continue };
        let idx = mapping.get(i).copied().unwrap_or(NO_VARIATION);
        let d = ot_round(range.default * 16384.0);
        if idx == NO_VARIATION {
            self_contained.push((i, d.clamp(-16384.0, 16384.0) / 16384.0));
            continue;
        }
        let (outer, inner) = split(idx);
        let Some(sub) = subs.get(outer) else { continue };
        if sub.vars.iter().all(|v| v.deltas.get(inner).is_none_or(|d| *d == 0.0)) {
            let default = sub.defaults.get(inner).copied().unwrap_or(0.0);
            self_contained.push((i, (d + ot_round(default)).clamp(-16384.0, 16384.0) / 16384.0));
        }
    }
    let is_self_contained = |i: usize| self_contained.iter().any(|(j, _)| *j == i);

    // A row several axes share is copied for each limited axis that writes into it.
    let refs = |mapping: &[u32], idx: u32| mapping.iter().filter(|m| **m == idx).count();
    for i in 0..axis_count {
        if old(i).is_none() || is_self_contained(i) {
            continue;
        }
        let idx = mapping.get(i).copied().unwrap_or(NO_VARIATION);
        if idx == NO_VARIATION || refs(&mapping, idx) <= 1 {
            continue;
        }
        let (outer, inner) = split(idx);
        let Some(sub) = subs.get_mut(outer) else { continue };
        let new_inner = sub.count;
        if new_inner > 0xFFFF {
            return Err(bad("an avar variation subtable would hold more than 65,535 items"));
        }
        charge(budget, sub.vars.len().saturating_add(1))?;
        for v in &mut sub.vars {
            let d = v.deltas.get(inner).copied().unwrap_or(0.0);
            v.deltas.push(d);
        }
        let d = sub.defaults.get(inner).copied().unwrap_or(0.0);
        sub.defaults.push(d);
        sub.count += 1;
        if let Some(slot) = mapping.get_mut(i) {
            *slot = ((outer as u32) << 16) | new_inner as u32;
        }
    }

    // Offset compensation.
    let mut processed: Vec<u32> = Vec::new();
    for i in 0..axis_count {
        let Some(axis) = limits.axes.get(i) else { continue };
        let mut idx = mapping.get(i).copied().unwrap_or(NO_VARIATION);
        let Some(range) = old(i) else {
            // An axis left alone keeps the default part of its row as a constant.
            if idx == NO_VARIATION || processed.contains(&idx) {
                continue;
            }
            processed.push(idx);
            let (outer, inner) = split(idx);
            let Some(sub) = subs.get_mut(outer) else { continue };
            let bias = ot_round(sub.defaults.get(inner).copied().unwrap_or(0.0));
            if bias != 0.0 {
                push_delta(sub, inner, Vec::new(), bias);
            }
            continue;
        };
        if is_self_contained(i) {
            continue;
        }
        if idx == NO_VARIATION {
            let outer = subs.len();
            if outer > 0xFFFF {
                return Err(bad("the avar variation store would hold more than 65,536 subtables"));
            }
            subs.push(Sub { count: 1, vars: Vec::new(), defaults: vec![0.0] });
            idx = (outer as u32) << 16;
            if let Some(slot) = mapping.get_mut(i) {
                *slot = idx;
            }
        }
        let (outer, inner) = split(idx);
        let Some(sub) = subs.get_mut(outer) else { continue };
        let default_delta = sub.defaults.get(inner).copied().unwrap_or(0.0);
        let (a, d, b) = (range.min, range.default, range.max);
        let user = limits.user.get(i).copied().flatten().unwrap_or(UserRange { min: axis.default, default: axis.default, max: axis.default });
        if user.pinned() {
            let bias = ot_round(d * 16384.0) + ot_round(default_delta);
            if bias != 0.0 {
                push_delta(sub, inner, Vec::new(), bias);
            }
            continue;
        }
        let mut offsets: Vec<(f64, f64)> = vec![(-1.0, a + 1.0), (0.0, d), (1.0, b - 1.0)];
        let new_seg: &[(f64, f64)] = segments.get(i).and_then(Option::as_ref).map_or(&[], Vec::as_slice);
        let old_seg: &[(f64, f64)] = old_segments.get(i).map_or(&[], Vec::as_slice);
        let new_triple = (user.min, user.default, user.max);
        let old_triple = (axis.minimum, axis.default, axis.maximum);
        // Where the old default lands in the new space, when the default moved.
        let z_old_norm = normalize_triple(axis.default, new_triple);
        let z_old = quantize(if new_seg.is_empty() { z_old_norm } else { piecewise(z_old_norm, new_seg) });
        if -1.0 < z_old && z_old < 1.0 && !has_key(&offsets, z_old) {
            offsets.push((z_old, -z_old));
        }
        let mut with_breakpoints = offsets.clone();
        if !old_seg.is_empty() && !new_seg.is_empty() {
            let denorm_new = [(-1.0, new_triple.0), (0.0, new_triple.1), (1.0, new_triple.2)];
            for &(from_new, z) in new_seg {
                if from_new == -1.0 || from_new == 0.0 || from_new == 1.0 {
                    continue;
                }
                let z = quantize(z);
                if !(-1.0 < z && z < 1.0) || has_key(&with_breakpoints, z) {
                    continue;
                }
                let u = piecewise(from_new, &denorm_new);
                let x_old = piecewise(normalize_triple(u, old_triple), old_seg);
                with_breakpoints.push((z, quantize(x_old) - z));
            }
        }
        sort_map(&mut offsets);
        sort_map(&mut with_breakpoints);
        if with_breakpoints.len() != offsets.len() {
            let anchors = offset_error(old_seg, new_seg, old_triple, new_triple, &offsets);
            if offset_error(old_seg, new_seg, old_triple, new_triple, &with_breakpoints) <= anchors {
                offsets = with_breakpoints;
            }
        }
        for (support, delta) in model_deltas(&offsets) {
            let mut di = ot_round(delta * 16384.0);
            if support.is_none() {
                di += ot_round(default_delta);
            }
            if di != 0.0 {
                push_delta(sub, inner, support.map(|t| vec![(i, t)]).unwrap_or_default(), di);
            }
        }
    }

    // Self-contained axes leave the store, the index map and the segment maps.
    let mut next = 0;
    let new_index: Vec<Option<usize>> = (0..axis_count)
        .map(|i| {
            if is_self_contained(i) {
                None
            } else {
                next += 1;
                Some(next - 1)
            }
        })
        .collect();
    let subtables: Vec<(usize, Vec<TupleVar>)> = subs.into_iter().map(|s| (s.count, merge_regions(s.vars))).collect();
    let (store, _) = write_store(&subtables, &new_index, next)?;
    let kept_mapping: Vec<u32> = mapping.iter().enumerate().filter(|(i, _)| !is_self_contained(*i)).map(|(_, m)| *m).collect();
    let identity = kept_mapping.iter().enumerate().all(|(i, m)| *m == i as u32);
    let index_map = if had_map || !identity { write_index_map(&kept_mapping)? } else { Vec::new() };
    let maps: Vec<SegmentMap> =
        segments.into_iter().enumerate().filter(|(i, _)| !is_self_contained(*i)).map(|(_, m)| m.unwrap_or_default()).collect();
    out.insert(*b"avar", write_avar(&maps, Some((&index_map, &store)))?);
    Ok(Avar2 { self_contained })
}

/// The limits that instance the variation tables at the self-contained axes' final coordinates
/// (already in the space of the variation tables) and leave every other axis alone.
pub(super) fn self_contained_limits(limits: &Limits, self_contained: &[(usize, f64)]) -> Limits {
    let n = limits.axes.len();
    let at = |i: usize| self_contained.iter().find(|(j, _)| *j == i).map(|(_, v)| *v);
    let mut next = 0;
    let new_index = (0..n)
        .map(|i| {
            if at(i).is_some() {
                None
            } else {
                next += 1;
                Some(next - 1)
            }
        })
        .collect();
    let user = (0..n)
        .map(|i| at(i).and_then(|_| limits.user.get(i).copied().flatten()).map(|u| UserRange { min: u.default, default: u.default, max: u.default }))
        .collect();
    let norm: Vec<Option<AxisRange>> = (0..n).map(|i| at(i).map(|v| AxisRange::new(v, v, v))).collect();
    let mut order: Vec<usize> = self_contained.iter().map(|(i, _)| *i).collect();
    order.sort_by_key(|i| limits.axes.get(*i).map(|a| a.tag));
    Limits { axes: limits.axes.clone(), user, plain: norm.clone(), norm, new_index, order, avar: None }
}

fn split(idx: u32) -> (usize, usize) {
    ((idx >> 16) as usize, (idx & 0xFFFF) as usize)
}

/// Adds a variation with `axes` whose only non-zero delta is `value` at `inner`.
fn push_delta(sub: &mut Sub, inner: usize, axes: Vec<(usize, Tent)>, value: f64) {
    let mut deltas = vec![0.0; sub.count];
    if let Some(slot) = deltas.get_mut(inner) {
        *slot = value;
    }
    sub.vars.push(TupleVar { axes, deltas });
}

/// Variations with the same region summed, so a subtable names each region once.
fn merge_regions(vars: Vec<TupleVar>) -> Vec<TupleVar> {
    let mut index: HashMap<Vec<(usize, [u64; 3])>, usize> = HashMap::with_capacity(vars.len());
    let mut out: Vec<TupleVar> = Vec::with_capacity(vars.len());
    for mut v in vars {
        v.axes.sort_by_key(|(i, _)| *i);
        // Adding 0.0 turns −0 into +0, so the key treats them as equal, as `==` does.
        let key: Vec<(usize, [u64; 3])> = v.axes.iter().map(|(i, t)| (*i, [t.start, t.peak, t.end].map(|x| (x + 0.0).to_bits()))).collect();
        match index.get(&key).and_then(|k| out.get_mut(*k)) {
            Some(o) => {
                for (a, b) in o.deltas.iter_mut().zip(&v.deltas) {
                    *a += *b;
                }
            }
            None => {
                index.insert(key, out.len());
                out.push(v);
            }
        }
    }
    out
}

fn has_key(map: &[(f64, f64)], k: f64) -> bool {
    map.iter().any(|(x, _)| *x == k)
}

fn sort_map(map: &mut [(f64, f64)]) {
    map.sort_by(|a, b| a.0.total_cmp(&b.0));
}

/// fontTools' `normalizeValue` without extrapolation.
fn normalize_triple(v: f64, (lower, default, upper): (f64, f64, f64)) -> f64 {
    let v = v.max(lower).min(upper);
    if v == default || lower == upper {
        return 0.0;
    }
    if (v < default && lower != default) || (v > default && upper == default) {
        (v - default) / (default - lower)
    } else {
        (v - default) / (upper - default)
    }
}

/// fontTools' `_estimateAvar2OffsetError`: the largest difference, in F2Dot14 units, between the
/// old final coordinate and the one the new maps plus `offsets` give, over the retained range.
fn offset_error(
    old_seg: &[(f64, f64)],
    new_seg: &[(f64, f64)],
    old_triple: (f64, f64, f64),
    new_triple: (f64, f64, f64),
    offsets: &[(f64, f64)],
) -> f64 {
    let (lo, _, hi) = new_triple;
    let mut us: Vec<f64> = (0..ERROR_SAMPLES).map(|i| lo + (hi - lo) * i as f64 / (ERROR_SAMPLES - 1) as f64).collect();
    let denorm_old = [(-1.0, old_triple.0), (0.0, old_triple.1), (1.0, old_triple.2)];
    let denorm_new = [(-1.0, new_triple.0), (0.0, new_triple.1), (1.0, new_triple.2)];
    us.extend(old_seg.iter().map(|(n, _)| piecewise(*n, &denorm_old)));
    let mut z_to_norm: Vec<(f64, f64)> = Vec::new();
    if !new_seg.is_empty() {
        us.extend(new_seg.iter().map(|(n, _)| piecewise(*n, &denorm_new)));
        z_to_norm = super::as_map(new_seg.iter().map(|(k, v)| (*v, *k)).collect());
    }
    for (z, _) in offsets {
        let n = if z_to_norm.is_empty() { *z } else { piecewise(*z, &z_to_norm) };
        us.push(piecewise(n, &denorm_new));
    }
    let mut worst: f64 = 0.0;
    for u in us {
        if !(lo..=hi).contains(&u) {
            continue;
        }
        let n_old = normalize_triple(u, old_triple);
        let old_final = if old_seg.is_empty() { n_old } else { piecewise(n_old, old_seg) };
        let n_new = normalize_triple(u, new_triple);
        let z = if new_seg.is_empty() { n_new } else { piecewise(n_new, new_seg) };
        let new_final = z + piecewise(z, offsets);
        worst = worst.max((ot_round(old_final * 16384.0) - ot_round(new_final * 16384.0)).abs());
    }
    worst
}

/// fontTools' `VariationModel` on one axis, for masters at the first values of `points` (0 among
/// them): each master's support (`None` for the default) and delta, in the model's order.
fn model_deltas(points: &[(f64, f64)]) -> Vec<(Option<Tent>, f64)> {
    let mut order: Vec<(f64, f64)> = points.to_vec();
    // The default first, then the negative masters nearest 0 first, then the positive ones.
    order.sort_by(|a, b| {
        let key = |z: f64| {
            (
                if z == 0.0 {
                    0
                } else if z < 0.0 {
                    1
                } else {
                    2
                },
                z.abs(),
            )
        };
        let (ka, kb) = (key(a.0), key(b.0));
        ka.0.cmp(&kb.0).then(ka.1.total_cmp(&kb.1))
    });
    let min = order.iter().map(|p| p.0).filter(|z| *z != 0.0).fold(f64::INFINITY, f64::min);
    let max = order.iter().map(|p| p.0).filter(|z| *z != 0.0).fold(f64::NEG_INFINITY, f64::max);
    let mut supports: Vec<Option<Tent>> = Vec::with_capacity(order.len());
    for (k, &(z, _)) in order.iter().enumerate() {
        if z == 0.0 {
            supports.push(None);
            continue;
        }
        let mut t = if z > 0.0 { Tent { start: 0.0, peak: z, end: max } } else { Tent { start: min, peak: z, end: 0.0 } };
        for prev in order.iter().take(k).map(|p| p.0).filter(|p| *p != 0.0) {
            if !(prev == t.peak || (t.start < prev && prev < t.end)) {
                continue;
            }
            if prev < z {
                t.start = prev;
            } else if z < prev {
                t.end = prev;
            }
        }
        supports.push(Some(t));
    }
    let mut deltas: Vec<f64> = Vec::with_capacity(order.len());
    for (k, &(z, value)) in order.iter().enumerate() {
        let mut delta = value;
        for (j, support) in supports.iter().take(k).enumerate() {
            let scalar = match support {
                None => 1.0,
                Some(t) => crate::instance::axis_scalar(z, t.start, t.peak, t.end),
            };
            if scalar != 0.0 {
                delta -= deltas.get(j).copied().unwrap_or(0.0) * scalar;
            }
        }
        deltas.push(delta);
    }
    supports.into_iter().zip(deltas).collect()
}

/// The offsets of the delta-set index map and the item variation store after the segment maps.
fn v2_offsets(data: &[u8]) -> Result<(usize, usize)> {
    let u16_at = |at: usize| data.get(at..at + 2).and_then(|b| <[u8; 2]>::try_from(b).ok()).map(|b| usize::from(u16::from_be_bytes(b)));
    let count = u16_at(6).ok_or_else(|| bad("the avar table is truncated"))?;
    let mut at = 8usize;
    for _ in 0..count {
        let n = u16_at(at).ok_or_else(|| bad("the avar axis maps are cut short"))?;
        at += 2 + 4 * n;
    }
    let u32_at = |at: usize| data.get(at..at + 4).and_then(|b| <[u8; 4]>::try_from(b).ok()).map(|b| u32::from_be_bytes(b) as usize);
    let (Some(map), Some(store)) = (u32_at(at), u32_at(at + 4)) else {
        return Err(bad("the avar version 2 offsets are cut short"));
    };
    Ok((map, store))
}

/// The first `count` entries of the delta-set index map at `offset`, as outer << 16 | inner; an
/// index past the end of the map takes its last entry, and an empty map gives no variation.
fn read_index_map(data: &[u8], offset: usize, count: usize) -> Result<Vec<u32>> {
    let map = data.get(offset..).ok_or_else(|| bad("the avar index map offset points past the end of the table"))?;
    let byte = |at: usize| map.get(at).copied().ok_or_else(|| bad("the avar index map is truncated"));
    let format = byte(0)?;
    let entry_format = byte(1)?;
    let (entries, start) = match format {
        0 => ((usize::from(byte(2)?) << 8) | usize::from(byte(3)?), 4),
        1 => ((usize::from(byte(2)?) << 24) | (usize::from(byte(3)?) << 16) | (usize::from(byte(4)?) << 8) | usize::from(byte(5)?), 6),
        _ => return Err(bad(format!("the avar index map has format {format}, which is not defined"))),
    };
    let size = usize::from((entry_format >> 4) & 3) + 1;
    let inner_bits = u32::from(entry_format & 0xF) + 1;
    let mut out = Vec::with_capacity(count);
    for i in 0..count.min(entries) {
        let at = start + i * size;
        let bytes = map.get(at..at + size).ok_or_else(|| bad("the avar index map is truncated"))?;
        let v = bytes.iter().fold(0u32, |acc, b| (acc << 8) | u32::from(*b));
        let (outer, inner) = (v >> inner_bits, v & ((1u32 << inner_bits) - 1));
        out.push(if outer == 0xFFFF && inner == 0xFFFF { NO_VARIATION } else { (outer << 16) | inner });
    }
    let fill = out.last().copied().unwrap_or(NO_VARIATION);
    out.resize(count, fill);
    Ok(out)
}

/// A delta-set index map (format 0) in fontTools' smallest entry format.
fn write_index_map(mapping: &[u32]) -> Result<Vec<u8>> {
    let ored = mapping.iter().fold(0u32, |acc, m| acc | *m);
    let mut inner_bits = 32 - (ored & 0xFFFF).leading_zeros();
    inner_bits = inner_bits.max(1);
    let packed = (ored >> (16 - inner_bits)) | (ored & ((1 << inner_bits) - 1));
    let size: u32 = match packed {
        0..=0xFF => 1,
        0x100..=0xFFFF => 2,
        0x1_0000..=0xFF_FFFF => 3,
        _ => 4,
    };
    let count = u16::try_from(mapping.len()).map_err(|_| bad("too many axes for the avar index map"))?;
    let mut out = vec![0u8, (((size - 1) << 4) | (inner_bits - 1)) as u8];
    out.extend_from_slice(&count.to_be_bytes());
    for m in mapping {
        let v = ((m >> 16) << inner_bits) | (m & 0xFFFF);
        out.extend_from_slice(v.to_be_bytes().get((4 - size as usize)..).unwrap_or_default());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_one_axis_model_reproduces_its_masters() {
        // An offset function with a kink at z = 0.25 (a moved default).
        let points = [(-1.0, 0.2), (0.0, 0.1), (0.25, -0.25), (1.0, -0.4)];
        let model = model_deltas(&points);
        for &(z, want) in &points {
            let got: f64 = model.iter().map(|(s, d)| d * s.map_or(1.0, |t| crate::instance::axis_scalar(z, t.start, t.peak, t.end))).sum();
            assert!((got - want).abs() < 1e-12, "{z}: {got} != {want}");
        }
        // fontTools gives the 0.25 master the support (0, 0.25, 1) and the 1 master (0.25, 1, 1).
        let supports: Vec<Option<Tent>> = model.iter().map(|(s, _)| *s).collect();
        assert!(supports.contains(&Some(Tent { start: 0.0, peak: 0.25, end: 1.0 })));
        assert!(supports.contains(&Some(Tent { start: 0.25, peak: 1.0, end: 1.0 })));
    }

    #[test]
    fn merging_regions_sums_equal_regions_and_scales_to_many() {
        let tent = |peak: f64| Tent { start: 0.0, peak, end: 1.0 };
        let var = |axes: Vec<(usize, Tent)>, d: f64| TupleVar { axes, deltas: vec![d] };
        // Axis order and the sign of zero do not make two regions different.
        let a = vec![(1, tent(0.5)), (0, Tent { start: -0.0, peak: 1.0, end: 1.0 })];
        let b = vec![(0, tent(1.0)), (1, tent(0.5))];
        let merged = merge_regions(vec![var(a, 2.0), var(vec![(0, tent(0.25))], 5.0), var(b, 3.0)]);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged.first().map(|v| v.deltas.clone()), Some(vec![5.0]));
        // 60,000 distinct regions: a pairwise search would take billions of comparisons.
        let many: Vec<TupleVar> = (0..60_000).map(|k| var(vec![(0, tent(k as f64 / 65_536.0))], 1.0)).collect();
        assert_eq!(merge_regions(many).len(), 60_000);
    }

    #[test]
    fn index_maps_read_back_what_was_written() {
        let mapping = [0x0001_0002, 0xFFFF_0003, 7];
        let bytes = write_index_map(&mapping).unwrap();
        assert_eq!(read_index_map(&bytes, 0, 5).unwrap(), vec![0x0001_0002, 0xFFFF_0003, 7, 7, 7]);
    }
}
