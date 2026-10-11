//! Cubic to quadratic conversion across the sources of a variable glyph
//! (`compile.cu2quAcrossMasters`, M9.5).
//!
//! A variable TrueType glyph needs the same points in every source, so one cubic segment is
//! converted in all its sources at once and every source gets the same number of quadratic
//! pieces: the smallest number that keeps each source within the tolerance. This is fontTools'
//! `cu2qu.curves_to_quadratic` (MIT), which the reference compilers use; kurbo's
//! [`kurbo::cubics_to_quadratic_splines`] is a port of the same algorithm.
//!
//! Contract:
//! - The input curves are the same segment of one glyph in each source, in source order.
//! - The output has one entry per input curve, each with the same number of quadratic segments;
//!   consecutive segments share their end points, so the on-curve points between them can be
//!   left implied when the glyph is written.
//! - `None` when no split into at most [`MAX_QUADRATIC_PIECES`] pieces meets the tolerance, when
//!   there are no curves or more than [`MAX_CURVES`], or when a coordinate or the tolerance is
//!   not finite.

use kurbo::{CubicBez, QuadBez};

/// Most quadratic pieces one cubic segment is split into (fontTools' `MAX_N`).
pub const MAX_QUADRATIC_PIECES: usize = 100;
/// Most curves converted together: one per source, so at most the sources of one glyph.
pub const MAX_CURVES: usize = crate::variation::MAX_MASTERS;

/// The quadratic approximations of `curves` (one per source), with the same number of pieces
/// each, every piece within `tolerance` font units of its cubic.
pub fn curves_to_quadratic(curves: &[CubicBez], tolerance: f64) -> Option<Vec<Vec<QuadBez>>> {
    // M9.5: check kurbo's port against fontTools' (the all-quadratic shortcut, the error rule)
    // and port where they differ.
    let _ = (curves, tolerance);
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_curves_give_no_conversion() {
        assert!(curves_to_quadratic(&[], 1.0).is_none());
    }
}
