//! `GDEF`, `GSUB` and `GPOS` in a new glyph order, after fontTools' `ttLib.reorderGlyphs`
//! (MIT): every glyph id is mapped, every coverage table is sorted again with the lists that
//! run parallel to it, the pair sets of pair positioning are sorted by their second glyph, and
//! class definitions keep their class numbers. The tables are read into write-fonts' owned
//! types, changed there and written again.

use read_fonts::types::GlyphId16;
use read_fonts::{FontData, FontRead};
use write_fonts::from_obj::ToOwnedTable;
use write_fonts::tables::gdef::Gdef;
use write_fonts::tables::gpos::{self, Gpos, PairPos, PositionLookup, SinglePos};
use write_fonts::tables::gsub::{self, Gsub, SingleSubst, SingleSubstFormat1, SingleSubstFormat2, SubstitutionLookup};
use write_fonts::tables::layout::{
    ChainedSequenceContext, ClassDef, ClassDefFormat1, ClassDefFormat2, ClassRangeRecord, CoverageFormat1, CoverageFormat2, CoverageTable,
    RangeRecord, SequenceContext,
};

use super::tables::Order;
use crate::sfnt::{Tag, tag_str};
use crate::{Result, TablesError};

type G = GlyphId16;

fn read_err(tag: Tag, e: impl std::fmt::Display) -> TablesError {
    TablesError::invalid(format!("the '{}' table cannot be read: {e}", tag_str(tag)))
}

fn write_err(tag: Tag, e: impl std::fmt::Display) -> TablesError {
    TablesError::invalid(format!("the '{}' table cannot be written: {e}", tag_str(tag)))
}

fn map(order: &Order, g: G) -> Result<G> {
    Ok(G::new(order.map(g.to_u16())?))
}

fn map_all(order: &Order, gs: &mut [G]) -> Result<()> {
    for g in gs {
        *g = map(order, *g)?;
    }
    Ok(())
}

/// The glyphs of a coverage table in coverage-index order.
fn coverage_glyphs(c: &CoverageTable) -> Vec<G> {
    match c {
        CoverageTable::Format1(f) => f.glyph_array.clone(),
        CoverageTable::Format2(f) => {
            let mut out = Vec::new();
            for r in &f.range_records {
                for g in r.start_glyph_id.to_u16()..=r.end_glyph_id.to_u16() {
                    out.push(G::new(g));
                }
            }
            out
        }
    }
}

/// A coverage table of sorted glyphs, in the smaller format.
fn coverage(glyphs: &[G]) -> CoverageTable {
    let mut ranges: Vec<RangeRecord> = Vec::new();
    for (i, g) in glyphs.iter().enumerate() {
        match ranges.last_mut() {
            Some(r) if u32::from(r.end_glyph_id.to_u16()) + 1 == u32::from(g.to_u16()) => r.end_glyph_id = *g,
            _ => ranges.push(RangeRecord::new(*g, *g, i as u16)),
        }
    }
    if 3 * ranges.len() < glyphs.len() {
        CoverageTable::Format2(CoverageFormat2::new(ranges))
    } else {
        CoverageTable::Format1(CoverageFormat1::new(glyphs.to_vec()))
    }
}

/// Maps and sorts `c`; returns, for each new coverage index, the old one.
fn remap_coverage(c: &mut CoverageTable, order: &Order) -> Result<Vec<usize>> {
    let mut pairs: Vec<(G, usize)> = Vec::new();
    for (i, g) in coverage_glyphs(c).into_iter().enumerate() {
        pairs.push((map(order, g)?, i));
    }
    pairs.sort_by_key(|p| p.0);
    let glyphs: Vec<G> = pairs.iter().map(|p| p.0).collect();
    *c = coverage(&glyphs);
    Ok(pairs.into_iter().map(|p| p.1).collect())
}

/// Reorders `list`, which runs parallel to a coverage table, by `perm`.
fn permute<T: Clone>(list: &mut Vec<T>, perm: &[usize]) -> Result<()> {
    if list.len() != perm.len() {
        return Err(TablesError::invalid("a layout list does not match its coverage table"));
    }
    let out: Option<Vec<T>> = perm.iter().map(|i| list.get(*i).cloned()).collect();
    *list = out.ok_or_else(|| TablesError::invalid("a layout list does not match its coverage table"))?;
    Ok(())
}

fn remap_parallel<T: Clone>(c: &mut CoverageTable, list: &mut Vec<T>, order: &Order) -> Result<()> {
    let perm = remap_coverage(c, order)?;
    permute(list, &perm)
}

