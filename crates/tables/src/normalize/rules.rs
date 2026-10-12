//! The pieces every normalised table shares: language-system grouping, glyph sets, lookups
//! decomposed into rules, and the printing helpers.
//!
//! Ported from fontc's `otl-normalizer` (MIT OR Apache-2.0).

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use read_fonts::tables::layout::{FeatureList, LookupFlag, ScriptList};
use read_fonts::types::{GlyphId16, Tag};

use super::names::NameMap;
use super::{Budget, Out, read_err};
use crate::{Result, TablesError};

pub(super) struct LanguageSystem {
    script: Tag,
    lang: Tag,
}

/// The lookups of one feature, with the language systems that use exactly these lookups for it.
pub(super) struct Feature {
    pub(super) feature: Tag,
    pub(super) lang_systems: Vec<LanguageSystem>,
    pub(super) lookups: Vec<u16>,
}

/// One glyph or several.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum GlyphSet {
    Single(GlyphId16),
    Multiple(BTreeSet<GlyphId16>),
}

/// The rules of every subtable of one lookup, decomposed and with rules a shaper would never
/// reach dropped.
#[derive(Clone, Debug)]
pub(super) struct Lookup<Rule> {
    pub(super) lookup_id: u16,
    flag: LookupFlag,
    filter_set: Option<u16>,
    rules: Vec<Rule>,
}

/// One rule with the lookup properties it carries.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct SingleRule<'a, Rule: Clone> {
    rule: Cow<'a, Rule>,
    pub(super) lookup_id: u16,
    flag: LookupFlag,
    filter_set: Option<u16>,
}

/// Something printed with glyph names.
pub(super) trait PrintNames {
    fn fmt_names(&self, f: &mut fmt::Formatter<'_>, names: &NameMap) -> fmt::Result;
}

impl LanguageSystem {
    fn sort_key(&self) -> (u32, u32) {
        (tag_to_int(self.script), tag_to_int(self.lang))
    }
}

impl Feature {
    fn sort_key(&self) -> (u32, Option<(u32, u32)>) {
        (tag_to_int(self.feature), self.lang_systems.first().map(LanguageSystem::sort_key))
    }

    /// `# kern: DFLT/dflt, latn/dflt`, wrapped before 80 columns.
    pub(super) fn print_header(&self, out: &mut Out) -> Result<()> {
        const SCRIPTLANG_LEN: usize = "DFLT/dflt".len();
        const LINE_WIDTH: usize = 80;
        out.put(format_args!("# {}: ", self.feature))?;
        let mut pos = 8;
        for (i, LanguageSystem { script, lang }) in self.lang_systems.iter().enumerate() {
            if i > 0 {
                if pos + SCRIPTLANG_LEN + 2 > LINE_WIDTH {
                    out.put(format_args!(",\n  "))?;
                    pos = 2;
                } else {
                    out.put(format_args!(", "))?;
                    pos += 2;
                }
            }
            out.put(format_args!("{script}/{lang}"))?;
            pos += SCRIPTLANG_LEN;
        }
        out.put(format_args!("\n"))
    }
}

/// Sorting key for tags: `DFLT` and `dflt` before everything else.
fn tag_to_int(tag: Tag) -> u32 {
    if tag == Tag::new(b"DFLT") {
        0
    } else if tag == Tag::new(b"dflt") {
        1
    } else {
        u32::from_be_bytes(tag.to_be_bytes())
    }
}

/// Every feature with the language systems that share its lookups, sorted by feature tag.
pub(super) fn get_lang_systems(script_list: &ScriptList<'_>, feature_list: &FeatureList<'_>, budget: &mut Budget) -> Result<Vec<Feature>> {
    let script_err = read_err("the script list");
    let feature_err = read_err("the feature list");
    let data = script_list.offset_data();
    let mut groups: BTreeMap<(Tag, Vec<u16>), Vec<LanguageSystem>> = BTreeMap::new();
    for record in script_list.script_records() {
        let script_tag = record.script_tag();
        let script = record.script(data).map_err(&script_err)?;
        let mut systems = Vec::new();
        if let Some(default) = script.default_lang_sys() {
            let default = default.map_err(&script_err)?;
            systems.push((Tag::new(b"dflt"), default.feature_indices()));
        }
        for lang in script.lang_sys_records() {
            let sys = lang.lang_sys(script.offset_data()).map_err(&script_err)?;
            systems.push((lang.lang_sys_tag(), sys.feature_indices()));
        }
        for (lang, indices) in systems {
            for index in indices {
                let index = usize::from(index.get());
                let feature_record = feature_list.feature_records().get(index).ok_or_else(|| {
                    TablesError::invalid(format!("a language system refers to feature {index}, which the feature list does not have"))
                })?;
                let feature = feature_record.feature(feature_list.offset_data()).map_err(&feature_err)?;
                let lookups: Vec<u16> = feature.lookup_list_indices().iter().map(|x| x.get()).collect();
                budget.charge(lookups.len().saturating_add(1))?;
                groups.entry((feature_record.feature_tag(), lookups)).or_default().push(LanguageSystem { script: script_tag, lang });
            }
        }
    }
    let mut result: Vec<Feature> = groups.into_iter().map(|((feature, lookups), lang_systems)| Feature { feature, lang_systems, lookups }).collect();
    for feature in &mut result {
        feature.lang_systems.sort_by_key(LanguageSystem::sort_key);
    }
    result.sort_by_key(Feature::sort_key);
    Ok(result)
}

impl GlyphSet {
    pub(super) fn is_empty(&self) -> bool {
        match self {
            GlyphSet::Single(_) => false,
            GlyphSet::Multiple(set) => set.is_empty(),
        }
    }

