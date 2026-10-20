//! Subsetting GSUB, GPOS and GDEF: closure and pruning (M12.18). A port of the layout parts of
//! fontTools' `subset` module (MIT, Copyright 2013 Google, Inc.), with its rules and order:
//!
//! 1. Before the closure, each of GSUB and GPOS keeps only the scripts of `layout_scripts` and
//!    the features of `layout_features`, and the lookups those features reach (directly or
//!    through contextual lookups) stay live; the others are set aside, keeping their indices.
//! 2. [`close_glyphs`] adds every glyph the live GSUB lookups can produce from the glyph set,
//!    round after round until a round adds nothing. A lookup is visited again only when the
//!    glyph set has grown since its last visit or it is asked about glyphs it was not asked
//!    about before.
//! 3. [`apply`] subsets GDEF, then GPOS and GSUB, to the glyphs the GSUB closure reached (not the
//!    glyphs only components add), drops empty subtables, lookups, features, language systems
//!    and (GPOS only) scripts, renumbers what remains, merges duplicate features, removes
//!    language systems equal to their script's default, strips device tables under
//!    `hinting: false`, and finally maps every glyph id to the output's numbering.
//!
//! GDEF's variation store keeps only the delta sets GDEF and GPOS still use, renumbered
//! ([`super::var::prune_layout_store`]).
//!
//! Every table passes the size check of `layout_budget` before it is read into owned types,
//! nesting is bounded by [`MAX_LOOKUP_NESTING`] and the closure by [`MAX_CLOSURE_WORK`].

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::mem::take;

use read_fonts::{FontData, FontRead};
use write_fonts::from_obj::ToOwnedTable;
use write_fonts::tables::gdef::Gdef;
use write_fonts::tables::gpos::{
    AnchorFormat1, AnchorTable, CursivePosFormat1, ExtensionSubtable as PosExtension, Gpos, MarkBasePosFormat1, MarkLigPosFormat1,
    MarkMarkPosFormat1, PairPos, PositionChainContext, PositionLookup, PositionSequenceContext, SinglePos, SinglePosFormat1, ValueFormat,
    ValueRecord,
};
use write_fonts::tables::gsub::{
    AlternateSubstFormat1, ExtensionSubtable as SubstExtension, Gsub, LigatureSubstFormat1, MultipleSubstFormat1, ReverseChainSingleSubstFormat1,
    SingleSubst, SingleSubstFormat1, SingleSubstFormat2, SubstitutionChainContext, SubstitutionLookup, SubstitutionSequenceContext,
};
use write_fonts::tables::layout::{
    ChainedSequenceContext, ClassDef, CoverageFormat1, CoverageFormat2, CoverageTable, DeviceOrVariationIndex, FeatureList, FeatureParams,
    FeatureVariations, LangSys, Lookup, LookupFlag, LookupList, RangeRecord, Script, ScriptList, SequenceContext, SequenceLookupRecord,
};
use write_fonts::types::{GlyphId16, Tag as OtTag};
use write_fonts::{NullableOffsetMarker, OffsetMarker};

use super::{Plan, SubsetOptions};
use crate::limits::MAX_LOOKUP_NESTING;
use crate::reorder::tables::Order;
use crate::sfnt::{SfntFont, Tag, tag_str};
use crate::{Result, TablesError};

/// The features kept when `layout_features` is `None`: the union of the feature groups
/// fontTools 4.66 derives from HarfBuzz's shapers (common, fractions, horizontal, vertical,
/// direction, justification, East Asian spacing, and the Arabic, Hangul, Tibetan and Indic
/// shapers), without its four private test tags.
pub const DEFAULT_LAYOUT_FEATURES: [&str; 64] = [
    "abvf", "abvm", "abvs", "akhn", "blwf", "blwm", "blws", "calt", "ccmp", "cfar", "chws", "cjct", "clig", "cswh", "curs", "dist", "dnom", "fin2",
    "fin3", "fina", "frac", "half", "haln", "halt", "init", "isol", "jalt", "kern", "liga", "ljmo", "locl", "ltra", "ltrm", "mark", "med2", "medi",
    "mkmk", "mset", "nukt", "numr", "palt", "pref", "pres", "pstf", "psts", "rand", "rclt", "rkrf", "rlig", "rphf", "rtla", "rtlm", "rvrn", "stch",
    "tjmo", "valt", "vatu", "vchw", "vert", "vhal", "vjmo", "vkrn", "vpal", "vrt2",
];

/// Most glyph tests the GSUB closure may make in one run, summed over every subtable visit.
pub const MAX_CLOSURE_WORK: usize = 1 << 28;

type G = GlyphId16;

const NO_FEATURE: u16 = 0xFFFF;

fn read_err(tag: Tag, e: impl std::fmt::Display) -> TablesError {
    TablesError::invalid(format!("the '{}' table cannot be read: {e}", tag_str(tag)))
}

fn write_err(tag: Tag, e: impl std::fmt::Display) -> TablesError {
    TablesError::invalid(format!("the '{}' table cannot be written: {e}", tag_str(tag)))
}

/// A tag as the options give it, padded with spaces; `None` for one longer than four bytes,
/// which matches nothing.
fn padded(s: &str) -> Option<[u8; 4]> {
    let b = s.as_bytes();
    if b.len() > 4 {
        return None;
    }
    let mut out = [b' '; 4];
    for (slot, c) in out.iter_mut().zip(b) {
        *slot = *c;
    }
    Some(out)
}

// ── Glyph sets ──

/// A set of 16-bit glyph ids.
#[derive(Clone)]
struct Bits {
    words: Vec<u64>,
    len: usize,
}

impl Bits {
    fn new() -> Self {
        Bits { words: vec![0; 1024], len: 0 }
    }

    fn from_set(set: &BTreeSet<u32>) -> Self {
        let mut b = Bits::new();
        for g in set {
            if let Ok(g) = u16::try_from(*g) {
                b.insert(g);
            }
        }
        b
    }

    fn contains(&self, g: u16) -> bool {
        self.words.get(usize::from(g >> 6)).is_some_and(|w| (w >> (g & 63)) & 1 == 1)
    }

    fn insert(&mut self, g: u16) -> bool {
        let Some(w) = self.words.get_mut(usize::from(g >> 6)) else { return false };
        let bit = 1u64 << (g & 63);
        if *w & bit != 0 {
            return false;
        }
        *w |= bit;
        self.len += 1;
        true
    }

    fn iter(&self) -> impl Iterator<Item = u16> + '_ {
        self.words.iter().enumerate().flat_map(|(i, w)| BitIter { word: *w, base: (i as u32) << 6 })
    }
}

struct BitIter {
    word: u64,
    base: u32,
}

impl Iterator for BitIter {
    type Item = u16;
    fn next(&mut self) -> Option<u16> {
        if self.word == 0 {
            return None;
        }
        let bit = self.word.trailing_zeros();
        self.word &= self.word.wrapping_sub(1);
        u16::try_from(self.base + bit).ok()
    }
}

/// `cur` is sorted and free of duplicates.
fn has(cur: &[u16], g: u16) -> bool {
    cur.binary_search(&g).is_ok()
}

fn sorted(mut v: Vec<u16>) -> Vec<u16> {
    v.sort_unstable();
    v.dedup();
    v
}

// ── Coverage tables and class definitions ──

fn glyphs_of(c: &CoverageTable) -> Vec<u16> {
    c.iter().map(|g| g.to_u16()).collect()
}

/// A coverage table of `glyphs` in their order, in the smaller format.
pub(crate) fn coverage(glyphs: &[u16]) -> CoverageTable {
    let mut ranges: Vec<RangeRecord> = Vec::new();
    for (i, g) in glyphs.iter().enumerate() {
        match ranges.last_mut() {
            Some(r) if u32::from(r.end_glyph_id.to_u16()) + 1 == u32::from(*g) => r.end_glyph_id = G::new(*g),
            _ => ranges.push(RangeRecord::new(G::new(*g), G::new(*g), i as u16)),
        }
    }
    if 3 * ranges.len() < glyphs.len() {
        CoverageTable::Format2(CoverageFormat2::new(ranges))
    } else {
        CoverageTable::Format1(CoverageFormat1::new(glyphs.iter().map(|g| G::new(*g)).collect()))
    }
}

/// Keeps the glyphs of `c` that are in `s`; returns whether any are left.
fn subset_coverage(c: &mut CoverageTable, s: &Bits) -> bool {
    let kept: Vec<u16> = c.iter().map(|g| g.to_u16()).filter(|g| s.contains(*g)).collect();
    *c = coverage(&kept);
    !kept.is_empty()
}

/// Keeps the entries of a coverage table and the list that runs parallel to it for which `keep`
/// says yes (it may change the item); returns the new coverage table and list.
fn filter_parallel<T>(c: &CoverageTable, list: Vec<T>, mut keep: impl FnMut(u16, &mut T) -> bool) -> (CoverageTable, Vec<T>) {
    let mut glyphs = Vec::new();
    let mut out = Vec::new();
    for (g, mut item) in c.iter().zip(list) {
        if keep(g.to_u16(), &mut item) {
            glyphs.push(g.to_u16());
            out.push(item);
        }
    }
    (coverage(&glyphs), out)
}

/// Glyph → class for every glyph with a class other than 0; a later range wins.
fn classes(cd: &ClassDef) -> BTreeMap<u16, u16> {
    let mut out = BTreeMap::new();
    for (g, class) in cd.iter() {
        if class == 0 {
            out.remove(&g.to_u16());
        } else {
            out.insert(g.to_u16(), class);
        }
    }
    out
}

fn class_def(map: &BTreeMap<u16, u16>) -> ClassDef {
    map.iter().filter(|(_, c)| **c != 0).map(|(g, c)| (G::new(*g), *c)).collect()
}

/// fontTools' `ClassDef.subset(glyphs, remap=True)`: keeps the glyphs of the set (`in_set`
/// says which, `set_len` counts the set) and returns the classes left in ascending order, with
/// class 0 when some glyph of the set has no class (always when `!use_class0`), and the
/// definition with each class replaced by its position in that list.
fn subset_classes(map: &BTreeMap<u16, u16>, in_set: impl Fn(u16) -> bool, set_len: usize, use_class0: bool) -> (Vec<u16>, BTreeMap<u16, u16>) {
    let kept: BTreeMap<u16, u16> = map.iter().filter(|(g, _)| in_set(**g)).map(|(g, c)| (*g, *c)).collect();
    let mut indices: Vec<u16> = kept.values().copied().collect();
    if !use_class0 || set_len > kept.len() {
        indices.push(0);
    }
    let indices = sorted(indices);
    let renumbered = kept.iter().filter_map(|(g, c)| indices.binary_search(c).ok().map(|i| (*g, i as u16))).collect();
    (indices, renumbered)
}

/// The classes of `glyphs` (sorted), with 0 when one of them has no class.
fn intersect_classes(map: &BTreeMap<u16, u16>, glyphs: &[u16]) -> Vec<u16> {
    sorted(glyphs.iter().map(|g| map.get(g).copied().unwrap_or(0)).collect())
}

/// Which classes of one class definition the glyph set touches, taken when a subtable visit
/// starts.
struct ClassHits {
    present: BTreeSet<u16>,
    unclassed: bool,
}

impl ClassHits {
    fn new(map: &BTreeMap<u16, u16>, s: &Bits) -> Self {
        let mut present = BTreeSet::new();
        let mut classed = 0usize;
        for (g, c) in map {
            if s.contains(*g) {
                present.insert(*c);
                classed += 1;
            }
        }
        ClassHits { present, unclassed: s.len > classed }
    }

