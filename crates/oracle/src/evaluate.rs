//! Layout and metrics variations evaluated at one location (M9.10).
//!
//! Each function reads one font at normalized coordinates (after `avar`, as skrifa computes them)
//! and returns plain maps keyed by glyph names, so two fonts with different glyph orders, lookup
//! splits or class definitions compare by what a shaper would do:
//!
//! - [`mvar`]: each `MVAR` value tag's delta.
//! - [`substitutions`]: the single substitutions the first `GSUB` feature variation record whose
//!   conditions hold puts in place, as `feature: input -> output`.
//! - [`kerning`]: every pair adjustment of the `GPOS` pair lookups, device deltas applied. Within a
//!   lookup the first subtable that covers a pair decides it; lookups add up. Class 0 of a
//!   second class definition is not enumerated (both oracles write it with zero values).
//! - [`attachments`]: the offset from each mark's anchor to its base's (mark-to-base,
//!   mark-to-ligature per component, mark-to-mark), device deltas applied; the first lookup that
//!   attaches a pair decides it.
//!
//! The fonts are untrusted: every walk is capped (`MAX_*`, and [`MAX_WORK`] steps in all) and a
//! malformed table yields fewer entries, never a panic.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use skrifa::FontRef;
use skrifa::raw::TableProvider;
use skrifa::raw::tables::gdef::Gdef;
use skrifa::raw::tables::gpos::ValueRecord;
use skrifa::raw::tables::gpos::{AnchorTable, DeviceOrVariationIndex, PairPos, PositionSubtables};
use skrifa::raw::tables::gsub::{SingleSubst, SubstitutionSubtables};
use skrifa::raw::tables::layout::{Condition, FeatureVariations};
use skrifa::raw::tables::variations::{DeltaSetIndex, ItemVariationStore};
use skrifa::raw::types::{F2Dot14, GlyphId, GlyphId16};

/// Lookups read per table.
pub const MAX_LOOKUPS: usize = 2_000;
/// Subtables read per lookup.
pub const MAX_SUBTABLES: usize = 1_000;
/// Kerning pairs or attachments kept per font and location; beyond it the map is cut and the
/// caller notes it.
pub const MAX_ENTRIES: usize = 250_000;
/// Feature variation records and conditions read.
pub const MAX_RECORDS: usize = 1_000;
/// Subtables, glyphs, pairs and attachments visited per font and location: a font whose tables
/// would take longer is cut there, as at [`MAX_ENTRIES`].
pub const MAX_WORK: usize = 20_000_000;

/// What is left of [`MAX_WORK`].
struct Budget(usize);

impl Budget {
    fn new() -> Self {
        Budget(MAX_WORK)
    }

    /// Spends one step; false once none is left.
    fn spend(&mut self) -> bool {
        if self.0 == 0 {
            return false;
        }
        self.0 -= 1;
        true
    }
}

/// Glyph names by glyph id, as both fonts are compared.
pub type Names = HashMap<u32, String>;

fn name(names: &Names, gid: impl Into<GlyphId>) -> String {
    let gid: GlyphId = gid.into();
    names.get(&gid.to_u32()).cloned().unwrap_or_else(|| format!("gid{}", gid.to_u32()))
}

/// The `MVAR` deltas at `coords`, by value tag. Empty without `MVAR`.
pub fn mvar(font: &FontRef, coords: &[F2Dot14]) -> BTreeMap<String, f64> {
    let mut out = BTreeMap::new();
    let Ok(mvar) = font.mvar() else { return out };
    for r in mvar.value_records().iter().take(MAX_RECORDS) {
        let tag = r.value_tag();
        if let Some(d) = mvar.metric_delta(tag, coords) {
            out.insert(tag.to_string(), d.to_f64());
        }
    }
    out
}

/// Whether every condition of a set holds at `coords`. A condition format other than an axis
/// range does not hold (neither oracle writes one).
fn conditions_hold(set: &skrifa::raw::tables::layout::ConditionSet, coords: &[F2Dot14]) -> bool {
    set.conditions().iter().take(MAX_RECORDS).all(|c| match c {
        Ok(Condition::Format1AxisRange(c)) => {
            let v = coords.get(usize::from(c.axis_index())).copied().unwrap_or_default();
            c.filter_range_min_value() <= v && v <= c.filter_range_max_value()
        }
        _ => false,
    })
}

