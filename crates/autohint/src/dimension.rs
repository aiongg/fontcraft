//! What differs between the two dimensions: zones apply to horizontal stems only, the dominant
//! stems and counter-glyph lists of each direction, the special-glyph rules.
//!
//! Ported from otfautohint's `hinter.py` lines 2277–2420 (`hhinter`, `vhinter`; AFDKO 5.0.1,
//! Apache-2.0, Copyright 2021 Adobe; see `NOTICE`). Package M10.10 fills the bodies marked
//! `// M10.10:`. Every function here is *shared*.

use crate::fddict::BlueValue;
use crate::glyph::Dim;
use crate::hinter::DimHinter;
use crate::state::SegId;

impl DimHinter<'_> {
    /// Prepares the zone lists for hinting (`startHint`): horizontal only.
    pub fn start_hint(&mut self) {
        // M10.10: hhinter.startHint (top_pairs, bottom_pairs).
    }

    /// The (decreasing, increasing) segment lists in the order this dimension reads them
    /// (`segmentLists`).
    pub fn segment_lists(&self) -> (Vec<SegId>, Vec<SegId>) {
        // M10.10: hhinter / vhinter.segmentLists.
        (self.st.hs.decreasing_segs.clone(), self.st.hs.increasing_segs.clone())
    }

    /// `DominantH` or `DominantV` of the main dictionary (`dominantStems`).
    pub fn dominant_stems(&self) -> Vec<f64> {
        let Some(d) = self.st.fddict() else { return Vec::new() };
        match self.dim() {
            Dim::H => d.dominant_h.clone(),
            Dim::V => d.dominant_v.clone(),
        }
    }

    /// The glyph is in this direction's counter-hinting list (`isCounterGlyph`).
    pub fn is_counter_glyph(&self) -> bool {
        // M10.10: also the dictionary's HCounterChars / VCounterChars.
        let o = &self.st.options;
        match self.dim() {
            Dim::H => o.h_counter_glyphs.contains(&self.st.name),
            Dim::V => o.v_counter_glyphs.contains(&self.st.name),
        }
    }

    /// `loc` lies in a top zone, or a bottom zone when `is_bottom` (`inBand`); always false for
    /// vertical stems.
    pub fn in_band(&self, loc: f64, is_bottom: bool) -> bool {
        // M10.10: hhinter.inBand.
        let _ = (loc, is_bottom);
        false
    }

    /// The dictionary has zones and this dimension uses them (`hasBands`).
    pub fn has_bands(&self) -> bool {
        // M10.10: hasBands.
        false
    }

    /// "horizontal" or "vertical", for reports (`aDesc`).
    pub fn a_desc(&self) -> &'static str {
        match self.dim() {
            Dim::H => "horizontal",
            Dim::V => "vertical",
        }
    }

    /// The glyph is in the upper (or lower) special list (`isSpecial`).
    pub fn is_special(&self, lower: bool) -> bool {
        // M10.10: isSpecial.
        let _ = lower;
        false
    }

    /// Reports segments near but outside the zones (`checkTfm`).
    pub fn check_tfm(&mut self) {
        // M10.10: checkTfm, checkTfmVal, checkInsideBands, checkNearBands.
    }

    /// `checkTfmVal`.
    pub fn check_tfm_val(&mut self, segs: &[SegId], pairs: &[BlueValue]) {
        // M10.10: checkTfmVal.
        let _ = (segs, pairs);
    }

    /// `checkInsideBands`.
    pub fn check_inside_bands(&self, loc: f64, pairs: &[BlueValue]) -> bool {
        // M10.10: checkInsideBands.
        let _ = (loc, pairs);
        false
    }

    /// `checkNearBands`.
    pub fn check_near_bands(&mut self, loc: f64, pairs: &[BlueValue]) {
        // M10.10: checkNearBands.
        let _ = (loc, pairs);
    }
}
