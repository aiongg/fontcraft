//! The stroke construction shared by path strokes ([`crate::stroke`]) and Offset Curve
//! ([`crate::filters::offset_curve`]): the offset of each segment to one side of the path, the
//! mitre and the arcs at corners, cap edges, and axis-aligned cuts.
//!
//! A pen moves a point whose path runs in unit direction `t` by [`reach`]: the left normal with
//! its x part scaled by `pen.x` and its y part by `pen.y`, times a side factor `k` (positive left
//! of the path direction, negative right). With `pen.x == pen.y` this is the usual parallel
//! curve. The two features use these parts differently:
//!
//! | | Path strokes | Offset Curve |
//! |---|---|---|
//! | Pen | Round, in a space squeezed along the longer axis of an elliptical pen | The horizontal and vertical distances, as given |
//! | Curves | [`Fit::Parallel`]: kurbo's offset curves | [`Fit::Pen`]: cubics fitted to the pen's offset; Keep Compatible refits one cubic per segment ([`fit`]) |
//! | Assembly | One piece per segment, corner and cap, united with the nonzero rule | Each side as one chain of nodes, the two sides and the caps joined into one contour |
//! | Arcs | Within a tolerance | Two quarter ellipses per half turn, so the node count stays fixed |
//!
//! An elliptical pen is not the anisotropic pen of Offset Curve: no squeeze turns one into the
//! other, so each feature keeps its own pen.

use std::f64::consts::PI;

use fontcraft_geom::{Affine, BezPath, CubicBez, Line, ParamCurve, ParamCurveNearest, Point, Vec2};
use kurbo::{Arc, PathSeg};

use crate::contour::{DIRECTION_EPSILON, Node, NodeKind, Piece, RNode};
use crate::stroke::CapAxis;
use crate::{MAX_SEGMENTS, PathOpError};

/// A mitre reaches at most this many times the offset (half the stroke width) from its node;
/// beyond that the corner is bevelled (the SVG default).
pub const MITER_LIMIT: f64 = 4.0;
/// Points closer than this, in units, are one point; a handle this close to its node is
/// retracted.
pub(crate) const JOIN_EPSILON: f64 = 1e-6;
/// A fitted curve is halved at most this many times over, into at most 64 pieces.
const MAX_DEPTH: u32 = 6;
/// Points of a curve its fitted offset is compared with.
const SAMPLES: usize = 8;
/// Step, in curve parameter, of a direction read from two nearby points.
const STEP: f64 = 1e-3;

/// A straight or cubic piece of a path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Bez {
    Line(Point, Point),
    Cubic(CubicBez),
}

impl Bez {
    /// A quadratic piece becomes the cubic that draws the same curve.
    pub(crate) fn of_piece(piece: Piece) -> Bez {
        match piece {
            Piece::Line(a, b) => Bez::Line(a, b),
            Piece::Quad(q) => Bez::Cubic(CubicBez::new(q.p0, q.p0 + (q.p1 - q.p0) * (2.0 / 3.0), q.p2 + (q.p1 - q.p2) * (2.0 / 3.0), q.p2)),
            Piece::Cubic(c) => Bez::Cubic(c),
        }
    }

    pub(crate) fn of_seg(seg: PathSeg) -> Bez {
        match seg {
            PathSeg::Line(l) => Bez::Line(l.p0, l.p1),
            PathSeg::Quad(q) => Bez::Cubic(q.raise()),
            PathSeg::Cubic(c) => Bez::Cubic(c),
        }
    }

    pub(crate) fn path_seg(&self) -> PathSeg {
        match *self {
            Bez::Line(a, b) => PathSeg::Line(Line::new(a, b)),
            Bez::Cubic(c) => PathSeg::Cubic(c),
        }
    }

    pub(crate) fn start(&self) -> Point {
        match self {
            Bez::Line(a, _) => *a,
            Bez::Cubic(c) => c.p0,
        }
    }

