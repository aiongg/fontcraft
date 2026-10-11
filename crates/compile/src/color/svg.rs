//! The `SVG ` table (M11.9 from paints, M11.10 from svg layers and bitmaps): one uncompressed
//! document per glyph, built by `fontcraft_color::svg` and `fontcraft_color::svg_import`.
//!
//! Rows: `SVG `, `color.export.exportSvgTable`, `color.export.fullColorToSvg`,
//! `color.export.sbixToSvg`, `color.svg.threeSources`.

use fontcraft_color::AssetStore;
use fontcraft_color::export::SvgPlan;
use fontcraft_model::Font;

use super::GlyphOrder;
use crate::CompileError;

/// The table. `font` gives the outlines of the paints' shapes and the svg layers' placements.
pub fn build(font: &Font, plan: &SvgPlan, order: &GlyphOrder, assets: &dyn AssetStore, warnings: &mut Vec<String>) -> Result<Vec<u8>, CompileError> {
    // M11.9, M11.10
    let _ = (font, plan, order, assets, warnings);
    Err(CompileError::NotYetImplemented("the SVG table"))
}
