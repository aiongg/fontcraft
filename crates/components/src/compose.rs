//! Composition (M6.3): the components a glyph name is built from, and a new composite glyph.
//!
//! Rows: `components.autoComposeOnAddGlyph`, `components.autoComposeNeedsAllParts`,
//! `components.preferMarkWithSameSuffix`, `components.preferCaseMarksForUppercase`,
//! `components.preferNarrowMarksForIJ`, `components.ijBuiltFromDotless`.
//!
//! Contract:
//! - [`components_for`] maps a name to its parts: the glyph database's decomposition (a custom
//!   glyph data file first, then the Unicode canonical decomposition,
//!   [`fontcraft_glyphdata::GlyphData::canonical_decomposition`]) turned into glyph names, the
//!   name's suffix carried to the parts, the preferred-mark rules, and our own table for what
//!   Unicode does not decompose (i, j, ligatures by name parts).
//! - All or nothing: when a part is missing from the font the result is
//!   [`ComponentsError::MissingParts`] naming every missing part, and nothing is built.
//!   `Ok(None)` means no rule decomposes the name.
//! - [`composite_glyph`] builds the glyph: one master layer per master holding the parts as
//!   components, aligned ([`crate::align`]). It does not add the glyph to the font.

use fontcraft_glyphdata::GlyphData;
use fontcraft_model::{Font, Glyph};

use crate::ComponentsError;

/// The glyph names `name` is composed of, base first; see the module doc.
pub fn components_for(font: &Font, data: &GlyphData, name: &str) -> Result<Option<Vec<String>>, ComponentsError> {
    // M6.3
    let _ = (font, data, name);
    Ok(None)
}

/// A new glyph `name` holding `parts` as components on a master layer of every master, aligned.
/// Every part must exist in `font` ([`ComponentsError::MissingParts`] otherwise); the code
/// points come from the glyph database.
pub fn composite_glyph(font: &Font, data: &GlyphData, name: &str, parts: &[String]) -> Result<Glyph, ComponentsError> {
    // M6.3
    let _ = (font, data, name, parts);
    Err(ComponentsError::NotYetImplemented("composing a glyph"))
}