    fn hits(&self, class: u16) -> bool {
        if class == 0 { self.unclassed } else { self.present.contains(&class) }
    }
}

/// Class 0 of a missing class definition holds every glyph.
fn class_hits(h: Option<&ClassHits>, class: u16, s: &Bits) -> bool {
    match h {
        Some(h) => h.hits(class),
        None => class == 0 && s.len > 0,
    }
}

// ── Feature lists, scripts and feature variations ──

fn feature_subset_lookups(f: &mut write_fonts::tables::layout::Feature, map: &BTreeMap<u16, u16>) -> bool {
    f.lookup_list_indices = f.lookup_list_indices.iter().filter_map(|l| map.get(l).copied()).collect();
    !f.lookup_list_indices.is_empty() || matches!(f.feature_params.as_ref(), Some(FeatureParams::Size(_)))
}

fn langsys_collect(ls: &LangSys, out: &mut Vec<u16>) {
    out.extend(ls.feature_indices.iter().copied());
    if ls.required_feature_index != NO_FEATURE {
        out.push(ls.required_feature_index);
    }
}

/// Keeps and renumbers the features `map` lists; returns whether the language system still
/// has a feature.
fn langsys_prune(ls: &mut LangSys, map: &BTreeMap<u16, u16>) -> bool {
    ls.required_feature_index = map.get(&ls.required_feature_index).copied().unwrap_or(NO_FEATURE);
    ls.feature_indices = ls.feature_indices.iter().filter_map(|f| map.get(f).copied()).collect();
    !ls.feature_indices.is_empty() || ls.required_feature_index != NO_FEATURE
}

fn script_prune(script: &mut Script, map: &BTreeMap<u16, u16>, keep_empty_default: bool) -> bool {
    let empty_default = script.default_lang_sys.as_mut().is_some_and(|d| !langsys_prune(d, map));
    if empty_default && !keep_empty_default {
        script.default_lang_sys.clear();
    }
    script.lang_sys_records.retain_mut(|r| langsys_prune(&mut r.lang_sys, map));
    !script.lang_sys_records.is_empty() || script.default_lang_sys.as_ref().is_some()
}

fn scripts_prune(list: &mut ScriptList, map: &BTreeMap<u16, u16>, retain_empty: bool) {
    list.script_records.retain_mut(|r| {
        let keep = script_prune(&mut r.script, map, r.script_tag == OtTag::new(b"DFLT"));
        keep || retain_empty
    });
}

fn scripts_collect(list: &ScriptList) -> Vec<u16> {
    let mut out = Vec::new();
    for r in &list.script_records {
        if let Some(d) = r.script.default_lang_sys.as_ref() {
            langsys_collect(d, &mut out);
        }
        for l in &r.script.lang_sys_records {
            langsys_collect(&l.lang_sys, &mut out);
        }
    }
    sorted(out)
}

/// Feature index → position in `indices`.
fn index_map(indices: &[u16]) -> BTreeMap<u16, u16> {
    indices.iter().enumerate().map(|(new, old)| (*old, new as u16)).collect()
}

fn variations_collect(fv: &FeatureVariations, features: &[u16], out: &mut Vec<u16>) {
    for r in &fv.feature_variation_records {
        let Some(subst) = r.feature_table_substitution.as_ref() else { continue };
        for s in &subst.substitutions {
            if features.binary_search(&s.feature_index).is_ok() {
                out.extend(s.alternate_feature.lookup_list_indices.iter().copied());
            }
        }
    }
}

fn variations_subset_lookups(fv: &mut FeatureVariations, map: &BTreeMap<u16, u16>, out: &mut Vec<u16>) {
    for r in &mut fv.feature_variation_records {
        let Some(subst) = r.feature_table_substitution.as_mut() else { continue };
        for s in &mut subst.substitutions {
            if feature_subset_lookups(&mut s.alternate_feature, map) {
                out.push(s.feature_index);
            }
        }
    }
}

/// Keeps and renumbers the substituted features `map` lists; drops trailing records left
/// without substitutions.
fn variations_prune(fv: &mut FeatureVariations, map: &BTreeMap<u16, u16>) {
    for r in &mut fv.feature_variation_records {
        if let Some(subst) = r.feature_table_substitution.as_mut() {
            subst.substitutions.retain_mut(|s| match map.get(&s.feature_index) {
                Some(new) => {
                    s.feature_index = *new;
                    true
                }
                None => false,
            });
        }
    }
    while fv.feature_variation_records.last().is_some_and(|r| r.feature_table_substitution.as_ref().is_none_or(|s| s.substitutions.is_empty())) {
        fv.feature_variation_records.pop();
    }
}

// ── Lookups ──

/// A subtable of GSUB or GPOS.
trait Subtable {
    /// Keeps what the glyph set `s` can use; returns whether anything is left.
    fn subset(&mut self, s: &Bits) -> bool;
    /// The lookups a contextual subtable calls.
    fn nested(&self, _out: &mut Vec<u16>) {}
    /// Renumbers the called lookups (old index → new); calls to lookups not in `map` go.
    fn remap_nested(&mut self, _map: &BTreeMap<u16, u16>) {}
    fn prune_post(&mut self, _hinting: bool) {}
}

/// A GSUB or GPOS lookup.
trait LayoutLookup: Default {
    fn nested(&self, out: &mut Vec<u16>);
    fn remap_nested(&mut self, map: &BTreeMap<u16, u16>);
    /// Subsets every subtable and drops the empty ones; returns whether any is left.
    fn subset_subtables(&mut self, s: &Bits) -> bool;
    fn prune_post(&mut self, hinting: bool);
    fn flags(&mut self) -> (&mut LookupFlag, &mut Option<u16>);
}

fn subset_lookup<T: Subtable>(l: &mut Lookup<T>, s: &Bits) -> bool {
    l.subtables.retain_mut(|st| st.subset(s));
    !l.subtables.is_empty()
}

macro_rules! layout_lookup {
    ($enum:ident: $($variant:ident),+) => {
        impl LayoutLookup for $enum {
            fn nested(&self, out: &mut Vec<u16>) {
                match self { $($enum::$variant(l) => l.subtables.iter().for_each(|st| st.nested(out)),)+ }
            }
            fn remap_nested(&mut self, map: &BTreeMap<u16, u16>) {
                match self { $($enum::$variant(l) => l.subtables.iter_mut().for_each(|st| st.remap_nested(map)),)+ }
            }
            fn subset_subtables(&mut self, s: &Bits) -> bool {
                match self { $($enum::$variant(l) => subset_lookup(l, s),)+ }
            }
            fn prune_post(&mut self, hinting: bool) {
                match self { $($enum::$variant(l) => l.subtables.iter_mut().for_each(|st| st.prune_post(hinting)),)+ }
            }
            fn flags(&mut self) -> (&mut LookupFlag, &mut Option<u16>) {
                match self { $($enum::$variant(l) => (&mut l.lookup_flag, &mut l.mark_filtering_set),)+ }
            }
        }
    };
}

layout_lookup!(SubstitutionLookup: Single, Multiple, Alternate, Ligature, Contextual, ChainContextual, Extension, Reverse);
layout_lookup!(PositionLookup: Single, Pair, Cursive, MarkToBase, MarkToLig, MarkToMark, Contextual, ChainContextual, Extension);

macro_rules! extension_subtable {
    ($enum:ident: $($variant:ident),+) => {
        impl Subtable for $enum {
            fn subset(&mut self, s: &Bits) -> bool {
                match self { $($enum::$variant(e) => e.extension.subset(s),)+ }
            }
            fn nested(&self, out: &mut Vec<u16>) {
                match self { $($enum::$variant(e) => e.extension.nested(out),)+ }
            }
            fn remap_nested(&mut self, map: &BTreeMap<u16, u16>) {
                match self { $($enum::$variant(e) => e.extension.remap_nested(map),)+ }
            }
            fn prune_post(&mut self, hinting: bool) {
                match self { $($enum::$variant(e) => e.extension.prune_post(hinting),)+ }
            }
        }
    };
}

extension_subtable!(SubstExtension: Single, Multiple, Alternate, Ligature, Contextual, ChainContextual, Reverse);
extension_subtable!(PosExtension: Single, Pair, Cursive, MarkToBase, MarkToLig, MarkToMark, Contextual, ChainContextual);

macro_rules! context_subtable {
    ($ty:ty, $subset:ident, $nested:ident, $remap:ident) => {
        impl Subtable for $ty {
            fn subset(&mut self, s: &Bits) -> bool {
                $subset(self, s)
            }
            fn nested(&self, out: &mut Vec<u16>) {
                $nested(self, out)
            }
            fn remap_nested(&mut self, map: &BTreeMap<u16, u16>) {
                $remap(self, map)
            }
        }
    };
}

context_subtable!(SubstitutionSequenceContext, subset_sequence, sequence_nested, sequence_remap);
context_subtable!(PositionSequenceContext, subset_sequence, sequence_nested, sequence_remap);
context_subtable!(SubstitutionChainContext, subset_chain, chain_nested, chain_remap);
context_subtable!(PositionChainContext, subset_chain, chain_nested, chain_remap);

// ── The tables ──

/// GSUB or GPOS in owned form. A lookup set aside by the pruning before the closure is `None`
/// and keeps its index until the lookups are renumbered.
struct Layout<L> {
    scripts: ScriptList,
    features: FeatureList,
    variations: Option<FeatureVariations>,
    lookups: Vec<Option<L>>,
    /// GSUB keeps scripts left without features; GPOS drops them.
    retain_empty_scripts: bool,
}

