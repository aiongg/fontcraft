//! Curves fitted to freehand input (the Pencil tool).
//!
//! The pointer samples are first joined by a quadratic B-spline (each sample is a control point,
//! the curve runs through the midpoints between them). That spline is smooth everywhere, which is
//! what kurbo's simplifier needs: it then fits as few cubic curves as stay within the accuracy.

use kurbo::simplify::{SimplifyOptions, simplify_bezpath};
use kurbo::{BezPath, Point};

/// Most samples one fit takes; later ones are dropped.
pub const MAX_FIT_POINTS: usize = 20_000;
/// The accuracy range, in font units: finer fits subdivide without end, coarser ones lose the
/// stroke.
pub const FIT_ACCURACY: std::ops::RangeInclusive<f64> = 0.05..=1_000.0;
/// Samples farther than this from the origin, in font units, are dropped: the fit subdivides
/// without end when its arithmetic overflows.
pub const MAX_FIT_COORDINATE: f64 = 10_000_000.0;
/// Samples closer than this to the previous one are dropped.
const MIN_STEP: f64 = 1e-6;

/// Cubic curves that follow the polyline through `points` within about `accuracy` font units,
/// as one subpath (closed when `closed`). Samples that are not finite, lie beyond
/// [`MAX_FIT_COORDINATE`] or repeat the previous one are skipped; fewer than two usable samples give an empty path, exactly two a straight line.
pub fn fit_polyline(points: &[Point], accuracy: f64, closed: bool) -> BezPath {
    let mut pts: Vec<Point> = Vec::with_capacity(points.len().min(MAX_FIT_POINTS));
    for &p in points.iter().take(MAX_FIT_POINTS) {
        if !(p.x.is_finite() && p.y.is_finite() && p.x.abs() <= MAX_FIT_COORDINATE && p.y.abs() <= MAX_FIT_COORDINATE) {
            continue;
        }
        if pts.last().is_some_and(|q| q.distance(p) < MIN_STEP) {
            continue;
        }
        pts.push(p);
    }
    if closed && pts.len() > 2 && pts.first().zip(pts.last()).is_some_and(|(a, b)| a.distance(*b) < MIN_STEP) {
        pts.pop();
    }
    let accuracy = if accuracy.is_finite() { accuracy.clamp(*FIT_ACCURACY.start(), *FIT_ACCURACY.end()) } else { 1.0 };
    let mut out = BezPath::new();
    match pts.as_slice() {
        [] | [_] => out,
        [a, b] => {
            out.move_to(*a);
            out.line_to(*b);
            if closed {
                out.close_path();
            }
            out
        }
        _ => simplify_bezpath(spline(&pts, closed), accuracy, &SimplifyOptions::default()),
    }
}

/// The quadratic B-spline of `pts` (at least three): an open one starts and ends at the first
/// and last sample, with a straight piece to the first midpoint and from the last.
fn spline(pts: &[Point], closed: bool) -> BezPath {
    let mut path = BezPath::new();
    let mid = |a: Point, b: Point| a.midpoint(b);
    if closed {
        let (Some(&first), Some(&last)) = (pts.first(), pts.last()) else { return path };
        path.move_to(mid(last, first));
        for (i, &p) in pts.iter().enumerate() {
            let next = pts.get(i + 1).copied().unwrap_or(first);
            path.quad_to(p, mid(p, next));
        }
        path.close_path();
        return path;
    }
    let (Some(&first), Some(&second), Some(&last)) = (pts.first(), pts.get(1), pts.last()) else { return path };
    path.move_to(first);
    path.line_to(mid(first, second));
    for w in pts.windows(3) {
        if let [_, p, next] = w {
            path.quad_to(*p, mid(*p, *next));
        }
    }
    path.line_to(last);
    path
}

#[cfg(test)]
mod tests {
    use kurbo::{ParamCurve, PathEl, PathSeg, Shape};

    use super::*;

    fn arc(n: usize, r: f64) -> Vec<Point> {
        (0..n).map(|i| Point::new(r * (i as f64 * 0.01).cos(), r * (i as f64 * 0.01).sin())).collect()
    }

    #[test]
    fn a_sampled_arc_becomes_a_few_curves_that_stay_on_it() {
        let pts = arc(300, 200.0);
        let path = fit_polyline(&pts, 1.0, false);
        let segs: Vec<PathSeg> = path.segments().collect();
        assert!(!segs.is_empty() && segs.len() < 20, "{} segments", segs.len());
        for seg in &segs {
            for k in 0..=10 {
                let p = seg.eval(k as f64 / 10.0);
                assert!((p.to_vec2().hypot() - 200.0).abs() < 3.0, "{p:?} is off the arc");
            }
        }
        assert!(matches!(path.elements().first(), Some(PathEl::MoveTo(p)) if p.distance(pts[0]) < 1e-9));
    }

    #[test]
    fn a_closed_stroke_closes_and_bad_input_gives_little_or_nothing() {
        let mut pts = arc(629, 100.0);
        pts.push(pts[0]);
        let closed = fit_polyline(&pts, 1.0, true);
        assert!(matches!(closed.elements().last(), Some(PathEl::ClosePath)));
        assert!((closed.area().abs() - std::f64::consts::PI * 100.0 * 100.0).abs() < 1500.0);
        assert!(fit_polyline(&[], 1.0, false).is_empty());
        assert!(fit_polyline(&[Point::new(f64::NAN, 0.0), Point::new(1.0, 1.0)], 1.0, false).is_empty());
        assert!(fit_polyline(&[Point::new(1e300, 0.0), Point::ZERO, Point::new(-1e300, 5.0)], 1.0, false).is_empty());
        let same = vec![Point::new(5.0, 5.0); 50];
        assert!(fit_polyline(&same, 0.0, false).is_empty());
        assert_eq!(fit_polyline(&[Point::ZERO, Point::new(10.0, 0.0)], f64::NAN, false).segments().count(), 1);
    }
}
