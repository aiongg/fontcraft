//! `sbix` (M11.8): version 1, flags with bit 0 set and the draw-outlines bit clear, one strike
//! per iColor size at 72 pixels per inch. PNG and JPEG data stored as read (`png `, `jpg `),
//! TIFF converted to PNG. Serialised here and added raw, because write-fonts' strikes hold only
//! offsets.
//!
//! Rows: `sbix`, `color.sbix.bitmapFormats`, `color.export.sbixAutomatic`,
//! `image.ignoredAtExport`.

use fontcraft_color::AssetStore;
use fontcraft_color::export::SbixPlan;

use super::GlyphOrder;
use crate::CompileError;

/// The table. Origin offsets in pixels come from each image's placement and `units_per_em`. A
/// missing or unreadable image leaves the glyph out of its strike, with a warning.
pub fn build(
    plan: &SbixPlan,
    order: &GlyphOrder,
    assets: &dyn AssetStore,
    units_per_em: u16,
    warnings: &mut Vec<String>,
) -> Result<Vec<u8>, CompileError> {
    // M11.8
    let _ = (plan, order, assets, units_per_em, warnings);
    Err(CompileError::NotYetImplemented("the sbix table"))
}
