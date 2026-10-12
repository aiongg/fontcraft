//! Segment generation: the candidate stem edges of a glyph in one dimension, from lines, curve
//! extremes, bends and bounding boxes.
//!
//! Ported from otfautohint's `hinter.py` lines 78–1053 (`dimensionHinter`; AFDKO 5.0.1,
//! Apache-2.0, Copyright 2021 Adobe; see `NOTICE`). Functions marked *shared* are called from other
//! packages and keep their signature.
//!
//! Segments are generated from the outline [`DimHinter::seg_glyph`] returns: the main master's
//! glyph, or the overlap-free outline in [`crate::state::GlyphHintState::overlap_removed`] when
//! the glyph is hinted with overlaps removed. Whoever prepares the hint state of a glyph puts
//! [`crate::overlap::associate`]'s result there (`Removed` or `NoOverlap`) before
//! [`DimHinter::calc_hint_values`] or [`DimHinter::gen_segs`] runs; [`DimHinter::add_segment`]
//! follows each element's association back to the main glyph, so every segment names an element
//! of the main glyph.

use crate::bezier::solve_cubic;
use crate::glyph::{FlexPart, GlyphData, PeRef, Pt, SegmentSub, feq, fne};
use crate::hinter::DimHinter;
use crate::hinter::{
    AUTO_LINEAR_CURVE_FIX, BEND_LENGTH, BEND_TANGENT, BIG_DIST_FACTOR, CP_FRAC, EXTREMA_DIST, FLAT_MIN, MAX_STEM_DIST, OPPO_FLAT_MAX,
    SPECIAL_CHAR_BONUS, THETA,
};
use crate::state::{Links, OverlapState, SegType};

/// Segments generated before compaction, per direction, beyond which a glyph is not compacted
/// but left unhinted in that dimension at once, as [`DimHinter::limit_segs`] would after
/// compaction. Compaction compares every pair of segments, so this bounds its work; no glyph
/// that compacts to the segment limit comes near it.
const RAW_SEGMENT_FACTOR: usize = 16;

/// `diffSign`: both non-zero and of different signs.
pub fn diff_sign(a: f64, b: f64) -> bool {
    fne(a, 0.0) && fne(b, 0.0) && ((a > 0.0) != (b > 0.0))
}

/// `sameSign`: both non-zero and of the same sign.
pub fn same_sign(a: f64, b: f64) -> bool {
    fne(a, 0.0) && fne(b, 0.0) && ((a > 0.0) == (b > 0.0))
}

/// `interpolate`: the value at `q` on the line through `(q0, v0)` and `(q1, v1)`; `v0` when
/// `q0` and `q1` coincide.
pub fn interpolate(q: f64, v0: f64, q0: f64, v1: f64, q1: f64) -> f64 {
    let dq = q1 - q0;
    if dq == 0.0 {
        return v0;
    }
    v0 + (q - q0) * (v1 - v0) / dq
}

/// The parts of an element that segment generation reads, copied out so the glyph can be
/// changed while they are in use.
#[derive(Clone, Copy)]
struct ElView {
    s: Pt,
    cs: Pt,
    ce: Pt,
    e: Pt,
    is_line: bool,
    flex: Option<FlexPart>,
    flex_unset: bool,
    is_start: bool,
    is_tiny: bool,
}

impl ElView {
    fn of(g: &GlyphData, c: PeRef) -> Option<Self> {
        let el = g.element(c)?;
        Some(Self {
            s: el.s,
            cs: el.cs,
            ce: el.ce,
            e: el.e,
            is_line: el.is_line,
            flex: el.flex,
            flex_unset: el.flex_unset,
            is_start: el.is_start(),
            is_tiny: el.is_tiny(),
        })
    }
}

/// `pt.__eq__` with a tolerance.
fn near_within(a: Pt, b: Pt, factor: f64) -> bool {
    (a.x - b.x).abs() < factor && (a.y - b.y).abs() < factor
}

