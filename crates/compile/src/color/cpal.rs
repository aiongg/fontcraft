//! `CPAL` (M11.6): version 0, every palette with the one `Color Palette for CPAL` names first,
//! the colours COLR added after the palette rows, no labels or types.
//!
//! Rows: `CPAL`, `color.export.colorPaletteForCpal`, `color.export.defaultPalette`.

use fontcraft_color::Rgba;
use fontcraft_color::export::CpalPlan;

use crate::CompileError;

/// The table: `plan`'s palettes, each extended by `added` (the colours COLR uses that are not
/// palette references, the same rows in every palette).
pub fn build(plan: &CpalPlan, added: &[Rgba]) -> Result<Vec<u8>, CompileError> {
    // M11.6
    let _ = (plan, added);
    Err(CompileError::NotYetImplemented("the CPAL table"))
}