    pub(crate) fn end(&self) -> Point {
        match self {
            Bez::Line(_, b) => *b,
            Bez::Cubic(c) => c.p3,
        }
    }

    pub(crate) fn is_line(&self) -> bool {
        matches!(self, Bez::Line(..))
    }

    /// Whether every point is within [`JOIN_EPSILON`] of the start.
    pub(crate) fn degenerate(&self) -> bool {
        match self {
            Bez::Line(a, b) => a.distance(*b) <= JOIN_EPSILON,
            Bez::Cubic(c) => [c.p1, c.p2, c.p3].iter().all(|p| p.distance(c.p0) <= JOIN_EPSILON),
        }
    }

    /// The unit direction the piece starts in.
    pub(crate) fn start_dir(&self) -> Option<Vec2> {
        match self {
            Bez::Line(a, b) => unit(*b - *a),
            Bez::Cubic(c) => tangent(c, 0.0),
        }
    }

    /// The unit direction the piece ends in.
    pub(crate) fn end_dir(&self) -> Option<Vec2> {
        match self {
            Bez::Line(a, b) => unit(*b - *a),
            Bez::Cubic(c) => tangent(c, 1.0),
        }
    }

    /// Moves the start; a handle moves with its node.
    pub(crate) fn set_start(&mut self, p: Point) {
        match self {
            Bez::Line(a, _) => *a = p,
            Bez::Cubic(c) => {
                let d = p - c.p0;
                c.p0 = p;
                c.p1 += d;
            }
        }
    }

    /// Moves the end; a handle moves with its node.
    pub(crate) fn set_end(&mut self, p: Point) {
        match self {
            Bez::Line(_, b) => *b = p,
            Bez::Cubic(c) => {
                let d = p - c.p3;
                c.p3 = p;
                c.p2 += d;
            }
        }
    }

    /// The same piece drawn the other way.
    pub(crate) fn reversed(&self) -> Bez {
        match *self {
            Bez::Line(a, b) => Bez::Line(b, a),
            Bez::Cubic(c) => Bez::Cubic(CubicBez::new(c.p3, c.p2, c.p1, c.p0)),
        }
    }

    /// Appends the piece to `path`, whose current point is the piece's start.
    pub(crate) fn push_to(&self, path: &mut BezPath) {
        match self {
            Bez::Line(_, b) => path.line_to(*b),
            Bez::Cubic(c) => path.curve_to(c.p1, c.p2, c.p3),
        }
    }

    /// The ring node that ends this piece.
    pub(crate) fn rnode(&self) -> RNode {
        match self {
            Bez::Line(_, b) => RNode::new(Node::new(*b, NodeKind::Line), Vec::new()),
            Bez::Cubic(c) => RNode::new(Node::new(c.p3, NodeKind::Curve), vec![Node::off(c.p1), Node::off(c.p2)]),
        }
    }
}

pub(crate) fn unit(v: Vec2) -> Option<Vec2> {
    (v.is_finite() && v.hypot() > DIRECTION_EPSILON).then(|| v.normalize())
}

/// How far side `k` of the pen moves a point whose path runs in unit direction `t` (see the
/// module docs).
pub(crate) fn reach(t: Vec2, pen: Vec2, k: f64) -> Vec2 {
    Vec2::new(-t.y * pen.x * k, t.x * pen.y * k)
}

fn deriv(c: &CubicBez, t: f64) -> Vec2 {
    let mt = 1.0 - t;
    (c.p1 - c.p0) * (3.0 * mt * mt) + (c.p2 - c.p1) * (6.0 * mt * t) + (c.p3 - c.p2) * (3.0 * t * t)
}

fn deriv2(c: &CubicBez, t: f64) -> Vec2 {
    ((c.p2 - c.p1) - (c.p1 - c.p0)) * (6.0 * (1.0 - t)) + ((c.p3 - c.p2) - (c.p2 - c.p1)) * (6.0 * t)
}

