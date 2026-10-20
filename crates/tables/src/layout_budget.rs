//! A size check for `GSUB`, `GPOS` and `GDEF` before they are read into write-fonts' owned
//! types. The owned conversion follows every offset on its own, so a subtable that many offsets
//! share is copied once per offset, and a small table can describe billions of records. The
//! walk below counts what the conversion would build, per reference, and refuses a table that
//! counts more than [`MAX_LAYOUT_ITEMS`]. Coverage and class ranges count every glyph they
//! hold, because the callers expand them.
//!
//! Parts that cannot be read count as one item: the owned conversion skips or defaults them too.
//! `BASE` and `MATH` have their checks in `base_math`.

use read_fonts::tables::gdef::{CaretValue, Gdef};
use read_fonts::tables::gpos::{
    AnchorTable, CursivePosFormat1, ExtensionSubtable as PosExtension, Gpos, MarkArray, MarkBasePosFormat1, MarkLigPosFormat1, MarkMarkPosFormat1,
    PairPos, PositionLookup, SinglePos, ValueRecord,
};
use read_fonts::tables::gsub::{
    AlternateSubstFormat1, ExtensionSubtable as SubstExtension, Gsub, LigatureSubstFormat1, MultipleSubstFormat1, ReverseChainSingleSubstFormat1,
    SingleSubst, SubstitutionLookup,
};
use read_fonts::tables::layout::{
    ChainedSequenceContext, ClassDef, Condition, CoverageTable, DeviceOrVariationIndex, Feature, FeatureList, FeatureParams, FeatureVariations,
    LangSys, ScriptList, SequenceContext,
};
use read_fonts::tables::variations::ItemVariationStore;
use read_fonts::{FontData, FontRead, ReadError};

use crate::limits::{MAX_LAYOUT_ITEMS, MAX_LOOKUP_NESTING};
use crate::{Result, TablesError};

mod base_math;
pub(crate) use base_math::{check_base, check_math};

/// The value-format bits of the four device offsets.
const DEVICE_BITS: u16 = 0x00F0;

struct Walk {
    tag: &'static str,
    limit: usize,
    left: usize,
    depth: usize,
}

impl Walk {
    fn new(tag: &'static str, limit: usize) -> Self {
        Walk { tag, limit, left: limit, depth: 0 }
    }

    fn charge(&mut self, n: usize) -> Result<()> {
        match self.left.checked_sub(n) {
            Some(left) => {
                self.left = left;
                Ok(())
            }
            None => {
                Err(TablesError::invalid(format!("the '{}' table is too large to rewrite: it expands to more than {} records", self.tag, self.limit)))
            }
        }
    }
}

/// Refuses a `GSUB` table whose owned form would hold more than [`MAX_LAYOUT_ITEMS`] records.
pub(crate) fn check_gsub(data: &[u8]) -> Result<()> {
    check_gsub_within(data, MAX_LAYOUT_ITEMS)
}

/// Refuses a `GPOS` table whose owned form would hold more than [`MAX_LAYOUT_ITEMS`] records.
pub(crate) fn check_gpos(data: &[u8]) -> Result<()> {
    check_gpos_within(data, MAX_LAYOUT_ITEMS)
}

/// Refuses a `GDEF` table whose owned form would hold more than [`MAX_LAYOUT_ITEMS`] records.
pub(crate) fn check_gdef(data: &[u8]) -> Result<()> {
    let mut w = Walk::new("GDEF", MAX_LAYOUT_ITEMS);
    let Ok(t) = Gdef::read(FontData::new(data)) else { return Ok(()) };
    if let Some(Ok(c)) = t.glyph_class_def() {
        class_def(&mut w, &c)?;
    }
    if let Some(Ok(c)) = t.mark_attach_class_def() {
        class_def(&mut w, &c)?;
    }
    if let Some(Ok(list)) = t.attach_list() {
        coverage_res(&mut w, list.coverage())?;
        let points = list.attach_points();
        w.charge(points.len())?;
        for p in points.iter().flatten() {
            w.charge(p.point_indices().len())?;
        }
    }
    if let Some(Ok(list)) = t.lig_caret_list() {
        coverage_res(&mut w, list.coverage())?;
        let ligs = list.lig_glyphs();
        w.charge(ligs.len())?;
        for lig in ligs.iter().flatten() {
            let carets = lig.caret_values();
            w.charge(carets.len())?;
            for caret in carets.iter().flatten() {
                if let CaretValue::Format3(c) = caret {
                    device_res(&mut w, c.device())?;
                }
            }
        }
    }
    if let Some(Ok(sets)) = t.mark_glyph_sets_def() {
        let covs = sets.coverages();
        w.charge(covs.len())?;
        for c in covs.iter().flatten() {
            coverage(&mut w, &c)?;
        }
    }
    if let Some(Ok(store)) = t.item_var_store() {
        variation_store(&mut w, &store)?;
    }
    Ok(())
}