impl<L: LayoutLookup> Layout<L> {
    fn feature_tags(&self) -> impl Iterator<Item = Tag> + '_ {
        self.features.feature_records.iter().map(|r| r.feature_tag.to_be_bytes())
    }

    fn subset_script_tags(&mut self, tags: &[String]) {
        let mut langs: BTreeMap<[u8; 4], BTreeSet<[u8; 4]>> = BTreeMap::new();
        for tag in tags {
            let (script, lang) = tag.split_once('.').unwrap_or((tag.as_str(), "*"));
            let (Some(script), Some(lang)) = (padded(script), padded(lang)) else { continue };
            langs.entry(script).or_default().insert(lang);
        }
        self.scripts.script_records.retain(|r| langs.contains_key(&r.script_tag.to_be_bytes()));
        for r in &mut self.scripts.script_records {
            let Some(wanted) = langs.get(&r.script_tag.to_be_bytes()) else { continue };
            if wanted.contains(b"*   ") {
                continue;
            }
            r.script.lang_sys_records.retain(|l| wanted.contains(&l.lang_sys_tag.to_be_bytes()));
            if !wanted.contains(b"dflt") {
                r.script.default_lang_sys.clear();
            }
        }
    }

    fn subset_feature_tags(&mut self, tags: &BTreeSet<[u8; 4]>) {
        let indices: Vec<u16> = self
            .features
            .feature_records
            .iter()
            .enumerate()
            .filter(|(_, r)| tags.contains(&r.feature_tag.to_be_bytes()))
            .map(|(i, _)| i as u16)
            .collect();
        self.subset_features(&indices);
    }

    /// Keeps the features at `indices` (sorted) and renumbers every reference to them.
    fn subset_features(&mut self, indices: &[u16]) {
        let map = index_map(indices);
        self.keep_feature_records(indices);
        if let Some(fv) = self.variations.as_mut() {
            variations_prune(fv, &map);
        }
        scripts_prune(&mut self.scripts, &map, self.retain_empty_scripts);
    }

    fn keep_feature_records(&mut self, indices: &[u16]) {
        let mut old: Vec<Option<_>> = take(&mut self.features.feature_records).into_iter().map(Some).collect();
        self.features.feature_records = indices.iter().filter_map(|i| old.get_mut(usize::from(*i)).and_then(Option::take)).collect();
    }

    /// The lookups the features of the scripts use directly, sorted.
    fn feature_lookups(&self) -> Vec<u16> {
        let features: Vec<u16> =
            scripts_collect(&self.scripts).into_iter().filter(|f| usize::from(*f) < self.features.feature_records.len()).collect();
        let mut out = Vec::new();
        for f in &features {
            if let Some(r) = self.features.feature_records.get(usize::from(*f)) {
                out.extend(r.feature.lookup_list_indices.iter().copied());
            }
        }
        if let Some(fv) = self.variations.as_ref() {
            variations_collect(fv, &features, &mut out);
        }
        sorted(out).into_iter().filter(|l| usize::from(*l) < self.lookups.len()).collect()
    }

    /// `roots` and every lookup they call, at any depth, sorted.
    fn closure_lookups(&self, roots: Vec<u16>) -> Vec<u16> {
        let mut all: BTreeSet<u16> = roots.iter().copied().collect();
        let mut frontier = roots;
        // Each pass adds at least one lookup, so there are at most as many passes as lookups.
        while !frontier.is_empty() {
            let mut next = BTreeSet::new();
            for i in &frontier {
                let Some(Some(l)) = self.lookups.get(usize::from(*i)) else { continue };
                let mut called = Vec::new();
                l.nested(&mut called);
                for c in called {
                    if usize::from(c) < self.lookups.len() && !all.contains(&c) {
                        next.insert(c);
                    }
                }
            }
            all.extend(next.iter().copied());
            frontier = next.into_iter().collect();
        }
        all.into_iter().collect()
    }

    fn prune_lookups(&mut self, remap: bool) {
        let keep = self.closure_lookups(self.feature_lookups());
        if remap {
            self.subset_lookups(&keep);
        } else {
            for (i, l) in self.lookups.iter_mut().enumerate() {
                if !u16::try_from(i).is_ok_and(|i| keep.binary_search(&i).is_ok()) {
                    *l = None;
                }
            }
        }
    }

    /// Keeps the lookups at `keep` (sorted), renumbers every reference to them, and drops the
    /// features, language systems and scripts left empty.
    fn subset_lookups(&mut self, keep: &[u16]) {
        let keep: Vec<u16> = keep.iter().copied().filter(|l| usize::from(*l) < self.lookups.len()).collect();
        let map = index_map(&keep);
        let mut old = take(&mut self.lookups);
        self.lookups = keep.iter().map(|i| old.get_mut(usize::from(*i)).and_then(Option::take)).collect();
        for l in self.lookups.iter_mut().flatten() {
            l.remap_nested(&map);
        }
        let mut features = Vec::new();
        for (i, r) in self.features.feature_records.iter_mut().enumerate() {
            if feature_subset_lookups(&mut r.feature, &map) || r.feature_tag == OtTag::new(b"pref") {
                features.push(i as u16);
            }
        }
        if let Some(fv) = self.variations.as_mut() {
            variations_subset_lookups(fv, &map, &mut features);
        }
        let features: Vec<u16> = sorted(features).into_iter().filter(|f| usize::from(*f) < self.features.feature_records.len()).collect();
        self.subset_features(&features);
    }

    fn prune_pre(&mut self, options: &SubsetOptions) {
        if let Some(scripts) = options.layout_scripts.as_ref().filter(|s| !s.iter().any(|t| t == "*")) {
            self.subset_script_tags(scripts);
        }
        let wanted: Option<BTreeSet<[u8; 4]>> = match options.layout_features.as_ref() {
            None => Some(DEFAULT_LAYOUT_FEATURES.iter().filter_map(|t| padded(t)).collect()),
            Some(list) if list.iter().any(|t| t == "*") => None,
            Some(list) => Some(list.iter().filter_map(|t| padded(t)).collect()),
        };
        if let Some(wanted) = wanted {
            self.subset_feature_tags(&wanted);
        }
        self.prune_lookups(false);
    }

    fn subset_glyphs(&mut self, s: &Bits, used_mark_sets: &[u16]) {
        let mut nonempty = Vec::new();
        for (i, slot) in self.lookups.iter_mut().enumerate() {
            let Some(l) = slot.as_mut() else { continue };
            let any = l.subset_subtables(s);
            let (flag, set) = l.flags();
            if let Some(old) = *set {
                match used_mark_sets.binary_search(&old) {
                    Ok(new) => *set = Some(new as u16),
                    Err(_) => {
                        *set = None;
                        *flag = LookupFlag::from_bits_truncate((flag.to_bits() & !0x0010) | 0x0008);
                    }
                }
            }
            if any {
                nonempty.push(i as u16);
            }
        }
        self.subset_lookups(&nonempty);
    }

    /// Drops language systems whose features equal their script's default language system's.
    fn remove_redundant_langsys(&mut self) {
        let features = &self.features.feature_records;
        let same = |a: u16, b: u16| match (features.get(usize::from(a)), features.get(usize::from(b))) {
            (Some(x), Some(y)) => x == y,
            _ => false,
        };
        for r in &mut self.scripts.script_records {
            let Some(d) = r.script.default_lang_sys.as_ref().cloned() else { continue };
            r.script.lang_sys_records.retain(|lr| {
                let l = &lr.lang_sys;
                let equal = d.feature_indices.len() == l.feature_indices.len()
                    && (d.required_feature_index == NO_FEATURE) == (l.required_feature_index == NO_FEATURE)
                    && (d.required_feature_index == NO_FEATURE || same(d.required_feature_index, l.required_feature_index))
                    && d.feature_indices.iter().zip(&l.feature_indices).all(|(a, b)| same(*a, *b));
                !equal
            });
        }
    }

    /// Drops features no language system uses and merges features equal to an earlier one with
    /// the same tag.
    fn prune_features(&mut self) {
        let count = self.features.feature_records.len();
        let used: Vec<u16> = scripts_collect(&self.scripts).into_iter().filter(|f| usize::from(*f) < count).collect();
        let records = &self.features.feature_records;
        let mut unique: BTreeMap<OtTag, Vec<u16>> = BTreeMap::new();
        let mut duplicate_of: BTreeMap<u16, u16> = BTreeMap::new();
        for i in &used {
            let Some(f) = records.get(usize::from(*i)) else { continue };
            let same_tag = unique.entry(f.feature_tag).or_default();
            match same_tag.iter().find(|o| records.get(usize::from(**o)) == Some(f)) {
                Some(o) => {
                    duplicate_of.insert(*i, *o);
                }
                None => {
                    same_tag.push(*i);
                    duplicate_of.insert(*i, *i);
                }
            }
        }
        let mut map: BTreeMap<u16, u16> = BTreeMap::new();
        let mut next = 0u16;
        for i in &used {
            let u = duplicate_of.get(i).copied().unwrap_or(*i);
            match map.get(&u).copied() {
                Some(v) => {
                    map.insert(*i, v);
                }
                None => {
                    map.insert(*i, next);
                    next = next.saturating_add(1);
                }
            }
        }
        let retained = sorted(unique.into_values().flatten().collect());
        self.keep_feature_records(&retained);
        if let Some(fv) = self.variations.as_mut() {
            variations_prune(fv, &map);
        }
        scripts_prune(&mut self.scripts, &map, self.retain_empty_scripts);
    }

    fn prune_post(&mut self, hinting: bool) {
        self.prune_lookups(true);
        for l in self.lookups.iter_mut().flatten() {
            l.prune_post(hinting);
        }
        self.remove_redundant_langsys();
        self.prune_features();
        if self.variations.as_ref().is_some_and(|fv| fv.feature_variation_records.is_empty()) {
            self.variations = None;
        }
    }

    fn into_parts(self) -> (ScriptList, FeatureList, LookupList<L>, Option<FeatureVariations>) {
        let lookups = self.lookups.into_iter().map(Option::unwrap_or_default).collect();
        (self.scripts, self.features, LookupList::new(lookups), self.variations)
    }
}

fn read_gsub(bytes: &[u8]) -> Result<Layout<SubstitutionLookup>> {
    let tag = *b"GSUB";
    crate::layout_budget::check_gsub(bytes)?;
    let table = read_fonts::tables::gsub::Gsub::read(FontData::new(bytes)).map_err(|e| read_err(tag, e))?;
    let owned: Gsub = table.to_owned_table();
    let Gsub { script_list, feature_list, lookup_list, feature_variations } = owned;
    Ok(Layout {
        scripts: script_list.into_inner(),
        features: feature_list.into_inner(),
        variations: feature_variations.into_inner(),
        lookups: lookup_list.into_inner().lookups.into_iter().map(|l| Some(l.into_inner())).collect(),
        retain_empty_scripts: true,
    })
}

fn read_gpos(bytes: &[u8]) -> Result<Layout<PositionLookup>> {
    let tag = *b"GPOS";
    crate::layout_budget::check_gpos(bytes)?;
    let table = read_fonts::tables::gpos::Gpos::read(FontData::new(bytes)).map_err(|e| read_err(tag, e))?;
    let mut owned: Gpos = table.to_owned_table();
    crate::layout_budget::restore_empty_pair_records(&mut owned)?;
    let Gpos { script_list, feature_list, lookup_list, feature_variations } = owned;
    Ok(Layout {
        scripts: script_list.into_inner(),
        features: feature_list.into_inner(),
        variations: feature_variations.into_inner(),
        lookups: lookup_list.into_inner().lookups.into_iter().map(|l| Some(l.into_inner())).collect(),
        retain_empty_scripts: false,
    })
}

fn write_gsub(t: Layout<SubstitutionLookup>) -> Result<Vec<u8>> {
    let (scripts, features, lookups, variations) = t.into_parts();
    let mut table = Gsub::new(scripts, features, lookups);
    table.feature_variations = variations.into();
    write_fonts::dump_table(&table).map_err(|e| write_err(*b"GSUB", e))
}

fn write_gpos(t: Layout<PositionLookup>) -> Result<Vec<u8>> {
    let (scripts, features, lookups, variations) = t.into_parts();
    let mut table = Gpos::new(scripts, features, lookups);
    table.feature_variations = variations.into();
    write_fonts::dump_table(&table).map_err(|e| write_err(*b"GPOS", e))
}