/// The feature indices and lookup lists the first matching record of `fv` substitutes.
fn active_substitutions(fv: &FeatureVariations, coords: &[F2Dot14]) -> Vec<(u16, Vec<u16>)> {
    let data = fv.offset_data();
    for record in fv.feature_variation_records().iter().take(MAX_RECORDS) {
        let holds = match record.condition_set(data) {
            Some(Ok(set)) => conditions_hold(&set, coords),
            // A null condition set matches everywhere.
            None => true,
            Some(Err(_)) => false,
        };
        if !holds {
            continue;
        }
        let Some(Ok(subst)) = record.feature_table_substitution(data) else { return Vec::new() };
        let sdata = subst.offset_data();
        return subst
            .substitutions()
            .iter()
            .take(MAX_RECORDS)
            .filter_map(|s| {
                let feature = s.alternate_feature(sdata).ok()?;
                Some((s.feature_index(), feature.lookup_list_indices().iter().take(MAX_LOOKUPS).map(|i| i.get()).collect()))
            })
            .collect();
    }
    Vec::new()
}

/// The single substitutions the `GSUB` feature variations put in place at `coords`, as
/// `feature: input -> output` lines. Other lookup types are listed by type.
pub fn substitutions(font: &FontRef, coords: &[F2Dot14], names: &Names) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let Ok(gsub) = font.gsub() else { return out };
    let Some(Ok(fv)) = gsub.feature_variations() else { return out };
    let (Ok(features), Ok(lookups)) = (gsub.feature_list(), gsub.lookup_list()) else { return out };
    let mut budget = Budget::new();
    for (feature, indices) in active_substitutions(&fv, coords) {
        let tag =
            features.feature_records().get(usize::from(feature)).map(|r| r.feature_tag().to_string()).unwrap_or_else(|| format!("feature {feature}"));
        for index in indices {
            let Ok(lookup) = lookups.lookups().get(usize::from(index)) else { continue };
            match lookup.subtables() {
                Ok(SubstitutionSubtables::Single(subtables)) => {
                    for sub in subtables.iter().take(MAX_SUBTABLES).flatten() {
                        single_pairs(&sub, &tag, names, &mut out, &mut budget);
                        if out.len() >= MAX_ENTRIES || !budget.spend() {
                            return out;
                        }
                    }
                }
                Ok(_) => {
                    out.insert(format!("{tag}: a lookup of type {}", lookup.lookup_type()));
                }
                Err(_) => {}
            }
        }
    }
    out
}

fn single_pairs(sub: &SingleSubst, tag: &str, names: &Names, out: &mut BTreeSet<String>, budget: &mut Budget) {
    match sub {
        SingleSubst::Format1(s) => {
            let Ok(cov) = s.coverage() else { return };
            let delta = i32::from(s.delta_glyph_id());
            for g in cov.iter().take(MAX_ENTRIES) {
                if !budget.spend() {
                    return;
                }
                // The delta wraps modulo 65536 (OpenType).
                let to = GlyphId16::new(((i32::from(g.to_u16()) + delta).rem_euclid(65_536)) as u16);
                out.insert(format!("{tag}: {} -> {}", name(names, g), name(names, to)));
            }
        }
        SingleSubst::Format2(s) => {
            let Ok(cov) = s.coverage() else { return };
            for (g, to) in cov.iter().zip(s.substitute_glyph_ids()).take(MAX_ENTRIES) {
                if !budget.spend() {
                    return;
                }
                out.insert(format!("{tag}: {} -> {}", name(names, g), name(names, to.get())));
            }
        }
    }
}

/// The variation store `GDEF` holds for `GPOS` device deltas.
fn var_store<'a>(font: &FontRef<'a>) -> Option<ItemVariationStore<'a>> {
    let gdef: Gdef = font.gdef().ok()?;
    gdef.item_var_store()?.ok()
}

/// The delta of a device or variation index table at `coords`; a hinting device table adds
/// nothing (it applies at sizes, not locations).
fn device_delta(
    device: Option<Result<DeviceOrVariationIndex, skrifa::raw::ReadError>>,
    store: Option<&ItemVariationStore>,
    coords: &[F2Dot14],
) -> f64 {
    match (device, store) {
        (Some(Ok(DeviceOrVariationIndex::VariationIndex(v))), Some(store)) => {
            let index = DeltaSetIndex { outer: v.delta_set_outer_index(), inner: v.delta_set_inner_index() };
            store.compute_delta(index, coords).map(|d| d.to_f64()).unwrap_or(0.0)
        }
        _ => 0.0,
    }
}

/// A value record at `coords`: x placement, y placement, x advance, y advance.
fn value(v: &ValueRecord, store: Option<&ItemVariationStore>, coords: &[F2Dot14]) -> [f64; 4] {
    let n = |x: Option<i16>| f64::from(x.unwrap_or(0));
    [
        n(v.x_placement()) + device_delta(v.x_placement_device(), store, coords),
        n(v.y_placement()) + device_delta(v.y_placement_device(), store, coords),
        n(v.x_advance()) + device_delta(v.x_advance_device(), store, coords),
        n(v.y_advance()) + device_delta(v.y_advance_device(), store, coords),
    ]
}