/// Maps a class definition, keeping every class number. Work and memory follow the glyphs the
/// definition lists, not the glyph count, because one definition can be shared by many
/// subtables.
fn remap_classes(cd: &mut ClassDef, order: &Order) -> Result<()> {
    let n = order.len();
    // (new glyph, class) in the order the definition assigns them; a later range wins.
    let mut entries: Vec<(usize, u16)> = Vec::new();
    let mut work = 0usize;
    let budget = 16 * n + 65_536;
    let mut set = |g: usize, class: u16| -> Result<()> {
        work += 1;
        if work > budget {
            return Err(TablesError::invalid("a class definition has too many overlapping ranges"));
        }
        if let Some(new) = order.old_to_new.get(g).filter(|new| **new != Order::DROPPED) {
            let new = usize::from(*new);
            if new < n {
                entries.push((new, class));
            }
        }
        Ok(())
    };
    match cd {
        ClassDef::Format1(f) => {
            let start = usize::from(f.start_glyph_id.to_u16());
            for (i, class) in f.class_value_array.iter().enumerate() {
                set(start + i, *class)?;
            }
        }
        ClassDef::Format2(f) => {
            for r in &f.class_range_records {
                // Ranges hold input glyph ids; a subset has fewer output glyphs than input ones.
                let end = usize::from(r.end_glyph_id.to_u16()).min(order.old_to_new.len().saturating_sub(1));
                for g in usize::from(r.start_glyph_id.to_u16())..=end {
                    set(g, r.class)?;
                }
            }
        }
    }
    entries.sort_by_key(|e| e.0);
    let mut used: Vec<(usize, u16)> = Vec::with_capacity(entries.len());
    for (g, class) in entries {
        match used.last_mut() {
            Some(last) if last.0 == g => last.1 = class,
            _ => used.push((g, class)),
        }
    }
    used.retain(|u| u.1 != 0);
    let mut ranges: Vec<ClassRangeRecord> = Vec::new();
    for (g, class) in &used {
        match ranges.last_mut() {
            Some(r) if usize::from(r.end_glyph_id.to_u16()) + 1 == *g && r.class == *class => r.end_glyph_id = G::new(*g as u16),
            _ => ranges.push(ClassRangeRecord::new(G::new(*g as u16), G::new(*g as u16), *class)),
        }
    }
    let (first, last) = (used.first().map_or(0, |u| u.0), used.last().map_or(0, |u| u.0));
    let span = if used.is_empty() { 0 } else { last - first + 1 };
    *cd = if !used.is_empty() && 2 * span + 2 <= 6 * ranges.len() {
        let mut dense = vec![0u16; span];
        for (g, class) in &used {
            if let Some(slot) = g.checked_sub(first).and_then(|i| dense.get_mut(i)) {
                *slot = *class;
            }
        }
        ClassDef::Format1(ClassDefFormat1::new(G::new(first as u16), dense))
    } else {
        ClassDef::Format2(ClassDefFormat2::new(ranges))
    };
    Ok(())
}

fn context(c: &mut SequenceContext, order: &Order) -> Result<()> {
    match c {
        SequenceContext::Format1(f) => {
            remap_parallel(&mut f.coverage, &mut f.seq_rule_sets, order)?;
            for set in f.seq_rule_sets.iter_mut().filter_map(|s| s.as_mut()) {
                for rule in &mut set.seq_rules {
                    map_all(order, &mut rule.input_sequence)?;
                }
            }
        }
        SequenceContext::Format2(f) => {
            remap_coverage(&mut f.coverage, order)?;
            remap_classes(&mut f.class_def, order)?;
        }
        SequenceContext::Format3(f) => {
            for c in &mut f.coverages {
                remap_coverage(c, order)?;
            }
        }
    }
    Ok(())
}

fn chain_context(c: &mut ChainedSequenceContext, order: &Order) -> Result<()> {
    match c {
        ChainedSequenceContext::Format1(f) => {
            remap_parallel(&mut f.coverage, &mut f.chained_seq_rule_sets, order)?;
            for set in f.chained_seq_rule_sets.iter_mut().filter_map(|s| s.as_mut()) {
                for rule in &mut set.chained_seq_rules {
                    map_all(order, &mut rule.backtrack_sequence)?;
                    map_all(order, &mut rule.input_sequence)?;
                    map_all(order, &mut rule.lookahead_sequence)?;
                }
            }
        }
        ChainedSequenceContext::Format2(f) => {
            remap_coverage(&mut f.coverage, order)?;
            remap_classes(&mut f.backtrack_class_def, order)?;
            remap_classes(&mut f.input_class_def, order)?;
            remap_classes(&mut f.lookahead_class_def, order)?;
        }
        ChainedSequenceContext::Format3(f) => {
            for c in f.backtrack_coverages.iter_mut().chain(f.input_coverages.iter_mut()).chain(f.lookahead_coverages.iter_mut()) {
                remap_coverage(c, order)?;
            }
        }
    }
    Ok(())
}

