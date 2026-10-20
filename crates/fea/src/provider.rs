//! Generated lookups ([`GeneratedLookups`]) checked and handed to fea-rs's feature writer
//! interface, which places them at the code's `# Automatic Code` markers or after the
//! hand-written lookups, as fontc does.
//!
//! fea-rs merges generated `GPOS` lookups only for `curs`, `kern`, `dist`, `abvm`, `blwm`,
//! `mark` and `mkmk`, and it panics when one lookup is claimed by two of its merge groups
//! (`curs`; `kern` with `dist`; each mark feature on its own), when a mark builder gets a base
//! before any mark of its class, and when a builder with no rules is built. [`check`] refuses
//! the first two; [`Provider`] leaves out lookups without rules.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use fea_rs::GlyphSet;
use fea_rs::compile::{FeatureBuilder, FeatureKey as FeaKey, FeatureProvider, LookupId, PendingLookup};
use write_fonts_052::tables::gpos::builders::{
    AnchorBuilder, CursivePosBuilder, MarkToBaseBuilder, MarkToLigBuilder, MarkToMarkBuilder, PairPosBuilder, ValueRecordBuilder,
};
use write_fonts_052::tables::gsub::Gsub;
use write_fonts_052::tables::gsub::builders::SingleSubBuilder;
use write_fonts_052::tables::layout::builders::{CaretValueBuilder, DeviceOrDeltas};
use write_fonts_052::tables::layout::{Condition as RawCondition, ConditionFormat1, ConditionSet, FeatureVariationRecord, LookupFlag};
use write_fonts_052::tables::variations::{RegionAxisCoordinates, VariationRegion};
use write_fonts_052::types::F2Dot14;

use crate::{Anchor, Condition, Diagnostic, GeneratedLookup, GeneratedLookups, GeneratedRules, GlyphId16, LookupFlags, Metric, Severity, Tag, Value};

/// Most distinct mark filtering sets the generated lookups may use; the code's own sets and
/// these share one 16-bit index space.
const MAX_FILTER_SETS: usize = 4_096;

/// Which merge group of fea-rs a generated feature belongs to; `None` for a tag fea-rs does
/// not merge.
fn merge_group(tag: Tag) -> Option<&'static str> {
    match &tag.to_be_bytes() {
        b"curs" => Some("curs"),
        b"kern" | b"dist" => Some("kern"),
        b"abvm" => Some("abvm"),
        b"blwm" => Some("blwm"),
        b"mark" => Some("mark"),
        b"mkmk" => Some("mkmk"),
        _ => None,
    }
}

/// Refuses generated lookups fea-rs cannot take: glyphs outside the font, values that need a
/// variable font, features it does not merge, a lookup shared between merge groups, counts
/// over the limits.
pub(crate) fn check(generated: &GeneratedLookups, glyph_count: usize, rules_limit: usize, ligature_components: usize) -> Result<(), Diagnostic> {
    let refuse = |m: String| Err(Diagnostic::request(Severity::Error, m));
    if generated.lookups.len() > crate::MAX_GENERATED_LOOKUPS {
        return refuse(format!("at most {} generated lookups are allowed", crate::MAX_GENERATED_LOOKUPS));
    }
    let rules_limit = rules_limit.min(crate::MAX_GENERATED_RULES);
    if generated.rule_count() > rules_limit {
        return refuse(format!("the generated lookups have more than {rules_limit} rules"));
    }
    if let Some(problem) = feature_variation_problem(generated, glyph_count) {
        return refuse(problem);
    }
    let mut owner: BTreeMap<usize, &'static str> = BTreeMap::new();
    for (key, indices) in &generated.features {
        let Some(group) = merge_group(key.feature) else {
            return refuse(format!("the feature `{}` cannot hold generated lookups", key.feature));
        };
        for &i in indices {
            if i >= generated.lookups.len() {
                return refuse(format!("the feature `{}` names generated lookup {i}, which does not exist", key.feature));
            }
            if owner.insert(i, group).is_some_and(|g| g != group) {
                return refuse(format!("generated lookup {i} is used by features that are merged separately"));
            }
        }
    }
    let filter_sets: HashSet<&Vec<GlyphId16>> = generated.lookups.iter().filter_map(|l| l.filter_set.as_ref()).collect();
    if filter_sets.len() > MAX_FILTER_SETS {
        return refuse(format!("the generated lookups use more than {MAX_FILTER_SETS} mark filtering sets"));
    }
    let glyph_ok = |g: &GlyphId16| usize::from(g.to_u16()) < glyph_count;
    for (i, lookup) in generated.lookups.iter().enumerate() {
        let glyphs_ok = lookup_glyphs(lookup).all(|g| glyph_ok(&g)) && lookup.filter_set.iter().flatten().all(glyph_ok);
        if !glyphs_ok {
            return refuse(format!("generated lookup {i} names a glyph the font does not have"));
        }
        if let Some(problem) = lookup_metrics(lookup).find_map(|m| metric_problem(m, &generated.axes)) {
            return refuse(format!("generated lookup {i}: {problem}"));
        }
        if let GeneratedRules::MarkToLigature { ligatures, .. } = &lookup.rules
            && ligatures.iter().any(|(_, c)| c.len() > ligature_components)
        {
            return refuse(format!("a generated ligature may have at most {ligature_components} components"));
        }
    }
    let carets_ok = generated.carets.iter().all(|(g, c)| glyph_ok(g) && c.len() <= ligature_components && !c.iter().any(Metric::is_variable));
    if generated.axes.len() > crate::MAX_AXES {
        return refuse(format!("a variable font may have at most {} axes", crate::MAX_AXES));
    }
    if !carets_ok || !generated.glyph_classes.keys().all(glyph_ok) {
        return refuse("the generated glyph classes or carets name a glyph the font does not have, or carets need a variable font".into());
    }
    Ok(())
}

