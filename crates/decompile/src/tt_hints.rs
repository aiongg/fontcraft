//! TrueType hinting of an opened font: `cvt ` is stored as the cvt Table parameter and `prep`
//! and `fpgm` as assembly text in the prep and fpgm Table Assembly parameters on every master;
//! glyph programs go to layer user data with an outline fingerprint
//! (`fontcraft_hinting::settings::IMPORTED_GLYPH_PROGRAM_KEY`, `IMPORTED_OUTLINE_KEY`) and are
//! written back at export while the outline is unchanged; `gasp` becomes the gasp Table
//! parameter (`docs/plans/M10.md` § 3 › Imported hinting).
//!
//! Rows: `hinting.tt.import.glyphPrograms`, `hinting.tt.param.controlValueProgram` (import). M10.22
//! fills the body; the hook in `lib.rs` calls it after the font is built.

use fontcraft_model::Font;
use skrifa::raw::FontRef;

use crate::DecompileError;

/// The tables [`import`] reads, which `lib.rs` leaves out of its "tables not read" warning.
pub const TABLES: &[&str] = &[];

/// Adds the TrueType hinting of `sfnt` to `font`. A font without hinting tables is left alone.
/// Programs that do not disassemble go to `warnings` and are dropped; only a model error fails.
pub fn import(sfnt: &FontRef, font: &mut Font, warnings: &mut Vec<String>) -> Result<(), DecompileError> {
    // M10.22: cvt, prep, fpgm, gasp, glyph programs (fontcraft_ttasm::disassemble).
    let _ = (sfnt, font, warnings);
    Ok(())
}