// ── GSUB ──

fn single_subst(s: &mut SingleSubst, order: &Order) -> Result<()> {
    let pairs: Vec<(G, G)> = match s {
        SingleSubst::Format1(f) => {
            coverage_glyphs(&f.coverage).into_iter().map(|g| (g, G::new(g.to_u16().wrapping_add(f.delta_glyph_id as u16)))).collect()
        }
        SingleSubst::Format2(f) => coverage_glyphs(&f.coverage).into_iter().zip(f.substitute_glyph_ids.iter().copied()).collect(),
    };
    let mut mapped = Vec::with_capacity(pairs.len());
    for (a, b) in pairs {
        mapped.push((map(order, a)?, map(order, b)?));
    }
    mapped.sort_by_key(|p| p.0);
    let glyphs: Vec<G> = mapped.iter().map(|p| p.0).collect();
    let delta = |(a, b): &(G, G)| b.to_u16().wrapping_sub(a.to_u16());
    let first = mapped.first().map(delta);
    *s = match first {
        Some(d) if mapped.iter().all(|p| delta(p) == d) => SingleSubst::Format1(SingleSubstFormat1::new(coverage(&glyphs), d as i16)),
        _ => SingleSubst::Format2(SingleSubstFormat2::new(coverage(&glyphs), mapped.iter().map(|p| p.1).collect())),
    };
    Ok(())
}

fn gsub_subtable(l: &mut SubstitutionLookup, order: &Order) -> Result<()> {
    match l {
        SubstitutionLookup::Single(l) => l.subtables.iter_mut().try_for_each(|s| single_subst(s, order)),
        SubstitutionLookup::Multiple(l) => l.subtables.iter_mut().try_for_each(|s| multiple(s, order)),
        SubstitutionLookup::Alternate(l) => l.subtables.iter_mut().try_for_each(|s| alternate(s, order)),
        SubstitutionLookup::Ligature(l) => l.subtables.iter_mut().try_for_each(|s| ligature(s, order)),
        SubstitutionLookup::Contextual(l) => l.subtables.iter_mut().try_for_each(|s| context(s, order)),
        SubstitutionLookup::ChainContextual(l) => l.subtables.iter_mut().try_for_each(|s| chain_context(s, order)),
        SubstitutionLookup::Reverse(l) => l.subtables.iter_mut().try_for_each(|s| reverse(s, order)),
        SubstitutionLookup::Extension(l) => l.subtables.iter_mut().try_for_each(|s| match &mut **s {
            gsub::ExtensionSubtable::Single(e) => single_subst(&mut e.extension, order),
            gsub::ExtensionSubtable::Multiple(e) => multiple(&mut e.extension, order),
            gsub::ExtensionSubtable::Alternate(e) => alternate(&mut e.extension, order),
            gsub::ExtensionSubtable::Ligature(e) => ligature(&mut e.extension, order),
            gsub::ExtensionSubtable::Contextual(e) => context(&mut e.extension, order),
            gsub::ExtensionSubtable::ChainContextual(e) => chain_context(&mut e.extension, order),
            gsub::ExtensionSubtable::Reverse(e) => reverse(&mut e.extension, order),
        }),
    }
}

fn multiple(s: &mut gsub::MultipleSubstFormat1, order: &Order) -> Result<()> {
    remap_parallel(&mut s.coverage, &mut s.sequences, order)?;
    s.sequences.iter_mut().try_for_each(|seq| map_all(order, &mut seq.substitute_glyph_ids))
}

fn alternate(s: &mut gsub::AlternateSubstFormat1, order: &Order) -> Result<()> {
    remap_parallel(&mut s.coverage, &mut s.alternate_sets, order)?;
    s.alternate_sets.iter_mut().try_for_each(|set| map_all(order, &mut set.alternate_glyph_ids))
}