/// The unit direction of `c` at `t`. At an end it is read from the nearest control point that is
/// not on the node, which is exact for a retracted handle; inside, where the derivative vanishes,
/// from two nearby points.
fn tangent(c: &CubicBez, t: f64) -> Option<Vec2> {
    let first = |ds: [Vec2; 3]| ds.into_iter().find(|d| d.hypot2() > JOIN_EPSILON * JOIN_EPSILON).and_then(unit);
    if t <= 0.0 {
        return first([c.p1 - c.p0, c.p2 - c.p0, c.p3 - c.p0]);
    }
    if t >= 1.0 {
        return first([c.p3 - c.p2, c.p3 - c.p1, c.p3 - c.p0]);
    }
    unit(deriv(c, t)).or_else(|| unit(c.eval((t + STEP).min(1.0)) - c.eval((t - STEP).max(0.0))))
}

/// Where the line through `p` along `a` meets the line through `q` along `b`, with the distances
/// along each: `p + a·s = q + b·u`. `None` for parallel lines.
pub(crate) fn intersect(p: Point, a: Vec2, q: Point, b: Vec2) -> Option<(Point, f64, f64)> {
    let cross = a.cross(b);
    if cross.abs() < 1e-9 {
        return None;
    }
    let d = q - p;
    let (s, u) = (d.cross(b) / cross, d.cross(a) / cross);
    Some((p + a * s, s, u))
}

// ── Offsetting a segment ────────────────────────────────────────────────────────────────────────

/// How a curve's offset is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Fit {
    /// kurbo's offset curves, within the tolerance. Only for a pen with `pen.x == pen.y`.
    Parallel(f64),
    /// Cubics fitted to the pen's offset ([`fit`]), halved until they are within the tolerance at
    /// the sample points, at most [`MAX_DEPTH`] times.
    Pen(f64),
}

/// Side `k`'s offset of `piece` with `pen`: a line exactly, a curve as `fit` says. A line without
/// length gives nothing. `budget` counts the pieces made; more than [`MAX_SEGMENTS`] is an error.
pub(crate) fn offset_piece(piece: &Bez, pen: Vec2, k: f64, fit: Fit, budget: &mut usize) -> Result<Vec<Bez>, PathOpError> {
    let mut out = Vec::new();
    match *piece {
        Bez::Line(a, b) => {
            if let Some(t) = unit(b - a) {
                let w = reach(t, pen, k);
                push(&mut out, Bez::Line(a + w, b + w), budget)?;
            }
        }
        Bez::Cubic(c) => match fit {
            Fit::Pen(tolerance) => fit_offset(c, pen, k, tolerance, 0, &mut out, budget)?,
            Fit::Parallel(tolerance) => {
                // kurbo offsets toward the left normal for a positive distance, as `reach` does.
                let d = k * pen.x;
                if d == 0.0 {
                    push(&mut out, Bez::Cubic(c), budget)?;
                } else {
                    let mut path = BezPath::new();
                    kurbo::offset::offset_cubic(c, d, tolerance, &mut path);
                    for seg in path.segments() {
                        push(&mut out, Bez::of_seg(seg), budget)?;
                    }
                }
            }
        },
    }
    Ok(out)
}

fn push(out: &mut Vec<Bez>, b: Bez, budget: &mut usize) -> Result<(), PathOpError> {
    *budget += 1;
    if *budget > MAX_SEGMENTS {
        return Err(PathOpError::TooManySegments(*budget));
    }
    out.push(b);
    Ok(())
}

/// The point of side `k`'s offset of `c` at `t`.
fn offset_at(c: &CubicBez, t: f64, pen: Vec2, k: f64) -> Option<Point> {
    Some(c.eval(t) + reach(tangent(c, t)?, pen, k))
}

