//! `GSUB`, `GPOS` and `GDEF` merged, after fontTools' `merge/layout.py` (MIT License).
//!
//! Each font's tables are first moved to the merged glyph ids (with the glyph-order tool's
//! remapper). Then:
//! - lookups are concatenated in font order, nested lookup indices and mark filtering sets
//!   moved with them;
//! - feature records are concatenated and sorted by tag (stably);
//! - a script only one font has keeps its language systems; for a script several fonts share,
//!   each language system gets new features, one per tag, whose lookups are the shared
//!   features' lookups in font order (these new features follow the others);
//! - features no language system uses are dropped, then lookups nothing uses.
//!
//! Before that, a font that maps a code point an earlier font already maps gets a single
//! substitution in its `locl` feature, first in the feature's lookups, under every script but
//! `DFLT`.

use std::collections::{BTreeMap, BTreeSet};

use read_fonts::{FontData, FontRead};
use write_fonts::from_obj::ToOwnedTable;
use write_fonts::tables::gdef::{AttachList, AttachPoint, Gdef, LigCaretList, LigGlyph, MarkGlyphSets};
use write_fonts::tables::gpos::{self, Gpos, PositionLookup, PositionLookupList};
use write_fonts::tables::gsub::{self, Gsub, SingleSubst, SingleSubstFormat1, SingleSubstFormat2, SubstitutionLookup, SubstitutionLookupList};
use write_fonts::tables::layout::{
    ChainedSequenceContext, ClassDef, ClassDefFormat1, ClassDefFormat2, ClassRangeRecord, CoverageFormat1, CoverageFormat2, CoverageTable, Feature,
    FeatureList, FeatureRecord, LangSys, LangSysRecord, Lookup, LookupFlag, RangeRecord, Script, ScriptList, ScriptRecord, SequenceContext,
};
use write_fonts::types::{GlyphId16, Tag as OtTag};

use super::plan::Plan;
use crate::limits::MAX_LAYOUT_ITEMS;
use crate::reorder::layout as remap;
use crate::reorder::tables::Order;
use crate::sfnt::{SfntFont, Tag, tag_str};
use crate::{Result, TablesError};

type G = GlyphId16;
const GSUB: Tag = *b"GSUB";
const GPOS: Tag = *b"GPOS";
const GDEF: Tag = *b"GDEF";

fn read_err(tag: Tag, e: impl std::fmt::Display) -> TablesError {
    TablesError::invalid(format!("the '{}' table cannot be read: {e}", tag_str(tag)))
}

fn write_err(tag: Tag, e: impl std::fmt::Display) -> TablesError {
    TablesError::invalid(format!("the merged '{}' table cannot be written: {e}", tag_str(tag)))
}

fn to_u16(v: usize, what: &'static str) -> Result<u16> {
    u16::try_from(v).map_err(|_| TablesError::TooLarge { what, limit: usize::from(u16::MAX) })
}

/// The order that moves font `font`'s glyph ids to the merged ones. The remapper reads only
/// `old_to_new` and, through `len()`, the merged glyph count.
fn order_for(plan: &Plan, font: usize) -> Result<Order> {
    if !plan.takes_all_of(font) {
        return Err(TablesError::BadOptions(format!(
            "the layout of font {} can be merged only when the result takes each of its glyphs once: use layout base or drop",
            font + 1
        )));
    }
    let old_to_new = plan.maps.get(font).map(|m| m.iter().map(|g| g.unwrap_or(0)).collect()).unwrap_or_default();
    Ok(Order { new_to_old: vec![0; plan.slots.len()], old_to_new })
}

/// The first font's layout tables, moved to the merged glyph ids.
pub(super) fn base(fonts: &[SfntFont], plan: &Plan, out: &mut SfntFont) -> Result<()> {
    let Some(font) = fonts.first() else { return Ok(()) };
    if [GSUB, GPOS, GDEF].iter().all(|t| font.get(*t).is_none()) {
        return Ok(());
    }
    let order = order_for(plan, 0)?;
    type Rewrite = fn(&[u8], &Order) -> Result<Vec<u8>>;
    let rewrites: [(Tag, Rewrite); 3] = [(GSUB, remap::gsub), (GPOS, remap::gpos), (GDEF, remap::gdef)];
    for (tag, rewrite) in rewrites {
        if let Some(bytes) = font.get(tag) {
            out.insert(tag, rewrite(bytes, &order)?);
        }
    }
    Ok(())
}

