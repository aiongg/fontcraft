//! Bounds and curve maths of path elements.
//!
//! Ported from otfautohint's `glyphData.py` (`boundsState`, `pathBoundsState` and the curve
//! methods of `pathElement`; AFDKO 5.0.1, Apache-2.0, Copyright 2021 Adobe; see `NOTICE`), which
//! use fontTools' `bezierTools` (MIT): the quadratic and cubic solvers, `calcCubicParameters`,
//! `splitCubicAtT`, `segmentPointAtT`, `approximateCubicArcLength`. Package M10.8 fills the bodies
//! marked `// M10.8:`.

use crate::glyph::{PathElement, PeRef, Pt};

/// `[[min x, min y], [max x, max y]]`, as the original stores bounds.
pub type Rect = [[f64; 2]; 2];

/// The bounds of one path element and the points and curve parameters that define each side
/// (`boundsState`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoundsState {
    /// Bounds of the chord from start to end point (`lb`).
    pub linear: Rect,
    /// The element's true bounds, curve extremes included.
    pub bounds: Rect,
    /// The point that sets each side: `extpts[min/max][x/y]`.
    pub extpts: [[Pt; 2]; 2],
    /// The curve parameter of each of those points.
    pub tmap: [[f64; 2]; 2],
}

impl BoundsState {
    /// Bounds of `pe` (`boundsState.__init__`).
    pub fn new(pe: &PathElement) -> Self {
        // M10.8: linearBounds, then calcCurveBounds when a control point leaves them.
        let _ = pe;
        Self::default()
    }

    /// Adds `p` at parameter `t` as a possible extreme (`mergePt`).
    pub fn merge_pt(&mut self, b: &mut Rect, p: Pt, t: f64, record: bool) {
        // M10.8: mergePt.
        let _ = (b, p, t, record);
    }

    /// The side farthest from the chord: (location, point, t) (`farthestExtreme`).
    pub fn farthest_extreme(&self, do_y: bool) -> (f64, Pt, f64) {
        // M10.8: farthestExtreme.
        let _ = do_y;
        (0.0, Pt::default(), 0.0)
    }

    /// The bounds overlap `other`'s, grown by `margin` (`intersects`).
    pub fn intersects(&self, other: &BoundsState, margin: f64) -> bool {
        // M10.8: intersects.
        let _ = (other, margin);
        false
    }
}

/// The bounds of a glyph or subpath and the elements that set each side (`pathBoundsState`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PathBoundsState {
    pub bounds: Rect,
    /// `extpes[min/max][x/y]`.
    pub extpes: [[PeRef; 2]; 2],
}

impl PathBoundsState {
    /// The bounds of one element (`pathBoundsState.__init__`).
    pub fn new(pe: PeRef, bounds: &BoundsState) -> Self {
        Self { bounds: bounds.bounds, extpes: [[pe, pe], [pe, pe]] }
    }

    /// Grows to include `other` (`merge`).
    pub fn merge(&mut self, other: &PathBoundsState) {
        // M10.8: merge.
        let _ = other;
    }

    /// These bounds lie inside `other`'s (`within`).
    pub fn within(&self, other: &PathBoundsState) -> bool {
        // M10.8: within.
        let _ = other;
        false
    }
}

impl PathElement {
    /// The polynomial coefficients `(a, b, c, d)` of the cubic (`cubicParameters`). *Shared.*
    pub fn cubic_parameters(&self) -> (Pt, Pt, Pt, Pt) {
        // M10.8: fontTools calcCubicParameters.
        (Pt::default(), Pt::default(), Pt::default(), self.s)
    }

    /// Whether `p` lies on the element within `factor`; with `want_t`, also the parameter
    /// (`containsPoint`). *Shared.*
    pub fn contains_point(&self, p: Pt, factor: f64, want_t: bool) -> (bool, Option<f64>) {
        // M10.8: containsPoint.
        let _ = (p, factor, want_t);
        (false, None)
    }

    /// A point that gives the direction of the element at `t` (0 or 1) (`slopePoint`). *Shared.*
    pub fn slope_point(&self, t: f64) -> Pt {
        // M10.8: slopePoint.
        let _ = t;
        self.e
    }

    /// Splits the element at `t`, keeping the first part and returning the second (`splitAt`).
    pub fn split_at(&mut self, t: f64) -> PathElement {
        // M10.8: splitAt.
        let _ = t;
        self.clone()
    }

    /// The point at parameter `t` (`atT`). *Shared.*
    pub fn at_t(&self, t: f64) -> Pt {
        // M10.8: atT.
        let _ = t;
        self.s
    }

    /// The element as a fontTools segment: its points after the start (`fonttoolsSegment`).
    pub fn fonttools_segment(&self) -> Vec<Pt> {
        if self.is_line { vec![self.s, self.e] } else { vec![self.s, self.cs, self.ce, self.e] }
    }
}