/// Refuses `GPOS` tables whose owned forms would hold more than `limit` records together.
pub(crate) fn check_gpos_together<'a>(tables: impl IntoIterator<Item = &'a [u8]>, limit: usize) -> Result<()> {
    let mut w = Walk::new("GPOS", limit);
    for data in tables {
        gpos_walk(&mut w, data)?;
    }
    Ok(())
}

fn check_gpos_within(data: &[u8], limit: usize) -> Result<()> {
    gpos_walk(&mut Walk::new("GPOS", limit), data)
}

fn gpos_walk(w: &mut Walk, data: &[u8]) -> Result<()> {
    let Ok(t) = Gpos::read(FontData::new(data)) else { return Ok(()) };
    if let Ok(list) = t.script_list() {
        script_list(w, &list)?;
    }
    if let Ok(list) = t.feature_list() {
        feature_list(w, &list)?;
    }
    if let Some(Ok(fv)) = t.feature_variations() {
        feature_variations(w, &fv)?;
    }
    if let Ok(list) = t.lookup_list() {
        let lookups = list.lookups();
        w.charge(lookups.len())?;
        for lookup in lookups.iter().flatten() {
            position_lookup(w, &lookup)?;
        }
    }
    Ok(())
}

fn check_gsub_within(data: &[u8], limit: usize) -> Result<()> {
    let mut w = Walk::new("GSUB", limit);
    let Ok(t) = Gsub::read(FontData::new(data)) else { return Ok(()) };
    if let Ok(list) = t.script_list() {
        script_list(&mut w, &list)?;
    }
    if let Ok(list) = t.feature_list() {
        feature_list(&mut w, &list)?;
    }
    if let Some(Ok(fv)) = t.feature_variations() {
        feature_variations(&mut w, &fv)?;
    }
    if let Ok(list) = t.lookup_list() {
        let lookups = list.lookups();
        w.charge(lookups.len())?;
        for lookup in lookups.iter().flatten() {
            substitution_lookup(&mut w, &lookup)?;
        }
    }
    Ok(())
}

// ── Shared parts ──

fn script_list(w: &mut Walk, list: &ScriptList) -> Result<()> {
    let records = list.script_records();
    w.charge(records.len())?;
    for r in records {
        let Ok(script) = r.script(list.offset_data()) else { continue };
        if let Some(Ok(ls)) = script.default_lang_sys() {
            lang_sys(w, &ls)?;
        }
        let records = script.lang_sys_records();
        w.charge(records.len())?;
        for r in records {
            if let Ok(ls) = r.lang_sys(script.offset_data()) {
                lang_sys(w, &ls)?;
            }
        }
    }
    Ok(())
}

fn lang_sys(w: &mut Walk, ls: &LangSys) -> Result<()> {
    w.charge(1 + ls.feature_indices().len())
}

fn feature_list(w: &mut Walk, list: &FeatureList) -> Result<()> {
    let records = list.feature_records();
    w.charge(records.len())?;
    for r in records {
        if let Ok(f) = r.feature(list.offset_data()) {
            feature(w, &f)?;
        }
    }
    Ok(())
}

fn feature(w: &mut Walk, f: &Feature) -> Result<()> {
    w.charge(1 + f.lookup_list_indices().len())?;
    if let Some(Ok(FeatureParams::CharacterVariant(p))) = f.feature_params() {
        w.charge(p.character().len())?;
    }
    Ok(())
}

fn feature_variations(w: &mut Walk, fv: &FeatureVariations) -> Result<()> {
    let records = fv.feature_variation_records();
    w.charge(records.len())?;
    for r in records {
        if let Some(Ok(set)) = r.condition_set(fv.offset_data()) {
            let conditions = set.conditions();
            w.charge(conditions.len())?;
            for c in conditions.iter().flatten() {
                condition(w, &c)?;
            }
        }
        if let Some(Ok(subst)) = r.feature_table_substitution(fv.offset_data()) {
            let records = subst.substitutions();
            w.charge(records.len())?;
            for r in records {
                if let Ok(f) = r.alternate_feature(subst.offset_data()) {
                    feature(w, &f)?;
                }
            }
        }
    }
    Ok(())
}