/// The unit direction of side `k`'s offset of `c` at `t`.
fn offset_dir(c: &CubicBez, t: f64, pen: Vec2, k: f64) -> Option<Vec2> {
    let d1 = deriv(c, t);
    let speed = d1.hypot();
    if speed > DIRECTION_EPSILON {
        // The pen is linear in the direction, so the offset's derivative times the speed is
        // c′·|c′| plus the pen applied to the part of c″ square to the direction.
        let along = d1 / speed;
        let d2 = deriv2(c, t);
        if let Some(u) = unit(d1 * speed + reach(d2 - along * along.dot(d2), pen, k)) {
            return Some(u);
        }
    }
    let (a, b) = ((t - STEP).max(0.0), (t + STEP).min(1.0));
    unit(offset_at(c, b, pen, k)? - offset_at(c, a, pen, k)?)
}

/// A cubic from `p0` to `p3` that follows side `k`'s offset of `c`: it leaves and arrives in the
/// offset's directions, with handle lengths fitted (least squares) to points of the true offset.
/// A handle marked in `retracted` stays on its node.
pub(crate) fn fit(c: &CubicBez, p0: Point, p3: Point, pen: Vec2, k: f64, retracted: [bool; 2]) -> CubicBez {
    let d0 = offset_dir(c, 0.0, pen, k).or_else(|| tangent(c, 0.0)).unwrap_or(Vec2::ZERO);
    let d1 = offset_dir(c, 1.0, pen, k).or_else(|| tangent(c, 1.0)).unwrap_or(Vec2::ZERO);
    let (mut m11, mut m12, mut m22, mut r1, mut r2) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for i in 1..=SAMPLES {
        let t = i as f64 / (SAMPLES + 1) as f64;
        let Some(target) = offset_at(c, t, pen, k) else { continue };
        let mt = 1.0 - t;
        let base = p0.to_vec2() * (mt * mt * mt + 3.0 * mt * mt * t) + p3.to_vec2() * (3.0 * mt * t * t + t * t * t);
        let r = target.to_vec2() - base;
        let (u, w) = (d0 * (3.0 * mt * mt * t), d1 * (-3.0 * mt * t * t));
        m11 += u.dot(u);
        m12 += u.dot(w);
        m22 += w.dot(w);
        r1 += u.dot(r);
        r2 += w.dot(r);
    }
    let chord = p0.distance(p3);
    let fallback = chord / 3.0;
    let (a, b) = match retracted {
        [true, true] => (0.0, 0.0),
        [true, false] => (0.0, if m22 > 0.0 { r2 / m22 } else { fallback }),
        [false, true] => (if m11 > 0.0 { r1 / m11 } else { fallback }, 0.0),
        [false, false] => {
            let det = m11 * m22 - m12 * m12;
            // Never negative (Cauchy–Schwarz); near zero when the samples cannot tell the two
            // lengths apart.
            if det > 0.0 && det > 1e-12 * m11 * m22 { ((r1 * m22 - r2 * m12) / det, (r2 * m11 - r1 * m12) / det) } else { (fallback, fallback) }
        }
    };
    // A handle longer than this is a fit gone wrong.
    let limit = 2.0 * (chord + c.p0.distance(c.p1) + c.p1.distance(c.p2) + c.p2.distance(c.p3));
    let sane = |l: f64, keep: bool| {
        if keep {
            0.0
        } else if l.is_finite() && (0.0..=limit).contains(&l) {
            l
        } else {
            fallback
        }
    };
    let [keep0, keep1] = retracted;
    CubicBez::new(p0, p0 + d0 * sane(a, keep0), p3 - d1 * sane(b, keep1), p3)
}

/// The largest distance from points of the true offset to `fitted`.
fn fit_error(c: &CubicBez, fitted: &CubicBez, pen: Vec2, k: f64) -> f64 {
    let mut worst: f64 = 0.0;
    for i in 0..SAMPLES {
        let t = (i as f64 + 0.5) / SAMPLES as f64;
        if let Some(p) = offset_at(c, t, pen, k) {
            worst = worst.max(fitted.nearest(p, 1e-6).distance_sq);
        }
    }
    worst.sqrt()
}