/// Adds to `glyphs` (ids below `n`) every glyph the GSUB lookups of the kept features can
/// substitute in, when `options.layout_closure` asks for it; returns the feature tags that
/// GSUB and GPOS keep before subsetting.
pub(crate) fn close_glyphs(font: &SfntFont, options: &SubsetOptions, glyphs: &mut BTreeSet<u32>, n: usize) -> Result<BTreeSet<Tag>> {
    let mut tags = BTreeSet::new();
    if let Some(bytes) = font.get(*b"GPOS") {
        let mut gpos = read_gpos(bytes)?;
        gpos.prune_pre(options);
        tags.extend(gpos.feature_tags());
    }
    let Some(bytes) = font.get(*b"GSUB") else { return Ok(tags) };
    let mut gsub = read_gsub(bytes)?;
    gsub.prune_pre(options);
    tags.extend(gsub.feature_tags());
    if !options.layout_closure {
        return Ok(tags);
    }
    let roots = gsub.feature_lookups();
    let mut cx = Closure { lookups: &gsub.lookups, glyphs: Bits::from_set(glyphs), done: HashMap::new(), depth: 0, work: MAX_CLOSURE_WORK };
    // Every round but the last adds a glyph, so there are at most 65,537 rounds.
    loop {
        let before = cx.glyphs.len;
        for li in &roots {
            cx.close_lookup(usize::from(*li), None)?;
        }
        if cx.glyphs.len == before {
            break;
        }
    }
    glyphs.extend(cx.glyphs.iter().map(u32::from).filter(|g| (*g as usize) < n));
    Ok(tags)
}

/// Writes the subset of the tables this module handles from `input` into `output`.
pub fn apply(plan: &Plan, input: &SfntFont, output: &mut SfntFont) -> Result<()> {
    let s = Bits::from_set(&plan.layout_glyphs);
    let order = output_order(plan);
    let mut used_mark_sets = Vec::new();
    let mut gdef = None;
    if let Some(bytes) = input.get(*b"GDEF") {
        let (g, used) = subset_gdef(bytes, &s)?;
        used_mark_sets = used;
        gdef = Some(g);
    }
    let mut gpos = None;
    if let Some(bytes) = input.get(*b"GPOS") {
        let mut t = read_gpos(bytes)?;
        t.prune_pre(&plan.options);
        t.subset_glyphs(&s, &used_mark_sets);
        gpos = Some(t);
    }
    // GDEF's store is pruned before GPOS's own pruning, as pyftsubset's table order has it.
    if let (Some(g), Some(bytes)) = (gdef.as_mut(), input.get(*b"GDEF")) {
        prune_variation_store(g, bytes, gpos.as_mut())?;
    }
    if let Some(gdef) = gdef.filter(gdef_keep) {
        let bytes = write_fonts::dump_table(&gdef).map_err(|e| write_err(*b"GDEF", e))?;
        output.insert(*b"GDEF", crate::reorder::layout::gdef(&bytes, &order)?);
    }
    if let Some(mut gpos) = gpos {
        gpos.prune_post(plan.options.hinting);
        output.insert(*b"GPOS", crate::reorder::layout::gpos(&write_gpos(gpos)?, &order)?);
    }
    if let Some(bytes) = input.get(*b"GSUB") {
        let mut gsub = read_gsub(bytes)?;
        gsub.prune_pre(&plan.options);
        gsub.subset_glyphs(&s, &used_mark_sets);
        gsub.prune_post(plan.options.hinting);
        output.insert(*b"GSUB", crate::reorder::layout::gsub(&write_gsub(gsub)?, &order)?);
    }
    Ok(())
}

/// Input glyph id → output glyph id, for every input glyph; dropped ones map to
/// [`Order::DROPPED`], which no subset table may refer to.
fn output_order(plan: &Plan) -> Order {
    let mut old_to_new = vec![Order::DROPPED; plan.num_glyphs];
    let mut new_to_old = Vec::with_capacity(plan.new_to_old.len());
    for (new, old) in plan.new_to_old.iter().enumerate() {
        let old16 = u16::try_from(*old).unwrap_or(u16::MAX);
        new_to_old.push(old16);
        if let Some(slot) = old_to_new.get_mut(*old as usize) {
            *slot = new as u16;
        }
    }
    Order { new_to_old, old_to_new }
}

// ── GDEF ──

/// GDEF subset to `s` (see [`gdef_keep`] for whether anything is left), and the indices of the
/// mark glyph sets still in use, which the lookups' mark filtering sets are renumbered by.
fn subset_gdef(bytes: &[u8], s: &Bits) -> Result<(Gdef, Vec<u16>)> {
    let tag = *b"GDEF";
    crate::layout_budget::check_gdef(bytes)?;
    let table = read_fonts::tables::gdef::Gdef::read(FontData::new(bytes)).map_err(|e| read_err(tag, e))?;
    let mut g: Gdef = table.to_owned_table();
    let mut empty = false;
    if let Some(list) = g.lig_caret_list.as_mut() {
        let (cov, ligs) = filter_parallel(&list.coverage, take(&mut list.lig_glyphs), |glyph, _| s.contains(glyph));
        list.coverage = cov.into();
        list.lig_glyphs = ligs;
        empty = list.lig_glyphs.is_empty();
    }
    if empty {
        g.lig_caret_list.clear();
    }
    for cd in [&mut g.mark_attach_class_def, &mut g.glyph_class_def] {
        let kept = cd.as_ref().map(|c| classes(c).into_iter().filter(|(glyph, _)| s.contains(*glyph)).collect::<BTreeMap<_, _>>());
        match kept {
            Some(kept) if kept.is_empty() => cd.clear(),
            Some(kept) => cd.set(class_def(&kept)),
            None => {}
        }
    }
    empty = false;
    if let Some(list) = g.attach_list.as_mut() {
        let (cov, points) = filter_parallel(&list.coverage, take(&mut list.attach_points), |glyph, _| s.contains(glyph));
        list.coverage = cov.into();
        list.attach_points = points;
        empty = list.attach_points.is_empty();
    }
    if empty {
        g.attach_list.clear();
    }
    let mut used = Vec::new();
    empty = false;
    if let Some(sets) = g.mark_glyph_sets_def.as_mut() {
        let mut kept = Vec::new();
        for (i, mut c) in take(&mut sets.coverages).into_iter().enumerate() {
            if subset_coverage(&mut c, s) {
                used.push(i as u16);
                kept.push(c);
            }
        }
        sets.coverages = kept;
        empty = sets.coverages.is_empty();
    }
    if empty {
        g.mark_glyph_sets_def.clear();
    }
    Ok((g, used))
}

/// GDEF still holds something.
fn gdef_keep(g: &Gdef) -> bool {
    g.lig_caret_list.as_ref().is_some()
        || g.mark_attach_class_def.as_ref().is_some()
        || g.glyph_class_def.as_ref().is_some()
        || g.attach_list.as_ref().is_some()
        || g.mark_glyph_sets_def.as_ref().is_some()
        || g.item_var_store.as_ref().is_some()
}

/// Prunes GDEF's variation store to the delta sets GDEF's caret values and GPOS's value
/// records and anchors still use, and renumbers their variation indices (`_pruneGDEF`).
fn prune_variation_store(g: &mut Gdef, bytes: &[u8], mut gpos: Option<&mut Layout<PositionLookup>>) -> Result<()> {
    if g.item_var_store.as_ref().is_none() {
        return Ok(());
    }
    let mut used = BTreeSet::new();
    layout_devices(g, gpos.as_deref_mut(), &mut |d| {
        if let DeviceOrVariationIndex::VariationIndex(v) = d {
            used.insert(u32::from(v.delta_set_outer_index) << 16 | u32::from(v.delta_set_inner_index));
        }
    });
    let Some((store, map)) = super::var::prune_layout_store(bytes, &used)? else { return Ok(()) };
    if store.is_empty() {
        g.item_var_store.clear();
    } else {
        let read = read_fonts::tables::variations::ItemVariationStore::read(FontData::new(&store)).map_err(|e| read_err(*b"GDEF", e))?;
        let owned: write_fonts::tables::variations::ItemVariationStore = read.to_owned_table();
        g.item_var_store.set(owned);
    }
    layout_devices(g, gpos, &mut |d| {
        if let DeviceOrVariationIndex::VariationIndex(v) = d {
            let old = u32::from(v.delta_set_outer_index) << 16 | u32::from(v.delta_set_inner_index);
            let new = map.get(&old).copied().unwrap_or(super::var::store::NO_VARIATION);
            v.delta_set_outer_index = (new >> 16) as u16;
            v.delta_set_inner_index = new as u16;
        }
    });
    Ok(())
}

pub(crate) type DeviceFn<'f> = dyn FnMut(&mut DeviceOrVariationIndex) + 'f;

/// Calls `f` on every device table and variation index of GDEF's caret values and of the live
/// GPOS lookups.
fn layout_devices(g: &mut Gdef, gpos: Option<&mut Layout<PositionLookup>>, f: &mut DeviceFn<'_>) {
    if let Some(list) = g.lig_caret_list.as_mut() {
        for lig in &mut list.lig_glyphs {
            for caret in &mut lig.caret_values {
                if let write_fonts::tables::gdef::CaretValue::Format3(c) = &mut **caret {
                    f(&mut c.device);
                }
            }
        }
    }
    for lookup in gpos.into_iter().flat_map(|t| t.lookups.iter_mut()).flatten() {
        lookup.devices(f);
    }
}

fn value_devices(v: &mut ValueRecord, f: &mut DeviceFn<'_>) {
    for d in [&mut v.x_placement_device, &mut v.y_placement_device, &mut v.x_advance_device, &mut v.y_advance_device] {
        if let Some(d) = d.as_mut() {
            f(d);
        }
    }
}

fn anchor_devices(a: &mut AnchorTable, f: &mut DeviceFn<'_>) {
    if let AnchorTable::Format3(a) = a {
        for d in [&mut a.x_device, &mut a.y_device] {
            if let Some(d) = d.as_mut() {
                f(d);
            }
        }
    }
}

fn anchors_devices<'a>(anchors: impl Iterator<Item = &'a mut NullableOffsetMarker<AnchorTable>>, f: &mut DeviceFn<'_>) {
    for a in anchors {
        if let Some(a) = a.as_mut() {
            anchor_devices(a, f);
        }
    }
}