/// The layout tables of every font, merged.
pub(super) fn merge(
    fonts: &[SfntFont],
    plan: &Plan,
    duplicates: &[BTreeMap<u16, u16>],
    skip: &BTreeSet<Tag>,
    out: &mut SfntFont,
    warnings: &mut Vec<String>,
) -> Result<()> {
    // Tables the caller drops are not merged, so a table that cannot be merged can be dropped.
    fn table<'a>(f: &'a SfntFont, tag: Tag, skip: &BTreeSet<Tag>) -> Option<&'a [u8]> {
        if skip.contains(&tag) { None } else { f.get(tag) }
    }
    let has_layout = |f: &SfntFont| [GSUB, GPOS, GDEF].iter().any(|t| table(f, *t, skip).is_some());
    let mut orders: Vec<Option<Order>> = Vec::with_capacity(fonts.len());
    for (i, font) in fonts.iter().enumerate() {
        orders.push(if has_layout(font) { Some(order_for(plan, i)?) } else { None });
    }

    // GDEF first: its mark glyph sets number the lookups' mark filtering sets.
    let mut gdefs: Vec<Gdef> = Vec::new();
    let mut mark_offsets: Vec<u16> = vec![0; fonts.len()];
    let mut mark_sets = 0usize;
    for (i, font) in fonts.iter().enumerate() {
        if let Some(slot) = mark_offsets.get_mut(i) {
            *slot = to_u16(mark_sets, "the merged mark glyph set count")?;
        }
        let (Some(bytes), Some(order)) = (table(font, GDEF, skip), orders.get(i).and_then(Option::as_ref)) else { continue };
        let moved = remap::gdef(bytes, order)?;
        let gdef: Gdef = read_fonts::tables::gdef::Gdef::read(FontData::new(&moved)).map_err(|e| read_err(GDEF, e))?.to_owned_table();
        if gdef.item_var_store.is_some() {
            return Err(TablesError::Unsupported(format!("merging a 'GDEF' table with variation data (font {})", i + 1)));
        }
        mark_sets += gdef.mark_glyph_sets_def.as_ref().map_or(0, |m| m.coverages.len());
        gdefs.push(gdef);
    }
    if !gdefs.is_empty() {
        out.insert(GDEF, write_fonts::dump_table(&merge_gdef(gdefs)?).map_err(|e| write_err(GDEF, e))?);
    }

    let mut gsub_parts: Vec<(usize, Part<SubstitutionLookup>)> = Vec::new();
    let mut gpos_parts: Vec<(usize, Part<PositionLookup>)> = Vec::new();
    for (i, font) in fonts.iter().enumerate() {
        let dups = duplicates.get(i).filter(|d| !d.is_empty());
        let order = orders.get(i).and_then(Option::as_ref);
        match (table(font, GSUB, skip), order) {
            (Some(bytes), Some(order)) => {
                let moved = remap::gsub(bytes, order)?;
                let table: Gsub = read_fonts::tables::gsub::Gsub::read(FontData::new(&moved)).map_err(|e| read_err(GSUB, e))?.to_owned_table();
                let mut part = Part {
                    scripts: table.script_list.into_inner().script_records,
                    features: table.feature_list.into_inner().feature_records,
                    lookups: table.lookup_list.into_inner().lookups.into_iter().map(|l| l.into_inner()).collect(),
                };
                if let Some(dups) = dups {
                    add_locl(&mut part, dups)?;
                }
                gsub_parts.push((i, part));
            }
            _ => {
                if dups.is_some() {
                    warnings.push(format!(
                        "font {} maps code points an earlier font maps too, but has no 'GSUB' table to choose its own glyphs for them",
                        i + 1
                    ));
                }
            }
        }
        if let (Some(bytes), Some(order)) = (table(font, GPOS, skip), order) {
            let moved = remap::gpos(bytes, order)?;
            let table: Gpos = read_fonts::tables::gpos::Gpos::read(FontData::new(&moved)).map_err(|e| read_err(GPOS, e))?.to_owned_table();
            gpos_parts.push((
                i,
                Part {
                    scripts: table.script_list.into_inner().script_records,
                    features: table.feature_list.into_inner().feature_records,
                    lookups: table.lookup_list.into_inner().lookups.into_iter().map(|l| l.into_inner()).collect(),
                },
            ));
        }
    }
    if !gsub_parts.is_empty() {
        let (scripts, features, lookups) = merge_parts(gsub_parts, &mark_offsets)?;
        let table = Gsub::new(scripts, features, SubstitutionLookupList::new(lookups));
        out.insert(GSUB, write_fonts::dump_table(&table).map_err(|e| write_err(GSUB, e))?);
    }
    if !gpos_parts.is_empty() {
        let (scripts, features, lookups) = merge_parts(gpos_parts, &mark_offsets)?;
        let table = Gpos::new(scripts, features, PositionLookupList::new(lookups));
        out.insert(GPOS, write_fonts::dump_table(&table).map_err(|e| write_err(GPOS, e))?);
    }
    Ok(())
}

