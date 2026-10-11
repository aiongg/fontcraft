//! The generated `GDEF` table: glyph classes, the ligature caret list and the mark filtering sets
//! of the mark writer's lookups.
//!
//! Rows: `compile.gdefClasses`, `compile.gdefLigatureCarets`, `anchors.ligatureCarets`.
//!
//! Ported from fontc (https://github.com/googlefonts/fontc), Apache-2.0, at commit
//! 4504578c3f74c2739f7e55cae133bbfe773f6d8c: the classes of a Glyphs source
//! (`category_for_glyph_preliminary` in `glyphs2fontir/src/source.rs` and
//! `recompute_gdef_categories` in `fontir/src/glyph.rs`, which follow glyphsLib's
//! `builder/features.py`), those of a UFO (`ufo2fontir/src/source.rs`), the carets
//! (`get_ligature_carets` in `fontbe/src/features/marks.rs`) and their order
//! (`fea-rs/src/compile/tables/gdef.rs`).
//!
//! Classes, for the reference's format:
//! - mark: category `Mark` and subcategory `Nonspacing` or `Spacing Combining`;
//! - ligature: subcategory `Ligature` and an attaching anchor (any anchor but a mark's `_name`);
//!   a ligature without one has no class;
//! - base: any other glyph with an attaching anchor.
//!
//! For a UFO the classes are the ones it states (`public.openTypeCategories`), with nothing
//! worked out from anchors.
//!
//! Carets: each `caret_N` anchor gives its x, each `vcaret_N` its y, sorted by value.

use std::collections::BTreeMap;

use write_fonts::tables::gdef::{CaretValue, Gdef, GlyphClassDef, LigCaretList, LigGlyph, MarkGlyphSets};
use write_fonts::tables::layout::{ClassDef, CoverageTable};
use write_fonts::types::GlyphId16;

use super::{Anchor, AnchorKind, MarkGlyph, MarkInput};

/// The class a glyph has before its anchors are looked at: mark and ligature only.
fn preliminary(glyph: &MarkGlyph) -> Option<GlyphClassDef> {
    match (glyph.category.as_deref(), glyph.sub_category.as_deref()) {
        (Some("Mark"), Some("Nonspacing" | "Spacing Combining")) => Some(GlyphClassDef::Mark),
        (_, Some("Ligature")) => Some(GlyphClassDef::Ligature),
        _ => None,
    }
}

/// A UFO's stated class. `unassigned` and an unknown word give none.
fn stated(glyph: &MarkGlyph) -> Result<Option<GlyphClassDef>, &str> {
    match glyph.opentype_category.as_deref() {
        Some("base") => Ok(Some(GlyphClassDef::Base)),
        Some("ligature") => Ok(Some(GlyphClassDef::Ligature)),
        Some("mark") => Ok(Some(GlyphClassDef::Mark)),
        Some("component") => Ok(Some(GlyphClassDef::Component)),
        Some("unassigned") | None => Ok(None),
        Some(other) => Err(other),
    }
}

/// The `GDEF` class of each glyph that has one, by glyph id. `anchors` are the glyphs' anchors on
/// the master compiled.
pub(super) fn glyph_classes(
    input: &MarkInput,
    anchors: &BTreeMap<GlyphId16, Vec<Anchor>>,
    warnings: &mut Vec<String>,
) -> BTreeMap<GlyphId16, GlyphClassDef> {
    let mut out = BTreeMap::new();
    let mut unknown: Vec<String> = Vec::new();
    for (i, glyph) in input.glyphs.iter().enumerate() {
        let Ok(id) = u16::try_from(i) else { break };
        let gid = GlyphId16::new(id);
        let class = if input.explicit_classes {
            match stated(glyph) {
                Ok(class) => class,
                Err(word) => {
                    if unknown.len() < 5 {
                        unknown.push(format!("`{word}` for `{}`", glyph.name));
                    }
                    None
                }
            }
        } else {
            let attaching = anchors.get(&gid).is_some_and(|list| list.iter().any(|a| !a.is_mark()));
            match preliminary(glyph) {
                Some(GlyphClassDef::Mark) => Some(GlyphClassDef::Mark),
                Some(GlyphClassDef::Ligature) => attaching.then_some(GlyphClassDef::Ligature),
                _ => attaching.then_some(GlyphClassDef::Base),
            }
        };
        if let Some(class) = class {
            out.insert(gid, class);
        }
    }
    if !unknown.is_empty() {
        warnings.push(format!("glyph classes the GDEF table does not know were left out: {}", unknown.join(", ")));
    }
    out
}

/// The caret positions of each glyph with `caret_N` or `vcaret_N` anchors, sorted.
pub(super) fn ligature_carets(anchors: &BTreeMap<GlyphId16, Vec<Anchor>>) -> BTreeMap<GlyphId16, Vec<i16>> {
    let mut out = BTreeMap::new();
    for (gid, list) in anchors {
        let mut carets: Vec<i16> = list
            .iter()
            .filter_map(|a| match a.kind {
                AnchorKind::Caret(_) => Some(a.x),
                AnchorKind::VCaret(_) => Some(a.y),
                _ => None,
            })
            .collect();
        if !carets.is_empty() {
            carets.sort_unstable();
            out.insert(*gid, carets);
        }
    }
    out
}

/// The table, or `None` when it would be empty.
pub(super) fn build(
    classes: &BTreeMap<GlyphId16, GlyphClassDef>,
    carets: &BTreeMap<GlyphId16, Vec<i16>>,
    filter_sets: Vec<Vec<GlyphId16>>,
) -> Option<Gdef> {
    if classes.is_empty() && carets.is_empty() && filter_sets.is_empty() {
        return None;
    }
    let class_def: Option<ClassDef> = (!classes.is_empty()).then(|| classes.iter().map(|(gid, class)| (*gid, *class as u16)).collect());
    let caret_list = (!carets.is_empty()).then(|| {
        let coverage: CoverageTable = carets.keys().copied().collect();
        let glyphs = carets.values().map(|list| LigGlyph::new(list.iter().map(|x| CaretValue::format_1(*x)).collect())).collect();
        LigCaretList::new(coverage, glyphs)
    });
    let mut gdef = Gdef::new(class_def, None, caret_list, None);
    if !filter_sets.is_empty() {
        gdef.mark_glyph_sets_def.set(MarkGlyphSets::new(filter_sets.into_iter().map(|set| set.into_iter().collect::<CoverageTable>()).collect()));
    }
    Some(gdef)
}