/// Side `k`'s offset of `c` as cubics within `tolerance`, appended to `out`.
fn fit_offset(c: CubicBez, pen: Vec2, k: f64, tolerance: f64, depth: u32, out: &mut Vec<Bez>, budget: &mut usize) -> Result<(), PathOpError> {
    let (Some(p0), Some(p3)) = (offset_at(&c, 0.0, pen, k), offset_at(&c, 1.0, pen, k)) else { return Ok(()) };
    let fitted = fit(&c, p0, p3, pen, k, [false, false]);
    if depth >= MAX_DEPTH || fit_error(&c, &fitted, pen, k) <= tolerance {
        return push(out, Bez::Cubic(fitted), budget);
    }
    fit_offset(c.subsegment(0.0..0.5), pen, k, tolerance, depth + 1, out, budget)?;
    fit_offset(c.subsegment(0.5..1.0), pen, k, tolerance, depth + 1, out, budget)
}

// ── Corners ─────────────────────────────────────────────────────────────────────────────────────

/// The tip of a mitre on the outer side of a corner at `node`: where the line through the end
/// `a` of the incoming offset, along its direction `da`, meets the line through the start `b` of
/// the outgoing one, along `db`. `None` (a bevel) when the lines do not meet ahead of `a` and
/// behind `b`, or meet more than [`MITER_LIMIT`] times `width` from the node.
pub(crate) fn miter_tip(a: Point, da: Vec2, b: Point, db: Vec2, node: Point, width: f64) -> Option<Point> {
    let (m, s, u) = intersect(a, da, b, db)?;
    (s >= 0.0 && u <= 0.0 && m.distance(node) <= MITER_LIMIT * width).then_some(m)
}

/// The arc `centre + u·cos θ + w·sin θ` for θ from 0 to `sweep` radians, as cubics within
/// `tolerance`: a circle when `u` and `w` are square to each other and equally long, an ellipse
/// with conjugate radii `u` and `w` otherwise. The first cubic starts at `centre + u`. An infinite
/// tolerance gives the fewest cubics, whatever the size: two for a half turn.
pub(crate) fn arc(centre: Point, u: Vec2, w: Vec2, sweep: f64, tolerance: f64) -> Vec<CubicBez> {
    let size = u.hypot().max(w.hypot());
    let mut out: Vec<CubicBez> = Vec::new();
    if !(size > 0.0 && size.is_finite() && sweep.is_finite() && tolerance > 0.0) {
        return out;
    }
    // The unit circle, mapped onto the arc.
    let map = Affine::new([u.x, u.y, w.x, w.y, centre.x, centre.y]);
    let start = centre + u;
    Arc::new(Point::ZERO, (1.0, 1.0), 0.0, sweep, 0.0).to_cubic_beziers(tolerance / size, |p1, p2, p3| {
        let from = out.last().map_or(start, |c| c.p3);
        out.push(CubicBez::new(from, map * p1, map * p2, map * p3));
    });
    out
}

// ── Caps ────────────────────────────────────────────────────────────────────────────────────────

/// The edge of a cap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CapShape {
    /// Straight across.
    Flat,
    /// Half the gap beyond the ends, straight across.
    Square,
    /// A half ellipse with radii half the gap across and half the gap out.
    Round,
}

