//! `public.openTypeCategories` for a UFO written from a font that was not read from a UFO
//! (`docs/spec/30-source-formats.md` § Glyph classification). The rules are
//! [`fontcraft_model::ufo_classes::rule_classes`]; a glyph that states its own class keeps it.
//!
//! Reading a UFO, a class its map gives a glyph is stored on the glyph only where it differs from
//! the class the rules give ([`strip_derived`]): a font written from another format and read
//! back states no classes it did not state before. The map stays in the master's lib as read, and
//! writing a UFO puts those entries back, so a UFO's own map is written as it was read.

use std::collections::HashMap;

use fontcraft_model::Font;
use fontcraft_model::glyphdata::GlyphData;
use fontcraft_model::ufo_classes::rule_classes;

/// The class of each glyph by the rules, for the master `master`, from the built-in glyph
/// database; empty when that cannot be read.
pub(crate) fn derived(font: &Font, master: &str) -> HashMap<String, &'static str> {
    GlyphData::new().map(|data| rule_classes(font, master, &data)).unwrap_or_default()
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
