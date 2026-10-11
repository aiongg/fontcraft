//! Flex: pairs of shallow curves that a rasterizer may flatten at small sizes.
//!
//! Ported from otfautohint's `hinter.py` (`linearFlexOK`, `addFlex`, `tryFlex`, `markFlex`;
//! AFDKO 5.0.1, Apache-2.0, Copyright 2021 Adobe; see `NOTICE`). Package M10.9 fills the bodies
//! marked `// M10.9:`. Rows: `hinting.ps.flex.*`.

use crate::hinter::{DimHinter, GlyphPe};

impl DimHinter<'_> {
    /// `linearFlexOK`.
    pub fn linear_flex_ok(&self) -> bool {
        false
    }

    /// Finds and marks flex in every master (`addFlex`). *Shared* (M10.12).
    pub fn add_flex(&mut self, force: bool, inited: bool) {
        // M10.9: addFlex.
        let _ = (force, inited);
    }

    /// Tests the curve pair starting at element `c` of master `glyph` (`tryFlex`).
    pub fn try_flex(&mut self, glyph: usize, c: crate::glyph::PeRef) -> bool {
        // M10.9: tryFlex.
        let _ = (glyph, c);
        false
    }

    /// Marks the pair in every master (`markFlex`).
    pub fn mark_flex(&mut self, cl: &[GlyphPe]) {
        // M10.9: markFlex.
        let _ = cl;
    }
}
