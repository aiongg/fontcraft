//! The kerning a compiled font applies between two glyphs: what a shaper does to a run of just
//! those two glyphs with the `kern` and `dist` features of one script. Tests and the oracle
//! comparison read compiled fonts with it.
//!
//! The lookups of the script's default language system run in lookup-list order. A lookup that
//! skips marks (by its flag or its mark filtering set, with the classes of `GDEF`) does not apply
//! when either glyph is such a mark. In a lookup, the first subtable whose coverage holds the
//! first glyph decides: a glyph-pair subtable without the pair passes on to the next subtable, a
//! class subtable applies its record, zero or not. A script the font has no records for is read
//! as `DFLT`, as shapers fall back to it.

use std::collections::BTreeMap;

use read_fonts::tables::gdef::{Gdef, MarkGlyphSets};
use read_fonts::tables::gpos::{PairPos, PositionSubtables};
use read_fonts::tables::layout::{ClassDef, LookupFlag};
use read_fonts::{FontRef, ReadError, TableProvider};
use write_fonts::types::{GlyphId16, Tag};

use crate::kern::{DIST, KERN};
use crate::properties::{COMMON_SCRIPT, DFLT_SCRIPT, INHERITED_SCRIPT, script_to_ot_tags, unicode_script_extensions};

/// Most lookups and subtables the evaluator reads from one font.
pub const MAX_EVAL_SUBTABLES: usize = 1_000_000;

/// The OpenType script tags of `cp`'s Script_Extensions, without the common and inherited
/// scripts: where the kern writer puts the pairs of a glyph with that code point. Empty for a
/// common or inherited code point.
pub fn script_tags(cp: u32) -> Vec<Tag> {
    let mut out: Vec<Tag> =
        unicode_script_extensions(cp).filter(|s| *s != COMMON_SCRIPT && *s != INHERITED_SCRIPT).flat_map(|s| script_to_ot_tags(&s)).collect();
    out.sort_unstable();
    out.dedup();
    out
}

struct EvalLookup<'a> {
    flags: LookupFlag,
    mark_set: Option<u16>,
    subtables: Vec<PairPos<'a>>,
}

/// A compiled font's pair kerning, ready for [`PairEvaluator::value`].
pub struct PairEvaluator<'a> {
    scripts: BTreeMap<Tag, Vec<usize>>,
    lookups: Vec<Option<EvalLookup<'a>>>,
    glyph_classes: Option<ClassDef<'a>>,
    mark_sets: Option<MarkGlyphSets<'a>>,
}

fn err(e: ReadError) -> String {
    format!("cannot read the font's kerning: {e}")
}

impl<'a> PairEvaluator<'a> {
    /// Reads `font`'s `GPOS` and `GDEF`. A font without `GPOS` kerns nothing.
    pub fn new(font: &'a [u8]) -> Result<Self, String> {
        let font = FontRef::new(font).map_err(err)?;
        let gdef: Option<Gdef> = font.gdef().ok();
        let glyph_classes = gdef.as_ref().and_then(|g| g.glyph_class_def()).and_then(Result::ok);
        let mark_sets = gdef.as_ref().and_then(|g| g.mark_glyph_sets_def()).and_then(Result::ok);
        let mut out = PairEvaluator { scripts: BTreeMap::new(), lookups: Vec::new(), glyph_classes, mark_sets };
        let Ok(gpos) = font.gpos() else { return Ok(out) };
        let script_list = gpos.script_list().map_err(err)?;
        let feature_list = gpos.feature_list().map_err(err)?;
        let lookup_list = gpos.lookup_list().map_err(err)?;
        let feature_records = feature_list.feature_records();
        let mut read = 0usize;
        for record in script_list.script_records() {
            let script = record.script(script_list.offset_data()).map_err(err)?;
            let Some(lang_sys) = script.default_lang_sys().transpose().map_err(err)? else { continue };
            let mut lookups = Vec::new();
            for index in lang_sys.feature_indices() {
                let Some(feature_record) = feature_records.get(usize::from(index.get())) else { continue };
                if ![KERN, DIST].contains(&feature_record.feature_tag()) {
                    continue;
                }
                let feature = feature_record.feature(feature_list.offset_data()).map_err(err)?;
                lookups.extend(feature.lookup_list_indices().iter().map(|i| usize::from(i.get())));
            }
            lookups.sort_unstable();
            lookups.dedup();
            out.scripts.insert(record.script_tag(), lookups);
        }
        let count = lookup_list.lookup_count();
        out.lookups.resize_with(usize::from(count), || None);
        let used: std::collections::BTreeSet<usize> = out.scripts.values().flatten().copied().collect();
        for index in used {
            let lookup = lookup_list.lookups().get(index).map_err(err)?;
            let Ok(PositionSubtables::Pair(subtables)) = lookup.subtables() else { continue };
            let mut list = Vec::new();
            for subtable in subtables.iter() {
                read = read.saturating_add(1);
                if read > MAX_EVAL_SUBTABLES {
                    return Err(format!("the font's kerning has more than {MAX_EVAL_SUBTABLES} subtables"));
                }
                list.push(subtable.map_err(err)?);
            }
            if let Some(slot) = out.lookups.get_mut(index) {
                *slot = Some(EvalLookup { flags: lookup.lookup_flag(), mark_set: lookup.mark_filtering_set(), subtables: list });
            }
        }
        Ok(out)
    }