/// The edge of a cap from `a` to `b`, the ends of the two sides, reaching out along the unit
/// direction `out`. `tolerance` is the round edge's ([`arc`]). Ends closer than
/// [`JOIN_EPSILON`] get a straight edge.
pub(crate) fn cap_edge(a: Point, b: Point, out: Vec2, shape: CapShape, tolerance: f64) -> Vec<Bez> {
    let half = a.distance(b) / 2.0;
    if half <= JOIN_EPSILON {
        return vec![Bez::Line(a, b)];
    }
    let ext = out * half;
    match shape {
        CapShape::Flat => vec![Bez::Line(a, b)],
        CapShape::Square => vec![Bez::Line(a, a + ext), Bez::Line(a + ext, b + ext), Bez::Line(b + ext, b)],
        CapShape::Round => {
            let c = a.midpoint(b);
            arc(c, a - c, ext, PI, tolerance).into_iter().map(Bez::Cubic).collect()
        }
    }
}

/// The normal of the line an axis-aligned end is cut along, for a path that ends in unit
/// direction `t` (either way along it): a vertical cut when the path ends nearer horizontal
/// (or at 45°), else a horizontal one. [`CapAxis::X`] forces a horizontal cut, [`CapAxis::Y`] a
/// vertical one.
pub(crate) fn cut_normal(t: Vec2, axis: CapAxis) -> Vec2 {
    let vertical = match axis {
        CapAxis::X => false,
        CapAxis::Y => true,
        CapAxis::Auto => t.x.abs() >= t.y.abs(),
    };
    if vertical { Vec2::new(1.0, 0.0) } else { Vec2::new(0.0, 1.0) }
}

/// How far `q` moves along `dir` to reach the line through `through` with normal `normal`;
/// `None` when `dir` runs along that line.
pub(crate) fn to_cut(q: Point, dir: Vec2, through: Point, normal: Vec2) -> Option<f64> {
    let across = dir.dot(normal);
    (across.abs() > 1e-9).then(|| (through - q).dot(normal) / across)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    use crate::filters::offset_curve::{CapStyle, OffsetOptions, offset_curve};
    use crate::stroke::{Cap, StrokeStyle, stroke_outline};
    use crate::{contours_from_bez, coverage_difference};

    #[test]
    fn a_path_stroke_and_an_offset_curve_stroke_cover_the_same_area() {
        // Equal distances, centred, mitre joins and round caps mean the same stroke in both
        // features: an open path with a corner and a curve, and a closed rectangle.
        let style = StrokeStyle { cap_start: Cap::Round, cap_end: Cap::Round, ..StrokeStyle::new(20.0) };
        let options =
            OffsetOptions { horizontal: 10.0, vertical: 10.0, stroke: true, auto: false, position: 50.0, compatible: false, cap: CapStyle::Round };
        for svg in ["M0,0 L100,0 L100,100 C100,160 160,200 220,200", "M0,0 L200,0 L200,150 L0,150 Z"] {
            let path = BezPath::from_svg(svg).unwrap();
            let stroked = stroke_outline(&path, &style).unwrap();
            let offset: Vec<BezPath> = offset_curve(&contours_from_bez(&path), &options, 0.0).unwrap().iter().map(|c| c.to_bez().unwrap()).collect();
            let d = coverage_difference(&stroked, &offset).unwrap();
            assert!(d.ink > 1000.0, "{svg}: {d:?}");
            assert!(d.within(0.5, 0.005), "{svg}: {d:?}");
        }
    }

    #[test]
    fn a_half_turn_arc_ends_where_it_should() {
        // A half ellipse with conjugate radii: through c + u, c + w and c − u.
        let (c, u, w) = (Point::new(10.0, 20.0), Vec2::new(5.0, 0.0), Vec2::new(1.0, 8.0));
        let fixed = arc(c, u, w, PI, f64::INFINITY);
        assert_eq!(fixed.len(), 2);
        assert!(fixed[0].p0.distance(c + u) < 1e-12);
        assert!(fixed[0].p3.distance(c + w) < 1e-9);
        assert!(fixed[1].p3.distance(c - u) < 1e-9);
        // A big circle within a tight tolerance takes more cubics.
        assert!(arc(c, Vec2::new(1000.0, 0.0), Vec2::new(0.0, 1000.0), PI, 0.05).len() > 2);
    }
}
