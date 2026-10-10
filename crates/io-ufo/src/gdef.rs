//! `public.openTypeCategories` for a UFO written from a font that was not read from a UFO
//! (`docs/spec/30-source-formats.md` § Glyph classification).
//!
//! The rules are glyphsLib's `_build_public_opentype_categories` (`builder/features.py`,
//! glyphsLib 6.15.0, Apache-2.0, see `NOTICE`), on each glyph's effective category and
//! subcategory (stored, else the built-in glyph database's) and on the anchors of its layer for
//! the UFO's master:
//!
//! | Class | When |
//! |---|---|
//! | `ligature` | subcategory `Ligature` and an attaching anchor |
//! | `mark` | category `Mark`, subcategory `Nonspacing` or `Spacing Combining` |
//! | `base` | any other glyph with an attaching anchor |
//!
//! An attaching anchor is one whose name does not start with `_`. glyphsLib classifies after
//! propagating anchors into composites; here a layer without anchors of its own counts the
//! attaching anchors of its components' glyphs, to [`MAX_DEPTH`] levels. A glyph none of the rules
//! picks gets no entry. A glyph that states its own class keeps it.
//!
//! Reading a UFO, a class its map gives a glyph is stored on the glyph only where it differs from
//! the class these rules give ([`strip_derived`]): a font written from another format and read
//! back states no classes it did not state before. The map stays in the master's lib as read, and
//! writing a UFO puts those entries back, so a UFO's own map is written as it was read.

use std::collections::HashMap;

use fontcraft_model::glyphdata::GlyphData;
use fontcraft_model::{Font, Glyph, Shape};

/// The class of each glyph by the rules, for the master `master`, from the built-in glyph
/// database; empty when that cannot be read.
pub(crate) fn derived(font: &Font, master: &str) -> HashMap<String, &'static str> {
    GlyphData::new().map(|data| categories(font, master, &data)).unwrap_or_default()
}

/// Clears each stored class that equals the class the rules give the glyph for `master`.
pub(crate) fn strip_derived(font: &mut Font, master: &str) {
    if !font.glyphs.iter().any(|g| g.info.opentype_category.is_some()) {
        return;
    }
    let derived = derived(font, master);
    let same: Vec<String> = font
        .glyphs
        .iter()
        .filter(|g| g.info.opentype_category.as_deref().is_some_and(|c| derived.get(&g.name) == Some(&c)))
        .map(|g| g.name.clone())
        .collect();
    for name in same {
        if let Some(mut g) = font.glyphs.get_mut(&name) {
            g.info.opentype_category = None;
        }
    }
}

/// How deep components are followed in search of an attaching anchor.
const MAX_DEPTH: usize = 16;

/// The class of each glyph the rules pick, for the master `master`.
pub(crate) fn categories(font: &Font, master: &str, data: &GlyphData) -> HashMap<String, &'static str> {
    let mut out = HashMap::new();
    let mut memo = HashMap::new();
    for g in font.glyphs.iter() {
        let stated = |v: &Option<String>| v.as_deref().filter(|v| !v.is_empty()).map(str::to_string);
        let (mut category, mut sub_category) = (stated(&g.info.category), stated(&g.info.sub_category));
        if category.is_none() || sub_category.is_none() {
            let info = data.lookup(&g.name, &g.unicodes);
            category = category.or_else(|| info.category.map(|c| c.value.as_str().to_string()));
            sub_category = sub_category.or_else(|| info.sub_category.map(|c| c.value.as_str().to_string()));
        }
        let attaching = has_attaching_anchor(font, g, master, 0, &mut memo);
        let class = match (category.as_deref(), sub_category.as_deref()) {
            (_, Some("Ligature")) if attaching => "ligature",
            (Some("Mark"), Some("Nonspacing" | "Spacing Combining")) => "mark",
            _ if attaching => "base",
            _ => continue,
        };
        out.insert(g.name.clone(), class);
    }
    out
}

/// Whether the glyph's layer for `master` has an attaching anchor, or (without anchors of its own)
/// one of its components' glyphs has. `memo` holds the answers found so far, so shared components
/// are looked at once.
fn has_attaching_anchor(font: &Font, g: &Glyph, master: &str, depth: usize, memo: &mut HashMap<String, bool>) -> bool {
    if let Some(&known) = memo.get(&g.name) {
        return known;
    }
    let Some(layer) = g.layer(master) else { return false };
    if !layer.anchors.is_empty() {
        return layer.anchors.iter().any(|a| !a.name.starts_with('_'));
    }
    // Marked as not attaching while it is looked at, so a component cycle ends.
    memo.insert(g.name.clone(), false);
    let mut found = false;
    if depth < MAX_DEPTH {
        for s in &layer.shapes {
            if let Shape::Component(c) = s
                && let Some(base) = font.glyph(&c.glyph)
                && has_attaching_anchor(font, base, master, depth + 1, memo)
            {
                found = true;
                break;
            }
        }
    }
    memo.insert(g.name.clone(), found);
    found
}
