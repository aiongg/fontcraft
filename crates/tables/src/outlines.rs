//! Outline repairs on compiled fonts (M12.28; rows `outlines remove-overlaps`, `direction`,
//! `extremes`, `round`, `decompose` and their `outlines.*` commands). Oracles: fontTools
//! `ttLib.removeOverlaps`, checkoutlinesufo `-e`, ttfdecomponentizer. Untouched tables are copied
//! byte for byte.

use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

/// One repair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutlineAction {
    RemoveOverlaps,
    CorrectDirection,
    AddExtremes,
    Round,
    Decompose,
}

/// What [`repair`] does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlineOptions {
    pub action: OutlineAction,
    /// Glyphs to change; empty changes every glyph.
    pub glyphs: Vec<String>,
}

/// The changed font and the glyphs that changed (shared with [`crate::curves`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlineResult {
    pub font: SfntFont,
    pub changed: Vec<String>,
}

/// Applies one repair.
pub fn repair(font: &SfntFont, options: &OutlineOptions) -> Result<OutlineResult> {
    // M12.28
    let _ = (font, options);
    Err(TablesError::NotBuilt("outline repairs"))
}