/// A GPOS lookup or subtable whose device tables can be visited.
pub(crate) trait Devices {
    fn devices(&mut self, f: &mut DeviceFn<'_>);
}

macro_rules! no_devices {
    ($($ty:ty),+) => { $(impl Devices for $ty { fn devices(&mut self, _f: &mut DeviceFn<'_>) {} })+ };
}

no_devices!(PositionSequenceContext, PositionChainContext);

impl Devices for SinglePos {
    fn devices(&mut self, f: &mut DeviceFn<'_>) {
        match self {
            SinglePos::Format1(t) => value_devices(&mut t.value_record, f),
            SinglePos::Format2(t) => t.value_records.iter_mut().for_each(|v| value_devices(v, f)),
        }
    }
}

impl Devices for PairPos {
    fn devices(&mut self, f: &mut DeviceFn<'_>) {
        match self {
            PairPos::Format1(t) => {
                for r in t.pair_sets.iter_mut().flat_map(|s| s.pair_value_records.iter_mut()) {
                    value_devices(&mut r.value_record1, f);
                    value_devices(&mut r.value_record2, f);
                }
            }
            PairPos::Format2(t) => {
                for r in t.class1_records.iter_mut().flat_map(|c| c.class2_records.iter_mut()) {
                    value_devices(&mut r.value_record1, f);
                    value_devices(&mut r.value_record2, f);
                }
            }
        }
    }
}

impl Devices for CursivePosFormat1 {
    fn devices(&mut self, f: &mut DeviceFn<'_>) {
        for r in &mut self.entry_exit_record {
            anchors_devices([&mut r.entry_anchor, &mut r.exit_anchor].into_iter(), f);
        }
    }
}

fn mark_array_devices(m: &mut write_fonts::tables::gpos::MarkArray, f: &mut DeviceFn<'_>) {
    for r in &mut m.mark_records {
        anchor_devices(&mut r.mark_anchor, f);
    }
}

impl Devices for MarkBasePosFormat1 {
    fn devices(&mut self, f: &mut DeviceFn<'_>) {
        mark_array_devices(&mut self.mark_array, f);
        anchors_devices(self.base_array.base_records.iter_mut().flat_map(|r| r.base_anchors.iter_mut()), f);
    }
}

impl Devices for MarkLigPosFormat1 {
    fn devices(&mut self, f: &mut DeviceFn<'_>) {
        mark_array_devices(&mut self.mark_array, f);
        let anchors =
            self.ligature_array.ligature_attaches.iter_mut().flat_map(|a| a.component_records.iter_mut()).flat_map(|c| c.ligature_anchors.iter_mut());
        anchors_devices(anchors, f);
    }
}

impl Devices for MarkMarkPosFormat1 {
    fn devices(&mut self, f: &mut DeviceFn<'_>) {
        mark_array_devices(&mut self.mark1_array, f);
        anchors_devices(self.mark2_array.mark2_records.iter_mut().flat_map(|r| r.mark2_anchors.iter_mut()), f);
    }
}

impl Devices for PosExtension {
    fn devices(&mut self, f: &mut DeviceFn<'_>) {
        match self {
            PosExtension::Single(e) => e.extension.devices(f),
            PosExtension::Pair(e) => e.extension.devices(f),
            PosExtension::Cursive(e) => e.extension.devices(f),
            PosExtension::MarkToBase(e) => e.extension.devices(f),
            PosExtension::MarkToLig(e) => e.extension.devices(f),
            PosExtension::MarkToMark(e) => e.extension.devices(f),
            PosExtension::Contextual(e) => e.extension.devices(f),
            PosExtension::ChainContextual(e) => e.extension.devices(f),
        }
    }
}

impl Devices for PositionLookup {
    fn devices(&mut self, f: &mut DeviceFn<'_>) {
        macro_rules! each {
            ($l:expr) => {
                for st in &mut $l.subtables {
                    st.devices(f);
                }
            };
        }
        match self {
            PositionLookup::Single(l) => each!(l),
            PositionLookup::Pair(l) => each!(l),
            PositionLookup::Cursive(l) => each!(l),
            PositionLookup::MarkToBase(l) => each!(l),
            PositionLookup::MarkToLig(l) => each!(l),
            PositionLookup::MarkToMark(l) => each!(l),
            PositionLookup::Contextual(l) => each!(l),
            PositionLookup::ChainContextual(l) => each!(l),
            PositionLookup::Extension(l) => each!(l),
        }
    }
}

// ── GSUB subtables ──

fn single_subst(map: &BTreeMap<u16, u16>) -> SingleSubst {
    let glyphs: Vec<u16> = map.keys().copied().collect();
    let delta = |(a, b): (&u16, &u16)| b.wrapping_sub(*a);
    let first = map.iter().next().map(delta);
    match first {
        Some(d) if map.iter().all(|p| delta(p) == d) => SingleSubst::Format1(SingleSubstFormat1::new(coverage(&glyphs), d as i16)),
        _ => SingleSubst::Format2(SingleSubstFormat2::new(coverage(&glyphs), map.values().map(|g| G::new(*g)).collect())),
    }
}

fn single_pairs(s: &SingleSubst) -> Vec<(u16, u16)> {
    match s {
        SingleSubst::Format1(f) => f.coverage.iter().map(|g| (g.to_u16(), g.to_u16().wrapping_add(f.delta_glyph_id as u16))).collect(),
        SingleSubst::Format2(f) => f.coverage.iter().zip(&f.substitute_glyph_ids).map(|(a, b)| (a.to_u16(), b.to_u16())).collect(),
    }
}

impl Subtable for SingleSubst {
    fn subset(&mut self, s: &Bits) -> bool {
        let mut map: BTreeMap<u16, u16> = BTreeMap::new();
        for (a, b) in single_pairs(self) {
            map.insert(a, b);
        }
        map.retain(|a, b| s.contains(*a) && s.contains(*b));
        *self = single_subst(&map);
        !map.is_empty()
    }
}

impl Subtable for MultipleSubstFormat1 {
    fn subset(&mut self, s: &Bits) -> bool {
        let (cov, seqs) = filter_parallel(&self.coverage, take(&mut self.sequences), |g, seq| {
            s.contains(g) && seq.substitute_glyph_ids.iter().all(|x| s.contains(x.to_u16()))
        });
        self.coverage = cov.into();
        self.sequences = seqs;
        !self.sequences.is_empty()
    }
}

impl Subtable for AlternateSubstFormat1 {
    fn subset(&mut self, s: &Bits) -> bool {
        let (cov, sets) = filter_parallel(&self.coverage, take(&mut self.alternate_sets), |g, set| {
            if !s.contains(g) {
                return false;
            }
            set.alternate_glyph_ids.retain(|a| s.contains(a.to_u16()));
            !set.alternate_glyph_ids.is_empty()
        });
        self.coverage = cov.into();
        self.alternate_sets = sets;
        !self.alternate_sets.is_empty()
    }
}

impl Subtable for LigatureSubstFormat1 {
    fn subset(&mut self, s: &Bits) -> bool {
        let (cov, sets) = filter_parallel(&self.coverage, take(&mut self.ligature_sets), |g, set| {
            if !s.contains(g) {
                return false;
            }
            set.ligatures.retain(|l| s.contains(l.ligature_glyph.to_u16()) && l.component_glyph_ids.iter().all(|c| s.contains(c.to_u16())));
            !set.ligatures.is_empty()
        });
        self.coverage = cov.into();
        self.ligature_sets = sets;
        !self.ligature_sets.is_empty()
    }
}

impl Subtable for ReverseChainSingleSubstFormat1 {
    fn subset(&mut self, s: &Bits) -> bool {
        let (cov, subs) = filter_parallel(&self.coverage, take(&mut self.substitute_glyph_ids), |g, sub| s.contains(g) && s.contains(sub.to_u16()));
        self.coverage = cov.into();
        self.substitute_glyph_ids = subs;
        let mut ok = !self.substitute_glyph_ids.is_empty();
        for c in self.lookahead_coverages.iter_mut().chain(self.backtrack_coverages.iter_mut()) {
            ok &= subset_coverage(c, s);
        }
        ok
    }
}

// ── Contextual subtables (GSUB and GPOS) ──

fn remap_records(records: &mut Vec<SequenceLookupRecord>, map: &BTreeMap<u16, u16>) {
    records.retain_mut(|r| match map.get(&r.lookup_list_index) {
        Some(new) => {
            r.lookup_list_index = *new;
            true
        }
        None => false,
    });
}

fn records_nested(records: &[SequenceLookupRecord], out: &mut Vec<u16>) {
    out.extend(records.iter().map(|r| r.lookup_list_index));
}

fn sequence_nested(c: &SequenceContext, out: &mut Vec<u16>) {
    match c {
        SequenceContext::Format1(f) => {
            for set in f.seq_rule_sets.iter().filter_map(|s| s.as_ref()) {
                set.seq_rules.iter().for_each(|r| records_nested(&r.seq_lookup_records, out));
            }
        }
        SequenceContext::Format2(f) => {
            for set in f.class_seq_rule_sets.iter().filter_map(|s| s.as_ref()) {
                set.class_seq_rules.iter().for_each(|r| records_nested(&r.seq_lookup_records, out));
            }
        }
        SequenceContext::Format3(f) => records_nested(&f.seq_lookup_records, out),
    }
}

fn sequence_remap(c: &mut SequenceContext, map: &BTreeMap<u16, u16>) {
    match c {
        SequenceContext::Format1(f) => {
            for set in f.seq_rule_sets.iter_mut().filter_map(|s| s.as_mut()) {
                set.seq_rules.iter_mut().for_each(|r| remap_records(&mut r.seq_lookup_records, map));
            }
        }
        SequenceContext::Format2(f) => {
            for set in f.class_seq_rule_sets.iter_mut().filter_map(|s| s.as_mut()) {
                set.class_seq_rules.iter_mut().for_each(|r| remap_records(&mut r.seq_lookup_records, map));
            }
        }
        SequenceContext::Format3(f) => remap_records(&mut f.seq_lookup_records, map),
    }
}

fn chain_nested(c: &ChainedSequenceContext, out: &mut Vec<u16>) {
    match c {
        ChainedSequenceContext::Format1(f) => {
            for set in f.chained_seq_rule_sets.iter().filter_map(|s| s.as_ref()) {
                set.chained_seq_rules.iter().for_each(|r| records_nested(&r.seq_lookup_records, out));
            }
        }
        ChainedSequenceContext::Format2(f) => {
            for set in f.chained_class_seq_rule_sets.iter().filter_map(|s| s.as_ref()) {
                set.chained_class_seq_rules.iter().for_each(|r| records_nested(&r.seq_lookup_records, out));
            }
        }
        ChainedSequenceContext::Format3(f) => records_nested(&f.seq_lookup_records, out),
    }
}

fn chain_remap(c: &mut ChainedSequenceContext, map: &BTreeMap<u16, u16>) {
    match c {
        ChainedSequenceContext::Format1(f) => {
            for set in f.chained_seq_rule_sets.iter_mut().filter_map(|s| s.as_mut()) {
                set.chained_seq_rules.iter_mut().for_each(|r| remap_records(&mut r.seq_lookup_records, map));
            }
        }
        ChainedSequenceContext::Format2(f) => {
            for set in f.chained_class_seq_rule_sets.iter_mut().filter_map(|s| s.as_mut()) {
                set.chained_class_seq_rules.iter_mut().for_each(|r| remap_records(&mut r.seq_lookup_records, map));
            }
        }
        ChainedSequenceContext::Format3(f) => remap_records(&mut f.seq_lookup_records, map),
    }
}

/// Classes renumbered by their position in `map`; `None` when a class is not in it.
fn renumber(classes: &[u16], map: &[u16]) -> Option<Vec<u16>> {
    classes.iter().map(|k| map.binary_search(k).ok().map(|i| i as u16)).collect()
}

/// The rule sets of a class-based context after its input class definition was subset:
/// `old` indexed by old class, `input_map` the old classes kept (in new-class order),
/// `reachable` the new classes of the kept coverage glyphs. `keep` filters and renumbers the
/// rules of one set and says whether any is left.
fn class_sets<S>(
    old: Vec<NullableOffsetMarker<S>>,
    input_map: &[u16],
    reachable: &[u16],
    mut keep: impl FnMut(&mut S) -> bool,
) -> Vec<NullableOffsetMarker<S>> {
    let mut old: Vec<Option<S>> = old.into_iter().map(NullableOffsetMarker::into_inner).collect();
    let mut sets: Vec<Option<S>> = input_map.iter().filter_map(|k| old.get_mut(usize::from(*k)).map(Option::take)).collect();
    for (j, set) in sets.iter_mut().enumerate() {
        if !reachable.contains(&(j as u16)) {
            *set = None;
            continue;
        }
        if set.as_mut().is_some_and(|rs| !keep(rs)) {
            *set = None;
        }
    }
    while sets.last().is_some_and(Option::is_none) {
        sets.pop();
    }
    sets.into_iter().map(NullableOffsetMarker::from).collect()
}

fn subset_sequence(c: &mut SequenceContext, s: &Bits) -> bool {
    match c {
        SequenceContext::Format1(f) => {
            let (cov, sets) = filter_parallel(&f.coverage, take(&mut f.seq_rule_sets), |g, set| {
                if !s.contains(g) {
                    return false;
                }
                let Some(set) = set.as_mut() else { return false };
                set.seq_rules.retain(|r| r.input_sequence.iter().all(|x| s.contains(x.to_u16())));
                !set.seq_rules.is_empty()
            });
            f.coverage = cov.into();
            f.seq_rule_sets = sets;
            !f.seq_rule_sets.is_empty()
        }
        SequenceContext::Format2(f) => {
            if !subset_coverage(&mut f.coverage, s) {
                return false;
            }
            let (input_map, input) = subset_classes(&classes(&f.class_def), |g| s.contains(g), s.len, true);
            f.class_def = class_def(&input).into();
            let reachable = intersect_classes(&input, &glyphs_of(&f.coverage));
            f.class_seq_rule_sets = class_sets(take(&mut f.class_seq_rule_sets), &input_map, &reachable, |rs| {
                rs.class_seq_rules.retain_mut(|r| match renumber(&r.input_sequence, &input_map) {
                    Some(new) => {
                        r.input_sequence = new;
                        true
                    }
                    None => false,
                });
                !rs.class_seq_rules.is_empty()
            });
            !f.class_seq_rule_sets.is_empty()
        }
        SequenceContext::Format3(f) => {
            let mut ok = true;
            for c in &mut f.coverages {
                ok &= subset_coverage(c, s);
            }
            ok
        }
    }
}

fn subset_chain(c: &mut ChainedSequenceContext, s: &Bits) -> bool {
    match c {
        ChainedSequenceContext::Format1(f) => {
            let (cov, sets) = filter_parallel(&f.coverage, take(&mut f.chained_seq_rule_sets), |g, set| {
                if !s.contains(g) {
                    return false;
                }
                let Some(set) = set.as_mut() else { return false };
                set.chained_seq_rules
                    .retain(|r| r.backtrack_sequence.iter().chain(&r.input_sequence).chain(&r.lookahead_sequence).all(|x| s.contains(x.to_u16())));
                !set.chained_seq_rules.is_empty()
            });
            f.coverage = cov.into();
            f.chained_seq_rule_sets = sets;
            !f.chained_seq_rule_sets.is_empty()
        }
        ChainedSequenceContext::Format2(f) => {
            if !subset_coverage(&mut f.coverage, s) {
                return false;
            }
            let (back_map, back) = subset_classes(&classes(&f.backtrack_class_def), |g| s.contains(g), s.len, true);
            let (input_map, input) = subset_classes(&classes(&f.input_class_def), |g| s.contains(g), s.len, true);
            let (ahead_map, ahead) = subset_classes(&classes(&f.lookahead_class_def), |g| s.contains(g), s.len, true);
            f.backtrack_class_def = class_def(&back).into();
            f.input_class_def = class_def(&input).into();
            f.lookahead_class_def = class_def(&ahead).into();
            let reachable = intersect_classes(&input, &glyphs_of(&f.coverage));
            f.chained_class_seq_rule_sets = class_sets(take(&mut f.chained_class_seq_rule_sets), &input_map, &reachable, |rs| {
                rs.chained_class_seq_rules.retain_mut(|r| {
                    match (
                        renumber(&r.backtrack_sequence, &back_map),
                        renumber(&r.input_sequence, &input_map),
                        renumber(&r.lookahead_sequence, &ahead_map),
                    ) {
                        (Some(b), Some(i), Some(a)) => {
                            r.backtrack_sequence = b;
                            r.input_sequence = i;
                            r.lookahead_sequence = a;
                            true
                        }
                        _ => false,
                    }
                });
                !rs.chained_class_seq_rules.is_empty()
            });
            !f.chained_class_seq_rule_sets.is_empty()
        }
        ChainedSequenceContext::Format3(f) => {
            let mut ok = true;
            for c in f.backtrack_coverages.iter_mut().chain(f.input_coverages.iter_mut()).chain(f.lookahead_coverages.iter_mut()) {
                ok &= subset_coverage(c, s);
            }
            ok
        }
    }
}

// ── GPOS subtables ──

fn is_device(d: &NullableOffsetMarker<DeviceOrVariationIndex>) -> bool {
    matches!(d.as_ref(), Some(DeviceOrVariationIndex::Device(_)))
}

/// Removes the device tables (not the variation indices) of a value record.
fn prune_value(v: &mut ValueRecord) {
    for d in [&mut v.x_placement_device, &mut v.y_placement_device, &mut v.x_advance_device, &mut v.y_advance_device] {
        if is_device(d) {
            d.clear();
        }
    }
}

/// The fields of a value record that do something: values other than 0 and present devices.
fn effective(v: &ValueRecord) -> ValueFormat {
    let mut f = ValueFormat::empty();
    for (value, flag) in [
        (v.x_placement, ValueFormat::X_PLACEMENT),
        (v.y_placement, ValueFormat::Y_PLACEMENT),
        (v.x_advance, ValueFormat::X_ADVANCE),
        (v.y_advance, ValueFormat::Y_ADVANCE),
    ] {
        if value.is_some_and(|x| x != 0) {
            f |= flag;
        }
    }
    for (device, flag) in [
        (&v.x_placement_device, ValueFormat::X_PLACEMENT_DEVICE),
        (&v.y_placement_device, ValueFormat::Y_PLACEMENT_DEVICE),
        (&v.x_advance_device, ValueFormat::X_ADVANCE_DEVICE),
        (&v.y_advance_device, ValueFormat::Y_ADVANCE_DEVICE),
    ] {
        if device.as_ref().is_some() {
            f |= flag;
        }
    }
    f
}

/// Gives every record the union of their effective formats.
fn shrink<'a>(records: impl Iterator<Item = &'a mut ValueRecord>) {
    let mut records: Vec<&mut ValueRecord> = records.collect();
    let format = records.iter().fold(ValueFormat::empty(), |f, v| f | effective(v));
    for v in &mut records {
        v.set_explicit_value_format(format);
    }
}

fn prune_anchor(a: &mut AnchorTable) {
    let plain = match a {
        AnchorTable::Format1(_) => None,
        AnchorTable::Format2(f) => Some((f.x_coordinate, f.y_coordinate)),
        AnchorTable::Format3(f) => {
            if is_device(&f.x_device) {
                f.x_device.clear();
            }
            if is_device(&f.y_device) {
                f.y_device.clear();
            }
            (f.x_device.as_ref().is_none() && f.y_device.as_ref().is_none()).then_some((f.x_coordinate, f.y_coordinate))
        }
    };
    if let Some((x, y)) = plain {
        *a = AnchorTable::Format1(AnchorFormat1::new(x, y));
    }
}

fn prune_anchors<'a>(anchors: impl Iterator<Item = &'a mut NullableOffsetMarker<AnchorTable>>) {
    for a in anchors {
        if let Some(a) = a.as_mut() {
            prune_anchor(a);
        }
    }
}

impl Subtable for SinglePos {
    fn subset(&mut self, s: &Bits) -> bool {
        match self {
            SinglePos::Format1(f) => subset_coverage(&mut f.coverage, s),
            SinglePos::Format2(f) => {
                let (cov, values) = filter_parallel(&f.coverage, take(&mut f.value_records), |g, _| s.contains(g));
                f.coverage = cov.into();
                f.value_records = values;
                !f.value_records.is_empty()
            }
        }
    }

    fn prune_post(&mut self, hinting: bool) {
        let single = match self {
            SinglePos::Format1(f) => {
                if !hinting {
                    prune_value(&mut f.value_record);
                }
                shrink(std::iter::once(&mut f.value_record));
                None
            }
            SinglePos::Format2(f) => {
                if !hinting {
                    f.value_records.iter_mut().for_each(prune_value);
                }
                shrink(f.value_records.iter_mut());
                match f.value_records.first() {
                    Some(first) if f.value_records.iter().all(|v| v == first) => Some((CoverageTable::clone(&f.coverage), first.clone())),
                    _ => None,
                }
            }
        };
        if let Some((coverage, value)) = single {
            *self = SinglePos::Format1(SinglePosFormat1::new(coverage, value));
        }
    }
}

impl Subtable for PairPos {
    fn subset(&mut self, s: &Bits) -> bool {
        match self {
            PairPos::Format1(f) => {
                let (cov, sets) = filter_parallel(&f.coverage, take(&mut f.pair_sets), |g, set| {
                    if !s.contains(g) {
                        return false;
                    }
                    set.pair_value_records.retain(|r| s.contains(r.second_glyph.to_u16()));
                    !set.pair_value_records.is_empty()
                });
                f.coverage = cov.into();
                f.pair_sets = sets;
                !f.pair_sets.is_empty()
            }
            PairPos::Format2(f) => {
                let covered: BTreeSet<u16> = f.coverage.iter().map(|g| g.to_u16()).filter(|g| s.contains(*g)).collect();
                let (map1, classes1) = subset_classes(&classes(&f.class_def1), |g| covered.contains(&g), covered.len(), true);
                let (map2, classes2) = subset_classes(&classes(&f.class_def2), |g| s.contains(g), s.len, false);
                let class2_count = f.class1_records.first().map_or(0, |r| r.class2_records.len());
                let map1: Vec<u16> = map1.into_iter().filter(|c| usize::from(*c) < f.class1_records.len()).collect();
                let map2: Vec<u16> = map2.into_iter().filter(|c| usize::from(*c) < class2_count).collect();
                let old = take(&mut f.class1_records);
                f.class1_records = map1
                    .iter()
                    .filter_map(|i| old.get(usize::from(*i)).cloned())
                    .map(|mut r| {
                        r.class2_records = map2.iter().filter_map(|j| r.class2_records.get(usize::from(*j)).cloned()).collect();
                        r
                    })
                    .collect();
                let fits = |m: &BTreeMap<u16, u16>, n: usize| -> BTreeMap<u16, u16> {
                    m.iter().filter(|(_, c)| usize::from(**c) < n).map(|(g, c)| (*g, *c)).collect()
                };
                f.class_def1 = class_def(&fits(&classes1, map1.len())).into();
                f.class_def2 = class_def(&fits(&classes2, map2.len())).into();
                !map1.is_empty() && map2.len() > 1 && subset_coverage(&mut f.coverage, s)
            }
        }
    }

    fn prune_post(&mut self, hinting: bool) {
        if hinting {
            return;
        }
        match self {
            PairPos::Format1(f) => {
                for set in &mut f.pair_sets {
                    for r in &mut set.pair_value_records {
                        prune_value(&mut r.value_record1);
                        prune_value(&mut r.value_record2);
                    }
                }
                shrink(f.pair_sets.iter_mut().flat_map(|set| set.pair_value_records.iter_mut()).map(|r| &mut r.value_record1));
                shrink(f.pair_sets.iter_mut().flat_map(|set| set.pair_value_records.iter_mut()).map(|r| &mut r.value_record2));
            }
            PairPos::Format2(f) => {
                for c1 in &mut f.class1_records {
                    for r in &mut c1.class2_records {
                        prune_value(&mut r.value_record1);
                        prune_value(&mut r.value_record2);
                    }
                }
                shrink(f.class1_records.iter_mut().flat_map(|c| c.class2_records.iter_mut()).map(|r| &mut r.value_record1));
                shrink(f.class1_records.iter_mut().flat_map(|c| c.class2_records.iter_mut()).map(|r| &mut r.value_record2));
            }
        }
    }
}

impl Subtable for CursivePosFormat1 {
    fn subset(&mut self, s: &Bits) -> bool {
        let (cov, records) = filter_parallel(&self.coverage, take(&mut self.entry_exit_record), |g, _| s.contains(g));
        self.coverage = cov.into();
        self.entry_exit_record = records;
        !self.entry_exit_record.is_empty()
    }

    fn prune_post(&mut self, hinting: bool) {
        if !hinting {
            prune_anchors(self.entry_exit_record.iter_mut().flat_map(|r| [&mut r.entry_anchor, &mut r.exit_anchor]));
        }
    }
}

/// Subsets marks (coverage and records) to `s`; returns the mark classes left, sorted.
fn subset_marks(coverage: &mut OffsetMarker<CoverageTable>, records: &mut Vec<write_fonts::tables::gpos::MarkRecord>, s: &Bits) -> Vec<u16> {
    let (cov, kept) = filter_parallel(coverage, take(records), |g, _| s.contains(g));
    *coverage = cov.into();
    *records = kept;
    sorted(records.iter().map(|r| r.mark_class).collect())
}

/// Keeps the entries of `coverage` (glyphs in `s`) whose record has an anchor for one of
/// `classes`; returns the kept records, or `None` when none is left.
fn subset_attachments<T: Clone>(
    coverage: &mut OffsetMarker<CoverageTable>,
    records: &[T],
    s: &Bits,
    has_anchor: impl Fn(&T, usize) -> bool,
    classes: &[u16],
) -> Option<Vec<T>> {
    let mut glyphs = Vec::new();
    let mut kept = Vec::new();
    for (i, g) in coverage.iter().map(|g| g.to_u16()).enumerate() {
        if !s.contains(g) {
            continue;
        }
        let Some(r) = records.get(i) else { continue };
        if classes.iter().any(|c| has_anchor(r, usize::from(*c))) {
            glyphs.push(g);
            kept.push(r.clone());
        }
    }
    if kept.is_empty() {
        return None;
    }
    *coverage = coverage_of(&glyphs);
    Some(kept)
}

fn coverage_of(glyphs: &[u16]) -> OffsetMarker<CoverageTable> {
    coverage(glyphs).into()
}

fn anchor_at(anchors: &[NullableOffsetMarker<AnchorTable>], i: usize) -> bool {
    anchors.get(i).is_some_and(|a| a.as_ref().is_some())
}

/// The anchors of `anchors` at `classes`, in that order.
fn anchors_at(anchors: &[NullableOffsetMarker<AnchorTable>], classes: &[u16]) -> Vec<NullableOffsetMarker<AnchorTable>> {
    classes.iter().filter_map(|c| anchors.get(usize::from(*c)).cloned()).collect()
}

fn renumber_marks(records: &mut [write_fonts::tables::gpos::MarkRecord], classes: &[u16]) {
    for r in records {
        r.mark_class = classes.binary_search(&r.mark_class).map_or(0, |i| i as u16);
    }
}

impl Subtable for MarkBasePosFormat1 {
    fn subset(&mut self, s: &Bits) -> bool {
        let classes = subset_marks(&mut self.mark_coverage, &mut self.mark_array.mark_records, s);
        let Some(mut bases) =
            subset_attachments(&mut self.base_coverage, &self.base_array.base_records, s, |r, c| anchor_at(&r.base_anchors, c), &classes)
        else {
            return false;
        };
        renumber_marks(&mut self.mark_array.mark_records, &classes);
        for b in &mut bases {
            b.base_anchors = anchors_at(&b.base_anchors, &classes);
        }
        self.base_array.base_records = bases;
        !classes.is_empty() && !self.mark_array.mark_records.is_empty()
    }

    fn prune_post(&mut self, hinting: bool) {
        if !hinting {
            self.mark_array.mark_records.iter_mut().for_each(|m| prune_anchor(&mut m.mark_anchor));
            prune_anchors(self.base_array.base_records.iter_mut().flat_map(|b| b.base_anchors.iter_mut()));
        }
    }
}

impl Subtable for MarkLigPosFormat1 {
    fn subset(&mut self, s: &Bits) -> bool {
        let classes = subset_marks(&mut self.mark_coverage, &mut self.mark_array.mark_records, s);
        let Some(mut ligs) = subset_attachments(
            &mut self.ligature_coverage,
            &self.ligature_array.ligature_attaches,
            s,
            |l, c| l.component_records.iter().any(|comp| anchor_at(&comp.ligature_anchors, c)),
            &classes,
        ) else {
            return false;
        };
        renumber_marks(&mut self.mark_array.mark_records, &classes);
        for l in &mut ligs {
            for comp in &mut l.component_records {
                comp.ligature_anchors = anchors_at(&comp.ligature_anchors, &classes);
            }
        }
        self.ligature_array.ligature_attaches = ligs;
        !classes.is_empty() && !self.mark_array.mark_records.is_empty()
    }

    fn prune_post(&mut self, hinting: bool) {
        if !hinting {
            self.mark_array.mark_records.iter_mut().for_each(|m| prune_anchor(&mut m.mark_anchor));
            prune_anchors(
                self.ligature_array
                    .ligature_attaches
                    .iter_mut()
                    .flat_map(|l| l.component_records.iter_mut())
                    .flat_map(|c| c.ligature_anchors.iter_mut()),
            );
        }
    }
}

impl Subtable for MarkMarkPosFormat1 {
    fn subset(&mut self, s: &Bits) -> bool {
        let classes = subset_marks(&mut self.mark1_coverage, &mut self.mark1_array.mark_records, s);
        let Some(mut marks) =
            subset_attachments(&mut self.mark2_coverage, &self.mark2_array.mark2_records, s, |r, c| anchor_at(&r.mark2_anchors, c), &classes)
        else {
            return false;
        };
        renumber_marks(&mut self.mark1_array.mark_records, &classes);
        for m in &mut marks {
            m.mark2_anchors = anchors_at(&m.mark2_anchors, &classes);
        }
        self.mark2_array.mark2_records = marks;
        !classes.is_empty() && !self.mark1_array.mark_records.is_empty()
    }

    fn prune_post(&mut self, hinting: bool) {
        if !hinting {
            self.mark1_array.mark_records.iter_mut().for_each(|m| prune_anchor(&mut m.mark_anchor));
            prune_anchors(self.mark2_array.mark2_records.iter_mut().flat_map(|m| m.mark2_anchors.iter_mut()));
        }
    }
}

// ── The GSUB closure ──

struct Closure<'a> {
    lookups: &'a [Option<SubstitutionLookup>],
    glyphs: Bits,
    /// Lookup index → (glyph count when last reset, glyphs it was asked about since).
    done: HashMap<usize, (usize, Bits)>,
    depth: usize,
    work: usize,
}

/// A rule of a contextual subtable: glyphs or classes before, in and after the input (the
/// input without its first position), and the lookups it calls.
struct Rule<'a, T> {
    backtrack: &'a [T],
    input: &'a [T],
    lookahead: &'a [T],
    records: &'a [SequenceLookupRecord],
}

type Sets<'a, T> = Vec<Option<Vec<Rule<'a, T>>>>;

/// A contextual subtable of either kind, for the closure.
enum Context<'a> {
    Glyphs {
        coverage: &'a CoverageTable,
        sets: Sets<'a, G>,
    },
    Classes {
        coverage: &'a CoverageTable,
        backtrack: Option<&'a ClassDef>,
        input: &'a ClassDef,
        lookahead: Option<&'a ClassDef>,
        sets: Sets<'a, u16>,
    },
    Coverages {
        backtrack: &'a [OffsetMarker<CoverageTable>],
        input: &'a [OffsetMarker<CoverageTable>],
        lookahead: &'a [OffsetMarker<CoverageTable>],
        records: &'a [SequenceLookupRecord],
    },
}

