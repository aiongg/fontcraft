//! `COLR` (M11.6 version 0, M11.7 version 1, M11.19 variable).
//!
//! Rows: `COLR`, `color.export.colrFromPaletteLayers`, `color.export.fullColorToColrV1`,
//! `color.export.colrVersionChoice`, `color.export.strokeInColr`, `compile.colrClipBoxes`,
//! `compile.colrGradientGeometry`, `color.export.variableColr`.
//!
//! Contract (`docs/plans/M11.md` § 3): base glyph and layer records for version 0; for version 1
//! `BaseGlyphList`, `LayerList` and `ClipList`. Layer glyphs are those
//! [`super::layer_glyphs`] appended, found by name in the [`GlyphOrder`]. `*` (the text colour)
//! is palette index 0xFFFF. Colours that are not palette references become new rows, added once
//! to every palette, with their alpha in CPAL only.

use fontcraft_color::Rgba;
use fontcraft_color::export::ColrPlan;

use super::GlyphOrder;
use crate::CompileError;

/// A built `COLR` table and the colours it added to the palettes (rows after `palette_rows`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ColrOutput {
    pub table: Vec<u8>,
    pub added: Vec<Rgba>,
}

/// The table for `plan`, numbering new colours from `palette_rows`.
pub fn build(plan: &ColrPlan, order: &GlyphOrder, palette_rows: usize) -> Result<ColrOutput, CompileError> {
    // M11.6: version 0; M11.7: version 1.
    let _ = (plan, order, palette_rows);
    Err(CompileError::NotYetImplemented("the COLR table"))
}