/// Why fea-rs cannot take the feature variations of `generated`, or `None` when it can: they
/// need a variable font, conditions on its axes with finite ranges within -1 to 1 and at most
/// one per axis, glyphs of the font, and at most [`crate::MAX_FEATURE_VARIATIONS`] records.
fn feature_variation_problem(generated: &GeneratedLookups, glyph_count: usize) -> Option<String> {
    let variations = &generated.feature_variations;
    if variations.is_empty() {
        return None;
    }
    if generated.axes.is_empty() {
        return Some("feature variations need a variable font".into());
    }
    if variations.len() > crate::MAX_FEATURE_VARIATIONS {
        return Some(format!("at most {} feature variations are allowed", crate::MAX_FEATURE_VARIATIONS));
    }
    for v in variations {
        let mut seen: Vec<Tag> = Vec::with_capacity(v.conditions.len());
        for c in &v.conditions {
            if !generated.axes.contains(&c.axis) || seen.contains(&c.axis) {
                return Some(format!("a feature variation names the axis `{}` twice or one the font does not have", c.axis));
            }
            seen.push(c.axis);
            let ok = c.min.is_finite() && c.max.is_finite() && (-1.0..=1.0).contains(&c.min) && (-1.0..=1.0).contains(&c.max) && c.min <= c.max;
            if !ok {
                return Some("a feature variation condition is not a range within -1 to 1".into());
            }
        }
        if v.substitutions.iter().any(|(a, b)| usize::from(a.to_u16()) >= glyph_count || usize::from(b.to_u16()) >= glyph_count) {
            return Some("a feature variation substitutes a glyph the font does not have".into());
        }
    }
    None
}

/// The condition set of `conditions`, axes as indices into `axes` (in `fvar` order), sorted by
/// axis index. [`feature_variation_problem`] has checked every axis is one of `axes`.
fn condition_set(conditions: &[Condition], axes: &[Tag]) -> ConditionSet {
    let mut indexed: Vec<(u16, F2Dot14, F2Dot14)> = conditions
        .iter()
        .filter_map(|c| {
            let axis = u16::try_from(axes.iter().position(|a| *a == c.axis)?).ok()?;
            Some((axis, F2Dot14::from_f64(c.min), F2Dot14::from_f64(c.max)))
        })
        .collect();
    indexed.sort_by_key(|c| c.0);
    ConditionSet::new(indexed.into_iter().map(|(axis, min, max)| ConditionFormat1::new(axis, min, max).into()).collect())
}

/// What orders feature variation records: each condition's axis index and range.
fn condition_key(set: &ConditionSet) -> Vec<(u16, i16, i16)> {
    set.conditions
        .iter()
        .map(|c| match &**c {
            RawCondition::Format1AxisRange(r) => (r.axis_index, r.filter_range_min_value.to_bits(), r.filter_range_max_value.to_bits()),
            _ => (u16::MAX, 0, 0),
        })
        .collect()
}

