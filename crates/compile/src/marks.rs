//! The generated mark attachment and `GDEF` of a static master build (`compile.markFeatureWriter`,
//! `compile.gdefClasses`, `compile.gdefLigatureCarets`): the master's anchors after propagation
//! handed to the mark writer of `fontcraft_features::marks`, and its `GPOS` merged with the
//! generated kerning's.
//!
//! - Anchors: the effective anchors of `fontcraft_components::propagate` on the compiled master,
//!   with categories from the compile's glyph data, for the reference's format; for a UFO or
//!   designspace source only when its ufo2ft filter list has a `propagateAnchors` entry, and then
//!   only for the glyphs the entry reaches, as both reference compilers decide. Every other glyph
//!   gives its own anchors.
//! - Classes: worked out from the categories and anchors for the reference's format; for a UFO,
//!   the stated `public.openTypeCategories`.
//! - Which features: the feature-writer setting (as for the kerning), then feature code. Feature
//!   code is compiled from M8; until then a hand-written `mark`, `mkmk`, `abvm`, `blwm` or `curs`
//!   feature keeps the generated feature of its tag out, and a top-level `include` of another
//!   file keeps all five out, as it does the kerning. The feature code's `languagesystem`
//!   statements say where the generated features are registered.
//! - Merging: the lookups in the order the reference compilers give generated features: `curs`,
//!   then the kerning, then `abvm`, `blwm`, `mark` and `mkmk`. Feature records are rebuilt for
//!   both sets of lookups ([`fontcraft_features::marks::gpos_from_keys`]).

use std::collections::BTreeMap;

use fontcraft_components::propagate::effective_anchors_for_master_with;
use fontcraft_features::marks::{
    MarkAnchor, MarkGlyph, MarkInput, MarkLimits, MarkRequest, MarkTags, MasterAnchors, gpos_from_keys, mark_features, mark_tags,
};
use fontcraft_features::{FEATURE_WRITERS_KEY, FeatureError};
use fontcraft_model::glyphdata::GlyphData;
use fontcraft_model::{Font, Resolved, ResolvedGlyph, is_ufo_family};
use write_fonts::tables::gdef::Gdef;

use crate::components_opt::{Scope, listed_filter, reaches};

/// The ufo2ft filter that propagates anchors, as [`listed_filter`] spells it.
const PROPAGATE_FILTER: &str = "propagateAnchors";
use write_fonts::tables::gpos::{Gpos, PositionLookup};
use write_fonts::tables::layout::LangSys;
use write_fonts::types::Tag;

use crate::kern;

const DFLT_LANG: Tag = Tag::new(b"dflt");

/// The layout tables of a compile.
#[derive(Debug, Default)]
pub(crate) struct Layout {
    pub(crate) gpos: Option<Gpos>,
    pub(crate) gdef: Option<Gdef>,
}

/// The `GPOS` (`kern`'s lookups and the generated mark lookups) and `GDEF` of `resolved`'s master
/// for the glyphs `order` (the final glyph order). `font` is the source `resolved` came from;
/// without it only the glyphs' own anchors are read and every generated feature is written.
pub(crate) fn generate(
    font: Option<&Font>,
    data: Option<&GlyphData>,
    resolved: &Resolved,
    order: &[ResolvedGlyph],
    kern: Option<Gpos>,
    warnings: &mut Vec<String>,
) -> Layout {
    let input = input(font, data, resolved, order, warnings);
    let any_anchor = input.glyphs.iter().any(|g| g.anchors.iter().any(|m| !m.anchors.is_empty()));
    let tags = match font {
        Some(font) => tags(font, &resolved.master, any_anchor, warnings),
        None => MarkTags::default(),
    };
    let request = MarkRequest { input: &input, master: &resolved.master, tags, limits: MarkLimits::default() };
    match mark_features(&request) {
        Ok(marks) => {
            warnings.extend(marks.warnings);
            let gpos = merge(kern, marks.gpos, marks.curs_lookups, warnings);
            Layout { gpos, gdef: marks.gdef }
        }
        Err(FeatureError::TooMany { what, limit }) => {
            warnings.push(format!("mark attachment and the GDEF table were not written to the font: they need more than {limit} {what}"));
            Layout { gpos: kern, gdef: None }
        }
        Err(e) => {
            warnings.push(format!("mark attachment and the GDEF table were not written to the font: {e}"));
            Layout { gpos: kern, gdef: None }
        }
    }
}

/// The mark writer's input: every glyph of `order` with its anchors on the compiled master.
fn input(font: Option<&Font>, data: Option<&GlyphData>, resolved: &Resolved, order: &[ResolvedGlyph], warnings: &mut Vec<String>) -> MarkInput {
    let ufo = is_ufo_family(resolved.format);
    let scopes = match font {
        Some(font) if ufo => listed_filter(font, &resolved.master, PROPAGATE_FILTER),
        _ => vec![Scope::All],
    };
    let propagated = match font {
        Some(font) if !scopes.is_empty() => match effective_anchors_for_master_with(font, data, &resolved.master) {
            Ok(map) => Some(map),
            Err(e) => {
                warnings.push(format!("anchors were not passed on from components for mark attachment, so each glyph's own anchors were used: {e}"));
                None
            }
        },
        _ => None,
    };
    let glyphs = order
        .iter()
        .map(|g| {
            let anchors: Vec<MarkAnchor> = match propagated.as_ref().filter(|_| reaches(&scopes, &g.name)).and_then(|map| map.get(&g.name)) {
                Some(list) => list.iter().map(|a| MarkAnchor { name: a.name.clone(), x: a.x, y: a.y }).collect(),
                None => g.anchors.iter().map(|(name, p)| MarkAnchor { name: name.clone(), x: p.x, y: p.y }).collect(),
            };
            let opentype_category =
                if ufo { font.and_then(|f| f.glyph(&g.name)).and_then(|source| source.info.opentype_category.clone()) } else { None };
            MarkGlyph {
                name: g.name.clone(),
                codepoints: g.unicodes.clone(),
                category: g.category.clone(),
                sub_category: g.sub_category.clone(),
                opentype_category,
                anchors: vec![MasterAnchors { master: resolved.master.clone(), anchors }],
            }
        })
        .collect();
    MarkInput { glyphs, explicit_classes: ufo, language_systems: font.map(kern::language_systems).unwrap_or_default() }
}

