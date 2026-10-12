//! Flex: pairs of shallow curves that a rasterizer may flatten at small sizes.
//!
//! Ported from otfautohint's `hinter.py` (`linearFlexOK`, `addFlex`, `tryFlex`, `markFlex`;
//! AFDKO 5.0.1, Apache-2.0, Copyright 2021 Adobe; see `NOTICE`). Rows: `hinting.ps.flex.*`.
//!
//! The pair is measured in this dimension's aligned coordinate ([`crate::glyph::Pt::a`]): the
//! vertical hinter ([`crate::Dim::V`], `a` = y) finds pairs that run horizontally with a shallow
//! dip in y, the horizontal hinter pairs that run vertically. Both mark the same elements.

use crate::glyph::{FlexPart, PeRef, TinySkip, feq, fne};
use crate::hinter::{DimHinter, FLEX_CAND, FLEX_LENGTH_RATIO_CUTOFF, FLEX_STRICT, GlyphPe, MAX_FLEX};
use crate::segments::diff_sign;

impl DimHinter<'_> {
    /// `linearFlexOK`: a pair whose second curve is straight in this dimension gets no flex.
    pub fn linear_flex_ok(&self) -> bool {
        false
    }

    /// Finds and marks flex in every master (`addFlex`). Existing flex is cleared first when
    /// `force`, or when only some masters have it; else a glyph with flex keeps it. `inited`:
    /// the other dimension has run already, so existing flex is the new one. *Shared* (M10.12).
    pub fn add_flex(&mut self, force: bool, inited: bool) {
        if !inited {
            let has: Vec<bool> = (0..self.glyphs.len()).map(|i| self.glyphs.get(i).is_some_and(|g| g.flex_count != 0)).collect();
            // The original tests `any` and then `all` on one generator, so `all` sees only the
            // masters after the first one with flex.
            if let Some(first) = has.iter().position(|&h| h) {
                let rest_all = has.get(first + 1..).is_none_or(|rest| rest.iter().all(|&h| h));
                if force || !rest_all {
                    for i in 0..self.glyphs.len() {
                        if let Some(g) = self.glyphs.get_mut(i) {
                            g.clear_flex();
                        }
                    }
                } else {
                    return;
                }
            }
        }
        // Marking flex changes no element's end points, so the tables stay valid in the loop.
        let skips: Vec<TinySkip> = (0..self.glyphs.len()).map(|i| self.glyphs.get(i).map(|g| g.tiny_skip(false)).unwrap_or_default()).collect();
        for cl in self.masters_elements(None) {
            if cl.iter().all(|x| skips.get(x.glyph).is_some_and(|skip| self.try_flex(x.glyph, x.pe, skip))) {
                self.mark_flex(&cl);
            }
        }
    }

    /// Tests the curve pair starting at element `c` of master `glyph` (`tryFlex`): two curves
    /// whose outer ends line up, at most [`MAX_FLEX`] deep, at least three times as long as deep,
    /// of similar length, bending the same way as the outline around them. `skip` is
    /// `tiny_skip(false)` of that master's glyph.
    pub fn try_flex(&self, glyph: usize, c: PeRef, skip: &TinySkip) -> bool {
        let dim = self.dim();
        let Some(gl) = self.glyphs.get(glyph) else { return false };
        let Some(ce) = gl.element(c) else { return false };
        // Not a curve, already flexed, or too deep.
        if ce.is_line || ce.flex.is_some() || ce.s.a_dist(ce.e, dim) > MAX_FLEX {
            return false;
        }
        let Some(nr) = gl.next_non_tiny(skip, c) else { return false };
        let Some(n) = gl.element(nr) else { return false };
        if n.is_line || n.flex.is_some() {
            return false;
        }

        let d = (ce.s - n.e).abs();
        let (da, d_o) = (d.a(dim), d.o(dim));
        // Ends too far apart in depth to report a near miss, or the pair too short.
        if da > FLEX_CAND || d_o < MAX_FLEX {
            return false;
        }
        // At least 3 times as long as deep.
        if d_o < 3.0 * ce.s.avg(n.e).a_dist(ce.e, dim) {
            return false;
        }
        // Both ends on the same side of the middle point.
        if diff_sign(ce.e.a(dim) - ce.s.a(dim), ce.e.a(dim) - n.e.a(dim)) {
            return false;
        }
        let d1sq = ce.s.distsq(ce.e);
        let d2sq = ce.e.distsq(n.e);
        let (num, den) = if d1sq > d2sq { (d2sq, d1sq) } else { (d1sq, d2sq) };
        if den == 0.0 || num / den < FLEX_LENGTH_RATIO_CUTOFF {
            return false;
        }

        if FLEX_STRICT {
            // The element after the pair must not bend back towards the middle point…
            let Some(nn) = gl.next_non_tiny(skip, nr).and_then(|r| gl.element(r)) else { return false };
            if diff_sign(nn.e.a(dim) - n.e.a(dim), ce.e.a(dim) - n.e.a(dim)) {
                return false;
            }
            // …nor the element before it.
            let Some(p) = gl.prev_non_tiny(skip, c).and_then(|r| gl.element(r)) else { return false };
            if diff_sign(p.s.a(dim) - ce.s.a(dim), ce.e.a(dim) - ce.s.a(dim)) {
                return false;
            }
            // The pair is convex.
            let convex = if self.is_v() { (ce.s.x > ce.e.x) == (ce.e.y < ce.s.y) } else { (ce.s.y > n.e.y) == (ce.s.x < ce.e.x) };
            if !convex {
                return false;
            }
        }

        // A tiny element between the curves, or the pair crossing the subpath's start, rules
        // flex out.
        let real_n = gl.next_in_subpath(c, false, true, false);
        if real_n != Some(nr) {
            return false;
        }
        if n.is_close || n.is_start() {
            return false;
        }
        if fne(ce.s.a(dim), n.e.a(dim)) {
            return false;
        }
        if feq(n.s.a(dim), n.cs.a(dim)) && feq(n.cs.a(dim), n.ce.a(dim)) && feq(n.ce.a(dim), n.e.a(dim)) && !self.linear_flex_ok() {
            return false;
        }
        true
    }

    /// Marks the pair in every master (`markFlex`).
    pub fn mark_flex(&mut self, cl: &[GlyphPe]) {
        for x in cl {
            let Some(gl) = self.glyphs.get_mut(x.glyph) else { continue };
            let n = gl.next_in_subpath(x.pe, false, true, false);
            if let Some(el) = gl.element_mut(x.pe) {
                el.flex = Some(FlexPart::First);
            }
            if let Some(el) = n.and_then(|n| gl.element_mut(n)) {
                el.flex = Some(FlexPart::Second);
            }
            gl.flex_count += 1;
            gl.changed = true;
        }
        self.st.has_flex = true;
    }
}