impl DimHinter<'_> {
    /// The outline segments are generated from (`self.glyph` inside `genSegs`): the overlap-free
    /// outline when one is in use, else the main master's glyph.
    pub fn seg_glyph(&self) -> &GlyphData {
        match &self.st.hs.overlap_removed {
            OverlapState::Removed(g) => g,
            _ => &self.glyphs.main,
        }
    }

    fn seg_glyph_mut(&mut self) -> &mut GlyphData {
        match &mut self.st.hs.overlap_removed {
            OverlapState::Removed(g) => g,
            _ => &mut self.glyphs.main,
        }
    }

    /// Hints the glyph in this dimension: segments, stem values, main values, counter hinting
    /// (`calcHintValues`). *Shared* (M10.12).
    pub fn calc_hint_values(&mut self, lnks: &mut Links, force: bool, try_counter: bool) {
        self.start_hint();
        let vert = self.is_v();
        if self.glyph().has_hints(vert) {
            if force {
                self.glyph_mut().clear_hints(vert);
                self.glyph_mut().changed = true;
            } else {
                self.st.keep_hints = true;
                return;
            }
        }
        self.st.keep_hints = false;
        self.st.big_dist = self.dominant_stems().into_iter().fold(MAX_STEM_DIST, f64::max) * BIG_DIST_FACTOR;
        let counter = try_counter && self.is_counter_glyph();
        self.gen_segs();
        self.limit_segs();
        self.st.pruning = !counter;
        self.gen_stem_vals();
        self.prune_stem_vals();
        self.highest_stem_vals();
        self.merge_vals();
        self.limit_vals();
        self.glyph_mut().sync_positions();
        lnks.mark(&self.st.hs, vert);
        self.check_vals();
        self.report_stems();
        self.main_vals();
        if counter {
            self.st.pruning = true;
            self.st.hs.counter_hinted = self.try_counter_hinting();
            if !self.st.hs.counter_hinted {
                self.add_bbox(true);
                self.st.hs.counter_hinted = self.try_counter_hinting();
                if !self.st.hs.counter_hinted {
                    self.st.reset_for_hinting();
                    self.calc_hint_values(lnks, true, false);
                    return;
                }
                self.highest_stem_vals();
            }
        }
        if self.st.hs.main_values.is_empty() {
            self.add_bbox(false);
        }
        self.mark_stray_segs();
        self.st.hs.prune_hint_segs();
    }

    /// Whether overlaps are removed for this glyph (`handleOverlap`).
    pub fn handle_overlap(&self) -> bool {
        match self.st.options.overlap_forcing {
            Some(force) => force,
            None => self.st.options.overlap_list.contains(&self.st.name) || self.st.is_multi,
        }
    }

    /// Records a segment for `pe1` (and `pe2`), resolving substitute elements (`addSegment`).
    /// `pe1` and `pe2` name elements of [`DimHinter::seg_glyph`]; the segment is recorded for
    /// the elements of the main glyph they stand for. *Shared*.
    #[allow(clippy::too_many_arguments)]
    pub fn add_segment(&mut self, fr: f64, to: f64, loc: f64, pe1: Option<PeRef>, pe2: Option<PeRef>, typ: SegType, desc: &str, mid: bool) {
        let g = self.seg_glyph();
        let mut mid = mid;
        let mut pe1 = pe1;
        let mut pe2 = pe2;
        if let Some(r) = pe1
            && let Some(i) = r.sub
        {
            let last = match g.element(r.parent()).and_then(|p| p.segment_sub.as_ref()) {
                Some(SegmentSub::Parts(parts)) => parts.len().checked_sub(1),
                _ => None,
            };
            if last != Some(i) {
                mid = true;
            }
            pe1 = Some(r.parent());
        }
        if let Some(r) = pe2
            && let Some(i) = r.sub
        {
            if i != 0 {
                mid = true;
            }
            pe2 = Some(r.parent());
        }
        if pe1 == pe2 {
            pe2 = None;
        }
        let (pe1, mid1) = self.follow_association(pe1, mid);
        let (pe2, mid2) = self.follow_association(pe2, mid);
        let loc = if self.st.options.round_coords { loc.round_ties_even() } else { loc };
        if pe1.is_none() && pe2.is_none() {
            return;
        }
        let (bonus, is_v) = (self.st.bonus, self.is_v());
        self.st.hs.add_segment(fr, to, loc, pe1, pe2, typ, bonus, is_v, mid1, mid2, desc);
    }

    /// An element of an overlap-free outline is replaced by the original element it was matched
    /// to; the segment counts as one from the middle of that element when the end points differ.
    fn follow_association(&self, pe: Option<PeRef>, mid: bool) -> (Option<PeRef>, bool) {
        let Some(r) = pe else { return (None, mid) };
        let Some(el) = self.seg_glyph().element(r) else { return (pe, mid) };
        let Some(assoc) = el.association else { return (pe, mid) };
        let mut mid = mid;
        if !mid
            && let Some(o) = assoc.and_then(|o| self.glyphs.main.element(o))
            && !near_within(el.e, o.e, o.assoc_factor(false))
        {
            mid = true;
        }
        (assoc, mid)
    }

    /// `CPFrom`: `cp3` moved towards `cp2` by [`CP_FRAC`], one coordinate.
    pub fn cp_from(&self, cp2: f64, cp3: f64) -> f64 {
        (cp3 - cp2) * (1.0 - CP_FRAC) + cp2
    }

    /// `CPTo`: `cp1` moved towards `cp0` by 1 − [`CP_FRAC`], one coordinate.
    pub fn cp_to(&self, cp0: f64, cp1: f64) -> f64 {
        (cp1 - cp0) * CP_FRAC + cp0
    }

    /// `adjustDist`.
    pub fn adjust_dist(&self, v: f64, q: f64) -> f64 {
        v * q
    }

    /// `testTan`: the direction `p` is flat enough in this dimension.
    pub fn test_tan(&self, p: Pt) -> bool {
        let dim = self.dim();
        p.a(dim).abs() > p.o(dim).abs() * BEND_TANGENT
    }

    /// `flatQuo`: how flat the run from `p1` to `p2` is, 1 for exactly flat in this dimension
    /// (in the other one with `do_oppo`), 0 for not usefully flat.
    pub fn flat_quo(&self, p1: Pt, p2: Pt, do_oppo: bool) -> f64 {
        let dim = self.dim();
        let mut d = (p1 - p2).abs();
        if do_oppo {
            d = Pt::new(d.y, d.x);
        }
        let (da, d_o) = (d.a(dim), d.o(dim));
        if feq(d_o, 0.0) {
            return 1.0;
        }
        if feq(da, 0.0) {
            return 0.0;
        }
        let q = (d_o * d_o) / (THETA * da);
        if q < 0.25 {
            interpolate(q, 1.0, 0.0, 0.841, 0.25)
        } else if q < 0.5 {
            interpolate(q, 0.841, 0.25, 0.707, 0.5)
        } else if q < 1.0 {
            interpolate(q, 0.707, 0.5, 0.5, 1.0)
        } else if q < 2.0 {
            interpolate(q, 0.5, 1.0, 0.25, 2.0)
        } else if q < 4.0 {
            interpolate(q, 0.25, 2.0, 0.0, 4.0)
        } else {
            0.0
        }
    }

    /// `testBend`: the turn from `p0`–`p1` to `p1`–`p2` is at least 45°.
    pub fn test_bend(&self, p0: Pt, p1: Pt, p2: Pt) -> bool {
        let d1 = p1 - p0;
        let d2 = p2 - p1;
        let dp = d1.dot(d2);
        let q = d1.normsq() * d2.normsq();
        if feq(q, 0.0) {
            return false;
        }
        dp * dp / q <= 0.5
    }

    /// `isCCW`: `p0` → `p1` → `p2` turns counter-clockwise in glyph space.
    pub fn is_ccw(&self, p0: Pt, p1: Pt, p2: Pt) -> bool {
        let d0 = p1 - p0;
        let d1 = p2 - p1;
        d0.x * d1.y >= d1.x * d0.y
    }

    /// `relPosition`: some subpath starts above element `c`'s start (below it when `lower`).
    pub fn rel_position(&self, c: PeRef, lower: bool) -> bool {
        let dim = self.dim();
        let g = self.seg_glyph();
        let Some(cs) = g.element(c).map(|el| el.s.a(dim)) else { return false };
        g.subpaths.iter().filter_map(|sp| sp.first()).any(|first| {
            let a = first.s.a(dim);
            if lower { a < cs } else { a > cs }
        })
    }

    /// `doBendsNext`: a short BEND segment at the end of element `c` where the outline turns
    /// there, in both directions in some cases.
    pub fn do_bends_next(&mut self, c: PeRef) {
        let dim = self.dim();
        let g = self.seg_glyph();
        let Some(el) = g.element(c) else { return };
        let (p0, p1) = (el.slope_point(1.0), el.e);
        let Some(p2) = g.next_slope_point(c) else { return };
        if feq(p0.o(dim), p1.o(dim)) {
            return;
        }
        let osame = diff_sign(p2.o(dim) - p1.o(dim), p1.o(dim) - p0.o(dim));
        let bends = osame
            || (self.test_tan(p1 - p2)
                && (diff_sign(p2.a(dim) - p1.a(dim), p1.a(dim) - p0.a(dim)) || (self.flat_quo(p0, p1, true) > 0.0 && self.test_bend(p0, p1, p2))));
        if !bends {
            return;
        }
        let (a0, a1, a2) = (p0.a(dim), p1.a(dim), p2.a(dim));
        let mut delta = BEND_LENGTH / 2.0;
        let mut doboth = false;
        let forward = (a0 <= a1 && a1 < a2) || (a0 < a1 && a1 <= a2);
        let backward = (a2 < a1 && a1 <= a0) || (a2 <= a1 && a1 < a0);
        if forward {
            // delta keeps its sign.
        } else if backward {
            delta = -delta;
        } else if osame {
            let tst = p0.o(dim) > p1.o(dim);
            if self.is_v() {
                delta = -delta;
            }
            if tst == self.is_ccw(p0, p1, p2) {
                delta = -delta;
            }
        } else {
            if self.is_v() {
                delta = -delta;
            }
            doboth = true;
        }
        let strt = a1 - delta;
        let end = a1 + delta;
        let loc = p1.o(dim);
        self.add_segment(strt, end, loc, Some(c), None, SegType::Bend, "next bend forward", false);
        if doboth {
            self.add_segment(end, strt, loc, Some(c), None, SegType::Bend, "next bend reverse", false);
        }
    }

    /// `doBendsPrev`: a short BEND segment at the start of element `c` where the outline turns
    /// there.
    pub fn do_bends_prev(&mut self, c: PeRef) {
        let dim = self.dim();
        let g = self.seg_glyph();
        let Some(el) = g.element(c) else { return };
        let (p0, p1) = (el.s, el.slope_point(0.0));
        let Some(p2) = g.prev_slope_point(c) else { return };
        if feq(p0.o(dim), p1.o(dim)) {
            return;
        }
        let cs = g.prev_in_subpath(c, false, true, true);
        let osame = diff_sign(p2.o(dim) - p0.o(dim), p0.o(dim) - p1.o(dim));
        let bends = osame
            || (self.test_tan(p0 - p2)
                && (diff_sign(p2.a(dim) - p0.a(dim), p0.a(dim) - p1.a(dim)) || (self.flat_quo(p1, p0, true) > 0.0 && self.test_bend(p2, p0, p1))));
        if !bends {
            return;
        }
        let (a0, a1, a2) = (p0.a(dim), p1.a(dim), p2.a(dim));
        let mut delta = BEND_LENGTH / 2.0;
        let forward = (a2 <= a0 && a0 < a1) || (a2 < a0 && a0 <= a1);
        let backward = (a1 < a0 && a0 <= a2) || (a1 <= a0 && a0 < a2);
        if forward {
            // delta keeps its sign.
        } else if backward {
            delta = -delta;
        } else if osame {
            let tst = p2.o(dim) < p0.o(dim);
            if self.is_v() {
                delta = -delta;
            }
            if tst == self.is_ccw(p2, p0, p1) {
                delta = -delta;
            }
        } else if self.is_v() {
            delta = -delta;
        }
        self.add_segment(a0 - delta, a0 + delta, p0.o(dim), cs, None, SegType::Bend, "prev bend forward", false);
    }

    /// `nodeIsFlat`: the join of `c` with the next element (the previous with `do_prev`) is
    /// flat, by [`OPPO_FLAT_MAX`] and [`FLAT_MIN`].
    pub fn node_is_flat(&self, c: PeRef, do_prev: bool) -> Option<bool> {
        let dim = self.dim();
        let g = self.seg_glyph();
        let el = g.element(c)?;
        let d = if do_prev {
            let Some(sp) = g.prev_slope_point(c) else { return Some(false) };
            (sp - el.cs).abs()
        } else {
            let Some(sp) = g.next_slope_point(c) else { return Some(false) };
            (sp - el.ce).abs()
        };
        Some(d.o(dim) <= OPPO_FLAT_MAX && d.a(dim) >= FLAT_MIN)
    }

    /// `sameDir`: the next element (the previous with `do_prev`) goes on in about the same
    /// direction as `c`; `None` when there is none.
    pub fn same_dir(&self, c: PeRef, do_prev: bool) -> Option<bool> {
        let g = self.seg_glyph();
        let Some(el) = g.element(c) else { return Some(false) };
        let (p0, p1, p2) = if do_prev {
            let p = g.prev_in_subpath(c, true, true, true).and_then(|p| g.element(p))?;
            (el.e, el.s, p.s)
        } else {
            let n = g.next_in_subpath(c, true, true, true).and_then(|n| g.element(n))?;
            (el.s, el.e, n.e)
        };
        if diff_sign(p0.y - p1.y, p1.y - p2.y) || diff_sign(p0.x - p1.x, p1.x - p2.x) {
            return Some(false);
        }
        Some(!self.test_bend(p0, p1, p2))
    }

    /// `extremaSegment`: the extent of a segment through the extreme `extp` (at `extt`) of a
    /// curve, kept within `EXTREMA_DIST` of it. `is_mn` is otfautohint's fourth value of
    /// `farthestExtreme`: true when the extreme lies beyond the chord's upper bound.
    pub fn extrema_segment(&self, pe: PeRef, extp: Pt, extt: f64, is_mn: bool) -> (f64, f64) {
        let dim = self.dim();
        let Some(el) = self.seg_glyph().element(pe) else { return (extp.a(dim), extp.a(dim)) };
        let (a, b, c, d) = el.cubic_parameters();
        let loc = extp.o(dim) + if is_mn { -EXTREMA_DIST } else { EXTREMA_DIST };
        let roots = solve_cubic(a.o(dim), b.o(dim), c.o(dim), d.o(dim) - loc);
        let mut mn: Option<(Pt, f64)> = None;
        let mut mx: Option<(Pt, f64)> = None;
        for t in roots.into_iter().filter(|t| (0.0..=1.0).contains(t)) {
            let p = Pt::new(a.x * t * t * t + b.x * t * t + c.x * t + d.x, a.y * t * t * t + b.y * t * t + c.y * t + d.y);
            let td = (t - extt).abs();
            if t < extt && mn.is_none_or(|(_, best)| td < best) {
                mn = Some((p, td));
            } else if t > extt && mx.is_none_or(|(_, best)| td < best) {
                mx = Some((p, td));
            }
        }
        // Without a solution on one side, that end of the curve lies within EXTREMA_DIST.
        let mn_p = mn.map_or(el.s, |(p, _)| p);
        let mx_p = mx.map_or(el.e, |(p, _)| p);
        (mn_p.a(dim), mx_p.a(dim))
    }

    /// `pickSpot`: where to put a segment between the candidates `p0` and `p1`, from the zones,
    /// the control points `pp0` and `pp1` and the slope points of the neighbours `prv` and
    /// `nxt`.
    #[allow(clippy::too_many_arguments)]
    pub fn pick_spot(&self, p0: Pt, p1: Pt, dist: f64, pp0: Pt, pp1: Pt, prv: Option<Pt>, nxt: Option<Pt>) -> f64 {
        let dim = self.dim();
        let (o0, o1) = (p0.o(dim), p1.o(dim));
        let in_band0 = self.in_band(o0, dist >= 0.0);
        let in_band1 = self.in_band(o1, dist >= 0.0);
        if in_band0 && !in_band1 {
            return o0;
        }
        if in_band1 && !in_band0 {
            return o1;
        }
        if feq(o0, pp0.o(dim)) && fne(o1, pp1.o(dim)) {
            return o0;
        }
        if fne(o0, pp0.o(dim)) && feq(o1, pp1.o(dim)) {
            return o1;
        }
        if prv.is_some_and(|p| feq(o0, p.o(dim))) && nxt.is_none_or(|n| fne(o1, n.o(dim))) {
            return o0;
        }
        // The original tests `prv` before reading `nxt` here, as written.
        if prv.is_none_or(|p| fne(o0, p.o(dim))) && prv.is_some() && nxt.is_some_and(|n| feq(o1, n.o(dim))) {
            return o1;
        }
        if in_band0 && in_band1 {
            let (upper, lower) = if o0 < o1 { (o1, o0) } else { (o0, o1) };
            return if dist < 0.0 { upper } else { lower };
        }
        let d0 = (pp0.a(dim) - p0.a(dim)).abs();
        let d1 = (pp1.a(dim) - p1.a(dim)).abs();
        if d0 > d1 {
            return o0;
        }
        if d1 > d0 {
            return o1;
        }
        if let (Some(p), Some(n)) = (prv, nxt)
            && feq(o0, p.o(dim))
            && feq(o1, n.o(dim))
        {
            return if (p0.a(dim) - p.a(dim)).abs() > (p1.a(dim) - n.a(dim)).abs() { o0 } else { o1 };
        }
        (o0 + o1) / 2.0
    }

    /// `cpDirection`: the turn of `p0` → `p1` → `p2`, −1, 0 or 1.
    pub fn cp_direction(&self, p0: Pt, p1: Pt, p2: Pt) -> i8 {
        let det = p0.x * (p1.y - p2.y) + p1.x * (p2.y - p0.y) + p2.x * (p0.y - p1.y);
        if det > 0.0 {
            1
        } else if det < 0.0 {
            -1
        } else {
            0
        }
    }

    /// `prepForSegs`: splits curves at inflections where the control polygon turns both ways.
    pub fn prep_for_segs(&mut self) {
        let refs: Vec<PeRef> = self.seg_glyph().elements().collect();
        for c in refs {
            let Some(el) = self.seg_glyph().element(c) else { continue };
            if el.is_line {
                continue;
            }
            let (s, cs, ce, e) = (el.s, el.cs, el.ce, el.e);
            if self.cp_direction(s, cs, ce) != self.cp_direction(cs, ce, e)
                && let Some(el) = self.seg_glyph_mut().element_mut(c)
            {
                el.split_at_inflections_for_segs();
            }
        }
    }

    /// `genSegs`: every segment of the glyph. *Shared*.
    pub fn gen_segs(&mut self) {
        if !self.handle_overlap() {
            if matches!(self.st.hs.overlap_removed, OverlapState::Removed(_)) {
                self.st.hs.overlap_removed = OverlapState::Unchecked;
            }
        } else if matches!(self.st.hs.overlap_removed, OverlapState::Unchecked) {
            self.st.hs.overlap_removed = match self.glyphs.without_overlap.first() {
                Some(OverlapState::Removed(g)) => OverlapState::Removed(g.clone()),
                _ => OverlapState::NoOverlap,
            };
        }

        self.prep_for_segs();
        self.st.bonus = 0.0;
        // Each element is visited once, or once per part when split; the bound only guards the
        // walk against a cycle.
        let bound = self.seg_glyph().subpaths.iter().flatten().map(|pe| 1 + part_count(pe)).sum::<usize>();
        let mut c = self.seg_glyph().first(true);
        let mut visited = 0usize;
        while let Some(cur) = c {
            if visited >= bound {
                break;
            }
            visited += 1;
            self.gen_segs_for_path_element(cur);
            c = self.seg_glyph().next(cur, true);
        }

        let raw = self.st.hs.increasing_segs.len().max(self.st.hs.decreasing_segs.len());
        if raw > self.st.options.segment_limit().saturating_mul(RAW_SEGMENT_FACTOR) {
            return;
        }
        self.st.hs.compact_lists();
        self.st.hs.rem_extra_bends();
        self.st.hs.cleanup();
        self.check_tfm();
    }

    /// `genSegsForPathElement`: the segments of element `c` of [`DimHinter::seg_glyph`]
    /// (a part, when the element was split): flat lines, flat curve ends, flat extremes and
    /// bends, each weighted by length.
    pub fn gen_segs_for_path_element(&mut self, c: PeRef) {
        let dim = self.dim();
        let Some(el) = ElView::of(self.seg_glyph(), c) else { return };
        let (s, e) = (el.s, el.e);
        let prv = self.seg_glyph().prev_in_subpath(c, false, true, true);
        if el.is_start {
            // A glyph whose shape has one contour above (or below) the others gets a bonus on
            // that contour's segments.
            self.st.bonus = 0.0;
            if self.is_special(false) && self.rel_position(c, false) {
                self.st.bonus = SPECIAL_CHAR_BONUS;
            } else if self.is_special(true) && self.rel_position(c, true) {
                self.st.bonus = SPECIAL_CHAR_BONUS;
            }
        }
        if el.is_line && !el.is_tiny {
            let mut q = self.flat_quo(s, e, false);
            if q > 0.0 {
                if feq(s.o(dim), e.o(dim)) {
                    self.add_segment(s.a(dim), e.a(dim), s.o(dim), prv, Some(c), SegType::Line, "flat line", false);
                } else {
                    if q < 0.25 {
                        q = 0.25;
                    }
                    let adist = self.adjust_dist(e.a(dim) - s.a(dim), q) / 2.0;
                    let aavg = (s.a(dim) + e.a(dim)) / 2.0;
                    let (ps, ns) = (self.seg_glyph().prev_slope_point(c), self.seg_glyph().next_slope_point(c));
                    let sp = self.pick_spot(s, e, adist, s, e, ps, ns);
                    self.add_segment(aavg - adist, aavg + adist, sp, prv, Some(c), SegType::Line, "line", false);
                }
            } else {
                self.do_bends_next(c);
                self.do_bends_prev(c);
            }
        } else if !el.is_line {
            if el.flex == Some(FlexPart::Second)
                && let Some(prv) = prv
            {
                // The second curve of a flex pair: a segment for the line between the pair's
                // ends, which a rasterizer draws at small sizes.
                let fl1prv = self.seg_glyph().prev_in_subpath(prv, false, true, true);
                if let Some(prv_s) = self.seg_glyph().element(prv).map(|p| p.s)
                    && self.flat_quo(prv_s, e, false) > 0.0
                {
                    self.add_segment(prv_s.a(dim), e.a(dim), e.o(dim), fl1prv, Some(c), SegType::Line, "flex", false);
                }
            }
            if el.flex != Some(FlexPart::Second) {
                self.curve_start_segment(c, &el, prv);
            }
            if el.flex != Some(FlexPart::First) {
                self.curve_end_segment(c, &el);
            }
            // The original tests `flex is None`, which skips elements whose flex is `False`.
            if el.flex.is_none() && !el.flex_unset {
                self.curve_extrema_segment(c, &el);
            }
        }
    }

    /// The start of a curve (not in the middle of a flex pair): a CURVE segment when the start
    /// is flat, else bends.
    fn curve_start_segment(&mut self, c: PeRef, el: &ElView, prv: Option<PeRef>) {
        let dim = self.dim();
        let (s, cs, ce, e) = (el.s, el.cs, el.ce, el.e);
        let q = self.flat_quo(cs, s, false);
        if q == 0.0 {
            self.do_bends_prev(c);
            return;
        }
        let flat_start = feq(cs.o(dim), s.o(dim))
            || (fne(ce.o(dim), e.o(dim)) && (self.node_is_flat(c, true) == Some(true) || !self.same_dir(c, true).unwrap_or(false)));
        if !flat_start {
            return;
        }
        let q2 = self.flat_quo(ce, s, false);
        let (sa, csa, cea) = (s.a(dim), cs.a(dim), ce.a(dim));
        if q2 > 0.0 && same_sign(csa - sa, cea - sa) && (cea - sa).abs() > (csa - sa).abs() {
            let mut adist = self.adjust_dist(self.cp_to(csa, cea) - sa, q2);
            let end = self.adjust_dist(self.cp_to(sa, csa) - sa, q);
            if end.abs() > adist.abs() {
                adist = end;
            }
            self.add_segment(sa, sa + adist, s.o(dim), prv, Some(c), SegType::Curve, "curve start 1", false);
        } else {
            let adist = self.adjust_dist(self.cp_to(sa, csa) - sa, q);
            self.add_segment(sa, sa + adist, s.o(dim), prv, Some(c), SegType::Curve, "curve start 2", false);
        }
    }

    /// The end of a curve (not in the middle of a flex pair): a CURVE segment when the end is
    /// flat (the first kind treats a nearly flat curve like a nearly flat line), else bends.
    fn curve_end_segment(&mut self, c: PeRef, el: &ElView) {
        let dim = self.dim();
        let (s, cs, ce, e) = (el.s, el.cs, el.ce, el.e);
        let q = self.flat_quo(ce, e, false);
        if q == 0.0 {
            self.do_bends_next(c);
            return;
        }
        let flat_end = feq(ce.o(dim), e.o(dim))
            || (fne(cs.o(dim), s.o(dim)) && (self.node_is_flat(c, false) == Some(true) || !self.same_dir(c, false).unwrap_or(false)));
        if !flat_end {
            return;
        }
        let (sa, csa, cea, ea) = (s.a(dim), cs.a(dim), ce.a(dim), e.a(dim));
        let mut adist = self.adjust_dist(ea - self.cp_from(cea, ea), q);
        let q2 = self.flat_quo(s, e, false);
        let ad2 = if q2 > 0.0 { self.adjust_dist(ea - sa, q2) } else { 0.0 };
        if q2 > 0.0 && ad2.abs() > adist.abs() {
            let (mut ccs, mut cce) = (cs, ce);
            if feq(s.o(dim), cs.o(dim)) && feq(cs.o(dim), ce.o(dim)) && feq(ce.o(dim), e.o(dim)) {
                let opts = &self.st.options;
                if opts.allow_changes && AUTO_LINEAR_CURVE_FIX && !self.st.is_multi {
                    if let Some(pe) = self.seg_glyph_mut().element_mut(c) {
                        pe.convert_to_line();
                    }
                    (ccs, cce) = (s, e);
                }
            }
            let adist = ad2 / 2.0;
            let aavg = (sa + ea) / 2.0;
            let (ps, ns) = (self.seg_glyph().prev_slope_point(c), self.seg_glyph().next_slope_point(c));
            let sp = self.pick_spot(s, e, adist, ccs, cce, ps, ns);
            self.add_segment(aavg - adist, aavg + adist, sp, Some(c), None, SegType::Curve, "curve end 1", false);
        } else {
            let q2 = self.flat_quo(cs, e, false);
            if q2 > 0.0 && same_sign(csa - ea, cea - ea) && (cea - ea).abs() < (csa - ea).abs() {
                let aend = self.adjust_dist(ea - self.cp_from(csa, cea), q2);
                if aend.abs() > adist.abs() {
                    adist = aend;
                }
                self.add_segment(ea - adist, ea, e.o(dim), Some(c), None, SegType::Curve, "curve end 2", false);
            } else {
                self.add_segment(ea - adist, ea, e.o(dim), Some(c), None, SegType::Curve, "curve end 3", false);
            }
        }
    }

    /// A curve outside flex pairs whose extreme, more than 2 units beyond its chord, is flat in
    /// this dimension: a CURVE segment through the extreme.
    fn curve_extrema_segment(&mut self, c: PeRef, el: &ElView) {
        let dim = self.dim();
        let do_y = !self.is_v();
        let Some(bounds) = self.seg_glyph_mut().element_mut(c).map(|pe| pe.get_bounds().clone()) else { return };
        let (d, extp, extt, is_max) = bounds.farthest_extreme(do_y);
        if d <= 2.0 {
            return;
        }
        let (frst, lst) = self.extrema_segment(c, extp, extt, is_max);
        let aavg = (frst + lst) / 2.0;
        let mut adist = if feq(frst, lst) { (el.e.a(dim) - el.s.a(dim)) / 10.0 } else { (lst - frst) / 2.0 };
        if adist.abs() < BEND_LENGTH {
            // `math.copysign(adist, BendLength)` in the original.
            adist = adist.abs();
        }
        self.add_segment(aavg - adist, aavg + adist, extp.o(dim), Some(c), None, SegType::Curve, "curve extrema", true);
    }

    /// `limitSegs`: leaves a direction with too many segments unhinted, with a warning.
    pub fn limit_segs(&mut self) {
        let maxsegs = self.st.hs.increasing_segs.len().max(self.st.hs.decreasing_segs.len());
        if maxsegs > self.st.options.segment_limit() {
            let desc = self.a_desc();
            self.st.warnings.push(format!("{}: calculated {maxsegs} segments, skipping {desc} stem testing", self.st.name));
            self.st.hs.delete_segments();
        }
    }
}