/// The generated features the source asks for: its feature-writer setting, less the tags its
/// feature code writes by hand. The warnings are given only when the font has anchors.
fn tags(font: &Font, master: &str, any_anchor: bool, warnings: &mut Vec<String>) -> MarkTags {
    // The font's setting first, then the master's, as for the kerning.
    let setting = font.info.user_data.get(FEATURE_WRITERS_KEY).or_else(|| font.master(master).ok()?.user_data.get(FEATURE_WRITERS_KEY));
    let mut quiet = Vec::new();
    let notes = if any_anchor { &mut *warnings } else { &mut quiet };
    let mut tags = mark_tags(setting, notes);
    let hand_written = kern::hand_written_features(font);
    let includes = kern::feature_code(font).includes;
    let mut left_out = Vec::new();
    for (tag, flag) in
        [("mark", &mut tags.mark), ("mkmk", &mut tags.mkmk), ("abvm", &mut tags.abvm), ("blwm", &mut tags.blwm), ("curs", &mut tags.curs)]
    {
        if !*flag {
            continue;
        }
        if hand_written.iter().any(|t| t == tag) {
            *flag = false;
            notes.push(format!(
                "the font has a hand-written {tag} feature, which is compiled from M8: the generated {tag} feature was not written to the font"
            ));
        } else if !includes.is_empty() {
            *flag = false;
            left_out.push(tag);
        }
    }
    if !left_out.is_empty() {
        notes.push(format!(
            "the feature code includes {}, which may hold mark features and is compiled from M8: the generated {} features were not written to the font",
            includes.join(", "),
            left_out.join(", ")
        ));
    }
    tags
}

/// Each feature key (feature, language, script) of `gpos` and its lookup indices.
fn keys(gpos: &Gpos) -> Vec<((Tag, Tag, Tag), Vec<u16>)> {
    let mut out = Vec::new();
    for record in &gpos.script_list.script_records {
        let mut systems: Vec<(Tag, &LangSys)> = Vec::new();
        if let Some(lang_sys) = record.script.default_lang_sys.as_deref() {
            systems.push((DFLT_LANG, lang_sys));
        }
        for lang in &record.script.lang_sys_records {
            systems.push((lang.lang_sys_tag, &*lang.lang_sys));
        }
        for (lang, sys) in systems {
            for index in &sys.feature_indices {
                if let Some(feature) = gpos.feature_list.feature_records.get(usize::from(*index)) {
                    out.push(((feature.feature_tag, lang, record.script_tag), feature.feature.lookup_list_indices.clone()));
                }
            }
        }
    }
    out
}

fn lookups(gpos: Gpos) -> Vec<PositionLookup> {
    gpos.lookup_list.into_inner().lookups.into_iter().map(|l| l.into_inner()).collect()
}

/// The kerning's and the mark writer's lookups in one table: the first `curs` mark lookups, the
/// kerning, then the other mark lookups.
fn merge(kern: Option<Gpos>, marks: Option<Gpos>, curs: usize, warnings: &mut Vec<String>) -> Option<Gpos> {
    let (kern, marks) = match (kern, marks) {
        (kern, None) => return kern,
        (None, marks) => return marks,
        (Some(kern), Some(marks)) => (kern, marks),
    };
    let mut keyed: BTreeMap<(Tag, Tag, Tag), Vec<u16>> = BTreeMap::new();
    let kern_keys = keys(&kern);
    let mark_keys = keys(&marks);
    let kern_lookups = lookups(kern);
    let mut mark_lookups = lookups(marks);
    let kern_count = kern_lookups.len();
    if kern_count.saturating_add(mark_lookups.len()) > usize::from(u16::MAX) {
        warnings.push(format!("mark attachment was not written to the font: with the kerning it needs more than {} lookups", u16::MAX));
        return Some(rebuild(kern_keys, kern_lookups));
    }
    let shift = |index: u16, by: usize| u16::try_from(usize::from(index).saturating_add(by)).unwrap_or(u16::MAX);
    for (key, indices) in kern_keys {
        keyed.insert(key, indices.into_iter().map(|i| shift(i, curs)).collect());
    }
    for (key, indices) in mark_keys {
        keyed.insert(key, indices.into_iter().map(|i| if usize::from(i) < curs { i } else { shift(i, kern_count) }).collect());
    }
    let rest = mark_lookups.split_off(curs.min(mark_lookups.len()));
    let mut all = mark_lookups;
    all.extend(kern_lookups);
    all.extend(rest);
    match gpos_from_keys(keyed, all) {
        Ok(gpos) => Some(gpos),
        Err(e) => {
            warnings.push(format!("neither the kerning nor mark attachment was written to the font: {e}"));
            None
        }
    }
}

/// `lookups` with the feature records of `keys`, built anew.
fn rebuild(keys: Vec<((Tag, Tag, Tag), Vec<u16>)>, lookups: Vec<PositionLookup>) -> Gpos {
    let keyed: BTreeMap<(Tag, Tag, Tag), Vec<u16>> = keys.into_iter().collect();
    gpos_from_keys(keyed, lookups).unwrap_or_default()
}

#[cfg(test)]
#[path = "marks_tests.rs"]
mod tests;
