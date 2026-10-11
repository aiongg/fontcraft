//! Segment generation: the candidate stem edges of a glyph in one dimension, from lines, curve
//! extremes, bends and bounding boxes.
//!
//! Ported from otfautohint's `hinter.py` lines 78–1053 (`dimensionHinter`; AFDKO 5.0.1,
//! Apache-2.0, Copyright 2021 Adobe; see `NOTICE`). Package M10.9 fills the bodies marked
//! `// M10.9:`. Functions marked *shared* are called from other packages and keep their
//! signature.

use crate::glyph::{PeRef, Pt, fne};
use crate::hinter::DimHinter;
use crate::state::{Links, SegType};

/// `diffSign`: both non-zero and of different signs.
pub fn diff_sign(a: f64, b: f64) -> bool {
    fne(a, 0.0) && fne(b, 0.0) && ((a > 0.0) != (b > 0.0))
}

/// `sameSign`: both non-zero and of the same sign.
pub fn same_sign(a: f64, b: f64) -> bool {
    fne(a, 0.0) && fne(b, 0.0) && ((a > 0.0) == (b > 0.0))
}

/// `interpolate`: the value at `q` on the line through `(q0, v0)` and `(q1, v1)`.
pub fn interpolate(q: f64, v0: f64, q0: f64, v1: f64, q1: f64) -> f64 {
    // M10.9: interpolate (guard q1 == q0).
    let _ = (q, q0, v1, q1);
    v0
}

impl DimHinter<'_> {
    /// Hints the glyph in this dimension: segments, stem values, main values, counter hinting
    /// (`calcHintValues`). *Shared* (M10.12).
    pub fn calc_hint_values(&mut self, lnks: &mut Links, force: bool, try_counter: bool) {
        // M10.9: calcHintValues; calls genSegs, limitSegs (here), the stem-value steps of
        // crate::stems and markStraySegs.
        let _ = (lnks, force, try_counter);
    }

    /// Whether overlaps are removed for this glyph (`handleOverlap`).
    pub fn handle_overlap(&self) -> bool {
        match self.st.options.overlap_forcing {
            Some(force) => force,
            None => self.st.options.overlap_list.contains(&self.st.name) || self.st.is_multi,
        }
    }

    /// Records a segment for `pe1` (and `pe2`), resolving substitute elements (`addSegment`).
    /// *Shared*.
    #[allow(clippy::too_many_arguments)]
    pub fn add_segment(&mut self, fr: f64, to: f64, loc: f64, pe1: Option<PeRef>, pe2: Option<PeRef>, typ: SegType, desc: &str, mid: bool) {
        // M10.9: addSegment, then GlyphHintState::add_segment.
        let _ = (fr, to, loc, pe1, pe2, typ, desc, mid);
    }

    /// `CPFrom`: the control point near `cp3` on the way from `cp2`.
    pub fn cp_from(&self, cp2: Pt, cp3: Pt) -> Pt {
        // M10.9: CPFrom.
        let _ = cp2;
        cp3
    }

    /// `CPTo`: the control point near `cp0` on the way to `cp1`.
    pub fn cp_to(&self, cp0: Pt, cp1: Pt) -> Pt {
        // M10.9: CPTo.
        let _ = cp1;
        cp0
    }

    /// `adjustDist`.
    pub fn adjust_dist(&self, v: f64, q: f64) -> f64 {
        // M10.9: adjustDist.
        let _ = q;
        v
    }

    /// `testTan`: the direction `p` is flat enough in this dimension.
    pub fn test_tan(&self, p: Pt) -> bool {
        // M10.9: testTan.
        let _ = p;
        false
    }

    /// `flatQuo`: how flat the run from `p1` to `p2` is, 0 to 1.
    pub fn flat_quo(&self, p1: Pt, p2: Pt, do_oppo: bool) -> f64 {
        // M10.9: flatQuo.
        let _ = (p1, p2, do_oppo);
        0.0
    }

    /// `testBend`.
    pub fn test_bend(&self, p0: Pt, p1: Pt, p2: Pt) -> bool {
        // M10.9: testBend.
        let _ = (p0, p1, p2);
        false
    }

    /// `isCCW`.
    pub fn is_ccw(&self, p0: Pt, p1: Pt, p2: Pt) -> bool {
        // M10.9: isCCW.
        let _ = (p0, p1, p2);
        false
    }

    /// `relPosition`.
    pub fn rel_position(&self, c: PeRef, lower: bool) -> bool {
        // M10.9: relPosition.
        let _ = (c, lower);
        false
    }

    /// `doBendsNext`.
    pub fn do_bends_next(&mut self, c: PeRef) {
        // M10.9: doBendsNext.
        let _ = c;
    }

    /// `doBendsPrev`.
    pub fn do_bends_prev(&mut self, c: PeRef) {
        // M10.9: doBendsPrev.
        let _ = c;
    }

    /// `nodeIsFlat`.
    pub fn node_is_flat(&self, c: PeRef, do_prev: bool) -> Option<bool> {
        // M10.9: nodeIsFlat.
        let _ = (c, do_prev);
        None
    }

    /// `sameDir`.
    pub fn same_dir(&self, c: PeRef, do_prev: bool) -> Option<bool> {
        // M10.9: sameDir.
        let _ = (c, do_prev);
        None
    }

    /// `extremaSegment`: the extent of a segment through the extreme `extp` (at `extt`) of a
    /// curve, kept within `EXTREMA_DIST` of it.
    pub fn extrema_segment(&self, pe: PeRef, extp: Pt, extt: f64, is_mn: bool) -> (f64, f64) {
        // M10.9: extremaSegment.
        let _ = (pe, extp, extt, is_mn);
        (0.0, 0.0)
    }

    /// `pickSpot`.
    #[allow(clippy::too_many_arguments)]
    pub fn pick_spot(&self, p0: Pt, p1: Pt, dist: f64, pp0: Pt, pp1: Pt, prv: Pt, nxt: Pt) -> f64 {
        // M10.9: pickSpot.
        let _ = (p1, dist, pp0, pp1, prv, nxt);
        p0.a(self.dim())
    }

    /// `cpDirection`: −1, 0 or 1.
    pub fn cp_direction(&self, p0: Pt, p1: Pt, p2: Pt) -> i8 {
        // M10.9: cpDirection.
        let _ = (p0, p1, p2);
        0
    }

    /// `prepForSegs`: splits curves at inflections where needed.
    pub fn prep_for_segs(&mut self) {
        // M10.9: prepForSegs.
    }

    /// `genSegs`: every segment of the glyph. *Shared*.
    pub fn gen_segs(&mut self) {
        // M10.9: genSegs (with the overlap-free outline when handle_overlap()).
    }

    /// `genSegsForPathElement`.
    pub fn gen_segs_for_path_element(&mut self, c: PeRef) {
        // M10.9: genSegsForPathElement.
        let _ = c;
    }

    /// `limitSegs`: leaves a direction with too many segments unhinted, with a warning.
    pub fn limit_segs(&mut self) {
        // M10.9: limitSegs (Options::segment_limit).
    }
}