/// The number of parts an element was split into for segment generation.
fn part_count(pe: &crate::glyph::PathElement) -> usize {
    match &pe.segment_sub {
        Some(SegmentSub::Parts(parts)) => parts.len(),
        _ => 0,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Arc;

    use fontcraft_geom::BezPath;

    use super::*;
    use crate::glyph::Dim;
    use crate::hinter::GlyphHinter;
    use crate::{FdDict, Options};

    /// A contour as the oracle script `tests/oracle/segments.py` writes it: a start point and
    /// steps of one point (a line) or three (a curve), closed.
    pub(crate) fn contour(start: (f64, f64), steps: &[&[(f64, f64)]]) -> BezPath {
        let mut p = BezPath::new();
        p.move_to(start);
        for st in steps {
            match st {
                [a] => p.line_to(*a),
                [a, b, c] => p.curve_to(*a, *b, *c),
                _ => {}
            }
        }
        p.close_path();
        p
    }

    pub(crate) fn hinter(name: &str, contours: &[BezPath], options: Options) -> GlyphHinter {
        let mut gh = GlyphHinter::new(Arc::new(options));
        gh.glyphs.main = GlyphData::from_contours(name, contours, true).unwrap();
        for dim in [Dim::H, Dim::V] {
            gh.dim(dim).st.set_glyph(vec![Arc::new(FdDict::default())], None, 1, name);
        }
        gh
    }

    pub(crate) fn no_overlap() -> Options {
        Options { overlap_forcing: Some(false), ..Options::default() }
    }

    type Seg = (f64, f64, f64, u8);

    /// Generates the segments in one dimension: (increasing, decreasing), each segment as
    /// (location, min, max, type).
    pub(crate) fn segments(gh: &mut GlyphHinter, dim: Dim) -> (Vec<Seg>, Vec<Seg>) {
        let mut d = gh.dim(dim);
        d.start_hint();
        d.gen_segs();
        let hs = &d.st.hs;
        let list =
            |ids: &[crate::state::SegId]| ids.iter().filter_map(|id| hs.seg(*id)).map(|s| (s.loc, s.min, s.max, s.typ as u8)).collect::<Vec<_>>();
        (list(&hs.increasing_segs), list(&hs.decreasing_segs))
    }

    #[track_caller]
    pub(crate) fn assert_segs(got: &[Seg], want: &[Seg]) {
        assert_eq!(got.len(), want.len(), "got {got:?}, want {want:?}");
        for (g, w) in got.iter().zip(want) {
            let close = (g.0 - w.0).abs() < 1e-6 && (g.1 - w.1).abs() < 1e-6 && (g.2 - w.2).abs() < 1e-6 && g.3 == w.3;
            assert!(close, "got {got:?}, want {want:?}");
        }
    }

    fn stem() -> Vec<BezPath> {
        vec![contour((100.0, 0.0), &[&[(180.0, 0.0)], &[(180.0, 700.0)], &[(100.0, 700.0)]])]
    }

    fn bowl() -> Vec<BezPath> {
        vec![
            contour(
                (250.0, -10.0),
                &[
                    &[(390.0, -10.0), (480.0, 100.0), (480.0, 250.0)],
                    &[(480.0, 400.0), (390.0, 510.0), (250.0, 510.0)],
                    &[(110.0, 510.0), (20.0, 400.0), (20.0, 250.0)],
                    &[(20.0, 100.0), (110.0, -10.0), (250.0, -10.0)],
                ],
            ),
            contour(
                (250.0, 60.0),
                &[
                    &[(160.0, 60.0), (105.0, 140.0), (105.0, 250.0)],
                    &[(105.0, 360.0), (160.0, 440.0), (250.0, 440.0)],
                    &[(340.0, 440.0), (395.0, 360.0), (395.0, 250.0)],
                    &[(395.0, 140.0), (340.0, 60.0), (250.0, 60.0)],
                ],
            ),
        ]
    }

    // Expected values: tests/oracle/segments.py (otfautohint 5.0.1).
    #[test]
    fn a_straight_stem_gives_one_line_segment_per_edge() {
        let mut gh = hinter("stem", &stem(), no_overlap());
        let (inc, dec) = segments(&mut gh, Dim::H);
        assert_segs(&inc, &[(0.0, 99.0, 181.0, 1)]);
        assert_segs(&dec, &[(700.0, 99.0, 181.0, 1)]);
        let (inc, dec) = segments(&mut gh, Dim::V);
        assert_segs(&inc, &[(180.0, -1.0, 701.0, 1)]);
        assert_segs(&dec, &[(100.0, -1.0, 701.0, 1)]);
    }

    #[test]
    fn a_bowl_gives_curve_segments_at_its_extremes() {
        let mut gh = hinter("bowl", &bowl(), no_overlap());
        let (inc, dec) = segments(&mut gh, Dim::H);
        assert_segs(&inc, &[(-10.0, 194.0, 306.0, 3), (440.0, 214.0, 286.0, 3)]);
        assert_segs(&dec, &[(60.0, 214.0, 286.0, 3), (510.0, 194.0, 306.0, 3)]);
        let (inc, dec) = segments(&mut gh, Dim::V);
        assert_segs(&inc, &[(105.0, 206.0, 294.0, 3), (480.0, 190.0, 310.0, 3)]);
        assert_segs(&dec, &[(20.0, 190.0, 310.0, 3), (395.0, 206.0, 294.0, 3)]);
    }

    #[test]
    fn curves_split_at_inflections_give_segments_for_their_parts() {
        // The top of this bar is two S-curves; each is split at its inflection, and the parts
        // carry no flex mark.
        let bar = vec![contour(
            (0.0, 0.0),
            &[&[(600.0, 0.0)], &[(600.0, 100.0)], &[(500.0, 100.0), (400.0, 95.0), (300.0, 95.0)], &[(200.0, 95.0), (100.0, 100.0), (0.0, 100.0)]],
        )];
        let mut gh = hinter("flexbar", &bar, no_overlap());
        let (inc, dec) = segments(&mut gh, Dim::H);
        assert_segs(&inc, &[(0.0, -1.0, 601.0, 1)]);
        assert_segs(
            &dec,
            &[
                (95.0, 305.23026315789474, 444.76973684210526, 3),
                (95.0, 231.83059210526318, 300.0, 3),
                (100.0, 531.8305921052631, 601.0, 3),
                (100.0, 5.23026315789474, 144.76973684210526, 3),
                (100.0, -1.0, 1.0, 2),
            ],
        );
    }

    #[test]
    fn too_many_segments_leave_the_dimension_unhinted_with_a_warning() {
        // 120 separate squares: 120 segments in each direction, over the limit of 100.
        let squares: Vec<BezPath> = (0..120)
            .map(|i| {
                let x = f64::from(i) * 50.0;
                contour((x, 0.0), &[&[(x + 20.0, 0.0)], &[(x + 20.0, 20.0)], &[(x, 20.0)]])
            })
            .collect();
        let mut gh = hinter("dots", &squares, no_overlap());
        let mut d = gh.dim(Dim::V);
        d.start_hint();
        d.gen_segs();
        d.limit_segs();
        assert_eq!(d.st.warnings.len(), 1, "{:?}", d.st.warnings);
        assert!(d.st.hs.increasing_segs.is_empty() && d.st.hs.decreasing_segs.is_empty());
    }

    #[test]
    fn flat_quo_follows_the_original_breakpoints() {
        let gh = &mut hinter("x", &[], no_overlap());
        let d = gh.dim(Dim::H);
        assert_eq!(d.flat_quo(Pt::new(0.0, 0.0), Pt::new(100.0, 0.0), false), 1.0);
        assert_eq!(d.flat_quo(Pt::new(0.0, 0.0), Pt::new(0.0, 100.0), false), 0.0);
        // d.o² / (0.38 · d.a) = 4 / 38 < 0.25.
        let q = 4.0 / 38.0;
        assert!((d.flat_quo(Pt::new(0.0, 0.0), Pt::new(100.0, 2.0), false) - (1.0 + q * (0.841 - 1.0) / 0.25)).abs() < 1e-12);
        assert_eq!(solve_cubic(1.0, -4.5, 6.75, -3.375), vec![1.5, 1.5, 1.5]);
        assert_eq!(solve_cubic(-10.0, -9.0, 48.0, -29.0), vec![-2.9, 1.0, 1.0]);
    }
}