fn condition(w: &mut Walk, c: &Condition) -> Result<()> {
    w.charge(1)?;
    if w.depth >= MAX_LOOKUP_NESTING {
        return Err(TablesError::invalid(format!("the '{}' table nests its conditions more than {MAX_LOOKUP_NESTING} deep", w.tag)));
    }
    w.depth += 1;
    let result = match c {
        Condition::Format3And(c) => conditions(w, c.conditions().iter()),
        Condition::Format4Or(c) => conditions(w, c.conditions().iter()),
        Condition::Format5Negate(c) => conditions(w, std::iter::once(c.condition())),
        Condition::Format1AxisRange(_) | Condition::Format2VariableValue(_) => Ok(()),
    };
    w.depth = w.depth.saturating_sub(1);
    result
}

fn conditions<'a>(w: &mut Walk, items: impl Iterator<Item = std::result::Result<Condition<'a>, ReadError>>) -> Result<()> {
    for c in items {
        w.charge(1)?;
        if let Ok(c) = c {
            condition(w, &c)?;
        }
    }
    Ok(())
}

fn coverage(w: &mut Walk, c: &CoverageTable) -> Result<()> {
    let records = match c {
        CoverageTable::Format1(f) => f.glyph_array().len(),
        CoverageTable::Format2(f) => f.range_records().len(),
    };
    w.charge(1 + records)?;
    w.charge(c.population())
}

fn coverage_res(w: &mut Walk, c: std::result::Result<CoverageTable, ReadError>) -> Result<()> {
    match c {
        Ok(c) => coverage(w, &c),
        Err(_) => w.charge(1),
    }
}

fn class_def(w: &mut Walk, c: &ClassDef) -> Result<()> {
    let records = match c {
        ClassDef::Format1(f) => f.class_value_array().len(),
        ClassDef::Format2(f) => f.class_range_records().len(),
    };
    w.charge(1 + records)?;
    w.charge(c.population())
}

fn class_def_res(w: &mut Walk, c: std::result::Result<ClassDef, ReadError>) -> Result<()> {
    match c {
        Ok(c) => class_def(w, &c),
        Err(_) => w.charge(1),
    }
}

fn device(w: &mut Walk, d: &DeviceOrVariationIndex) -> Result<()> {
    match d {
        DeviceOrVariationIndex::Device(d) => w.charge(1 + usize::from(d.end_size().saturating_sub(d.start_size()))),
        DeviceOrVariationIndex::VariationIndex(_) => w.charge(1),
    }
}

fn device_res(w: &mut Walk, d: std::result::Result<DeviceOrVariationIndex, ReadError>) -> Result<()> {
    match d {
        Ok(d) => device(w, &d),
        Err(_) => w.charge(1),
    }
}

fn devices<'a>(w: &mut Walk, items: [Option<std::result::Result<DeviceOrVariationIndex<'a>, ReadError>>; 4]) -> Result<()> {
    for d in items.into_iter().flatten() {
        device_res(w, d)?;
    }
    Ok(())
}

fn variation_store(w: &mut Walk, store: &ItemVariationStore) -> Result<()> {
    if let Ok(regions) = store.variation_region_list() {
        w.charge(1 + usize::from(regions.region_count()).saturating_mul(usize::from(regions.axis_count())))?;
    }
    let data = store.item_variation_data();
    w.charge(data.len())?;
    for d in data.iter().flatten().flatten() {
        let items = usize::from(d.item_count());
        w.charge(1 + items + d.region_indexes().len())?;
        w.charge(items.saturating_mul(usize::from(d.region_index_count())))?;
    }
    Ok(())
}

fn sequence_context(w: &mut Walk, c: &SequenceContext) -> Result<()> {
    match c {
        SequenceContext::Format1(f) => {
            coverage_res(w, f.coverage())?;
            let sets = f.seq_rule_sets();
            w.charge(sets.len())?;
            for set in sets.iter().flatten().flatten() {
                let rules = set.seq_rules();
                w.charge(rules.len())?;
                for r in rules.iter().flatten() {
                    w.charge(1 + r.input_sequence().len() + r.seq_lookup_records().len())?;
                }
            }
        }
        SequenceContext::Format2(f) => {
            coverage_res(w, f.coverage())?;
            class_def_res(w, f.class_def())?;
            let sets = f.class_seq_rule_sets();
            w.charge(sets.len())?;
            for set in sets.iter().flatten().flatten() {
                let rules = set.class_seq_rules();
                w.charge(rules.len())?;
                for r in rules.iter().flatten() {
                    w.charge(1 + r.input_sequence().len() + r.seq_lookup_records().len())?;
                }
            }
        }
        SequenceContext::Format3(f) => {
            let covs = f.coverages();
            w.charge(1 + covs.len() + f.seq_lookup_records().len())?;
            for c in covs.iter().flatten() {
                coverage(w, &c)?;
            }
        }
    }
    Ok(())
}

