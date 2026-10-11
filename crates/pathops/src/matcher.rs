//! Outline matching: where a pattern outline occurs in another outline, up to translation and,
//! when asked, flips (M13.15 fills it; M13.28 uses it for Find and Replace Outline).
//!
//! Method: a hash of each contour's point structure (node kinds and counts) finds candidates;
//! each candidate is compared exactly after the best translation (and flip), within a tolerance.
//! A closed pattern matches whole contours (Detect Composites: the outline of a base glyph found
//! in a composite's paths). An open pattern matches a run of consecutive nodes inside a contour
//! (Find Outline: a serif found in several glyphs); its end nodes match position only.

use fontcraft_geom::Affine;

use crate::Contour;

/// Most nodes in a pattern.
pub const MAX_PATTERN_NODES: usize = 1_024;
/// Most matches one call returns.
pub const MAX_MATCHES: usize = 100_000;

/// How to match.
#[derive(Clone, Debug, PartialEq)]
pub struct MatchOptions {
    /// Largest distance between a pattern node, moved by the match's transform, and its target
    /// node, in font units.
    pub tolerance: f64,
    /// Also try the pattern flipped horizontally, vertically and both.
    pub flips: bool,
    /// Stop after this many matches (at most [`MAX_MATCHES`]).
    pub max_matches: usize,
}

impl Default for MatchOptions {
    fn default() -> Self {
        Self { tolerance: 0.5, flips: false, max_matches: MAX_MATCHES }
    }
}

/// The target nodes one pattern contour matched: `len` consecutive nodes of contour `contour`
/// starting at `start` (wrapping around a closed contour). A whole-contour match covers every node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MatchedRun {
    pub contour: usize,
    pub start: usize,
    pub len: usize,
}

/// One occurrence of the pattern.
#[derive(Clone, Debug, PartialEq)]
pub struct Match {
    /// One run per pattern contour, in pattern order.
    pub runs: Vec<MatchedRun>,
    /// Maps the pattern onto the target: a translation, with a flip when `flips` found it.
    pub transform: Affine,
    /// The largest node distance after the transform.
    pub error: f64,
}

/// Every occurrence of `pattern` in `target`, matches not overlapping, in target order. A pattern
/// over [`MAX_PATTERN_NODES`] nodes, or empty, finds nothing.
pub fn match_outline(pattern: &[Contour], target: &[Contour], options: &MatchOptions) -> Vec<Match> {
    let _ = (pattern, target, options);
    // M13.15.
    Vec::new()
}
