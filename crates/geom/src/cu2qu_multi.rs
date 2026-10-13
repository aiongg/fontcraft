//! Cubic to quadratic conversion across the sources of a variable glyph
//! (`compile.cu2quAcrossMasters`, M9.5).
//!
//! A variable TrueType glyph needs the same points in every source, so one cubic segment is
//! converted in all its sources at once and every source gets the same number of quadratic
//! pieces: the smallest number that keeps each source within the tolerance. This is fontTools'
//! `cu2qu.curves_to_quadratic` (MIT), which the reference compilers use, through kurbo's port
//! [`kurbo::cubics_to_quadratic_splines`], the function fontc calls. Both try 1 to 100 pieces in
//! turn and keep the first count at which every source is within the tolerance; a cubic that is
//! already a quadratic converts to one piece in both.
//!
//! Contract:
//! - The input curves are the same segment of one glyph in each source, in source order.
//! - The output has one entry per input curve, each with the same number of quadratic segments;
//!   consecutive segments share their end points, so the on-curve points between them can be
//!   left implied when the glyph is written.
//! - `None` when no split into at most [`MAX_QUADRATIC_PIECES`] pieces meets the tolerance, when
//!   there are no curves or more than [`MAX_CURVES`], or when a coordinate or the tolerance is
//!   not finite.

use kurbo::{CubicBez, QuadBez, cubics_to_quadratic_splines};

/// Most quadratic pieces one cubic segment is split into (fontTools' `MAX_N`).
pub const MAX_QUADRATIC_PIECES: usize = 100;
/// Most curves converted together: one per source, so at most the sources of one glyph.
pub const MAX_CURVES: usize = crate::variation::MAX_MASTERS;

/// The quadratic approximations of `curves` (one per source), with the same number of pieces
/// each, every piece within `tolerance` font units of its cubic.
pub fn curves_to_quadratic(curves: &[CubicBez], tolerance: f64) -> Option<Vec<Vec<QuadBez>>> {
    if curves.is_empty() || curves.len() > MAX_CURVES || !(tolerance.is_finite() && tolerance > 0.0) {
        return None;
    }
    let finite = |c: &CubicBez| [c.p0, c.p1, c.p2, c.p3].iter().all(|p| p.x.is_finite() && p.y.is_finite());
    if !curves.iter().all(finite) {
        return None;
    }
    let splines = cubics_to_quadratic_splines(curves, tolerance)?;
    let out: Vec<Vec<QuadBez>> = splines.iter().map(|s| s.to_quads().collect()).collect();
    let pieces = out.first().map(Vec::len)?;
    (pieces <= MAX_QUADRATIC_PIECES && out.len() == curves.len() && out.iter().all(|q| q.len() == pieces)).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_source_gets_the_same_number_of_pieces() {
        let flat = CubicBez::new((0.0, 0.0), (10.0, 0.0), (20.0, 0.0), (30.0, 0.0));
        let round = CubicBez::new((0.0, 0.0), (0.0, 550.0), (450.0, 1000.0), (1000.0, 1000.0));
        let out = curves_to_quadratic(&[flat, round], 1.0).unwrap();
        assert_eq!(out.len(), 2);
        assert!(out[0].len() > 1 && out[0].len() == out[1].len());
        assert_eq!(out[1].last().unwrap().p2, round.p3);
        assert!(curves_to_quadratic(&[round], f64::NAN).is_none());
    }

    #[test]
    fn no_curves_give_no_conversion() {
        assert!(curves_to_quadratic(&[], 1.0).is_none());
    }
}
