//! PostScript hints of an opened CFF font: its stems become numeric layer hints and its Private
//! dict becomes zones, stems and the blue parameters (`docs/plans/M10.md` § 3 › Imported
//! hinting).
//!
//! Rows: `open.postscriptHintsPreserved`. M10.22 fills the body; the hook in `lib.rs` calls it
//! after the font is built.

use fontcraft_model::Font;
use skrifa::raw::FontRef;

use crate::DecompileError;

/// Most stem hints read from one charstring (the Type 2 limit).
pub const MAX_IMPORTED_STEMS: usize = 96;

/// Adds the CFF hints of `sfnt` to `font` (one master, glyphs in the font's order). A font
/// without a `CFF ` table, or without hints, is left alone. What cannot be read goes to
/// `warnings`; only a model error fails.
pub fn import(sfnt: &FontRef, font: &mut Font, warnings: &mut Vec<String>) -> Result<(), DecompileError> {
    // M10.22: charstring stems to layer hints, Private dict to zones, stems and parameters.
    let _ = (sfnt, font, warnings);
    Ok(())
}
