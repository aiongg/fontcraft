//! Pair kerning for text the proofs set as outlines: the pair lookups of the `kern` feature in
//! `GPOS`, else the horizontal pair subtables of the `kern` table.
//!
//! `GPOS`: the `kern` feature of the default language system of script `latn`, else `DFLT`, else
//! the first script that has one. Proof text is mostly Latin, and a compiler that splits kerning
//! by script registers the Latin pairs, with the shared ones, under `latn`. The feature's pair
//! lookups apply in lookup-list order and their values add up; inside a lookup, the first
//! subtable that applies to the pair decides it, as a shaper does. A glyph-pair subtable
//! (format 1) applies to the pairs it lists; a class subtable (format 2) to every pair whose
//! first glyph it covers and whose classes are in range. Only the first glyph's x advance is
//! read: device tables and variation deltas are left out. Contextual kerning is not read.
//!
//! `kern`: used when `GPOS` has no `kern` feature. The values of every horizontal subtable of
//! format 0, 2 or 3 that is neither cross-stream nor variable add up.
//!
//! Input is untrusted: at most [`MAX_PAIR_SUBTABLES`] subtables are read, and each pair is worked
//! out once.

use std::collections::HashMap;

use skrifa::GlyphId;
use skrifa::raw::tables::gpos::{Gpos, PairPos, PositionSubtables};
use skrifa::raw::tables::kern::SubtableKind;
use skrifa::raw::types::{GlyphId16, Tag};
use skrifa::raw::{FontRef, TableProvider};

/// Most pair subtables read from one font, over every lookup (or `kern` subtable).
pub(crate) const MAX_PAIR_SUBTABLES: usize = 512;
/// Most feature indices read from one language system.
const MAX_FEATURE_INDICES: usize = 1_024;
/// Most scripts looked at for a `kern` feature.
const MAX_SCRIPTS: usize = 256;
/// Most distinct pairs remembered; past this, pairs are worked out again each time.
const MAX_CACHED_PAIRS: usize = 1 << 20;

enum Source<'a> {
    None,
    /// The pair subtables of each `kern` lookup, in lookup-list order.
    Gpos(Vec<Vec<PairPos<'a>>>),
    Kern(Vec<SubtableKind<'a>>),
}

/// The pair kerning of one font, in font units.
pub(crate) struct Kerning<'a> {
    source: Source<'a>,
    cache: HashMap<(u32, u32), f64>,
}

impl<'a> Kerning<'a> {
    pub(crate) fn new(font: &FontRef<'a>) -> Self {
        let source = font
            .gpos()
            .ok()
            .and_then(|gpos| gpos_lookups(&gpos))
            .map(Source::Gpos)
            .or_else(|| kern_subtables(font).map(Source::Kern))
            .unwrap_or(Source::None);
        Kerning { source, cache: HashMap::new() }
    }

    /// The kerning between glyph `left` and glyph `right` that follows it.
    pub(crate) fn pair(&mut self, left: u32, right: u32) -> f64 {
        if matches!(self.source, Source::None) {
            return 0.0;
        }
        if let Some(v) = self.cache.get(&(left, right)) {
            return *v;
        }
        let value = match &self.source {
            Source::None => 0.0,
            Source::Gpos(lookups) => gpos_pair(lookups, left, right),
            Source::Kern(subtables) => kern_pair(subtables, left, right),
        };
        let value = if value.is_finite() { value } else { 0.0 };
        if self.cache.len() < MAX_CACHED_PAIRS {
            self.cache.insert((left, right), value);
        }
        value
    }
}

