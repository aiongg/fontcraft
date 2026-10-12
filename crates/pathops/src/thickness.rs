//! Strokes whose width changes along the path: the outline of a stroked path whose on-curve
//! nodes carry their own widths (the Thickness tool, `thickness` row).
//!
//! Each on-curve point has a left and a right width: the distances from the point to the band's
//! two edges, looking along the path direction. Along a segment both widths change linearly with
//! the curve parameter from one end's value to the other's. The pieces, united with the nonzero
//! rule, are:
//!
//! | Piece | Shape |
//! |---|---|
//! | Each segment | The area between its left and right edge curves |
//! | Each corner, outer side | Miter (bevel beyond [`MITER_LIMIT`]), bevel, or a round sector of that side's width |
//! | Each end of an open path | Butt (none), round (a half disk across the band) or square |
//! | A closed path with `fill` | The path itself |
//!
//! The pen is round: the stroke height is not used once widths vary. The edge curves are cubic
//! Hermite pieces through points of the exact edge, split until they lie within
//! [`STROKE_TOLERANCE`] of it.

use fontcraft_geom::{BezPath, CubicBez, ParamCurve, PathEl, Point, QuadBez, Shape, Vec2};
use kurbo::ParamCurveDeriv;

use crate::stroke::{Cap, Join, MAX_STROKE_SEGMENTS, MAX_STROKE_WIDTH, MITER_LIMIT, STROKE_TOLERANCE, StrokeStyle};
use crate::stroker::arc;
use crate::{MAX_SEGMENTS, PathOpError, check, union};

/// Deepest halving of one edge curve: at most 2^6 cubic pieces per segment side.
const MAX_EDGE_DEPTH: u32 = 6;
/// Step of the numeric derivative of an edge curve, in curve parameter.
const DERIV_STEP: f64 = 1e-4;
/// Tangents shorter than this count as zero.
const TANGENT_EPSILON: f64 = 1e-9;
/// Two segments whose directions differ by less than this (the sine of the angle) meet without
/// a corner piece.
const STRAIGHT: f64 = 1e-6;
/// Pieces with less area than this are dropped.
const MIN_PIECE_AREA: f64 = 1e-9;

/// `[left, right]` widths in font units.
pub type Widths = [f64; 2];

/// One segment of a subpath as a cubic, with the widths at its two ends.
#[derive(Clone, Copy, Debug)]
struct Seg {
    c: CubicBez,
    from: Widths,
    to: Widths,
}

struct Subpath {
    segs: Vec<Seg>,
    closed: bool,
    /// The subpath itself, for `fill`.
    outline: BezPath,
}

/// The outline of `path` stroked with `style`, where on-curve point `k` of `path` (in element
/// order: each move's point, then each line's, quadratic's and cubic's end point) has the widths
/// `widths[k]`. A point whose entry is `None`, or that has no entry, takes the stroke's own
/// widths: `style.width` split by `style.position`. The closing line of a closed subpath ends at
/// the subpath's first point and takes its widths.
pub fn stroke_outline_widths(path: &BezPath, style: &StrokeStyle, widths: &[Option<Widths>]) -> Result<Vec<BezPath>, PathOpError> {
    style.check()?;
    let half = style.width / 2.0;
    let p = style.position;
    let default = [half * (1.0 - p), half * (1.0 + p)];
    for w in widths.iter().flatten() {
        for v in w {
            if !(v.is_finite() && (0.0..=MAX_STROKE_WIDTH).contains(v)) {
                return Err(PathOpError::Invalid(format!("a node's stroke width must be from 0 to {MAX_STROKE_WIDTH}, got {v}")));
            }
        }
    }
    let at = |k: usize| widths.get(k).copied().flatten().unwrap_or(default);
    let subpaths = subpaths(path, &at)?;
    let mut budget = MAX_SEGMENTS;
    let mut pieces = Vec::new();
    for sub in &subpaths {
        for seg in &sub.segs {
            pieces.extend(strip(seg, &mut budget)?);
        }
        let count = sub.segs.len();
        let joins = if sub.closed { count } else { count.saturating_sub(1) };
        for i in 0..joins {
            if let (Some(a), Some(b)) = (sub.segs.get(i), sub.segs.get((i + 1) % count.max(1))) {
                pieces.extend(join(a, b, style.join));
            }
        }
        if !sub.closed
            && let (Some(first), Some(last)) = (sub.segs.first(), sub.segs.last())
        {
            if let Some(t) = tangent(&first.c, 0.0) {
                pieces.extend(cap(first.c.p0, t, -t, first.from, style.cap_start));
            }
            if let Some(t) = tangent(&last.c, 1.0) {
                pieces.extend(cap(last.c.p3, t, t, last.to, style.cap_end));
            }
        }
        if sub.closed && style.fill {
            pieces.push(sub.outline.clone());
        }
    }
    let pieces: Vec<BezPath> = pieces.into_iter().filter_map(oriented).collect();
    if pieces.is_empty() {
        return Ok(Vec::new());
    }
    union(&pieces)
}