/// Adds the feature variations: one single-substitution lookup per distinct substitution list,
/// at the front of the lookup list when the feature is `rvrn` alone (it runs before every other
/// feature), and the records under every tag the variations name.
fn add_feature_variations(builder: &mut FeatureBuilder, generated: &GeneratedLookups) {
    let mut tags: Vec<Tag> = generated.feature_variations.iter().map(|v| v.tag).collect();
    tags.sort();
    tags.dedup();
    let front = tags == [Tag::new(b"rvrn")];
    let mut lookups: BTreeMap<Vec<(GlyphId16, GlyphId16)>, LookupId> = BTreeMap::new();
    let mut conditions: Vec<(ConditionSet, Vec<LookupId>)> = Vec::with_capacity(generated.feature_variations.len());
    for v in &generated.feature_variations {
        let mut subs = v.substitutions.clone();
        subs.sort();
        subs.dedup_by_key(|s| s.0);
        let mut ids = Vec::new();
        if !subs.is_empty() {
            let id = match lookups.get(&subs) {
                Some(id) => *id,
                None => {
                    let mut sub = SingleSubBuilder::default();
                    for (from, to) in &subs {
                        sub.insert(*from, *to);
                    }
                    let id = builder.add_lookup(PendingLookup::new(vec![sub], LookupFlag::empty(), None).at_front_of_list(front));
                    lookups.insert(subs, id);
                    id
                }
            };
            ids.push(id);
        }
        let set = condition_set(&v.conditions, &generated.axes);
        // A record under several features comes as one entry per feature, side by side.
        if conditions.last().is_some_and(|(last, last_ids)| *last == set && *last_ids == ids) {
            continue;
        }
        conditions.push((set, ids));
    }
    builder.add_feature_variations(tags, conditions);
}

/// Puts the records of `gsub`'s feature variations in the order of `generated`'s: the first record
/// whose conditions hold is the one applied, and fea-rs keeps its records unordered. Records the
/// code made go after.
pub(crate) fn order_feature_variations(gsub: &mut Gsub, generated: &GeneratedLookups) {
    let Some(fv) = gsub.feature_variations.as_mut() else { return };
    let order: Vec<Vec<(u16, i16, i16)>> =
        generated.feature_variations.iter().map(|v| condition_key(&condition_set(&v.conditions, &generated.axes))).collect();
    let rank = |r: &FeatureVariationRecord| {
        let key = r.condition_set.as_ref().map(condition_key).unwrap_or_default();
        order.iter().position(|k| *k == key).unwrap_or(usize::MAX)
    };
    fv.feature_variation_records.sort_by_cached_key(rank);
}

fn lookup_glyphs(lookup: &GeneratedLookup) -> Box<dyn Iterator<Item = GlyphId16> + '_> {
    match &lookup.rules {
        GeneratedRules::Pair { glyph_pairs, class_pairs } => Box::new(
            glyph_pairs.iter().flat_map(|p| [p.left, p.right]).chain(class_pairs.iter().flat_map(|p| p.left.iter().chain(&p.right).copied())),
        ),
        GeneratedRules::MarkToBase { marks, bases } | GeneratedRules::MarkToMark { marks, bases } => {
            Box::new(marks.iter().chain(bases).map(|(g, _)| *g))
        }
        GeneratedRules::MarkToLigature { marks, ligatures } => Box::new(marks.iter().map(|(g, _)| *g).chain(ligatures.iter().map(|(g, _)| *g))),
        GeneratedRules::Cursive { glyphs } => Box::new(glyphs.iter().map(|(g, _, _)| *g)),
    }
}

/// Why fea-rs cannot take `m`'s deltas in a font with `axes`, or `None` when it can: deltas need
/// a variable font, regions must name its axes with finite tents within -1 to 1, and the count
/// is capped.
fn metric_problem(m: &Metric, axes: &[Tag]) -> Option<String> {
    if !m.is_variable() {
        return None;
    }
    if axes.is_empty() {
        return Some("values with deltas need a variable font".into());
    }
    if m.deltas.len() > crate::MAX_DELTAS {
        return Some(format!("a value has more than {} deltas", crate::MAX_DELTAS));
    }
    for (region, _) in &m.deltas {
        if region.0.len() > axes.len() {
            return Some("a region has more axes than the font".into());
        }
        for a in &region.0 {
            if !axes.contains(&a.axis) {
                return Some(format!("a region names the axis `{}`, which the font does not have", a.axis));
            }
            let ok = [a.start, a.peak, a.end].iter().all(|v| v.is_finite() && (-1.0..=1.0).contains(v)) && a.start <= a.peak && a.peak <= a.end;
            if !ok {
                return Some("a region's tent is not within -1 to 1 or not in order".into());
            }
        }
    }
    None
}