fn anchor(a: &AnchorTable, store: Option<&ItemVariationStore>, coords: &[F2Dot14]) -> [f64; 2] {
    let (x, y) = match a {
        AnchorTable::Format1(t) => (t.x_coordinate(), t.y_coordinate()),
        AnchorTable::Format2(t) => (t.x_coordinate(), t.y_coordinate()),
        AnchorTable::Format3(t) => (t.x_coordinate(), t.y_coordinate()),
    };
    [f64::from(x) + device_delta(a.x_device(), store, coords), f64::from(y) + device_delta(a.y_device(), store, coords)]
}

fn add(out: &mut BTreeMap<String, [f64; 4]>, key: String, v: [f64; 4]) {
    let slot = out.entry(key).or_insert([0.0; 4]);
    for (s, x) in slot.iter_mut().zip(v) {
        *s += x;
    }
}

/// Every `GPOS` pair adjustment at `coords`, keyed `first second`, first glyph's value record.
/// The bool is true when the walk stopped at [`MAX_ENTRIES`].
pub fn kerning(font: &FontRef, coords: &[F2Dot14], names: &Names) -> (BTreeMap<String, [f64; 4]>, bool) {
    let mut out = BTreeMap::new();
    let Ok(gpos) = font.gpos() else { return (out, false) };
    let Ok(lookups) = gpos.lookup_list() else { return (out, false) };
    let store = var_store(font);
    let store = store.as_ref();
    let mut budget = Budget::new();
    for lookup in lookups.lookups().iter().take(MAX_LOOKUPS).flatten() {
        let Ok(PositionSubtables::Pair(subtables)) = lookup.subtables() else { continue };
        // Pairs decided by an earlier subtable of this lookup.
        let mut taken: HashSet<(u16, u16)> = HashSet::new();
        let mut first_taken: HashSet<u16> = HashSet::new();
        for sub in subtables.iter().take(MAX_SUBTABLES).flatten() {
            match sub {
                PairPos::Format1(p) => {
                    let Ok(cov) = p.coverage() else { continue };
                    for (g1, set) in cov.iter().zip(p.pair_sets().iter()) {
                        let Ok(set) = set else { continue };
                        if first_taken.contains(&g1.to_u16()) {
                            continue;
                        }
                        for rec in set.pair_value_records().iter().flatten() {
                            if !budget.spend() {
                                return (out, true);
                            }
                            let g2 = rec.second_glyph();
                            if !taken.insert((g1.to_u16(), g2.to_u16())) {
                                continue;
                            }
                            add(&mut out, format!("{} {}", name(names, g1), name(names, g2)), value(rec.value_record1(), store, coords));
                            if out.len() >= MAX_ENTRIES {
                                return (out, true);
                            }
                        }
                    }
                }
                PairPos::Format2(p) => {
                    let (Ok(cov), Ok(cd1), Ok(cd2)) = (p.coverage(), p.class_def1(), p.class_def2()) else { continue };
                    let mut by_class: BTreeMap<u16, Vec<GlyphId16>> = BTreeMap::new();
                    for (g, c) in cd2.iter().take(MAX_ENTRIES) {
                        if c != 0 {
                            by_class.entry(c).or_default().push(g);
                        }
                    }
                    for g1 in cov.iter() {
                        if !first_taken.insert(g1.to_u16()) {
                            continue;
                        }
                        let c1 = cd1.get(g1);
                        for (c2, seconds) in &by_class {
                            if !budget.spend() {
                                return (out, true);
                            }
                            let Some([v1, _]) = p.value_record_refs(c1, *c2) else { continue };
                            let v = value(&v1, store, coords);
                            if v.iter().all(|x| x.abs() < 1e-6) {
                                continue;
                            }
                            for g2 in seconds {
                                if !budget.spend() {
                                    return (out, true);
                                }
                                if !taken.insert((g1.to_u16(), g2.to_u16())) {
                                    continue;
                                }
                                add(&mut out, format!("{} {}", name(names, g1), name(names, *g2)), v);
                                if out.len() >= MAX_ENTRIES {
                                    return (out, true);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    // Pairs whose adjustments add up to nothing are no adjustment.
    out.retain(|_, v| v.iter().any(|x| x.abs() >= 1e-6));
    (out, false)
}

/// Every mark attachment at `coords`: `base|ligature|mark <base> <mark>` → the offset from the
/// mark's anchor to the base's (for a ligature, `<ligature>#<component>`). The bool is true when
/// the walk stopped at [`MAX_ENTRIES`].
pub fn attachments(font: &FontRef, coords: &[F2Dot14], names: &Names) -> (BTreeMap<String, [f64; 2]>, bool) {
    let mut out: BTreeMap<String, [f64; 2]> = BTreeMap::new();
    let Ok(gpos) = font.gpos() else { return (out, false) };
    let Ok(lookups) = gpos.lookup_list() else { return (out, false) };
    let store = var_store(font);
    let store = store.as_ref();
    let mut budget = Budget::new();
    let put = |out: &mut BTreeMap<String, [f64; 2]>, key: String, base: [f64; 2], mark: [f64; 2]| -> bool {
        out.entry(key).or_insert([base[0] - mark[0], base[1] - mark[1]]);
        out.len() >= MAX_ENTRIES
    };
    for lookup in lookups.lookups().iter().take(MAX_LOOKUPS).flatten() {
        match lookup.subtables() {
            Ok(PositionSubtables::MarkToBase(subtables)) => {
                for sub in subtables.iter().take(MAX_SUBTABLES).flatten() {
                    let (Ok(mcov), Ok(bcov), Ok(marks), Ok(bases)) = (sub.mark_coverage(), sub.base_coverage(), sub.mark_array(), sub.base_array())
                    else {
                        continue;
                    };
                    let mdata = marks.offset_data();
                    let mark_list: Vec<(GlyphId16, u16, [f64; 2])> = mcov
                        .iter()
                        .zip(marks.mark_records())
                        .filter_map(|(g, r)| Some((g, r.mark_class(), anchor(&r.mark_anchor(mdata).ok()?, store, coords))))
                        .collect();
                    let bdata = bases.offset_data();
                    for (b, rec) in bcov.iter().zip(bases.base_records().iter()) {
                        let Ok(rec) = rec else { continue };
                        let anchors = rec.base_anchors(bdata);
                        for (m, class, ma) in &mark_list {
                            if !budget.spend() {
                                return (out, true);
                            }
                            let Some(Ok(ba)) = anchors.get(usize::from(*class)) else { continue };
                            if put(&mut out, format!("base {} {}", name(names, b), name(names, *m)), anchor(&ba, store, coords), *ma) {
                                return (out, true);
                            }
                        }
                    }
                }
            }
            Ok(PositionSubtables::MarkToLig(subtables)) => {
                for sub in subtables.iter().take(MAX_SUBTABLES).flatten() {
                    let (Ok(mcov), Ok(lcov), Ok(marks), Ok(ligs)) =
                        (sub.mark_coverage(), sub.ligature_coverage(), sub.mark_array(), sub.ligature_array())
                    else {
                        continue;
                    };
                    let mdata = marks.offset_data();
                    let mark_list: Vec<(GlyphId16, u16, [f64; 2])> = mcov
                        .iter()
                        .zip(marks.mark_records())
                        .filter_map(|(g, r)| Some((g, r.mark_class(), anchor(&r.mark_anchor(mdata).ok()?, store, coords))))
                        .collect();
                    for (l, attach) in lcov.iter().zip(ligs.ligature_attaches().iter()) {
                        let Ok(attach) = attach else { continue };
                        let adata = attach.offset_data();
                        for (k, comp) in attach.component_records().iter().take(MAX_RECORDS).enumerate() {
                            let Ok(comp) = comp else { continue };
                            let anchors = comp.ligature_anchors(adata);
                            for (m, class, ma) in &mark_list {
                                if !budget.spend() {
                                    return (out, true);
                                }
                                let Some(Ok(la)) = anchors.get(usize::from(*class)) else { continue };
                                if put(&mut out, format!("ligature {}#{k} {}", name(names, l), name(names, *m)), anchor(&la, store, coords), *ma) {
                                    return (out, true);
                                }
                            }
                        }
                    }
                }
            }
            Ok(PositionSubtables::MarkToMark(subtables)) => {
                for sub in subtables.iter().take(MAX_SUBTABLES).flatten() {
                    let (Ok(m1cov), Ok(m2cov), Ok(marks), Ok(bases)) =
                        (sub.mark1_coverage(), sub.mark2_coverage(), sub.mark1_array(), sub.mark2_array())
                    else {
                        continue;
                    };
                    let mdata = marks.offset_data();
                    let mark_list: Vec<(GlyphId16, u16, [f64; 2])> = m1cov
                        .iter()
                        .zip(marks.mark_records())
                        .filter_map(|(g, r)| Some((g, r.mark_class(), anchor(&r.mark_anchor(mdata).ok()?, store, coords))))
                        .collect();
                    let bdata = bases.offset_data();
                    for (b, rec) in m2cov.iter().zip(bases.mark2_records().iter()) {
                        let Ok(rec) = rec else { continue };
                        let anchors = rec.mark2_anchors(bdata);
                        for (m, class, ma) in &mark_list {
                            if !budget.spend() {
                                return (out, true);
                            }
                            let Some(Ok(ba)) = anchors.get(usize::from(*class)) else { continue };
                            if put(&mut out, format!("mark {} {}", name(names, b), name(names, *m)), anchor(&ba, store, coords), *ma) {
                                return (out, true);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    (out, false)
}
