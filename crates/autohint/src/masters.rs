//! Hinting one glyph in several compatible masters at once, so that every master gets the same
//! stems in the same order and the same masks (the input of a CFF2 font, written in M9).
//!
//! Ported from otfautohint's `hinter.py` (`compatiblePaths`, `otherInstanceStems`,
//! `otherInstanceMasks`; AFDKO 5.0.1, Apache-2.0, Copyright 2021 Adobe; see `NOTICE`). Package
//! M10.12 fills the bodies marked `// M10.12:`. Rows: `autohint.ps.variable.*` (data half).

use crate::hinter::GlyphHinter;
use crate::{AutohintError, FdDict, GlyphHints, HintInput, MAX_MASTERS};

/// Hints the same glyph in every master, main master first. Every input must draw the same path
/// structure (same subpaths, same element kinds); the result has one entry per input, in order.
/// More than [`MAX_MASTERS`] inputs is an error.
pub fn hint_masters(inputs: &[HintInput]) -> Result<Vec<GlyphHints>, AutohintError> {
    if inputs.len() > MAX_MASTERS {
        return Err(AutohintError::LimitExceeded { what: "masters hinted together", limit: MAX_MASTERS });
    }
    if inputs.iter().all(|i| i.contours.is_empty()) {
        return Ok(vec![GlyphHints::default(); inputs.len()]);
    }
    // M10.12: one GlyphHinter over every master.
    Err(AutohintError::NotYetImplemented("autohinting several masters"))
}

impl GlyphHinter {
    /// The masters' paths match element for element (`compatiblePaths`).
    pub fn compatible_paths(&mut self, fddicts: &[std::sync::Arc<FdDict>]) -> bool {
        // M10.12: compatiblePaths.
        let _ = fddicts;
        self.glyphs.others.is_empty()
    }

    /// Stems of the other masters from the main master's (`otherInstanceStems`).
    pub fn other_instance_stems(&mut self) {
        // M10.12: otherInstanceStems (DimHinter::calc_instance_stems per master).
    }

    /// Masks of the other masters from the main master's (`otherInstanceMasks`).
    pub fn other_instance_masks(&mut self) {
        // M10.12: otherInstanceMasks.
    }
}
