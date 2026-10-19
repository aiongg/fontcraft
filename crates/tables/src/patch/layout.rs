//! `tables.recompileLayout`: contextual positioning lookups (GPOS type 7) rewritten as chained
//! contextual lookups (type 8) with empty backtrack and lookahead, which apply the same rules.
//! Formats 1, 2 and 3 become the same formats of type 8; extension lookups that wrap type 7
//! subtables wrap type 8 subtables afterwards. Lookup flags, mark filtering sets and lookup
//! indices stay as they were.

use read_fonts::{FontData, FontRead};
use write_fonts::from_obj::ToOwnedTable;
use write_fonts::tables::gpos::{ExtensionPosFormat1, ExtensionSubtable, Gpos, PositionChainContext, PositionLookup, PositionSequenceContext};
use write_fonts::tables::layout::{
    ChainedClassSequenceRule, ChainedClassSequenceRuleSet, ChainedSequenceContext, ChainedSequenceContextFormat1, ChainedSequenceContextFormat2,
    ChainedSequenceContextFormat3, ChainedSequenceRule, ChainedSequenceRuleSet, ClassDef, ClassDefFormat2, Lookup, SequenceContext,
};
use write_fonts::{NullableOffsetMarker, OffsetMarker};

use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

const GPOS: Tag = *b"GPOS";

/// The same rules as a chained context with empty backtrack and lookahead.
fn chained(context: &SequenceContext) -> ChainedSequenceContext {
    match context {
        SequenceContext::Format1(f) => {
            let sets = f
                .seq_rule_sets
                .iter()
                .map(|set| {
                    NullableOffsetMarker::new(set.as_ref().map(|set| {
                        let rules = set
                            .seq_rules
                            .iter()
                            .map(|r| ChainedSequenceRule::new(Vec::new(), r.input_sequence.clone(), Vec::new(), r.seq_lookup_records.clone()))
                            .collect();
                        ChainedSequenceRuleSet::new(rules)
                    }))
                })
                .collect();
            ChainedSequenceContext::Format1(ChainedSequenceContextFormat1 { coverage: f.coverage.clone(), chained_seq_rule_sets: sets })
        }
        SequenceContext::Format2(f) => {
            let sets = f
                .class_seq_rule_sets
                .iter()
                .map(|set| {
                    NullableOffsetMarker::new(set.as_ref().map(|set| {
                        let rules = set
                            .class_seq_rules
                            .iter()
                            .map(|r| ChainedClassSequenceRule::new(Vec::new(), r.input_sequence.clone(), Vec::new(), r.seq_lookup_records.clone()))
                            .collect();
                        ChainedClassSequenceRuleSet::new(rules)
                    }))
                })
                .collect();
            let empty = || OffsetMarker::new(ClassDef::Format2(ClassDefFormat2::new(Vec::new())));
            ChainedSequenceContext::Format2(ChainedSequenceContextFormat2 {
                coverage: f.coverage.clone(),
                backtrack_class_def: empty(),
                input_class_def: f.class_def.clone(),
                lookahead_class_def: empty(),
                chained_class_seq_rule_sets: sets,
            })
        }
        SequenceContext::Format3(f) => ChainedSequenceContext::Format3(ChainedSequenceContextFormat3 {
            backtrack_coverages: Vec::new(),
            input_coverages: f.coverages.clone(),
            lookahead_coverages: Vec::new(),
            seq_lookup_records: f.seq_lookup_records.clone(),
        }),
    }
}

fn chain_lookup(lookup: &Lookup<PositionSequenceContext>) -> Lookup<PositionChainContext> {
    let subtables = lookup.subtables.iter().map(|s| OffsetMarker::new(PositionChainContext::from(chained(s.as_inner())))).collect();
    Lookup { lookup_flag: lookup.lookup_flag, subtables, mark_filtering_set: lookup.mark_filtering_set }
}

/// The font with every contextual positioning lookup written as a chained one, and the indices
/// of the lookups that changed. A font without such lookups comes back unchanged.
pub fn recompile_layout(font: &SfntFont) -> Result<(SfntFont, Vec<usize>)> {
    let Some(data) = font.get(GPOS) else { return Ok((font.clone(), Vec::new())) };
    crate::layout_budget::check_gpos(data)?;
    let read = read_fonts::tables::gpos::Gpos::read(FontData::new(data)).map_err(|e| TablesError::invalid(format!("the GPOS table: {e}")))?;
    let mut gpos: Gpos = read.to_owned_table();
    let mut converted = Vec::new();
    for (i, lookup) in gpos.lookup_list.lookups.iter_mut().enumerate() {
        let new = match &**lookup {
            PositionLookup::Contextual(l) => Some(PositionLookup::ChainContextual(chain_lookup(l))),
            PositionLookup::Extension(l) if l.subtables.iter().any(|s| matches!(&**s, ExtensionSubtable::Contextual(_))) => {
                let subtables = l
                    .subtables
                    .iter()
                    .map(|s| {
                        OffsetMarker::new(match &**s {
                            ExtensionSubtable::Contextual(e) => ExtensionSubtable::ChainContextual(ExtensionPosFormat1::new(
                                8,
                                PositionChainContext::from(chained(e.extension.as_inner())),
                            )),
                            other => other.clone(),
                        })
                    })
                    .collect();
                Some(PositionLookup::Extension(Lookup { lookup_flag: l.lookup_flag, subtables, mark_filtering_set: l.mark_filtering_set }))
            }
            _ => None,
        };
        if let Some(new) = new {
            **lookup = new;
            converted.push(i);
        }
    }
    if converted.is_empty() {
        return Ok((font.clone(), converted));
    }
    let bytes = write_fonts::dump_table(&gpos).map_err(|e| TablesError::invalid(format!("the GPOS table could not be written again: {e}")))?;
    let mut out = font.clone();
    out.insert(GPOS, bytes);
    Ok((out, converted))
}
