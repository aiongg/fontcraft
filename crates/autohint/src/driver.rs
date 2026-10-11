//! The glyph driver: hints both dimensions, removes flares, converts stems and distributes the
//! hint masks and counter masks over the path.
//!
//! Ported from otfautohint's `hinter.py` lines 2420–2992 (`glyphHinter`) and the per-glyph part
//! of `autohint.py` (AFDKO 5.0.1, Apache-2.0, Copyright 2021 Adobe; see `NOTICE`). Package
//! M10.12 fills the bodies marked `// M10.12:`.

use crate::glyph::{GlyphData, PeRef};
use crate::hinter::GlyphHinter;
use crate::state::SegId;
use crate::{AutohintError, GlyphHints, HintInput};

/// Hints one glyph. An empty glyph gets empty hints; a direction with more segments than the
/// limit is left unhinted with a warning; zones or stems the dictionary lacks are an error unless
/// [`crate::Options::allow_no_blues`] is set.
pub fn hint_glyph(input: &HintInput) -> Result<GlyphHints, AutohintError> {
    if input.contours.is_empty() {
        return Ok(GlyphHints::default());
    }
    // M10.12: GlyphData::from_contours, GlyphHinter::hint, GlyphData::to_hints.
    Err(AutohintError::NotYetImplemented("the PostScript autohinter"))
}

impl GlyphHinter {
    /// Hints the glyphs in [`GlyphHinter::glyphs`] (one per master) against their dictionaries
    /// (`_hint`). Returns false when the glyph keeps its own hints in both dimensions or the
    /// masters are not compatible.
    pub fn hint(&mut self, name: &str, fddicts: Vec<std::sync::Arc<crate::FdDict>>) -> Result<bool, AutohintError> {
        // M10.12: _hint.
        let _ = (name, fddicts);
        Err(AutohintError::NotYetImplemented("the PostScript autohinter"))
    }

    /// The segments of element `pe` in the current dimension, or the other when `oppo`
    /// (`getSegments`).
    pub fn get_segments(&self, pe: PeRef, oppo: bool) -> Vec<SegId> {
        // M10.12: getSegments.
        let _ = (pe, oppo);
        Vec::new()
    }

    /// The masks of element `pe` in both dimensions (`getMasks`).
    pub fn get_masks(&self, pe: PeRef) -> crate::HintMask {
        // M10.12: getMasks.
        let _ = pe;
        crate::HintMask::default()
    }

    /// Places the masks on the path and returns the masks used (`distributeMasks`).
    pub fn distribute_masks(&mut self) -> Option<Vec<crate::HintMask>> {
        // M10.12: distributeMasks.
        None
    }

    /// `buildCounterMasks`.
    pub fn build_counter_masks(&mut self) {
        // M10.12: buildCounterMasks.
    }

    /// `joinMasks`: (joined mask, whether it conflicts).
    pub fn join_masks(&self, m: &crate::HintMask, cm: &crate::HintMask, log: bool) -> (crate::HintMask, bool) {
        // M10.12: joinMasks.
        let _ = (cm, log);
        (m.clone(), false)
    }

    /// `bridgeMasks`.
    pub fn bridge_masks(&mut self, o: &crate::HintMask, n: &crate::HintMask, used: &mut Vec<bool>, pe: PeRef) {
        // M10.12: bridgeMasks.
        let _ = (o, n, used, pe);
    }

    /// `mergeMain`.
    pub fn merge_main(&self) -> bool {
        // M10.12: mergeMain.
        false
    }

    /// Drops stems no mask uses, in every master (`cleanupUnused`, `delUnused`).
    pub fn cleanup_unused(&mut self, used: &crate::HintMask) {
        // M10.12: cleanupUnused.
        let _ = used;
    }

    /// `listHintInfo`.
    pub fn list_hint_info(&mut self) {
        // M10.12: listHintInfo.
    }

    /// Removes flares in the current dimension (`remFlares`, `isFlare`, `isUSeg`,
    /// `reportRemFlare`).
    pub fn rem_flares(&mut self) {
        // M10.12: remFlares.
    }

    /// The result for master `glidx`.
    pub fn result(&self, glidx: usize) -> Option<GlyphHints> {
        let mut hints = self.glyphs.get(glidx).map(GlyphData::to_hints)?;
        hints.warnings.extend(self.h.warnings.iter().chain(&self.v.warnings).cloned());
        Some(hints)
    }
}