fn sequence_view(c: &SequenceContext) -> Context<'_> {
    match c {
        SequenceContext::Format1(f) => Context::Glyphs {
            coverage: &f.coverage,
            sets: f
                .seq_rule_sets
                .iter()
                .map(|set| {
                    set.as_ref().map(|set| {
                        set.seq_rules
                            .iter()
                            .map(|r| Rule { backtrack: &[], input: &r.input_sequence, lookahead: &[], records: &r.seq_lookup_records })
                            .collect()
                    })
                })
                .collect(),
        },
        SequenceContext::Format2(f) => Context::Classes {
            coverage: &f.coverage,
            backtrack: None,
            input: &f.class_def,
            lookahead: None,
            sets: f
                .class_seq_rule_sets
                .iter()
                .map(|set| {
                    set.as_ref().map(|set| {
                        set.class_seq_rules
                            .iter()
                            .map(|r| Rule { backtrack: &[], input: &r.input_sequence, lookahead: &[], records: &r.seq_lookup_records })
                            .collect()
                    })
                })
                .collect(),
        },
        SequenceContext::Format3(f) => Context::Coverages { backtrack: &[], input: &f.coverages, lookahead: &[], records: &f.seq_lookup_records },
    }
}

fn chain_view(c: &ChainedSequenceContext) -> Context<'_> {
    match c {
        ChainedSequenceContext::Format1(f) => Context::Glyphs {
            coverage: &f.coverage,
            sets: f
                .chained_seq_rule_sets
                .iter()
                .map(|set| {
                    set.as_ref().map(|set| {
                        set.chained_seq_rules
                            .iter()
                            .map(|r| Rule {
                                backtrack: &r.backtrack_sequence,
                                input: &r.input_sequence,
                                lookahead: &r.lookahead_sequence,
                                records: &r.seq_lookup_records,
                            })
                            .collect()
                    })
                })
                .collect(),
        },
        ChainedSequenceContext::Format2(f) => Context::Classes {
            coverage: &f.coverage,
            backtrack: Some(&f.backtrack_class_def),
            input: &f.input_class_def,
            lookahead: Some(&f.lookahead_class_def),
            sets: f
                .chained_class_seq_rule_sets
                .iter()
                .map(|set| {
                    set.as_ref().map(|set| {
                        set.chained_class_seq_rules
                            .iter()
                            .map(|r| Rule {
                                backtrack: &r.backtrack_sequence,
                                input: &r.input_sequence,
                                lookahead: &r.lookahead_sequence,
                                records: &r.seq_lookup_records,
                            })
                            .collect()
                    })
                })
                .collect(),
        },
        ChainedSequenceContext::Format3(f) => Context::Coverages {
            backtrack: &f.backtrack_coverages,
            input: &f.input_coverages,
            lookahead: &f.lookahead_coverages,
            records: &f.seq_lookup_records,
        },
    }
}