/// The subpaths of `path` as cubics with their end widths, after checking every coordinate and
/// the segment count. Segments without length are left out.
fn subpaths(path: &BezPath, at: &dyn Fn(usize) -> Widths) -> Result<Vec<Subpath>, PathOpError> {
    let mut out = Vec::new();
    let mut current: Option<(Subpath, Point, Widths)> = None;
    let mut last = Point::ZERO;
    let mut last_w = [0.0; 2];
    let mut knot = 0usize;
    let mut count = 0usize;
    for el in path.elements() {
        let points: &[Point] = match el {
            PathEl::MoveTo(p) | PathEl::LineTo(p) => std::slice::from_ref(p),
            PathEl::QuadTo(a, b) => &[*a, *b],
            PathEl::CurveTo(a, b, c) => &[*a, *b, *c],
            PathEl::ClosePath => &[],
        };
        for p in points {
            check(p.x)?;
            check(p.y)?;
        }
        let (c, end) = match *el {
            PathEl::MoveTo(p) => {
                out.extend(current.take().map(|(sub, _, _)| sub));
                let w = at(knot);
                knot += 1;
                let mut outline = BezPath::new();
                outline.move_to(p);
                current = Some((Subpath { segs: Vec::new(), closed: false, outline }, p, w));
                last = p;
                last_w = w;
                continue;
            }
            PathEl::LineTo(p) => (line(last, p), p),
            PathEl::QuadTo(a, p) => (QuadBez::new(last, a, p).raise(), p),
            PathEl::CurveTo(a, b, p) => (CubicBez::new(last, a, b, p), p),
            PathEl::ClosePath => {
                if let Some((mut sub, start, start_w)) = current.take() {
                    if last != start {
                        count += 1;
                        let c = line(last, start);
                        if !degenerate(&c) {
                            sub.segs.push(Seg { c, from: last_w, to: start_w });
                        }
                    }
                    sub.closed = true;
                    sub.outline.close_path();
                    last = start;
                    last_w = start_w;
                    out.push(sub);
                }
                continue;
            }
        };
        count += 1;
        if count > MAX_STROKE_SEGMENTS {
            return Err(PathOpError::TooManySegments(count));
        }
        let w = at(knot);
        knot += 1;
        if let Some((sub, _, _)) = current.as_mut() {
            sub.outline.push(*el);
            if !degenerate(&c) {
                sub.segs.push(Seg { c, from: last_w, to: w });
            }
        }
        last = end;
        last_w = w;
    }
    out.extend(current.map(|(sub, _, _)| sub));
    Ok(out)
}

fn line(a: Point, b: Point) -> CubicBez {
    CubicBez::new(a, a.lerp(b, 1.0 / 3.0), a.lerp(b, 2.0 / 3.0), b)
}

fn degenerate(c: &CubicBez) -> bool {
    c.p0 == c.p1 && c.p0 == c.p2 && c.p0 == c.p3
}

/// The unit direction of travel of `c` at `t`. Where the derivative vanishes (a handle on its
/// node) the direction comes from a nearby chord.
fn tangent(c: &CubicBez, t: f64) -> Option<Vec2> {
    let d = c.deriv().eval(t).to_vec2();
    let len = d.hypot();
    if len > TANGENT_EPSILON && len.is_finite() {
        return Some(d / len);
    }
    let (a, b) = if t < 0.5 { (t, (t + 1e-3).min(1.0)) } else { ((t - 1e-3).max(0.0), t) };
    for v in [c.eval(b) - c.eval(a), c.p3 - c.p0] {
        let len = v.hypot();
        if len > TANGENT_EPSILON && len.is_finite() {
            return Some(v / len);
        }
    }
    None
}

/// The unit normal on the left of direction `t` (y up).
fn left(t: Vec2) -> Vec2 {
    Vec2::new(-t.y, t.x)
}

/// A point of one edge of `seg` at `t`: `side` is 1 for the left edge, −1 for the right.
fn edge_point(seg: &Seg, side: f64, t: f64) -> Point {
    let i = if side > 0.0 { 0 } else { 1 };
    let (a, b) = (seg.from.get(i).copied().unwrap_or(0.0), seg.to.get(i).copied().unwrap_or(0.0));
    let w = a + (b - a) * t;
    let n = tangent(&seg.c, t).map_or(Vec2::ZERO, left);
    seg.c.eval(t) + n * (side * w)
}