fn chain_context(w: &mut Walk, c: &ChainedSequenceContext) -> Result<()> {
    match c {
        ChainedSequenceContext::Format1(f) => {
            coverage_res(w, f.coverage())?;
            let sets = f.chained_seq_rule_sets();
            w.charge(sets.len())?;
            for set in sets.iter().flatten().flatten() {
                let rules = set.chained_seq_rules();
                w.charge(rules.len())?;
                for r in rules.iter().flatten() {
                    let glyphs = r.backtrack_sequence().len() + r.input_sequence().len() + r.lookahead_sequence().len();
                    w.charge(1 + glyphs + r.seq_lookup_records().len())?;
                }
            }
        }
        ChainedSequenceContext::Format2(f) => {
            coverage_res(w, f.coverage())?;
            class_def_res(w, f.backtrack_class_def())?;
            class_def_res(w, f.input_class_def())?;
            class_def_res(w, f.lookahead_class_def())?;
            let sets = f.chained_class_seq_rule_sets();
            w.charge(sets.len())?;
            for set in sets.iter().flatten().flatten() {
                let rules = set.chained_class_seq_rules();
                w.charge(rules.len())?;
                for r in rules.iter().flatten() {
                    let glyphs = r.backtrack_sequence().len() + r.input_sequence().len() + r.lookahead_sequence().len();
                    w.charge(1 + glyphs + r.seq_lookup_records().len())?;
                }
            }
        }
        ChainedSequenceContext::Format3(f) => {
            w.charge(1 + f.seq_lookup_records().len())?;
            for covs in [f.backtrack_coverages(), f.input_coverages(), f.lookahead_coverages()] {
                w.charge(covs.len())?;
                for c in covs.iter().flatten() {
                    coverage(w, &c)?;
                }
            }
        }
    }
    Ok(())
}

// ── GPOS ──

fn position_lookup(w: &mut Walk, lookup: &PositionLookup) -> Result<()> {
    w.charge(1)?;
    match lookup {
        PositionLookup::Single(l) => each(w, l.subtables().len(), l.subtables().iter(), single_pos),
        PositionLookup::Pair(l) => each(w, l.subtables().len(), l.subtables().iter(), pair_pos),
        PositionLookup::Cursive(l) => each(w, l.subtables().len(), l.subtables().iter(), cursive),
        PositionLookup::MarkToBase(l) => each(w, l.subtables().len(), l.subtables().iter(), mark_base),
        PositionLookup::MarkToLig(l) => each(w, l.subtables().len(), l.subtables().iter(), mark_lig),
        PositionLookup::MarkToMark(l) => each(w, l.subtables().len(), l.subtables().iter(), mark_mark),
        PositionLookup::Contextual(l) => each(w, l.subtables().len(), l.subtables().iter(), sequence_context),
        PositionLookup::ChainContextual(l) => each(w, l.subtables().len(), l.subtables().iter(), chain_context),
        PositionLookup::Extension(l) => each(w, l.subtables().len(), l.subtables().iter(), position_extension),
    }
}

/// Charges an array of subtable offsets, then walks every subtable that reads.
fn each<T>(
    w: &mut Walk,
    len: usize,
    items: impl Iterator<Item = std::result::Result<T, ReadError>>,
    walk: fn(&mut Walk, &T) -> Result<()>,
) -> Result<()> {
    w.charge(len)?;
    for item in items.flatten() {
        walk(w, &item)?;
    }
    Ok(())
}

fn position_extension(w: &mut Walk, e: &PosExtension) -> Result<()> {
    w.charge(1)?;
    match e {
        PosExtension::Single(e) => e.extension().map_or(Ok(()), |s| single_pos(w, &s)),
        PosExtension::Pair(e) => e.extension().map_or(Ok(()), |s| pair_pos(w, &s)),
        PosExtension::Cursive(e) => e.extension().map_or(Ok(()), |s| cursive(w, &s)),
        PosExtension::MarkToBase(e) => e.extension().map_or(Ok(()), |s| mark_base(w, &s)),
        PosExtension::MarkToLig(e) => e.extension().map_or(Ok(()), |s| mark_lig(w, &s)),
        PosExtension::MarkToMark(e) => e.extension().map_or(Ok(()), |s| mark_mark(w, &s)),
        PosExtension::Contextual(e) => e.extension().map_or(Ok(()), |s| sequence_context(w, &s)),
        PosExtension::ChainContextual(e) => e.extension().map_or(Ok(()), |s| chain_context(w, &s)),
    }
}