/// Whether a lookup may change the number of glyphs (fontTools' `may_have_non_1to1`): after
/// one, the positions of the later glyphs of the context are no longer known.
fn non_one_to_one(l: &SubstitutionLookup) -> bool {
    match l {
        SubstitutionLookup::Single(_) | SubstitutionLookup::Alternate(_) | SubstitutionLookup::Reverse(_) => false,
        SubstitutionLookup::Multiple(l) => !l.subtables.is_empty(),
        SubstitutionLookup::Ligature(l) => !l.subtables.is_empty(),
        SubstitutionLookup::Contextual(l) => !l.subtables.is_empty(),
        SubstitutionLookup::ChainContextual(l) => !l.subtables.is_empty(),
        SubstitutionLookup::Extension(l) => l.subtables.iter().any(|e| {
            matches!(
                &**e,
                SubstExtension::Multiple(_) | SubstExtension::Ligature(_) | SubstExtension::Contextual(_) | SubstExtension::ChainContextual(_)
            )
        }),
    }
}

impl<'a> Closure<'a> {
    fn charge(&mut self, n: usize) -> Result<()> {
        self.work = self.work.checked_sub(n).ok_or(TablesError::TooLarge { what: "the GSUB closure", limit: MAX_CLOSURE_WORK })?;
        Ok(())
    }