/// One font's script, feature and lookup lists.
struct Part<L> {
    scripts: Vec<ScriptRecord>,
    features: Vec<FeatureRecord>,
    lookups: Vec<L>,
}

/// What merging needs to change inside a lookup.
trait LayoutLookup {
    /// Calls `f` on every lookup index the lookup's contextual rules hold.
    fn nested(&mut self, f: &mut dyn FnMut(&mut u16) -> Result<()>) -> Result<()>;
    fn mark_set(&mut self) -> &mut Option<u16>;
}

fn context_records(c: &mut SequenceContext, f: &mut dyn FnMut(&mut u16) -> Result<()>) -> Result<()> {
    match c {
        SequenceContext::Format1(t) => {
            for set in t.seq_rule_sets.iter_mut().filter_map(|s| s.as_mut()) {
                for rule in &mut set.seq_rules {
                    for r in &mut rule.seq_lookup_records {
                        f(&mut r.lookup_list_index)?;
                    }
                }
            }
        }
        SequenceContext::Format2(t) => {
            for set in t.class_seq_rule_sets.iter_mut().filter_map(|s| s.as_mut()) {
                for rule in &mut set.class_seq_rules {
                    for r in &mut rule.seq_lookup_records {
                        f(&mut r.lookup_list_index)?;
                    }
                }
            }
        }
        SequenceContext::Format3(t) => {
            for r in &mut t.seq_lookup_records {
                f(&mut r.lookup_list_index)?;
            }
        }
    }
    Ok(())
}

fn chain_records(c: &mut ChainedSequenceContext, f: &mut dyn FnMut(&mut u16) -> Result<()>) -> Result<()> {
    match c {
        ChainedSequenceContext::Format1(t) => {
            for set in t.chained_seq_rule_sets.iter_mut().filter_map(|s| s.as_mut()) {
                for rule in &mut set.chained_seq_rules {
                    for r in &mut rule.seq_lookup_records {
                        f(&mut r.lookup_list_index)?;
                    }
                }
            }
        }
        ChainedSequenceContext::Format2(t) => {
            for set in t.chained_class_seq_rule_sets.iter_mut().filter_map(|s| s.as_mut()) {
                for rule in &mut set.chained_class_seq_rules {
                    for r in &mut rule.seq_lookup_records {
                        f(&mut r.lookup_list_index)?;
                    }
                }
            }
        }
        ChainedSequenceContext::Format3(t) => {
            for r in &mut t.seq_lookup_records {
                f(&mut r.lookup_list_index)?;
            }
        }
    }
    Ok(())
}

impl LayoutLookup for SubstitutionLookup {
    fn nested(&mut self, f: &mut dyn FnMut(&mut u16) -> Result<()>) -> Result<()> {
        match self {
            SubstitutionLookup::Contextual(l) => l.subtables.iter_mut().try_for_each(|s| context_records(s, f)),
            SubstitutionLookup::ChainContextual(l) => l.subtables.iter_mut().try_for_each(|s| chain_records(s, f)),
            SubstitutionLookup::Extension(l) => l.subtables.iter_mut().try_for_each(|s| match &mut **s {
                gsub::ExtensionSubtable::Contextual(e) => context_records(&mut e.extension, f),
                gsub::ExtensionSubtable::ChainContextual(e) => chain_records(&mut e.extension, f),
                _ => Ok(()),
            }),
            _ => Ok(()),
        }
    }