fn ligature(s: &mut gsub::LigatureSubstFormat1, order: &Order) -> Result<()> {
    remap_parallel(&mut s.coverage, &mut s.ligature_sets, order)?;
    for set in &mut s.ligature_sets {
        for lig in &mut set.ligatures {
            lig.ligature_glyph = map(order, lig.ligature_glyph)?;
            map_all(order, &mut lig.component_glyph_ids)?;
        }
    }
    Ok(())
}

fn reverse(s: &mut gsub::ReverseChainSingleSubstFormat1, order: &Order) -> Result<()> {
    map_all(order, &mut s.substitute_glyph_ids)?;
    remap_parallel(&mut s.coverage, &mut s.substitute_glyph_ids, order)?;
    for c in s.backtrack_coverages.iter_mut().chain(s.lookahead_coverages.iter_mut()) {
        remap_coverage(c, order)?;
    }
    Ok(())
}

// ── GPOS ──

fn gpos_subtable(l: &mut PositionLookup, order: &Order) -> Result<()> {
    match l {
        PositionLookup::Single(l) => l.subtables.iter_mut().try_for_each(|s| single_pos(s, order)),
        PositionLookup::Pair(l) => l.subtables.iter_mut().try_for_each(|s| pair(s, order)),
        PositionLookup::Cursive(l) => l.subtables.iter_mut().try_for_each(|s| cursive(s, order)),
        PositionLookup::MarkToBase(l) => l.subtables.iter_mut().try_for_each(|s| mark_base(s, order)),
        PositionLookup::MarkToLig(l) => l.subtables.iter_mut().try_for_each(|s| mark_lig(s, order)),
        PositionLookup::MarkToMark(l) => l.subtables.iter_mut().try_for_each(|s| mark_mark(s, order)),
        PositionLookup::Contextual(l) => l.subtables.iter_mut().try_for_each(|s| context(s, order)),
        PositionLookup::ChainContextual(l) => l.subtables.iter_mut().try_for_each(|s| chain_context(s, order)),
        PositionLookup::Extension(l) => l.subtables.iter_mut().try_for_each(|s| match &mut **s {
            gpos::ExtensionSubtable::Single(e) => single_pos(&mut e.extension, order),
            gpos::ExtensionSubtable::Pair(e) => pair(&mut e.extension, order),
            gpos::ExtensionSubtable::Cursive(e) => cursive(&mut e.extension, order),
            gpos::ExtensionSubtable::MarkToBase(e) => mark_base(&mut e.extension, order),
            gpos::ExtensionSubtable::MarkToLig(e) => mark_lig(&mut e.extension, order),
            gpos::ExtensionSubtable::MarkToMark(e) => mark_mark(&mut e.extension, order),
            gpos::ExtensionSubtable::Contextual(e) => context(&mut e.extension, order),
            gpos::ExtensionSubtable::ChainContextual(e) => chain_context(&mut e.extension, order),
        }),
    }
}

fn single_pos(s: &mut SinglePos, order: &Order) -> Result<()> {
    match s {
        SinglePos::Format1(f) => remap_coverage(&mut f.coverage, order).map(drop),
        SinglePos::Format2(f) => remap_parallel(&mut f.coverage, &mut f.value_records, order),
    }
}

fn pair(s: &mut PairPos, order: &Order) -> Result<()> {
    match s {
        PairPos::Format1(f) => {
            remap_parallel(&mut f.coverage, &mut f.pair_sets, order)?;
            for set in &mut f.pair_sets {
                for record in &mut set.pair_value_records {
                    record.second_glyph = map(order, record.second_glyph)?;
                }
                set.pair_value_records.sort_by_key(|r| r.second_glyph);
            }
            Ok(())
        }
        PairPos::Format2(f) => {
            remap_coverage(&mut f.coverage, order)?;
            remap_classes(&mut f.class_def1, order)?;
            remap_classes(&mut f.class_def2, order)
        }
    }
}

fn cursive(s: &mut gpos::CursivePosFormat1, order: &Order) -> Result<()> {
    remap_parallel(&mut s.coverage, &mut s.entry_exit_record, order)
}

fn mark_base(s: &mut gpos::MarkBasePosFormat1, order: &Order) -> Result<()> {
    remap_parallel(&mut s.mark_coverage, &mut s.mark_array.mark_records, order)?;
    remap_parallel(&mut s.base_coverage, &mut s.base_array.base_records, order)
}

fn mark_lig(s: &mut gpos::MarkLigPosFormat1, order: &Order) -> Result<()> {
    remap_parallel(&mut s.mark_coverage, &mut s.mark_array.mark_records, order)?;
    remap_parallel(&mut s.ligature_coverage, &mut s.ligature_array.ligature_attaches, order)
}

