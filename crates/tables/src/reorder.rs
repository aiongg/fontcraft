//! Reordering the glyphs of a compiled font (M12.15; rows `glyphs reorder`,
//! `font.reorderGlyphs`). Oracle: fontTools `ttLib.reorderGlyphs`. Every glyph-indexed table is
//! rewritten.

use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

/// The glyph order [`reorder_glyphs`] gives the font.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReorderOptions {
    /// Glyph names in their new order; glyphs left out follow in their old order. `.notdef`
    /// stays first.
    pub order: Vec<String>,
}

/// The font with its glyphs in the new order.
pub fn reorder_glyphs(font: SfntFont, options: &ReorderOptions) -> Result<SfntFont> {
    // M12.15
    let _ = (font, options);
    Err(TablesError::NotBuilt("glyphs reorder"))
}
