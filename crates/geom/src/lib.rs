//! FontCraft geometry (L0).
//!
//! A thin layer over [`kurbo`]: the rest of the workspace names geometry types through this crate
//! so the curve library stays one decision in one place. Font-specific geometry lives here too:
//! cubic → quadratic conversion, and in [`segment`] the segments of a node list with their
//! addresses, nearest points, splits, extremes, inflections, bounds and grid rounding.
#![forbid(unsafe_code)]

pub mod crossings;
pub mod cu2qu_multi;
pub mod expr;
pub mod fit;
pub mod qu2cu;
pub mod segment;
pub mod spiro;
pub mod stop;
pub mod variation;

pub use kurbo::{
    Affine, BezPath, CubicBez, Line, ParamCurve, ParamCurveArclen, ParamCurveExtrema, ParamCurveNearest, PathEl, PathSeg, Point, QuadBez, QuadSpline,
    Rect, Shape, Vec2, cubics_to_quadratic_splines, flatten,
};
pub use segment::{Nearest, NodePoint, NodeType, Piece, Segment, bounds, grid_step, round_point, round_to_grid, segment_ending_at, segments};

/// Default tolerance, in font units, when a cubic curve is approximated by quadratic ones for
/// TrueType outlines. One unit matches what the reference compilers use for a 1000-unit em.
pub const CUBIC_TO_QUAD_TOLERANCE: f64 = 1.0;

/// `path` with every cubic segment replaced by quadratic segments within `tolerance` font units.
/// Lines, quadratic segments, moves and closes are kept as they are.
pub fn to_quadratic(path: &BezPath, tolerance: f64) -> BezPath {
    let tolerance = if tolerance.is_finite() && tolerance > 0.0 { tolerance } else { CUBIC_TO_QUAD_TOLERANCE };
    let mut out = BezPath::new();
    let mut last = Point::ZERO;
    let mut start = Point::ZERO;
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                out.move_to(p);
                last = p;
                start = p;
            }
            PathEl::LineTo(p) => {
                out.line_to(p);
                last = p;
            }
            PathEl::QuadTo(c, p) => {
                out.quad_to(c, p);
                last = p;
            }
            PathEl::CurveTo(c1, c2, p) => {
                for (_, _, q) in CubicBez::new(last, c1, c2, p).to_quads(tolerance) {
                    out.quad_to(q.p1, q.p2);
                }
                last = p;
            }
            PathEl::ClosePath => {
                out.close_path();
                last = start;
            }
        }
    }
    out
}

/// Are both coordinates finite (not NaN, not infinite)?
pub fn is_finite(p: Point) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cubics_become_quads_and_lines_stay() {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((100.0, 0.0));
        p.curve_to((100.0, 55.0), (55.0, 100.0), (0.0, 100.0));
        p.close_path();
        let q = to_quadratic(&p, 1.0);
        assert!(q.elements().iter().all(|e| !matches!(e, PathEl::CurveTo(..))));
        assert!(q.elements().iter().any(|e| matches!(e, PathEl::QuadTo(..))));
        assert_eq!(q.elements().first(), Some(&PathEl::MoveTo(Point::new(0.0, 0.0))));
        assert_eq!(q.elements().last(), Some(&PathEl::ClosePath));
        // The end point of the converted curve is the end point of the cubic.
        let ends: Vec<Point> = q.elements().iter().filter_map(|e| if let PathEl::QuadTo(_, p) = e { Some(*p) } else { None }).collect();
        assert_eq!(ends.last(), Some(&Point::new(0.0, 100.0)));
    }

    #[test]
    fn a_bad_tolerance_falls_back_to_the_default() {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.curve_to((0.0, 50.0), (50.0, 100.0), (100.0, 100.0));
        assert_eq!(to_quadratic(&p, f64::NAN).elements().len(), to_quadratic(&p, CUBIC_TO_QUAD_TOLERANCE).elements().len());
    }
}