fn mark_mark(s: &mut gpos::MarkMarkPosFormat1, order: &Order) -> Result<()> {
    remap_parallel(&mut s.mark1_coverage, &mut s.mark1_array.mark_records, order)?;
    remap_parallel(&mut s.mark2_coverage, &mut s.mark2_array.mark2_records, order)
}

// ── The tables ──

pub(crate) fn gsub(bytes: &[u8], order: &Order) -> Result<Vec<u8>> {
    let tag = *b"GSUB";
    crate::layout_budget::check_gsub(bytes)?;
    let table = read_fonts::tables::gsub::Gsub::read(FontData::new(bytes)).map_err(|e| read_err(tag, e))?;
    let mut owned: Gsub = table.to_owned_table();
    for lookup in &mut owned.lookup_list.lookups {
        gsub_subtable(lookup, order)?;
    }
    write_fonts::dump_table(&owned).map_err(|e| write_err(tag, e))
}

pub(crate) fn gpos(bytes: &[u8], order: &Order) -> Result<Vec<u8>> {
    let tag = *b"GPOS";
    crate::layout_budget::check_gpos(bytes)?;
    let table = read_fonts::tables::gpos::Gpos::read(FontData::new(bytes)).map_err(|e| read_err(tag, e))?;
    let mut owned: Gpos = table.to_owned_table();
    for lookup in &mut owned.lookup_list.lookups {
        gpos_subtable(lookup, order)?;
    }
    write_fonts::dump_table(&owned).map_err(|e| write_err(tag, e))
}

pub(crate) fn gdef(bytes: &[u8], order: &Order) -> Result<Vec<u8>> {
    let tag = *b"GDEF";
    crate::layout_budget::check_gdef(bytes)?;
    let table = read_fonts::tables::gdef::Gdef::read(FontData::new(bytes)).map_err(|e| read_err(tag, e))?;
    let mut owned: Gdef = table.to_owned_table();
    if let Some(cd) = owned.glyph_class_def.as_mut() {
        remap_classes(cd, order)?;
    }
    if let Some(cd) = owned.mark_attach_class_def.as_mut() {
        remap_classes(cd, order)?;
    }
    if let Some(list) = owned.attach_list.as_mut() {
        remap_parallel(&mut list.coverage, &mut list.attach_points, order)?;
    }
    if let Some(list) = owned.lig_caret_list.as_mut() {
        remap_parallel(&mut list.coverage, &mut list.lig_glyphs, order)?;
    }
    if let Some(sets) = owned.mark_glyph_sets_def.as_mut() {
        for c in &mut sets.coverages {
            remap_coverage(c, order)?;
        }
    }
    write_fonts::dump_table(&owned).map_err(|e| write_err(tag, e))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    #[test]
    fn coverage_and_parallel_lists_follow_the_new_order() {
        // Old glyphs 1, 2, 3 become 3, 1, 2.
        let order = Order::new(vec![0, 2, 3, 1]);
        let mut cov = coverage(&[G::new(1), G::new(2), G::new(3)]);
        let mut list = vec!["one", "two", "three"];
        remap_parallel(&mut cov, &mut list, &order).unwrap();
        assert_eq!(coverage_glyphs(&cov), [G::new(1), G::new(2), G::new(3)]);
        assert_eq!(list, ["two", "three", "one"]);

        let mut cd = ClassDef::Format2(ClassDefFormat2::new(vec![ClassRangeRecord::new(G::new(1), G::new(2), 5)]));
        remap_classes(&mut cd, &order).unwrap();
        let ClassDef::Format1(f) = cd else { panic!("{cd:?}") };
        assert_eq!((f.start_glyph_id, f.class_value_array), (G::new(1), vec![5, 0, 5]), "format 1 is the smaller one here");

        // A later range wins over an earlier one; class 0 drops the glyph.
        let identity = Order::new(vec![0, 1, 2, 3, 4]);
        let mut cd = ClassDef::Format2(ClassDefFormat2::new(vec![
            ClassRangeRecord::new(G::new(1), G::new(3), 5),
            ClassRangeRecord::new(G::new(2), G::new(2), 0),
            ClassRangeRecord::new(G::new(4), G::new(4), 7),
        ]));
        remap_classes(&mut cd, &identity).unwrap();
        let ClassDef::Format1(f) = cd else { panic!("{cd:?}") };
        assert_eq!((f.start_glyph_id, f.class_value_array), (G::new(1), vec![5, 0, 5, 7]));
    }
}