/// The derivative of an edge at `t`, by central differences (one-sided at the ends).
fn edge_deriv(seg: &Seg, side: f64, t: f64) -> Vec2 {
    let (a, b) = ((t - DERIV_STEP).max(0.0), (t + DERIV_STEP).min(1.0));
    if b <= a {
        return Vec2::ZERO;
    }
    (edge_point(seg, side, b) - edge_point(seg, side, a)) / (b - a)
}

/// One edge of `seg` as cubic pieces from t = 0 to t = 1.
fn edge(seg: &Seg, side: f64, budget: &mut usize) -> Result<Vec<CubicBez>, PathOpError> {
    let mut out = Vec::new();
    // Intervals still to fit, last first, so the pieces come out in order.
    let mut stack = vec![(0.0_f64, 1.0_f64, 0u32)];
    while let Some((t0, t1, depth)) = stack.pop() {
        let dt = t1 - t0;
        let (e0, e1) = (edge_point(seg, side, t0), edge_point(seg, side, t1));
        let piece = CubicBez::new(e0, e0 + edge_deriv(seg, side, t0) * (dt / 3.0), e1 - edge_deriv(seg, side, t1) * (dt / 3.0), e1);
        let off = [0.25, 0.5, 0.75].iter().map(|&u| (piece.eval(u) - edge_point(seg, side, t0 + u * dt)).hypot()).fold(0.0_f64, f64::max);
        if (off > STROKE_TOLERANCE || !off.is_finite()) && depth < MAX_EDGE_DEPTH {
            let tm = (t0 + t1) / 2.0;
            stack.push((tm, t1, depth + 1));
            stack.push((t0, tm, depth + 1));
            continue;
        }
        *budget = budget.checked_sub(1).ok_or(PathOpError::TooManySegments(MAX_SEGMENTS + 1))?;
        out.push(piece);
    }
    Ok(out)
}

/// The band along one segment: its left edge forward, its right edge back.
fn strip(seg: &Seg, budget: &mut usize) -> Result<Option<BezPath>, PathOpError> {
    let near = edge(seg, 1.0, budget)?;
    let far = edge(seg, -1.0, budget)?;
    let (Some(first), Some(back)) = (near.first(), far.last()) else { return Ok(None) };
    let mut p = BezPath::new();
    p.move_to(first.p0);
    for c in &near {
        p.curve_to(c.p1, c.p2, c.p3);
    }
    p.line_to(back.p3);
    for c in far.iter().rev() {
        p.curve_to(c.p2, c.p1, c.p0);
    }
    p.close_path();
    Ok(Some(p))
}

/// The corner piece where `a` meets `b`, on the outer side of the turn.
fn join(a: &Seg, b: &Seg, kind: Join) -> Option<BezPath> {
    let (t_in, t_out) = (tangent(&a.c, 1.0)?, tangent(&b.c, 0.0)?);
    let turn = t_in.cross(t_out);
    if turn.abs() < STRAIGHT && t_in.dot(t_out) > 0.0 {
        return None;
    }
    let k = b.c.p0;
    // A left turn has its outer side on the right.
    let [left_w, right_w] = b.from;
    let (side, d) = if turn > 0.0 { (-1.0, right_w) } else { (1.0, left_w) };
    if d <= 0.0 {
        return None;
    }
    let (n_in, n_out) = (left(t_in) * side, left(t_out) * side);
    let (p, q) = (k + n_in * d, k + n_out * d);
    match kind {
        Join::Bevel => Some(polygon(&[k, p, q])),
        Join::Miter => {
            let denom = t_in.cross(t_out);
            let u = if denom.abs() > STRAIGHT { (q - p).cross(t_out) / denom } else { f64::NAN };
            let tip = p + t_in * u;
            if u.is_finite() && u >= 0.0 && (tip - k).hypot() <= MITER_LIMIT * d { Some(polygon(&[k, p, tip, q])) } else { Some(polygon(&[k, p, q])) }
        }
        Join::Round | Join::RoundFull => {
            let sweep = turn.atan2(t_in.dot(t_out));
            Some(sector(k, p - k, sweep))
        }
    }
}

/// The cap at an end point `k` with direction of travel `t`, `out` pointing away from the path,
/// and the widths there.
fn cap(k: Point, t: Vec2, out: Vec2, w: Widths, kind: Cap) -> Option<BezPath> {
    let [l, r] = w;
    let h = (l + r) / 2.0;
    if h <= 0.0 {
        return None;
    }
    let n = left(t);
    let (a, b) = (k + n * l, k - n * r);
    match kind {
        Cap::Square => Some(polygon(&[a, a + out * h, b + out * h, b])),
        Cap::Round => {
            let centre = k + n * ((l - r) / 2.0);
            let sweep = std::f64::consts::PI.copysign(n.cross(out));
            Some(sector(centre, a - centre, sweep))
        }
        Cap::Butt | Cap::RoundInset | Cap::AxisAligned => None,
    }
}