    fn mark_set(&mut self) -> &mut Option<u16> {
        match self {
            SubstitutionLookup::Single(l) => &mut l.mark_filtering_set,
            SubstitutionLookup::Multiple(l) => &mut l.mark_filtering_set,
            SubstitutionLookup::Alternate(l) => &mut l.mark_filtering_set,
            SubstitutionLookup::Ligature(l) => &mut l.mark_filtering_set,
            SubstitutionLookup::Contextual(l) => &mut l.mark_filtering_set,
            SubstitutionLookup::ChainContextual(l) => &mut l.mark_filtering_set,
            SubstitutionLookup::Extension(l) => &mut l.mark_filtering_set,
            SubstitutionLookup::Reverse(l) => &mut l.mark_filtering_set,
        }
    }
}

impl LayoutLookup for PositionLookup {
    fn nested(&mut self, f: &mut dyn FnMut(&mut u16) -> Result<()>) -> Result<()> {
        match self {
            PositionLookup::Contextual(l) => l.subtables.iter_mut().try_for_each(|s| context_records(s, f)),
            PositionLookup::ChainContextual(l) => l.subtables.iter_mut().try_for_each(|s| chain_records(s, f)),
            PositionLookup::Extension(l) => l.subtables.iter_mut().try_for_each(|s| match &mut **s {
                gpos::ExtensionSubtable::Contextual(e) => context_records(&mut e.extension, f),
                gpos::ExtensionSubtable::ChainContextual(e) => chain_records(&mut e.extension, f),
                _ => Ok(()),
            }),
            _ => Ok(()),
        }
    }

    fn mark_set(&mut self) -> &mut Option<u16> {
        match self {
            PositionLookup::Single(l) => &mut l.mark_filtering_set,
            PositionLookup::Pair(l) => &mut l.mark_filtering_set,
            PositionLookup::Cursive(l) => &mut l.mark_filtering_set,
            PositionLookup::MarkToBase(l) => &mut l.mark_filtering_set,
            PositionLookup::MarkToLig(l) => &mut l.mark_filtering_set,
            PositionLookup::MarkToMark(l) => &mut l.mark_filtering_set,
            PositionLookup::Contextual(l) => &mut l.mark_filtering_set,
            PositionLookup::ChainContextual(l) => &mut l.mark_filtering_set,
            PositionLookup::Extension(l) => &mut l.mark_filtering_set,
        }
    }
}

/// fontTools' `GSUB.merge` duplicate handling: one single substitution (earlier glyph → this
/// font's glyph) put first in the `locl` feature of every language system outside `DFLT`,
/// adding the feature where a language system lacks it.
fn add_locl(part: &mut Part<SubstitutionLookup>, dups: &BTreeMap<u16, u16>) -> Result<()> {
    let locl = OtTag::new(b"locl");
    let dflt = OtTag::new(b"DFLT");
    let Part { scripts, features, lookups } = part;
    let mut synth_feature: Option<u16> = None;
    let mut synth_lookup: Option<u16> = None;
    for record in scripts.iter_mut() {
        if record.script_tag == dflt {
            continue;
        }
        let script = &mut *record.script;
        let mut systems: Vec<&mut LangSys> = Vec::new();
        if let Some(ls) = script.default_lang_sys.as_mut() {
            systems.push(ls);
        }
        for r in script.lang_sys_records.iter_mut() {
            systems.push(&mut *r.lang_sys);
        }
        for ls in systems {
            let tag_of = |i: u16| features.get(usize::from(i)).map(|r| r.feature_tag);
            let existing = ls.feature_indices.iter().copied().find(|i| tag_of(*i) == Some(locl));
            let fi = match existing {
                Some(i) => i,
                None => {
                    let i = match synth_feature {
                        Some(i) => i,
                        None => {
                            let i = to_u16(features.len(), "the feature count")?;
                            features.push(FeatureRecord::new(locl, Feature::new(None, Vec::new())));
                            synth_feature = Some(i);
                            i
                        }
                    };
                    ls.feature_indices.push(i);
                    ls.feature_indices.sort_by_key(|i| features.get(usize::from(*i)).map(|r| r.feature_tag));
                    i
                }
            };
            let li = match synth_lookup {
                Some(l) => l,
                None => {
                    let l = to_u16(lookups.len(), "the lookup count")?;
                    lookups.push(SubstitutionLookup::Single(Lookup::new(LookupFlag::empty(), vec![single_subst(dups)])));
                    synth_lookup = Some(l);
                    l
                }
            };
            if let Some(record) = features.get_mut(usize::from(fi))
                && record.feature.lookup_list_indices.first() != Some(&li)
            {
                record.feature.lookup_list_indices.insert(0, li);
            }
        }
    }
    Ok(())
}