/// The pair subtables of the chosen `kern` feature, or `None` when no script has one.
fn gpos_lookups<'a>(gpos: &Gpos<'a>) -> Option<Vec<Vec<PairPos<'a>>>> {
    let scripts = gpos.script_list().ok()?;
    let features = gpos.feature_list().ok()?;
    let records = features.feature_records();
    let kern = Tag::new(b"kern");
    let script_records = scripts.script_records();
    let preferred = [Tag::new(b"latn"), Tag::new(b"DFLT")];
    let order = preferred
        .iter()
        .filter_map(|tag| script_records.iter().find(|r| r.script_tag() == *tag))
        .chain(script_records.iter().filter(|r| !preferred.contains(&r.script_tag())).take(MAX_SCRIPTS));
    let mut indices: Vec<u16> = Vec::new();
    for record in order {
        let Some(Ok(langsys)) = record.script(scripts.offset_data()).ok().and_then(|s| s.default_lang_sys()) else { continue };
        for index in langsys.feature_indices().iter().take(MAX_FEATURE_INDICES) {
            let Some(feature) = records.get(usize::from(index.get())) else { continue };
            if feature.feature_tag() != kern {
                continue;
            }
            if let Ok(feature) = feature.feature(features.offset_data()) {
                indices.extend(feature.lookup_list_indices().iter().map(|i| i.get()));
            }
        }
        if !indices.is_empty() {
            break;
        }
    }
    if indices.is_empty() {
        return None;
    }
    // Shapers apply a feature's lookups in lookup-list order, whatever order the record lists.
    indices.sort_unstable();
    indices.dedup();
    let list = gpos.lookup_list().ok()?;
    let lookups = list.lookups();
    let mut budget = MAX_PAIR_SUBTABLES;
    let mut out = Vec::new();
    for index in indices {
        if budget == 0 {
            break;
        }
        let Ok(lookup) = lookups.get(usize::from(index)) else { continue };
        let Ok(PositionSubtables::Pair(subtables)) = lookup.subtables() else { continue };
        let pairs: Vec<PairPos<'a>> = subtables.iter().take(budget).filter_map(Result::ok).collect();
        budget = budget.saturating_sub(pairs.len().max(1));
        out.push(pairs);
    }
    Some(out)
}

fn gpos_pair(lookups: &[Vec<PairPos<'_>>], left: u32, right: u32) -> f64 {
    let (Ok(first), Ok(second)) = (u16::try_from(left), u16::try_from(right)) else { return 0.0 };
    let (first, second) = (GlyphId16::new(first), GlyphId16::new(second));
    let mut total = 0.0;
    for subtables in lookups {
        for subtable in subtables {
            if let Some(v) = subtable_pair(subtable, first, second) {
                total += v;
                break;
            }
        }
    }
    total
}

/// The value one subtable gives the pair, or `None` when it does not apply to it.
fn subtable_pair(subtable: &PairPos<'_>, first: GlyphId16, second: GlyphId16) -> Option<f64> {
    match subtable {
        PairPos::Format1(t) => {
            let index = t.coverage().ok()?.get(first)?;
            let set = t.pair_sets().get(usize::from(index)).ok()?;
            let record = set.find_pair_value_record(second)?;
            Some(record.value_record1().x_advance().map_or(0.0, f64::from))
        }
        PairPos::Format2(t) => {
            t.coverage().ok()?.get(first)?;
            let class1 = t.class_def1().ok()?.get(first);
            let class2 = t.class_def2().ok()?.get(second);
            if class1 >= t.class1_count() || class2 >= t.class2_count() {
                return None;
            }
            let [v, _] = t.class_value_records().get(class1, class2)?;
            Some(v.x_advance().map_or(0.0, f64::from))
        }
    }
}

/// The `kern` table's horizontal pair subtables, or `None` when there are none.
fn kern_subtables<'a>(font: &FontRef<'a>) -> Option<Vec<SubtableKind<'a>>> {
    let kern = font.kern().ok()?;
    let subtables: Vec<SubtableKind<'a>> = kern
        .subtables()
        .take(MAX_PAIR_SUBTABLES)
        .filter_map(Result::ok)
        .filter(|s| s.is_horizontal() && !s.is_cross_stream() && !s.is_variable() && !s.is_state_machine())
        .filter_map(|s| s.kind().ok())
        .collect();
    (!subtables.is_empty()).then_some(subtables)
}

fn kern_pair(subtables: &[SubtableKind<'_>], left: u32, right: u32) -> f64 {
    let (left, right) = (GlyphId::new(left), GlyphId::new(right));
    subtables
        .iter()
        .filter_map(|s| match s {
            SubtableKind::Format0(t) => t.kerning(left, right),
            SubtableKind::Format2(t) => t.kerning(left, right),
            SubtableKind::Format3(t) => t.kerning(left, right),
            SubtableKind::Format1(_) => None,
        })
        .map(f64::from)
        .sum()
}