/// The circle sector around `centre` from `centre + u`, turning by `sweep` radians.
fn sector(centre: Point, u: Vec2, sweep: f64) -> BezPath {
    let mut p = BezPath::new();
    p.move_to(centre);
    p.line_to(centre + u);
    for c in arc(centre, u, Vec2::new(-u.y, u.x), sweep, STROKE_TOLERANCE) {
        p.curve_to(c.p1, c.p2, c.p3);
    }
    p.close_path();
    p
}

fn polygon(points: &[Point]) -> BezPath {
    let mut p = BezPath::new();
    for (i, q) in points.iter().enumerate() {
        if i == 0 {
            p.move_to(*q);
        } else {
            p.line_to(*q);
        }
    }
    p.close_path();
    p
}

/// `p` drawn counter-clockwise, or `None` when it has no area or a point is not finite.
fn oriented(p: BezPath) -> Option<BezPath> {
    let area = p.area();
    if !p.is_finite() || !area.is_finite() || area.abs() < MIN_PIECE_AREA {
        return None;
    }
    Some(if area < 0.0 { p.reverse_subpaths() } else { p })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stroke::stroke_outline;

    fn bounds(paths: &[BezPath]) -> (f64, f64, f64, f64) {
        let mut b = paths.first().unwrap().bounding_box();
        for p in paths {
            b = b.union(p.bounding_box());
        }
        (b.x0, b.y0, b.x1, b.y1)
    }

    fn area(paths: &[BezPath]) -> f64 {
        paths.iter().map(|p| p.area()).sum()
    }

    fn open_line() -> BezPath {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((100.0, 0.0));
        p
    }

    #[test]
    fn widths_taper_a_straight_stroke_linearly() {
        // Left of a rightward line is up: 10 above and 10 below at the start, 30 and 0 at the end.
        let out = stroke_outline_widths(&open_line(), &StrokeStyle::new(20.0), &[None, Some([30.0, 0.0])]).unwrap();
        let (x0, y0, x1, y1) = bounds(&out);
        assert!((x0 - 0.0).abs() < 1e-6 && (x1 - 100.0).abs() < 1e-6);
        assert!((y0 + 10.0).abs() < 1e-6 && (y1 - 30.0).abs() < 1e-6, "{y0} {y1}");
        // A trapezoid of parallel sides 20 and 30, 100 long.
        assert!((area(&out) - 2500.0).abs() < 1e-3, "{}", area(&out));
    }

    #[test]
    fn without_node_widths_it_matches_the_uniform_stroke() {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.curve_to((0.0, 100.0), (200.0, 100.0), (200.0, 0.0));
        let style = StrokeStyle::new(30.0);
        let ours = area(&stroke_outline_widths(&p, &style, &[]).unwrap());
        let uniform = area(&stroke_outline(&p, &style).unwrap());
        assert!((ours - uniform).abs() / uniform < 0.005, "{ours} vs {uniform}");
    }

    #[test]
    fn a_closed_square_gets_miter_corners_and_its_own_widths() {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((100.0, 0.0));
        p.line_to((100.0, 100.0));
        p.line_to((0.0, 100.0));
        p.close_path();
        let out = stroke_outline_widths(&p, &StrokeStyle::new(20.0), &[Some([10.0, 10.0]); 4]).unwrap();
        // A frame from −10 to 110 with a hole from 10 to 90.
        assert!((area(&out).abs() - (120.0 * 120.0 - 80.0 * 80.0)).abs() < 1e-3, "{}", area(&out));
        let (x0, y0, x1, y1) = bounds(&out);
        assert!((x0 + 10.0).abs() < 1e-6 && (y0 + 10.0).abs() < 1e-6 && (x1 - 110.0).abs() < 1e-6 && (y1 - 110.0).abs() < 1e-6);
    }

    #[test]
    fn bad_widths_are_refused() {
        let style = StrokeStyle::new(20.0);
        assert!(stroke_outline_widths(&open_line(), &style, &[Some([f64::NAN, 1.0])]).is_err());
        assert!(stroke_outline_widths(&open_line(), &style, &[Some([-1.0, 1.0])]).is_err());
        assert!(stroke_outline_widths(&open_line(), &style, &[Some([0.0, 0.0]), Some([0.0, 0.0])]).unwrap().is_empty());
    }
}