fn lookup_metrics(lookup: &GeneratedLookup) -> Box<dyn Iterator<Item = &Metric> + '_> {
    fn value(v: &Value) -> impl Iterator<Item = &Metric> {
        [&v.x_placement, &v.y_placement, &v.x_advance, &v.y_advance].into_iter().flatten()
    }
    fn anchor(a: &Anchor) -> impl Iterator<Item = &Metric> {
        [&a.x, &a.y].into_iter()
    }
    match &lookup.rules {
        GeneratedRules::Pair { glyph_pairs, class_pairs } => {
            Box::new(glyph_pairs.iter().flat_map(|p| value(&p.value)).chain(class_pairs.iter().flat_map(|p| value(&p.value))))
        }
        GeneratedRules::MarkToBase { marks, bases } | GeneratedRules::MarkToMark { marks, bases } => {
            Box::new(marks.iter().chain(bases).flat_map(|(_, a)| anchor(a)))
        }
        GeneratedRules::MarkToLigature { marks, ligatures } => {
            Box::new(marks.iter().flat_map(|(_, a)| anchor(a)).chain(ligatures.iter().flat_map(|(_, c)| c.iter().flatten().flat_map(anchor))))
        }
        GeneratedRules::Cursive { glyphs } => Box::new(glyphs.iter().flat_map(|(_, e, x)| e.iter().chain(x).flat_map(anchor))),
    }
}

/// The feature writer fea-rs calls once per compile. Only built from lookups [`check`] passed.
pub(crate) struct Provider<'a> {
    pub generated: &'a GeneratedLookups,
}

impl FeatureProvider for Provider<'_> {
    fn add_features(&self, builder: &mut FeatureBuilder) {
        let generated = self.generated;
        let ids: Vec<Option<LookupId>> = generated.lookups.iter().map(|l| add_lookup(builder, l, &generated.axes)).collect();
        for (key, indices) in &generated.features {
            let lookups: Vec<LookupId> = indices.iter().filter_map(|&i| ids.get(i).copied().flatten()).collect::<BTreeSet<_>>().into_iter().collect();
            if !lookups.is_empty() {
                builder.add_feature(FeaKey::new(key.feature, key.language, key.script), lookups);
            }
        }
        for &tag in &generated.append {
            builder.force_append(tag);
        }
        if !generated.feature_variations.is_empty() {
            add_feature_variations(builder, generated);
        }
        if builder.gdef().is_none() && !generated.carets.is_empty() {
            let carets = generated
                .carets
                .iter()
                .map(|(g, c)| (*g, c.iter().map(|m| CaretValueBuilder::Coordinate { default: m.default, deltas: DeviceOrDeltas::None }).collect()))
                .collect();
            builder.add_lig_carets(carets);
        }
    }
}