    fn add(&mut self, g: u16) {
        self.glyphs.insert(g);
    }

    /// Runs lookup `li` on `cur` (sorted), or on the whole glyph set when `None`.
    fn close_lookup(&mut self, li: usize, cur: Option<Vec<u16>>) -> Result<()> {
        let lookups = self.lookups;
        let Some(Some(lookup)) = lookups.get(li) else { return Ok(()) };
        if self.depth >= MAX_LOOKUP_NESTING {
            return Err(TablesError::invalid(format!("the 'GSUB' table nests contextual lookups more than {MAX_LOOKUP_NESTING} deep")));
        }
        let cur = cur.unwrap_or_else(|| self.glyphs.iter().collect());
        self.charge(cur.len())?;
        let count = self.glyphs.len;
        let entry = self.done.entry(li).or_insert_with(|| (count, Bits::new()));
        if entry.0 != count {
            *entry = (count, Bits::new());
        }
        if cur.iter().all(|g| entry.1.contains(*g)) {
            return Ok(());
        }
        for g in &cur {
            entry.1.insert(*g);
        }
        self.depth += 1;
        let result = self.close_subtables(lookup, &cur);
        self.depth -= 1;
        result
    }

    fn close_subtables(&mut self, lookup: &'a SubstitutionLookup, cur: &[u16]) -> Result<()> {
        match lookup {
            SubstitutionLookup::Single(l) => l.subtables.iter().try_for_each(|st| self.close_single(st, cur)),
            SubstitutionLookup::Multiple(l) => l.subtables.iter().try_for_each(|st| self.close_multiple(st, cur)),
            SubstitutionLookup::Alternate(l) => l.subtables.iter().try_for_each(|st| self.close_alternate(st, cur)),
            SubstitutionLookup::Ligature(l) => l.subtables.iter().try_for_each(|st| self.close_ligature(st, cur)),
            SubstitutionLookup::Contextual(l) => l.subtables.iter().try_for_each(|st| self.close_context(&sequence_view(st), cur)),
            SubstitutionLookup::ChainContextual(l) => l.subtables.iter().try_for_each(|st| self.close_context(&chain_view(st), cur)),
            SubstitutionLookup::Reverse(l) => l.subtables.iter().try_for_each(|st| self.close_reverse(st, cur)),
            SubstitutionLookup::Extension(l) => l.subtables.iter().try_for_each(|st| match &**st {
                SubstExtension::Single(e) => self.close_single(&e.extension, cur),
                SubstExtension::Multiple(e) => self.close_multiple(&e.extension, cur),
                SubstExtension::Alternate(e) => self.close_alternate(&e.extension, cur),
                SubstExtension::Ligature(e) => self.close_ligature(&e.extension, cur),
                SubstExtension::Contextual(e) => self.close_context(&sequence_view(&e.extension), cur),
                SubstExtension::ChainContextual(e) => self.close_context(&chain_view(&e.extension), cur),
                SubstExtension::Reverse(e) => self.close_reverse(&e.extension, cur),
            }),
        }
    }

    fn close_single(&mut self, st: &SingleSubst, cur: &[u16]) -> Result<()> {
        let pairs = single_pairs(st);
        self.charge(pairs.len())?;
        for (a, b) in pairs {
            if has(cur, a) {
                self.add(b);
            }
        }
        Ok(())
    }

    fn close_multiple(&mut self, st: &MultipleSubstFormat1, cur: &[u16]) -> Result<()> {
        self.charge(st.coverage.len())?;
        for (g, seq) in st.coverage.iter().zip(&st.sequences) {
            if has(cur, g.to_u16()) {
                self.charge(seq.substitute_glyph_ids.len())?;
                seq.substitute_glyph_ids.iter().for_each(|x| self.add(x.to_u16()));
            }
        }
        Ok(())
    }

    fn close_alternate(&mut self, st: &AlternateSubstFormat1, cur: &[u16]) -> Result<()> {
        self.charge(st.coverage.len())?;
        for (g, set) in st.coverage.iter().zip(&st.alternate_sets) {
            if has(cur, g.to_u16()) {
                self.charge(set.alternate_glyph_ids.len())?;
                set.alternate_glyph_ids.iter().for_each(|x| self.add(x.to_u16()));
            }
        }
        Ok(())
    }

    fn close_ligature(&mut self, st: &LigatureSubstFormat1, cur: &[u16]) -> Result<()> {
        self.charge(st.coverage.len())?;
        for (g, set) in st.coverage.iter().zip(&st.ligature_sets) {
            if !has(cur, g.to_u16()) {
                continue;
            }
            for lig in &set.ligatures {
                self.charge(1 + lig.component_glyph_ids.len())?;
                if lig.component_glyph_ids.iter().all(|c| self.glyphs.contains(c.to_u16())) {
                    self.add(lig.ligature_glyph.to_u16());
                }
            }
        }
        Ok(())
    }

    fn close_reverse(&mut self, st: &ReverseChainSingleSubstFormat1, cur: &[u16]) -> Result<()> {
        self.charge(st.coverage.len())?;
        let hits: Vec<usize> = st.coverage.iter().enumerate().filter(|(_, g)| has(cur, g.to_u16())).map(|(i, _)| i).collect();
        if hits.is_empty() {
            return Ok(());
        }
        for c in st.lookahead_coverages.iter().chain(&st.backtrack_coverages) {
            self.charge(c.len())?;
            if !c.iter().any(|g| self.glyphs.contains(g.to_u16())) {
                return Ok(());
            }
        }
        for i in hits {
            if let Some(g) = st.substitute_glyph_ids.get(i) {
                self.add(g.to_u16());
            }
        }
        Ok(())
    }

    /// Runs the lookups one matched rule calls. `chaos_end` ends the positions a lookup that
    /// may change the glyph count makes unknown; `pos` gives the glyphs at an input position.
    fn run_records(
        &mut self,
        records: &[SequenceLookupRecord],
        chaos_end: usize,
        pos: impl Fn(&Closure<'a>, usize) -> Option<Vec<u16>>,
    ) -> Result<()> {
        let mut seen: Vec<usize> = Vec::new();
        let mut unknown_from: Option<usize> = None;
        for record in records {
            let seqi = usize::from(record.sequence_index);
            let li = usize::from(record.lookup_list_index);
            let lookups = self.lookups;
            let Some(Some(lookup)) = lookups.get(li) else { continue };
            let unknown = seen.contains(&seqi) || unknown_from.is_some_and(|f| seqi >= f && seqi < chaos_end);
            let cur = if unknown {
                None
            } else {
                let Some(glyphs) = pos(self, seqi) else { continue };
                let glyphs = sorted(glyphs);
                self.charge(glyphs.len())?;
                Some(glyphs)
            };
            seen.push(seqi);
            if non_one_to_one(lookup) {
                unknown_from = Some(unknown_from.map_or(seqi, |f| f.min(seqi)));
            }
            self.close_lookup(li, cur)?;
        }
        Ok(())
    }

    fn close_context(&mut self, ctx: &Context<'a>, cur: &[u16]) -> Result<()> {
        match ctx {
            Context::Glyphs { coverage, sets } => {
                let firsts = glyphs_of(coverage);
                self.charge(firsts.len())?;
                for (i, first) in firsts.iter().enumerate() {
                    if !has(cur, *first) {
                        continue;
                    }
                    let Some(Some(rules)) = sets.get(i) else { continue };
                    for r in rules {
                        self.charge(1 + r.backtrack.len() + r.input.len() + r.lookahead.len())?;
                        if !r.backtrack.iter().chain(r.input).chain(r.lookahead).all(|g| self.glyphs.contains(g.to_u16())) {
                            continue;
                        }
                        let first = *first;
                        let input = r.input;
                        self.run_records(r.records, input.len() + 2, |_, seqi| match seqi {
                            0 => Some(vec![first]),
                            _ => input.get(seqi - 1).map(|g| vec![g.to_u16()]),
                        })?;
                    }
                }
            }
            Context::Classes { coverage, backtrack, input, lookahead, sets } => {
                let firsts = glyphs_of(coverage);
                self.charge(firsts.len())?;
                let cur_glyphs = sorted(firsts.into_iter().filter(|g| has(cur, *g)).collect());
                if cur_glyphs.is_empty() {
                    return Ok(());
                }
                let input_map = classes(input);
                let back_map = backtrack.map(classes);
                let ahead_map = lookahead.map(classes);
                self.charge(input_map.len() + back_map.as_ref().map_or(0, BTreeMap::len) + ahead_map.as_ref().map_or(0, BTreeMap::len))?;
                let input_hits = ClassHits::new(&input_map, &self.glyphs);
                let back_hits = back_map.as_ref().map(|m| ClassHits::new(m, &self.glyphs));
                let ahead_hits = ahead_map.as_ref().map(|m| ClassHits::new(m, &self.glyphs));
                for k in intersect_classes(&input_map, &cur_glyphs) {
                    let Some(Some(rules)) = sets.get(usize::from(k)) else { continue };
                    for r in rules {
                        self.charge(1 + r.backtrack.len() + r.input.len() + r.lookahead.len())?;
                        let matched = r.backtrack.iter().all(|c| class_hits(back_hits.as_ref(), *c, &self.glyphs))
                            && r.input.iter().all(|c| input_hits.hits(*c))
                            && r.lookahead.iter().all(|c| class_hits(ahead_hits.as_ref(), *c, &self.glyphs));
                        if !matched {
                            continue;
                        }
                        let rule_input = r.input;
                        let (map, firsts) = (&input_map, &cur_glyphs);
                        self.run_records(r.records, rule_input.len() + 2, |cx, seqi| {
                            if seqi == 0 {
                                return Some(firsts.iter().copied().filter(|g| map.get(g).copied().unwrap_or(0) == k).collect());
                            }
                            let class = *rule_input.get(seqi - 1)?;
                            Some(if class == 0 {
                                cx.glyphs.iter().filter(|g| !map.contains_key(g)).collect()
                            } else {
                                map.iter().filter(|(g, c)| **c == class && cx.glyphs.contains(**g)).map(|(g, _)| *g).collect()
                            })
                        })?;
                    }
                }
            }
            Context::Coverages { backtrack, input, lookahead, records } => {
                let Some(first) = input.first() else { return Ok(()) };
                let firsts = glyphs_of(first);
                self.charge(firsts.len())?;
                let cur_glyphs = sorted(firsts.into_iter().filter(|g| has(cur, *g)).collect());
                if cur_glyphs.is_empty() {
                    return Ok(());
                }
                for c in backtrack.iter().chain(input.iter()).chain(lookahead.iter()) {
                    self.charge(c.len())?;
                    if !c.iter().any(|g| self.glyphs.contains(g.to_u16())) {
                        return Ok(());
                    }
                }
                let inputs = *input;
                self.run_records(records, inputs.len() + 1, |cx, seqi| {
                    if seqi == 0 {
                        return Some(cur_glyphs.clone());
                    }
                    inputs.get(seqi).map(|c| c.iter().map(|g| g.to_u16()).filter(|g| cx.glyphs.contains(*g)).collect())
                })?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