fn value_record(w: &mut Walk, r: &ValueRecord) -> Result<()> {
    w.charge(1)?;
    if r.format().bits() & DEVICE_BITS == 0 {
        return Ok(());
    }
    devices(w, [r.x_placement_device(), r.y_placement_device(), r.x_advance_device(), r.y_advance_device()])
}

fn single_pos(w: &mut Walk, s: &SinglePos) -> Result<()> {
    match s {
        SinglePos::Format1(f) => {
            coverage_res(w, f.coverage())?;
            value_record(w, &f.value_record())
        }
        SinglePos::Format2(f) => {
            coverage_res(w, f.coverage())?;
            let records = f.value_records();
            w.charge(records.len())?;
            if f.value_format().bits() & DEVICE_BITS != 0 {
                for r in records.iter().flatten() {
                    value_record(w, &r)?;
                }
            }
            Ok(())
        }
    }
}

fn pair_pos(w: &mut Walk, p: &PairPos) -> Result<()> {
    match p {
        PairPos::Format1(f) => {
            coverage_res(w, f.coverage())?;
            let devices = (f.value_format1().bits() | f.value_format2().bits()) & DEVICE_BITS != 0;
            let sets = f.pair_sets();
            w.charge(sets.len())?;
            for set in sets.iter().flatten() {
                let records = set.pair_value_records();
                w.charge(1 + records.len())?;
                if devices {
                    for r in records.iter().flatten() {
                        value_record(w, r.value_record1())?;
                        value_record(w, r.value_record2())?;
                    }
                }
            }
            Ok(())
        }
        PairPos::Format2(f) => {
            coverage_res(w, f.coverage())?;
            class_def_res(w, f.class_def1())?;
            class_def_res(w, f.class_def2())?;
            // Zero-size records read as an empty array, so the product counts only records
            // that the owned conversion builds.
            let class1 = f.class1_records();
            w.charge(class1.len().saturating_mul(1 + usize::from(f.class2_count())))?;
            if (f.value_format1().bits() | f.value_format2().bits()) & DEVICE_BITS != 0 {
                for c1 in class1.iter().flatten() {
                    for c2 in c1.class2_records().iter().flatten() {
                        value_record(w, c2.value_record1())?;
                        value_record(w, c2.value_record2())?;
                    }
                }
            }
            Ok(())
        }
    }
}

fn anchor(w: &mut Walk, a: &AnchorTable) -> Result<()> {
    w.charge(1)?;
    if let AnchorTable::Format3(f) = a {
        for d in [f.x_device(), f.y_device()].into_iter().flatten() {
            device_res(w, d)?;
        }
    }
    Ok(())
}

fn anchor_res(w: &mut Walk, a: std::result::Result<AnchorTable, ReadError>) -> Result<()> {
    match a {
        Ok(a) => anchor(w, &a),
        Err(_) => w.charge(1),
    }
}

fn cursive(w: &mut Walk, c: &CursivePosFormat1) -> Result<()> {
    coverage_res(w, c.coverage())?;
    let records = c.entry_exit_record();
    w.charge(records.len())?;
    for r in records {
        for a in [r.entry_anchor(c.offset_data()), r.exit_anchor(c.offset_data())].into_iter().flatten() {
            anchor_res(w, a)?;
        }
    }
    Ok(())
}

fn mark_array(w: &mut Walk, m: std::result::Result<MarkArray, ReadError>) -> Result<()> {
    let Ok(m) = m else { return w.charge(1) };
    let records = m.mark_records();
    w.charge(1 + records.len())?;
    for r in records {
        anchor_res(w, r.mark_anchor(m.offset_data()))?;
    }
    Ok(())
}

fn mark_base(w: &mut Walk, m: &MarkBasePosFormat1) -> Result<()> {
    coverage_res(w, m.mark_coverage())?;
    coverage_res(w, m.base_coverage())?;
    mark_array(w, m.mark_array())?;
    let Ok(bases) = m.base_array() else { return w.charge(1) };
    let records = bases.base_records();
    w.charge(1 + records.len())?;
    for r in records.iter().flatten() {
        let anchors = r.base_anchors(bases.offset_data());
        w.charge(anchors.len())?;
        for a in anchors.iter().flatten() {
            anchor_res(w, a)?;
        }
    }
    Ok(())
}

