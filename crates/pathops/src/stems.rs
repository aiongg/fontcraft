//! Stem detection (M13.24 fills it; M13.19, M13.34, M13.35 and M13.36 use it).
//!
//! A stem is a pair of roughly parallel, opposite-facing straight or nearly straight stretches of
//! outline with ink between them: the vertical bars of `H`, the horizontal bar of `e`, the
//! diagonals of `V`. For curves the detector also reports the thickest horizontal and vertical
//! cross-sections (the sides of `o`), flagged as round.

use fontcraft_geom::{BezPath, Line};

/// Most stems reported for one outline.
pub const MAX_STEMS: usize = 10_000;

/// Which way a stem runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StemDirection {
    /// Runs up and down: its width is measured horizontally.
    Vertical,
    /// Runs left and right: its width is measured vertically.
    Horizontal,
    /// Any other angle: its width is measured at right angles to it.
    Diagonal,
}

/// One stem.
#[derive(Clone, Debug, PartialEq)]
pub struct Stem {
    pub direction: StemDirection,
    /// The two edges, as the parts of the outline that face each other; `first` is the left (or
    /// lower) edge.
    pub first: Line,
    pub second: Line,
    /// The distance between the edges, in font units.
    pub width: f64,
    /// Measured across curves (the sides of `o`), not between straight edges.
    pub round: bool,
}

/// What [`detect_stems`] reports.
#[derive(Clone, Debug, PartialEq)]
pub struct StemOptions {
    /// Narrowest and widest stem, in font units.
    pub min_width: f64,
    pub max_width: f64,
    /// Shortest overlap of the two edges, in font units.
    pub min_length: f64,
    /// How far from parallel two edges may be, in degrees.
    pub angle_tolerance: f64,
    /// Report only stems running this way; `None`: every direction.
    pub direction: Option<StemDirection>,
}

impl Default for StemOptions {
    fn default() -> Self {
        Self { min_width: 1.0, max_width: 1_000.0, min_length: 10.0, angle_tolerance: 5.0, direction: None }
    }
}

/// The stems of `outline` (closed contours, resolved), widest overlap first, at most
/// [`MAX_STEMS`].
pub fn detect_stems(outline: &BezPath, options: &StemOptions) -> Vec<Stem> {
    let _ = (outline, options);
    // M13.24.
    Vec::new()
}
