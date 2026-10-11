//! Hinting with overlaps removed: segments come from the overlap-free outline, matched back to
//! the elements of the original.
//!
//! Ported from otfautohint's `overlap.py` (AFDKO 5.0.1, Apache-2.0, Copyright 2021 Adobe; see
//! `NOTICE`). The original removes overlaps itself with booleanOperations; this crate sits below
//! the path operations, so the caller passes the overlap-free contours
//! ([`crate::HintInput::without_overlap`]). Package M10.9 fills the body.

use fontcraft_geom::BezPath;

use crate::AutohintError;
use crate::glyph::GlyphData;

/// The overlap-free glyph, its elements associated with those of `glyph` (`removeOverlap`):
/// `None` when `without_overlap` draws the same outline as `glyph`.
pub fn associate(glyph: &GlyphData, without_overlap: &[BezPath], loose: bool) -> Result<Option<GlyphData>, AutohintError> {
    // M10.9: build GlyphData from the contours, compare, GlyphData::associate_path.
    let _ = (glyph, without_overlap, loose);
    Ok(None)
}