fn mark_lig(w: &mut Walk, m: &MarkLigPosFormat1) -> Result<()> {
    coverage_res(w, m.mark_coverage())?;
    coverage_res(w, m.ligature_coverage())?;
    mark_array(w, m.mark_array())?;
    let Ok(ligs) = m.ligature_array() else { return w.charge(1) };
    let attaches = ligs.ligature_attaches();
    w.charge(1 + attaches.len())?;
    for attach in attaches.iter().flatten() {
        let components = attach.component_records();
        w.charge(1 + components.len())?;
        for c in components.iter().flatten() {
            let anchors = c.ligature_anchors(attach.offset_data());
            w.charge(anchors.len())?;
            for a in anchors.iter().flatten() {
                anchor_res(w, a)?;
            }
        }
    }
    Ok(())
}

fn mark_mark(w: &mut Walk, m: &MarkMarkPosFormat1) -> Result<()> {
    coverage_res(w, m.mark1_coverage())?;
    coverage_res(w, m.mark2_coverage())?;
    mark_array(w, m.mark1_array())?;
    let Ok(marks) = m.mark2_array() else { return w.charge(1) };
    let records = marks.mark2_records();
    w.charge(1 + records.len())?;
    for r in records.iter().flatten() {
        let anchors = r.mark2_anchors(marks.offset_data());
        w.charge(anchors.len())?;
        for a in anchors.iter().flatten() {
            anchor_res(w, a)?;
        }
    }
    Ok(())
}

// ── GSUB ──

fn substitution_lookup(w: &mut Walk, lookup: &SubstitutionLookup) -> Result<()> {
    w.charge(1)?;
    match lookup {
        SubstitutionLookup::Single(l) => each(w, l.subtables().len(), l.subtables().iter(), single_subst),
        SubstitutionLookup::Multiple(l) => each(w, l.subtables().len(), l.subtables().iter(), multiple),
        SubstitutionLookup::Alternate(l) => each(w, l.subtables().len(), l.subtables().iter(), alternate),
        SubstitutionLookup::Ligature(l) => each(w, l.subtables().len(), l.subtables().iter(), ligature),
        SubstitutionLookup::Contextual(l) => each(w, l.subtables().len(), l.subtables().iter(), sequence_context),
        SubstitutionLookup::ChainContextual(l) => each(w, l.subtables().len(), l.subtables().iter(), chain_context),
        SubstitutionLookup::Extension(l) => each(w, l.subtables().len(), l.subtables().iter(), substitution_extension),
        SubstitutionLookup::Reverse(l) => each(w, l.subtables().len(), l.subtables().iter(), reverse),
    }
}

fn substitution_extension(w: &mut Walk, e: &SubstExtension) -> Result<()> {
    w.charge(1)?;
    match e {
        SubstExtension::Single(e) => e.extension().map_or(Ok(()), |s| single_subst(w, &s)),
        SubstExtension::Multiple(e) => e.extension().map_or(Ok(()), |s| multiple(w, &s)),
        SubstExtension::Alternate(e) => e.extension().map_or(Ok(()), |s| alternate(w, &s)),
        SubstExtension::Ligature(e) => e.extension().map_or(Ok(()), |s| ligature(w, &s)),
        SubstExtension::Contextual(e) => e.extension().map_or(Ok(()), |s| sequence_context(w, &s)),
        SubstExtension::ChainContextual(e) => e.extension().map_or(Ok(()), |s| chain_context(w, &s)),
        SubstExtension::Reverse(e) => e.extension().map_or(Ok(()), |s| reverse(w, &s)),
    }
}

fn single_subst(w: &mut Walk, s: &SingleSubst) -> Result<()> {
    match s {
        SingleSubst::Format1(f) => coverage_res(w, f.coverage()),
        SingleSubst::Format2(f) => {
            coverage_res(w, f.coverage())?;
            w.charge(f.substitute_glyph_ids().len())
        }
    }
}

fn multiple(w: &mut Walk, s: &MultipleSubstFormat1) -> Result<()> {
    coverage_res(w, s.coverage())?;
    let sequences = s.sequences();
    w.charge(sequences.len())?;
    for seq in sequences.iter().flatten() {
        w.charge(1 + seq.substitute_glyph_ids().len())?;
    }
    Ok(())
}

fn alternate(w: &mut Walk, s: &AlternateSubstFormat1) -> Result<()> {
    coverage_res(w, s.coverage())?;
    let sets = s.alternate_sets();
    w.charge(sets.len())?;
    for set in sets.iter().flatten() {
        w.charge(1 + set.alternate_glyph_ids().len())?;
    }
    Ok(())
}