#[cfg(test)]
mod tests {
    use fontcraft_geom::BezPath;

    use crate::glyph::{Dim, FlexPart, PeRef};
    use crate::segments::tests::{assert_segs, contour, hinter, no_overlap, segments};

    fn flexdip() -> Vec<BezPath> {
        vec![contour(
            (0.0, 0.0),
            &[&[(600.0, 0.0)], &[(600.0, 100.0)], &[(500.0, 95.0), (400.0, 95.0), (300.0, 95.0)], &[(200.0, 95.0), (100.0, 95.0), (0.0, 100.0)]],
        )]
    }

    // Expected values: tests/oracle/segments.py (otfautohint 5.0.1).
    #[test]
    fn a_shallow_dip_gets_flex_and_a_segment_along_its_chord() {
        let mut gh = hinter("flexdip", &flexdip(), no_overlap());
        gh.dim(Dim::H).add_flex(true, false);
        gh.dim(Dim::V).add_flex(true, true);
        let main = &gh.glyphs.main;
        assert_eq!(main.element(PeRef::new(0, 2)).and_then(|e| e.flex), Some(FlexPart::First));
        assert_eq!(main.element(PeRef::new(0, 3)).and_then(|e| e.flex), Some(FlexPart::Second));
        assert_eq!(main.flex_count, 1);
        assert!(gh.v.has_flex && !gh.h.has_flex);

        let (inc, dec) = segments(&mut gh, Dim::H);
        assert_segs(&inc, &[(0.0, -1.0, 601.0, 1)]);
        assert_segs(&dec, &[(100.0, -1.0, 601.0, 1)]);
        let (inc, dec) = segments(&mut gh, Dim::V);
        assert_segs(&inc, &[(600.0, -1.0, 100.0, 1)]);
        assert_segs(&dec, &[(0.0, -1.0, 101.0, 1)]);
    }

    #[test]
    fn a_dip_that_bulges_towards_the_next_edge_gets_no_flex() {
        // The same bar with the dip turned up: the closing line bends back towards it.
        let bar = vec![contour(
            (0.0, 0.0),
            &[&[(600.0, 0.0)], &[(600.0, 100.0)], &[(500.0, 105.0), (400.0, 105.0), (300.0, 105.0)], &[(200.0, 105.0), (100.0, 105.0), (0.0, 100.0)]],
        )];
        let mut gh = hinter("bump", &bar, no_overlap());
        gh.dim(Dim::H).add_flex(true, false);
        gh.dim(Dim::V).add_flex(true, true);
        assert_eq!(gh.glyphs.main.flex_count, 0);
    }
}
