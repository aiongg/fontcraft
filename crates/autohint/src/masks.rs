//! Stems in the other masters, conflicts between overlapping stems, and the hint masks of each
//! path element (hint substitution).
//!
//! Ported from otfautohint's `hinter.py` lines 1904–2277 (`calcInstanceStems` to `OKToRem`;
//! AFDKO 5.0.1, Apache-2.0, Copyright 2021 Adobe; see `NOTICE`). Package M10.11 fills the
//! bodies marked `// M10.11:`. Rows: `hinting.ps.export.hintReplacement`.

use crate::glyph::PeRef;
use crate::hinter::DimHinter;
use crate::state::GlyphHintState;

impl DimHinter<'_> {
    /// The stems of master `glidx` from the main master's (`calcInstanceStems`). *Shared*
    /// (M10.12).
    pub fn calc_instance_stems(&mut self, glidx: usize) {
        // M10.11: calcInstanceStems.
        let _ = glidx;
    }

    /// The best location of stem `sidx`'s lower (`ul` 0) or upper (`ul` 1) edge in another
    /// master (`bestLocation`).
    pub fn best_location(&self, sidx: usize, ul: usize, iss: &[crate::state::InstanceStemState], hs0: &GlyphHintState) -> Option<f64> {
        // M10.11: bestLocation.
        let _ = (sidx, ul, iss, hs0);
        None
    }

    /// Resolves conflicts in a mask (`unconflict`): returns the stems kept on.
    pub fn unconflict(&self, sc: &[bool], cur: Option<&[bool]>, pin: Option<&[bool]>) -> Vec<bool> {
        // M10.11: unconflict.
        let _ = (cur, pin);
        sc.to_vec()
    }

    /// Builds the overlap and ghost tables and every element's mask (`convertToMasks`).
    /// *Shared* (M10.12).
    pub fn convert_to_masks(&mut self) {
        // M10.11: convertToMasks.
    }

    /// The mask of element `c` (`makePEMask`).
    pub fn make_pe_mask(&mut self, c: PeRef) {
        // M10.11: makePEMask.
        let _ = c;
    }

    /// A stem at `loc` may be dropped (`OKToRem`).
    pub fn ok_to_rem(&self, loc: f64, spc: f64) -> bool {
        // M10.11: OKToRem.
        let _ = (loc, spc);
        false
    }
}