fn single_subst(map: &BTreeMap<u16, u16>) -> SingleSubst {
    let glyphs: Vec<G> = map.keys().map(|g| G::new(*g)).collect();
    let delta = |(a, b): (&u16, &u16)| b.wrapping_sub(*a);
    let first = map.iter().next().map(delta);
    match first {
        Some(d) if map.iter().all(|p| delta(p) == d) => SingleSubst::Format1(SingleSubstFormat1::new(coverage(&glyphs), d as i16)),
        _ => SingleSubst::Format2(SingleSubstFormat2::new(coverage(&glyphs), map.values().map(|g| G::new(*g)).collect())),
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

fn coverage_glyphs(c: &CoverageTable) -> Vec<G> {
    match c {
        CoverageTable::Format1(f) => f.glyph_array.clone(),
        CoverageTable::Format2(f) => {
            let mut out = Vec::new();
            for r in &f.range_records {
                for g in r.start_glyph_id.to_u16()..=r.end_glyph_id.to_u16() {
                    out.push(G::new(g));
                    if out.len() > MAX_LAYOUT_ITEMS {
                        return out;
                    }
                }
            }
            out
        }
    }
}

type Merged<L> = (ScriptList, FeatureList, Vec<L>);

fn merge_parts<L: LayoutLookup>(parts: Vec<(usize, Part<L>)>, mark_offsets: &[u16]) -> Result<Merged<L>> {
    // Lookups, concatenated.
    let mut lookups: Vec<L> = Vec::new();
    let mut feature_entries: Vec<(usize, FeatureRecord)> = Vec::new();
    let mut scripts_by_part: Vec<(usize, Vec<ScriptRecord>)> = Vec::new();
    for (p, (font, part)) in parts.into_iter().enumerate() {
        let offset = to_u16(lookups.len(), "the merged lookup count")?;
        let local_count = part.lookups.len();
        let marks = mark_offsets.get(font).copied().unwrap_or(0);
        for mut lookup in part.lookups {
            lookup.nested(&mut |i| {
                if usize::from(*i) >= local_count {
                    return Err(TablesError::invalid(format!("a lookup of font {} points at lookup {i}, which does not exist", font + 1)));
                }
                *i = i.checked_add(offset).ok_or(TablesError::TooLarge { what: "the merged lookup count", limit: 65_535 })?;
                Ok(())
            })?;
            if let Some(set) = lookup.mark_set().as_mut() {
                *set = set.saturating_add(marks);
            }
            lookups.push(lookup);
        }
        to_u16(lookups.len(), "the merged lookup count")?;
        for mut record in part.features {
            for i in record.feature.lookup_list_indices.iter_mut() {
                if usize::from(*i) >= local_count {
                    return Err(TablesError::invalid(format!("a feature of font {} points at lookup {i}, which does not exist", font + 1)));
                }
                *i += offset;
            }
            feature_entries.push((p, record));
        }
        scripts_by_part.push((p, part.scripts));
    }

    // Features, concatenated and sorted by tag; (part, local index) → index.
    let mut local_index: Vec<usize> = Vec::with_capacity(feature_entries.len());
    let mut counts: BTreeMap<usize, usize> = BTreeMap::new();
    for (p, _) in &feature_entries {
        let c = counts.entry(*p).or_insert(0);
        local_index.push(*c);
        *c += 1;
    }
    let mut sorted: Vec<usize> = (0..feature_entries.len()).collect();
    sorted.sort_by_key(|i| feature_entries.get(*i).map(|e| e.1.feature_tag));
    let mut global_of: BTreeMap<(usize, usize), u16> = BTreeMap::new();
    let mut features: Vec<FeatureRecord> = Vec::with_capacity(sorted.len());
    for (new, old) in sorted.iter().enumerate() {
        let (Some((p, record)), Some(local)) = (feature_entries.get(*old), local_index.get(*old)) else { continue };
        global_of.insert((*p, *local), to_u16(new, "the merged feature count")?);
        features.push(record.clone());
    }
    let lookup_global = |p: usize, i: u16| -> Result<u16> {
        global_of
            .get(&(p, usize::from(i)))
            .copied()
            .ok_or_else(|| TablesError::invalid(format!("a language system points at feature {i}, which does not exist")))
    };

    // Scripts, by tag.
    let mut by_tag: BTreeMap<OtTag, Vec<(usize, Script)>> = BTreeMap::new();
    for (p, records) in scripts_by_part {
        for r in records {
            by_tag.entry(r.script_tag).or_default().push((p, r.script.into_inner()));
        }
    }
    let mut new_features: Vec<FeatureRecord> = Vec::new();
    let base_count = features.len();
    let mut script_records: Vec<ScriptRecord> = Vec::new();
    for (tag, mut list) in by_tag {
        let script = if list.len() == 1 {
            let Some((p, mut script)) = list.pop() else { continue };
            let mut systems: Vec<&mut LangSys> = Vec::new();
            if let Some(ls) = script.default_lang_sys.as_mut() {
                systems.push(ls);
            }
            for r in script.lang_sys_records.iter_mut() {
                systems.push(&mut *r.lang_sys);
            }
            for ls in systems {
                for i in ls.feature_indices.iter_mut() {
                    *i = lookup_global(p, *i)?;
                }
                if ls.required_feature_index != 0xFFFF {
                    ls.required_feature_index = lookup_global(p, ls.required_feature_index)?;
                }
            }
            script
        } else {
            let defaults: Vec<(usize, &LangSys)> = list.iter().filter_map(|(p, s)| s.default_lang_sys.as_ref().map(|ls| (*p, ls))).collect();
            let default =
                if defaults.is_empty() { None } else { Some(merge_lang_sys(&defaults, &features, &lookup_global, base_count, &mut new_features)?) };
            let mut systems: BTreeMap<OtTag, Vec<(usize, &LangSys)>> = BTreeMap::new();
            for (p, s) in &list {
                for r in &s.lang_sys_records {
                    systems.entry(r.lang_sys_tag).or_default().push((*p, &*r.lang_sys));
                }
            }
            let mut records = Vec::with_capacity(systems.len());
            for (ls_tag, group) in systems {
                records.push(LangSysRecord::new(ls_tag, merge_lang_sys(&group, &features, &lookup_global, base_count, &mut new_features)?));
            }
            Script::new(default, records)
        };
        script_records.push(ScriptRecord::new(tag, script));
    }
    features.extend(new_features);
    to_u16(features.len(), "the merged feature count")?;

    // Drop the features no language system uses.
    let mut used = vec![false; features.len()];
    for_each_lang_sys(&mut script_records, |ls| {
        for i in ls.feature_indices.iter().copied().chain((ls.required_feature_index != 0xFFFF).then_some(ls.required_feature_index)) {
            if let Some(u) = used.get_mut(usize::from(i)) {
                *u = true;
            }
        }
    });
    let feature_map = compact(&used);
    features = features.into_iter().zip(&used).filter(|(_, u)| **u).map(|(f, _)| f).collect();
    for_each_lang_sys(&mut script_records, |ls| {
        for i in ls.feature_indices.iter_mut() {
            *i = feature_map.get(usize::from(*i)).copied().flatten().unwrap_or(*i);
        }
        if ls.required_feature_index != 0xFFFF {
            ls.required_feature_index = feature_map.get(usize::from(ls.required_feature_index)).copied().flatten().unwrap_or(0xFFFF);
        }
    });

    // Drop the lookups no feature and no lookup uses.
    let mut used = vec![false; lookups.len()];
    for f in &features {
        for i in &f.feature.lookup_list_indices {
            if let Some(u) = used.get_mut(usize::from(*i)) {
                *u = true;
            }
        }
    }
    for lookup in lookups.iter_mut() {
        lookup.nested(&mut |i| {
            if let Some(u) = used.get_mut(usize::from(*i)) {
                *u = true;
            }
            Ok(())
        })?;
    }
    let lookup_map = compact(&used);
    let mut kept: Vec<L> = Vec::with_capacity(lookups.len());
    for (mut lookup, u) in lookups.into_iter().zip(&used) {
        if !*u {
            continue;
        }
        lookup.nested(&mut |i| {
            *i = lookup_map.get(usize::from(*i)).copied().flatten().unwrap_or(*i);
            Ok(())
        })?;
        kept.push(lookup);
    }
    for f in features.iter_mut() {
        for i in f.feature.lookup_list_indices.iter_mut() {
            *i = lookup_map.get(usize::from(*i)).copied().flatten().unwrap_or(*i);
        }
    }
    Ok((ScriptList::new(script_records), FeatureList::new(features), kept))
}

/// fontTools' `mergeLangSyses`: one new feature per tag the language systems use, with the
/// lookups of all their features of that tag, in order. The new features go to `new_features`
/// and are numbered after the `base_count` existing ones.
fn merge_lang_sys(
    systems: &[(usize, &LangSys)],
    features: &[FeatureRecord],
    global: &dyn Fn(usize, u16) -> Result<u16>,
    base_count: usize,
    new_features: &mut Vec<FeatureRecord>,
) -> Result<LangSys> {
    if systems.iter().any(|(_, ls)| ls.required_feature_index != 0xFFFF) {
        return Err(TablesError::Unsupported("merging a required feature of a script several fonts share".into()));
    }
    let mut by_tag: BTreeMap<OtTag, Vec<u16>> = BTreeMap::new();
    for (p, ls) in systems {
        for i in &ls.feature_indices {
            let index = global(*p, *i)?;
            let Some(record) = features.get(usize::from(index)) else { continue };
            let entry = by_tag.entry(record.feature_tag).or_default();
            if entry.len().saturating_add(record.feature.lookup_list_indices.len()) > MAX_LAYOUT_ITEMS {
                return Err(TablesError::TooLarge { what: "a merged feature's lookup list", limit: MAX_LAYOUT_ITEMS });
            }
            entry.extend_from_slice(&record.feature.lookup_list_indices);
        }
    }
    let mut indices = Vec::with_capacity(by_tag.len());
    for (tag, lookups) in by_tag {
        indices.push(to_u16(base_count + new_features.len(), "the merged feature count")?);
        new_features.push(FeatureRecord::new(tag, Feature::new(None, lookups)));
    }
    Ok(LangSys::new(indices))
}

fn for_each_lang_sys(records: &mut [ScriptRecord], mut f: impl FnMut(&mut LangSys)) {
    for r in records {
        if let Some(ls) = r.script.default_lang_sys.as_mut() {
            f(ls);
        }
        for l in r.script.lang_sys_records.iter_mut() {
            f(&mut l.lang_sys);
        }
    }
}

/// The new index of each kept item (`None` for a dropped one).
fn compact(used: &[bool]) -> Vec<Option<u16>> {
    let mut next = 0u16;
    used.iter()
        .map(|u| {
            if *u {
                let i = next;
                next = next.saturating_add(1);
                Some(i)
            } else {
                None
            }
        })
        .collect()
}

// ── GDEF ──

/// The (glyph, class) pairs of a class definition, class 0 left out.
fn class_entries(cd: &ClassDef, out: &mut BTreeMap<u16, u16>) -> Result<()> {
    let mut work = 0usize;
    let mut set = |g: u16, class: u16| -> Result<()> {
        work += 1;
        if work > MAX_LAYOUT_ITEMS {
            return Err(TablesError::TooLarge { what: "a class definition", limit: MAX_LAYOUT_ITEMS });
        }
        if class != 0 {
            out.insert(g, class);
        }
        Ok(())
    };
    match cd {
        ClassDef::Format1(f) => {
            for (i, class) in f.class_value_array.iter().enumerate() {
                let Ok(i) = u16::try_from(i) else { break };
                let Some(g) = f.start_glyph_id.to_u16().checked_add(i) else { break };
                set(g, *class)?;
            }
        }
        ClassDef::Format2(f) => {
            for r in &f.class_range_records {
                for g in r.start_glyph_id.to_u16()..=r.end_glyph_id.to_u16() {
                    set(g, r.class)?;
                }
            }
        }
    }
    Ok(())
}

/// A class definition of `entries`, in the smaller format.
fn class_def(entries: &BTreeMap<u16, u16>) -> ClassDef {
    let mut ranges: Vec<ClassRangeRecord> = Vec::new();
    for (g, class) in entries {
        match ranges.last_mut() {
            Some(r) if r.class == *class && u32::from(r.end_glyph_id.to_u16()) + 1 == u32::from(*g) => r.end_glyph_id = G::new(*g),
            _ => ranges.push(ClassRangeRecord::new(G::new(*g), G::new(*g), *class)),
        }
    }
    let (first, last) = (entries.keys().next().copied().unwrap_or(0), entries.keys().next_back().copied().unwrap_or(0));
    let span = usize::from(last - first.min(last)) + 1;
    if !entries.is_empty() && 6 + 2 * span <= 4 + 6 * ranges.len() {
        let values = (first..=last).map(|g| entries.get(&g).copied().unwrap_or(0)).collect();
        ClassDef::Format1(ClassDefFormat1::new(G::new(first), values))
    } else {
        ClassDef::Format2(ClassDefFormat2::new(ranges))
    }
}

/// fontTools' GDEF merge: classes united (a later font's class wins for a shared glyph),
/// attachment points and ligature carets united by glyph, mark glyph sets concatenated.
fn merge_gdef(parts: Vec<Gdef>) -> Result<Gdef> {
    let mut glyph_classes: BTreeMap<u16, u16> = BTreeMap::new();
    let mut mark_classes: BTreeMap<u16, u16> = BTreeMap::new();
    let mut has_glyph_classes = false;
    let mut has_mark_classes = false;
    let mut attach: BTreeMap<u16, AttachPoint> = BTreeMap::new();
    let mut carets: BTreeMap<u16, LigGlyph> = BTreeMap::new();
    let mut has_attach = false;
    let mut has_carets = false;
    let mut mark_sets: Vec<CoverageTable> = Vec::new();
    let mut has_mark_sets = false;
    for gdef in parts {
        if let Some(cd) = gdef.glyph_class_def.as_ref() {
            has_glyph_classes = true;
            class_entries(cd, &mut glyph_classes)?;
        }
        if let Some(cd) = gdef.mark_attach_class_def.as_ref() {
            has_mark_classes = true;
            class_entries(cd, &mut mark_classes)?;
        }
        if let Some(list) = gdef.attach_list.into_inner() {
            has_attach = true;
            for (g, point) in coverage_glyphs(&list.coverage).into_iter().zip(list.attach_points) {
                attach.entry(g.to_u16()).or_insert_with(|| point.into_inner());
            }
        }
        if let Some(list) = gdef.lig_caret_list.into_inner() {
            has_carets = true;
            for (g, lig) in coverage_glyphs(&list.coverage).into_iter().zip(list.lig_glyphs) {
                carets.entry(g.to_u16()).or_insert_with(|| lig.into_inner());
            }
        }
        if let Some(sets) = gdef.mark_glyph_sets_def.into_inner() {
            has_mark_sets = true;
            mark_sets.extend(sets.coverages.into_iter().map(|c| c.into_inner()));
        }
    }
    let attach_list = has_attach.then(|| {
        let glyphs: Vec<G> = attach.keys().map(|g| G::new(*g)).collect();
        AttachList::new(coverage(&glyphs), attach.into_values().collect())
    });
    let caret_list = has_carets.then(|| {
        let glyphs: Vec<G> = carets.keys().map(|g| G::new(*g)).collect();
        LigCaretList::new(coverage(&glyphs), carets.into_values().collect())
    });
    let mut gdef =
        Gdef::new(has_glyph_classes.then(|| class_def(&glyph_classes)), attach_list, caret_list, has_mark_classes.then(|| class_def(&mark_classes)));
    if has_mark_sets {
        gdef.mark_glyph_sets_def = Some(MarkGlyphSets::new(mark_sets)).into();
    }
    Ok(gdef)
}