    /// The script tags the font has records for.
    pub fn scripts(&self) -> Vec<Tag> {
        self.scripts.keys().copied().collect()
    }

    /// Whether a lookup with these flags skips `gid`.
    fn skips(&self, lookup: &EvalLookup, gid: GlyphId16) -> bool {
        let is_mark = self.glyph_classes.as_ref().is_some_and(|c| c.get(gid) == 3);
        if !is_mark {
            return false;
        }
        if lookup.flags.contains(LookupFlag::IGNORE_MARKS) {
            return true;
        }
        if lookup.flags.contains(LookupFlag::USE_MARK_FILTERING_SET) {
            let in_set = lookup
                .mark_set
                .and_then(|set| self.mark_sets.as_ref().and_then(|s| s.coverages().get(usize::from(set)).ok()))
                .is_some_and(|c| c.get(gid).is_some());
            return !in_set;
        }
        false
    }

    /// The horizontal advance adjustment of `first` followed by `second` in `script`.
    pub fn value(&self, script: Tag, first: u16, second: u16) -> i32 {
        let Some(lookups) = self.scripts.get(&script).or_else(|| self.scripts.get(&DFLT_SCRIPT)) else { return 0 };
        let (a, b) = (GlyphId16::new(first), GlyphId16::new(second));
        let mut total = 0i32;
        for index in lookups {
            let Some(Some(lookup)) = self.lookups.get(*index) else { continue };
            if self.skips(lookup, a) || self.skips(lookup, b) {
                continue;
            }
            for subtable in &lookup.subtables {
                if let Some(value) = subtable_value(subtable, a, b) {
                    total = total.saturating_add(value);
                    break;
                }
            }
        }
        total
    }
}

/// What one pair subtable does to `a` then `b`: `None` when it does not apply.
fn subtable_value(subtable: &PairPos, a: GlyphId16, b: GlyphId16) -> Option<i32> {
    match subtable {
        PairPos::Format1(t) => {
            let index = t.coverage().ok()?.get(a)?;
            let set = t.pair_sets().get(usize::from(index)).ok()?;
            let records = set.pair_value_records();
            let (mut lo, mut hi) = (0usize, records.len());
            while lo < hi {
                let mid = lo + (hi - lo) / 2;
                let record = records.get(mid).ok()?;
                match record.second_glyph().cmp(&b) {
                    std::cmp::Ordering::Equal => return Some(i32::from(record.value_record1().x_advance().unwrap_or(0))),
                    std::cmp::Ordering::Less => lo = mid + 1,
                    std::cmp::Ordering::Greater => hi = mid,
                }
            }
            None
        }
        PairPos::Format2(t) => {
            t.coverage().ok()?.get(a)?;
            let class1 = t.class_def1().ok()?.get(a);
            let class2 = t.class_def2().ok()?.get(b);
            let record = t.class1_records().get(usize::from(class1)).ok()?;
            let value = record.class2_records().get(usize::from(class2)).ok()?;
            Some(i32::from(value.value_record1().x_advance().unwrap_or(0)))
        }
    }
}