    fn make_set(&mut self) {
        if let GlyphSet::Single(gid) = self {
            *self = GlyphSet::Multiple(BTreeSet::from([*gid]));
        }
    }

    pub(super) fn combine(&mut self, other: &GlyphSet) {
        self.make_set();
        if let GlyphSet::Multiple(gids) = self {
            gids.extend(other.iter());
        }
    }

    /// Removes `glyphs`; the set can end up empty, which the caller handles.
    pub(super) fn remove_glyphs(&mut self, glyphs: &[GlyphId16]) {
        match self {
            GlyphSet::Multiple(set) => set.retain(|gid| !glyphs.contains(gid)),
            GlyphSet::Single(gid) if glyphs.contains(gid) => *self = GlyphSet::Multiple(BTreeSet::new()),
            GlyphSet::Single(_) => {}
        }
    }

    pub(super) fn add(&mut self, gid: GlyphId16) {
        if matches!(self, GlyphSet::Single(x) if *x == gid) {
            return;
        }
        self.make_set();
        if let GlyphSet::Multiple(set) = self {
            set.insert(gid);
        }
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = GlyphId16> + '_ {
        let (single, many) = match self {
            GlyphSet::Single(gid) => (Some(*gid), None),
            GlyphSet::Multiple(gids) => (None, Some(gids.iter().copied())),
        };
        single.into_iter().chain(many.into_iter().flatten())
    }

    /// `name`, or `[a,b,c]` for a set.
    pub(super) fn printer<'a>(&'a self, names: &'a NameMap) -> impl fmt::Display + 'a {
        struct GlyphPrinter<'a> {
            glyphs: &'a GlyphSet,
            names: &'a NameMap,
        }
        impl fmt::Display for GlyphPrinter<'_> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                match self.glyphs {
                    GlyphSet::Single(gid) => f.write_str(&self.names.get(*gid)),
                    GlyphSet::Multiple(gids) => {
                        f.write_str("[")?;
                        for (i, gid) in gids.iter().enumerate() {
                            if i > 0 {
                                f.write_str(",")?;
                            }
                            f.write_str(&self.names.get(*gid))?;
                        }
                        f.write_str("]")
                    }
                }
            }
        }
        GlyphPrinter { glyphs: self, names }
    }
}

// The oracle orders a single glyph against a set by the set's first glyph; kept for identical
// sorting.
impl Ord for GlyphSet {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (GlyphSet::Single(left), GlyphSet::Single(right)) => left.cmp(right),
            (GlyphSet::Single(left), GlyphSet::Multiple(right)) => Some(left).cmp(&right.first()),
            (GlyphSet::Multiple(left), GlyphSet::Single(right)) => left.first().cmp(&Some(right)),
            (GlyphSet::Multiple(left), GlyphSet::Multiple(right)) => left.cmp(right),
        }
    }
}

impl PartialOrd for GlyphSet {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl From<GlyphId16> for GlyphSet {
    fn from(gid: GlyphId16) -> GlyphSet {
        GlyphSet::Single(gid)
    }
}

impl FromIterator<GlyphId16> for GlyphSet {
    fn from_iter<T: IntoIterator<Item = GlyphId16>>(iter: T) -> Self {
        GlyphSet::Multiple(iter.into_iter().collect())
    }
}

impl<Rule> Lookup<Rule> {
    pub(super) fn new(lookup_id: u16, rules: Vec<Rule>, flag: LookupFlag, filter_set: Option<u16>) -> Self {
        Lookup { lookup_id, flag, filter_set, rules }
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = SingleRule<'_, Rule>> + '_
    where
        Rule: Clone,
    {
        self.rules.iter().map(|rule| SingleRule {
            lookup_id: self.lookup_id,
            flag: self.flag,
            filter_set: self.filter_set,
            rule: Cow::Borrowed(rule),
        })
    }
}

impl<T: Clone> SingleRule<'_, T> {
    pub(super) fn lookup_flags(&self) -> (LookupFlag, Option<u16>) {
        (self.flag, self.filter_set)
    }

    pub(super) fn rule(&self) -> &T {
        self.rule.as_ref()
    }

    pub(super) fn rule_mut(&mut self) -> &mut T {
        self.rule.to_mut()
    }
}

impl<T: PrintNames + Clone> SingleRule<'_, T> {
    pub(super) fn printer<'a>(&'a self, names: &'a NameMap) -> impl fmt::Display + 'a {
        struct Printer<'a, T> {
            names: &'a NameMap,
            item: &'a T,
        }
        impl<T: PrintNames> fmt::Display for Printer<'_, T> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.item.fmt_names(f, self.names)
            }
        }
        Printer { names, item: self.rule.as_ref() }
    }
}