fn ligature(w: &mut Walk, s: &LigatureSubstFormat1) -> Result<()> {
    coverage_res(w, s.coverage())?;
    let sets = s.ligature_sets();
    w.charge(sets.len())?;
    for set in sets.iter().flatten() {
        let ligatures = set.ligatures();
        w.charge(1 + ligatures.len())?;
        for lig in ligatures.iter().flatten() {
            w.charge(1 + lig.component_glyph_ids().len())?;
        }
    }
    Ok(())
}

fn reverse(w: &mut Walk, s: &ReverseChainSingleSubstFormat1) -> Result<()> {
    coverage_res(w, s.coverage())?;
    w.charge(s.substitute_glyph_ids().len())?;
    for covs in [s.backtrack_coverages(), s.lookahead_coverages()] {
        w.charge(covs.len())?;
        for c in covs.iter().flatten() {
            coverage(w, &c)?;
        }
    }
    Ok(())
}

/// Gives back the class records that the owned conversion loses. read-fonts reads an array of
/// zero-size records as empty, so a `PairPosFormat2` whose two value formats are 0 (as
/// pyftsubset writes a pair table whose values are all 0) comes back without records, and
/// write-fonts refuses to write it. Each such subtable gets one empty record per class pair of
/// its class definitions; the records it gets count against [`MAX_LAYOUT_ITEMS`] for the whole
/// table.
pub(crate) fn restore_empty_pair_records(gpos: &mut write_fonts::tables::gpos::Gpos) -> Result<()> {
    use write_fonts::tables::gpos::{ExtensionSubtable as Ext, PositionLookup as Lookup};
    let mut w = Walk::new("GPOS", MAX_LAYOUT_ITEMS);
    for lookup in gpos.lookup_list.lookups.iter_mut() {
        match &mut **lookup {
            Lookup::Pair(l) => {
                for s in l.subtables.iter_mut() {
                    restore_pair(&mut w, s)?;
                }
            }
            Lookup::Extension(l) => {
                for s in l.subtables.iter_mut() {
                    if let Ext::Pair(e) = &mut **s {
                        restore_pair(&mut w, &mut e.extension)?;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// The classes of `cd`, with class 0, counted as write-fonts counts them for the header.
fn owned_class_count(cd: &write_fonts::tables::layout::ClassDef) -> usize {
    owned_classes(cd).len()
}

/// The distinct classes of `cd`, with class 0, in order.
fn owned_classes(cd: &write_fonts::tables::layout::ClassDef) -> std::collections::BTreeSet<u16> {
    use write_fonts::tables::layout::ClassDef;
    let mut classes = std::collections::BTreeSet::from([0u16]);
    match cd {
        ClassDef::Format1(t) => classes.extend(t.class_value_array.iter().copied()),
        ClassDef::Format2(t) => classes.extend(t.class_range_records.iter().map(|r| r.class)),
    }
    classes
}

fn restore_pair(w: &mut Walk, p: &mut write_fonts::tables::gpos::PairPos) -> Result<()> {
    use write_fonts::tables::gpos::{Class1Record, Class2Record, PairPos as Owned, ValueRecord as Value};
    let Owned::Format2(f) = p else { return Ok(()) };
    if !f.class1_records.is_empty() {
        return fit_pair_classes(w, f);
    }
    let n1 = owned_class_count(&f.class_def1);
    let n2 = owned_class_count(&f.class_def2);
    w.charge(n1.saturating_mul(1 + n2))?;
    let row = Class1Record::new(vec![Class2Record::new(Value::default(), Value::default()); n2]);
    f.class1_records = vec![row; n1];
    Ok(())
}

/// write-fonts writes a `PairPosFormat2` with one class record per distinct class of each class
/// definition (class 0 included) and refuses any other number of records. A subtable whose
/// class counts run past its highest class, or whose classes skip numbers, keeps the records
/// of the classes its definitions use, and those classes are numbered 0, 1, 2… in order.
/// Lookups give the same results. A class with no record is refused.
fn fit_pair_classes(w: &mut Walk, f: &mut write_fonts::tables::gpos::PairPosFormat2) -> Result<()> {
    use write_fonts::tables::gpos::Class1Record;
    let classes1 = owned_classes(&f.class_def1);
    let classes2 = owned_classes(&f.class_def2);
    if f.class1_records.len() == classes1.len() && f.class1_records.iter().all(|r| r.class2_records.len() == classes2.len()) {
        return Ok(());
    }
    w.charge(classes1.len().saturating_mul(1 + classes2.len()))?;
    let missing = |which: u8, class: u16| {
        TablesError::invalid(format!(
            "a class pair adjustment names class {class} of its {}, which has no record",
            if which == 1 { "first glyphs" } else { "second glyphs" }
        ))
    };
    let mut rows = Vec::with_capacity(classes1.len());
    for c1 in &classes1 {
        let row = f.class1_records.get(usize::from(*c1)).ok_or_else(|| missing(1, *c1))?;
        let mut cells = Vec::with_capacity(classes2.len());
        for c2 in &classes2 {
            cells.push(row.class2_records.get(usize::from(*c2)).cloned().ok_or_else(|| missing(2, *c2))?);
        }
        rows.push(Class1Record::new(cells));
    }
    f.class1_records = rows;
    renumber_classes(&mut f.class_def1, &classes1);
    renumber_classes(&mut f.class_def2, &classes2);
    Ok(())
}

/// Numbers the `classes` of `cd` 0, 1, 2… in order.
fn renumber_classes(cd: &mut write_fonts::tables::layout::ClassDef, classes: &std::collections::BTreeSet<u16>) {
    use write_fonts::tables::layout::ClassDef;
    if classes.iter().enumerate().all(|(i, c)| usize::from(*c) == i) {
        return;
    }
    let new: std::collections::HashMap<u16, u16> = classes.iter().zip(0u16..).map(|(c, i)| (*c, i)).collect();
    let map = |c: &mut u16| *c = new.get(c).copied().unwrap_or(0);
    match cd {
        ClassDef::Format1(t) => t.class_value_array.iter_mut().for_each(map),
        ClassDef::Format2(t) => t.class_range_records.iter_mut().for_each(|r| map(&mut r.class)),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    fn be16(out: &mut Vec<u8>, v: u16) {
        out.extend_from_slice(&v.to_be_bytes());
    }

    /// Most offsets [`shared_gpos`] can put in one array while every offset still fits 16 bits.
    pub(crate) const MAX_SHARED: u16 = 32_000;

    /// A `GPOS` table whose lookup list holds `lookups` offsets to one lookup, which holds
    /// `subtables` offsets to one pair-positioning subtable with `pairs` pairs: tens of bytes
    /// per offset that the owned conversion copies once per reference.
    pub(crate) fn shared_gpos(lookups: u16, subtables: u16, pairs: u16) -> Vec<u8> {
        let mut t = Vec::new();
        // Header: version 1.0, script list, feature list, lookup list.
        be16(&mut t, 1);
        be16(&mut t, 0);
        let script_list = 10u16;
        let feature_list = script_list + 2;
        let lookup_list = feature_list + 2;
        be16(&mut t, script_list);
        be16(&mut t, feature_list);
        be16(&mut t, lookup_list);
        be16(&mut t, 0); // no scripts
        be16(&mut t, 0); // no features
        // Lookup list: every offset points to the same lookup, right after the array.
        be16(&mut t, lookups);
        let lookup_at = 2 + 2 * lookups;
        for _ in 0..lookups {
            be16(&mut t, lookup_at);
        }
        // Lookup: type 2, flag 0, every subtable offset to the same subtable.
        be16(&mut t, 2);
        be16(&mut t, 0);
        be16(&mut t, subtables);
        let subtable_at = 6 + 2 * subtables;
        for _ in 0..subtables {
            be16(&mut t, subtable_at);
        }
        // PairPos format 1: coverage of glyph 1, one pair set with `pairs` x-advance records.
        be16(&mut t, 1);
        be16(&mut t, 12); // coverage
        be16(&mut t, 0x0004);
        be16(&mut t, 0);
        be16(&mut t, 1);
        be16(&mut t, 18); // pair set
        // Coverage format 1 at 12: one glyph.
        be16(&mut t, 1);
        be16(&mut t, 1);
        be16(&mut t, 1);
        // Pair set at 18.
        be16(&mut t, pairs);
        for i in 0..pairs {
            be16(&mut t, i);
            be16(&mut t, 0xFFF6);
        }
        t
    }

    #[test]
    fn shared_subtables_count_once_per_reference() {
        let small = shared_gpos(2, 2, 3);
        check_gpos(&small).unwrap();
        // 32,000 × 32,000 references to one subtable: about 1e9 copies, refused at once.
        let wide = shared_gpos(MAX_SHARED, MAX_SHARED, 3);
        let err = check_gpos(&wide).unwrap_err().to_string();
        assert!(err.contains("too large"), "{err}");
        assert!(check_gpos_within(&shared_gpos(2, 2, 3), 20).is_err(), "a small budget refuses even the small table");
    }
}