/// Adds one lookup; `None` when it has no rules, which fea-rs cannot build.
fn add_lookup(builder: &mut FeatureBuilder, lookup: &GeneratedLookup, axes: &[Tag]) -> Option<LookupId> {
    let value = |v: &Value| value(v, axes);
    let anchor = |a: &Anchor| anchor(a, axes);
    let mut bits = lookup.flags.0;
    if lookup.filter_set.is_some() {
        bits |= LookupFlags::USE_MARK_FILTERING_SET;
    }
    let flags = LookupFlag::from_bits_truncate(bits);
    let filter: Option<GlyphSet> = lookup.filter_set.as_ref().map(|s| s.iter().copied().collect());
    let class = lookup.class.as_str();
    match &lookup.rules {
        GeneratedRules::Pair { glyph_pairs, class_pairs } => {
            let mut sub = PairPosBuilder::default();
            for p in glyph_pairs {
                sub.insert_pair(p.left, value(&p.value), p.right, ValueRecordBuilder::new());
            }
            for p in class_pairs.iter().filter(|p| !p.left.is_empty() && !p.right.is_empty()) {
                sub.insert_classes(p.left.iter().copied().collect(), value(&p.value), p.right.iter().copied().collect(), ValueRecordBuilder::new());
            }
            (!sub.is_empty()).then(|| builder.add_lookup(PendingLookup::new(vec![sub], flags, filter)))
        }
        GeneratedRules::MarkToBase { marks, bases } => {
            if marks.is_empty() || bases.is_empty() {
                return None;
            }
            let mut sub = MarkToBaseBuilder::default();
            for (g, a) in marks {
                // A mark already in the lookup keeps its first anchor, as the mark writers do.
                let _ = sub.insert_mark(*g, class, anchor(a));
            }
            for (g, a) in bases {
                sub.insert_base(*g, class, anchor(a));
            }
            Some(builder.add_lookup(PendingLookup::new(vec![sub], flags, filter)))
        }
        GeneratedRules::MarkToMark { marks, bases } => {
            if marks.is_empty() || bases.is_empty() {
                return None;
            }
            let mut sub = MarkToMarkBuilder::default();
            for (g, a) in marks {
                let _ = sub.insert_mark1(*g, class, anchor(a));
            }
            for (g, a) in bases {
                sub.insert_mark2(*g, class, anchor(a));
            }
            Some(builder.add_lookup(PendingLookup::new(vec![sub], flags, filter)))
        }
        GeneratedRules::MarkToLigature { marks, ligatures } => {
            if marks.is_empty() || ligatures.is_empty() {
                return None;
            }
            let mut sub = MarkToLigBuilder::default();
            for (g, a) in marks {
                let _ = sub.insert_mark(*g, class, anchor(a));
            }
            for (g, components) in ligatures {
                let components = components.iter().map(|c| c.iter().map(|a| (class.to_string(), anchor(a))).collect()).collect();
                sub.add_ligature_components_directly(*g, components);
            }
            Some(builder.add_lookup(PendingLookup::new(vec![sub], flags, filter)))
        }
        GeneratedRules::Cursive { glyphs } => {
            let mut sub = CursivePosBuilder::default();
            for (g, entry, exit) in glyphs {
                sub.insert(*g, entry.as_ref().map(anchor), exit.as_ref().map(anchor));
            }
            (!sub.is_empty()).then(|| builder.add_lookup(PendingLookup::new(vec![sub], flags, filter)))
        }
    }
}

fn value(v: &Value, axes: &[Tag]) -> ValueRecordBuilder {
    let mut r = ValueRecordBuilder::new();
    if let Some(m) = &v.x_placement {
        r = r.with_x_placement(m.default);
        if let Some(d) = deltas(m, axes) {
            r = r.with_x_placement_device(d);
        }
    }
    if let Some(m) = &v.y_placement {
        r = r.with_y_placement(m.default);
        if let Some(d) = deltas(m, axes) {
            r = r.with_y_placement_device(d);
        }
    }
    if let Some(m) = &v.x_advance {
        r = r.with_x_advance(m.default);
        if let Some(d) = deltas(m, axes) {
            r = r.with_x_advance_device(d);
        }
    }
    if let Some(m) = &v.y_advance {
        r = r.with_y_advance(m.default);
        if let Some(d) = deltas(m, axes) {
            r = r.with_y_advance_device(d);
        }
    }
    r
}

fn anchor(a: &Anchor, axes: &[Tag]) -> AnchorBuilder {
    let mut b = AnchorBuilder::new(a.x.default, a.y.default);
    if let Some(d) = deltas(&a.x, axes) {
        b = b.with_x_device(d);
    }
    if let Some(d) = deltas(&a.y, axes) {
        b = b.with_y_device(d);
    }
    b
}

/// `m`'s deltas with full regions over `axes` (axes a region does not list are unconstrained),
/// for fea-rs to add to the font's variation store; `None` for a value without deltas.
pub(crate) fn deltas(m: &Metric, axes: &[Tag]) -> Option<DeviceOrDeltas> {
    if !m.is_variable() {
        return None;
    }
    let f2 = |v: f64| F2Dot14::from_f32(v as f32);
    let deltas = m
        .deltas
        .iter()
        .map(|(region, delta)| {
            let coords = axes
                .iter()
                .map(|tag| match region.0.iter().find(|a| a.axis == *tag) {
                    Some(a) => RegionAxisCoordinates::new(f2(a.start), f2(a.peak), f2(a.end)),
                    None => RegionAxisCoordinates::new(F2Dot14::ZERO, F2Dot14::ZERO, F2Dot14::ZERO),
                })
                .collect();
            (VariationRegion::new(coords), *delta)
        })
        .collect();
    Some(DeviceOrDeltas::Deltas(deltas))
}
