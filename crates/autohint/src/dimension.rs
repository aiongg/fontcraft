//! What differs between the two dimensions: zones apply to horizontal stems only, the dominant
//! stems and counter-glyph lists of each direction, the special-glyph rules.
//!
//! Ported from otfautohint's `hinter.py` lines 2277–2420 (`hhinter`, `vhinter`; AFDKO 5.0.1,
//! Apache-2.0, Copyright 2021 Adobe; see `NOTICE`). Every function here is *shared*.

use crate::fddict::BlueValue;
use crate::glyph::Dim;
use crate::hinter::DimHinter;
use crate::state::SegId;

impl DimHinter<'_> {
    /// Prepares the zone lists for hinting (`startHint`): horizontal only.
    pub fn start_hint(&mut self) {
        if self.is_v() {
            return;
        }
        let (top, bottom) = match self.st.fddict() {
            Some(d) => d.bands().cloned().partition(|p| !p.is_bottom),
            None => (Vec::new(), Vec::new()),
        };
        self.st.top_pairs = top;
        self.st.bottom_pairs = bottom;
    }

    /// The (decreasing, increasing) segment lists in the order this dimension reads them
    /// (`segmentLists`).
    pub fn segment_lists(&self) -> (Vec<SegId>, Vec<SegId>) {
        let hs = &self.st.hs;
        match self.dim() {
            Dim::H => (hs.increasing_segs.clone(), hs.decreasing_segs.clone()),
            Dim::V => (hs.decreasing_segs.clone(), hs.increasing_segs.clone()),
        }
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
        // otfautohint adds a fontinfo file's VCounterChars and HCounterChars to the option lists
        // for every glyph; here they arrive in the dictionary, so the main one is asked too.
        let o = &self.st.options;
        let name = &self.st.name;
        let dict = self.st.fddict();
        match self.dim() {
            Dim::H => o.h_counter_glyphs.contains(name) || dict.is_some_and(|d| d.h_counter_chars.contains(name)),
            Dim::V => o.v_counter_glyphs.contains(name) || dict.is_some_and(|d| d.v_counter_chars.contains(name)),
        }
    }

    /// `loc` lies in a top zone, or a bottom zone when `is_bottom` (`inBand`); always false for
    /// vertical stems.
    pub fn in_band(&self, loc: f64, is_bottom: bool) -> bool {
        if self.is_v() || self.st.options.no_blues.contains(&self.st.name) {
            return false;
        }
        let fuzz = self.st.fddict().map_or(0.0, |d| d.blue_fuzz);
        let pairs = if is_bottom { &self.st.bottom_pairs } else { &self.st.top_pairs };
        pairs.iter().any(|p| p.top + fuzz >= loc && p.bottom - fuzz <= loc)
    }

    /// The dictionary has zones and this dimension uses them (`hasBands`).
    pub fn has_bands(&self) -> bool {
        !self.is_v() && !(self.st.top_pairs.is_empty() && self.st.bottom_pairs.is_empty())
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
        if !self.is_v() {
            return false;
        }
        let o = &self.st.options;
        if lower { o.lower_specials.contains(&self.st.name) } else { o.upper_specials.contains(&self.st.name) }
    }

    /// Reports segments near but outside the zones (`checkTfm`).
    pub fn check_tfm(&mut self) {
        if self.is_v() {
            return;
        }
        let top = self.st.top_pairs.clone();
        let bottom = self.st.bottom_pairs.clone();
        let decreasing = self.st.hs.decreasing_segs.clone();
        let increasing = self.st.hs.increasing_segs.clone();
        self.check_tfm_val(&decreasing, &top);
        self.check_tfm_val(&increasing, &bottom);
    }

    /// `checkTfmVal`.
    pub fn check_tfm_val(&mut self, segs: &[SegId], pairs: &[BlueValue]) {
        for &s in segs {
            let Some(loc) = self.st.hs.seg(s).map(|s| s.loc) else { continue };
            if !self.check_inside_bands(loc, pairs) {
                self.check_near_bands(loc, pairs);
            }
        }
    }

    /// `checkInsideBands`.
    pub fn check_inside_bands(&self, loc: f64, pairs: &[BlueValue]) -> bool {
        pairs.iter().any(|p| loc <= p.top && loc >= p.bottom)
    }

    /// `checkNearBands`.
    pub fn check_near_bands(&mut self, loc: f64, pairs: &[BlueValue]) {
        // otfautohint logs these near misses at info level only; GlyphHints::warnings carries
        // warning-level messages, so nothing is kept.
        let _ = (loc, pairs);
    }
}
