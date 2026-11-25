//! Add Extremes and Force Extremes (`path.addExtremes`, `path.forceExtremes`,
//! `extremes.definitions`, docs/spec/03a › question 17).

use std::collections::HashSet;

use fontcraft_geom::{CubicBez, ParamCurve, Point, Vec2};

use crate::PathOpError;
use crate::contour::{Contour, Node, NodeKind, Piece, RNode, Ring, dist_to_segment, round_to_grid, split_segment, unit_roots};

/// Add Extremes skips an extreme that would leave a segment shorter than this many units (the
/// `shortSegments` threshold)…
pub const MIN_EXTREME_SEGMENT: f64 = 3.0;
/// …or shorter than both this many units and this share of the curve it is on (its chord). The
/// share keeps the extremes of small round shapes.
pub const SHORT_EXTREME_SEGMENT: f64 = 10.0;
pub const SHORT_EXTREME_SHARE: f64 = 0.1;
/// A smooth node between two cubics moves onto an extreme this close: this many units, or this
/// share of the length (along the chord) of the segment the extreme is on, whichever is more.
pub const NEAR_EXTREME_DISTANCE: f64 = 2.0;
pub const NEAR_EXTREME_SHARE: f64 = 0.01;
/// A node moves onto an extreme only when the two refitted curves stay within this many units of
/// the old ones.
pub const MAX_EXTREME_SHIFT: f64 = 0.5;
/// Where the node cannot move, a handle within this many degrees of horizontal or vertical, at a
/// node within [`NEAR_EXTREME_DISTANCE`] of an extreme, is made exactly horizontal or vertical (the
/// `almostOrthogonalHandle` thresholds).
pub const NEAR_EXTREME_DEGREES: f64 = 2.0;
/// Points sampled on a curve that is refitted, and rounds of the fit.
const FIT_SAMPLES: usize = 24;
const FIT_ROUNDS: usize = 4;
/// Even forced, an extreme closer than this many units to a node, or to the extreme before it,
/// is not a new point.
const SAME_POINT: f64 = 1e-3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExtremesOptions {
    /// Force Extremes: no short-segment exception.
    pub force: bool,
    /// New points are rounded to multiples of this many units; 0 leaves them unrounded.
    pub grid: f64,
}

/// Which coordinate has its extreme: the tangent there is perpendicular to that axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Axis {
    /// `x` is at a minimum or maximum: the tangent is vertical.
    X,
    /// `y` is at a minimum or maximum: the tangent is horizontal.
    Y,
}

impl Axis {
    fn of(self, v: Vec2) -> f64 {
        match self {
            Axis::X => v.x,
            Axis::Y => v.y,
        }
    }

    /// Degrees between a handle direction and the tangent this extreme has.
    fn off_by(self, v: Vec2) -> f64 {
        let (across, along) = match self {
            Axis::X => (v.x.abs(), v.y.abs()),
            Axis::Y => (v.y.abs(), v.x.abs()),
        };
        across.atan2(along).to_degrees()
    }

    /// Lines the handle `h` up with node `p` so the tangent at `p` has this extreme.
    fn align(self, h: &mut Node, p: Point) {
        match self {
            Axis::X => h.point.x = p.x,
            Axis::Y => h.point.y = p.y,
        }
    }
}

/// One extreme inside a segment.
#[derive(Clone, Copy, Debug)]
struct Extreme {
    piece: usize,
    t: f64,
    axis: Axis,
    point: Point,
}

/// Add Extremes (or Force Extremes) on one contour, on the segments whose two end nodes are in
/// `selected` (indices into `contour.nodes`), or on all of them.
///
/// First, a smooth node between two cubics with an extreme near it (within 2 units or 1% of the
/// segment) moves onto the extreme, its handles made exactly horizontal or vertical and the two
/// curves refitted, when they stay within half a unit of the old ones. Where a node cannot move, a
/// node whose handle is within 2° of horizontal or vertical, and that has an extreme of that
/// direction within 2 units on the handle's segment, keeps its place and has its handles that are
/// within 2° made exactly horizontal or vertical. Either way the extreme is on the node and no
/// point is added next to it. Then a node goes at each point inside a curve where the tangent is horizontal or vertical.
/// Add Extremes skips an extreme closer than 3 units, or than both 10 units and a tenth of its
/// segment's chord, to an end of its segment or to the extreme before it; Force Extremes adds every extreme that is a new point.
/// New nodes are smooth and rounded to `grid`, and their handles are lined up with them so the
/// tangent stays exactly horizontal or vertical. A quadratic spline that gets a node has each of
/// its implied on-curve points made a node.
pub fn add_extremes(contour: &Contour, selected: Option<&[usize]>, options: ExtremesOptions) -> Result<Contour, PathOpError> {
    let mut ring = Ring::from_contour(contour)?;
    let set: Option<HashSet<usize>> = selected.map(|s| s.iter().copied().collect());
    let chosen = |r: &RNode| set.as_ref().is_none_or(|s| r.src.is_some_and(|i| s.contains(&i)));

    for j in 0..ring.len() {
        if ring.nodes.get(j).is_some_and(chosen) && !move_onto_extreme(&mut ring, j, options.grid) {
            align_near_extreme(&mut ring, j);
        }
    }

    let mut out: Vec<RNode> = Vec::with_capacity(ring.len());
    let mut axes: Vec<Option<Axis>> = Vec::with_capacity(ring.len());
    let mut tail: Vec<(RNode, Option<Axis>)> = Vec::new();
    for j in 0..ring.len() {
        let Some(original) = ring.nodes.get(j) else { continue };
        let ends_chosen = chosen(original) && ring.prev(j).and_then(|p| ring.nodes.get(p)).is_some_and(chosen);
        let found = if ends_chosen { extremes(&ring, j, options) } else { Vec::new() };
        let cuts: Vec<(usize, f64)> = found.iter().map(|e| (e.piece, e.t)).collect();
        let Some(mut nodes) = split_segment(&ring, j, &cuts).filter(|_| !found.is_empty()) else {
            out.push(original.clone());
            axes.push(None);
            continue;
        };
        let new = nodes.len().saturating_sub(1);
        for (i, r) in nodes.iter_mut().enumerate() {
            for c in &mut r.ctrl {
                c.point = round_to_grid(c.point, options.grid);
            }
            if i < new {
                r.node.point = round_to_grid(r.node.point, options.grid);
            }
        }
        let mut marks: Vec<Option<Axis>> = found.iter().map(|e| Some(e.axis)).collect();
        marks.push(None);
        if j == 0 && ring.closed {
            // The start stays first; the closing segment's new nodes go at the end.
            let end = nodes.pop();
            marks.pop();
            tail = nodes.into_iter().zip(marks).collect();
            if let Some(end) = end {
                out.push(end);
                axes.push(None);
            }
        } else {
            out.extend(nodes);
            axes.extend(marks);
        }
    }
    for (r, a) in tail {
        out.push(r);
        axes.push(a);
    }
    ring.nodes = out;
    for (i, axis) in axes.iter().enumerate() {
        let (Some(axis), Some(p)) = (axis, ring.nodes.get(i).map(|r| r.node.point)) else { continue };
        if let Some(h) = ring.nodes.get_mut(i).and_then(|r| r.ctrl.last_mut()) {
            axis.align(h, p);
        }
        if let Some(h) = ring.next(i).and_then(|n| ring.nodes.get_mut(n)).and_then(|r| r.ctrl.first_mut()) {
            axis.align(h, p);
        }
    }
    Ok(ring.to_contour())
}

/// The extremes inside the segment that ends at node `j`, in path order, without those too close
/// to its ends or to each other.
fn extremes(ring: &Ring, j: usize, options: ExtremesOptions) -> Vec<Extreme> {
    let Some(seg) = ring.seg(j) else { return Vec::new() };
    if !seg.is_curve() {
        return Vec::new();
    }
    let mut found = Vec::new();
    for (i, piece) in seg.pieces().iter().enumerate() {
        if matches!(piece, Piece::Line(..)) {
            continue;
        }
        let (a, b, c) = piece.deriv_coeffs();
        for axis in [Axis::X, Axis::Y] {
            for t in unit_roots(axis.of(a), axis.of(b), axis.of(c)) {
                found.push(Extreme { piece: i, t, axis, point: piece.eval(t) });
            }
        }
    }
    found.sort_by(|x, y| x.piece.cmp(&y.piece).then(x.t.total_cmp(&y.t)));
    let short = SHORT_EXTREME_SEGMENT.min(SHORT_EXTREME_SHARE * seg.from.distance(seg.to));
    let min = if options.force { SAME_POINT } else { MIN_EXTREME_SEGMENT.max(short) };
    let end = round_to_grid(seg.to, options.grid);
    let mut last = round_to_grid(seg.from, options.grid);
    let mut kept = Vec::with_capacity(found.len());
    for e in found {
        let p = round_to_grid(e.point, options.grid);
        if p.distance(last) < min || p.distance(end) < min {
            continue;
        }
        last = p;
        kept.push(e);
    }
    kept
}

/// The segment that ends at node `j` when it is a cubic.
fn cubic(ring: &Ring, j: usize) -> Option<CubicBez> {
    let seg = ring.seg(j)?;
    match (seg.kind, seg.ctrl) {
        (NodeKind::Curve, [a, b]) => Some(CubicBez::new(seg.from, a.point, b.point, seg.to)),
        _ => None,
    }
}

/// Moves node `j`, smooth between two cubics, onto the nearest extreme close to it on either of
/// them, and refits the two. Returns whether it did.
fn move_onto_extreme(ring: &mut Ring, j: usize, grid: f64) -> bool {
    let Some((next, target, na, nb)) = moved_onto_extreme(ring, j, grid) else { return false };
    if let Some(r) = ring.nodes.get_mut(j) {
        r.node.point = target;
        r.node.smooth = true;
        for (h, p) in r.ctrl.iter_mut().zip([na.p1, na.p2]) {
            h.point = p;
        }
    }
    if let Some(r) = ring.nodes.get_mut(next) {
        for (h, p) in r.ctrl.iter_mut().zip([nb.p1, nb.p2]) {
            h.point = p;
        }
    }
    true
}

/// The node after `j`, where node `j` moves and its two curves after the move. The curves must stay
/// within [`MAX_EXTREME_SHIFT`] of the old ones, and a node whose handle is horizontal or vertical is
/// an extreme already.
fn moved_onto_extreme(ring: &Ring, j: usize, grid: f64) -> Option<(usize, Point, CubicBez, CubicBez)> {
    let next = ring.next(j).filter(|n| *n != j)?;
    if ring.looks_smooth(j, NEAR_EXTREME_DEGREES) != Some(true) {
        return None;
    }
    let (a, b) = (cubic(ring, j)?, cubic(ring, next)?);
    let node = a.p3;
    let level = |v: Vec2| v.x == 0.0 || v.y == 0.0;
    if level(a.p3 - a.p2) || level(b.p1 - b.p0) {
        return None;
    }
    // The nearest extreme, as (distance, on the incoming curve, parameter, axis).
    let mut best: Option<(f64, bool, f64, Axis)> = None;
    for (incoming, c) in [(true, a), (false, b)] {
        let near = NEAR_EXTREME_DISTANCE.max(NEAR_EXTREME_SHARE * c.p0.distance(c.p3));
        let (qa, qb, qc) = Piece::Cubic(c).deriv_coeffs();
        for axis in [Axis::X, Axis::Y] {
            for t in unit_roots(axis.of(qa), axis.of(qb), axis.of(qc)) {
                let d = c.eval(t).distance(node);
                let this_end = if incoming { t > 0.5 } else { t < 0.5 };
                if this_end && d <= near && best.is_none_or(|(e, ..)| d < e) {
                    best = Some((d, incoming, t, axis));
                }
            }
        }
    }
    let (_, incoming, t, axis) = best?;
    let c = if incoming { a } else { b };
    let (qa, qb, qc) = Piece::Cubic(c).deriv_coeffs();
    let along = qa * (t * t) + qb * t + qc;
    let direction = match axis {
        Axis::X => Vec2::new(0.0, along.y.signum()),
        Axis::Y => Vec2::new(along.x.signum(), 0.0),
    };
    let target = round_to_grid(c.eval(t), grid);
    let sample = |c: CubicBez| (1..=FIT_SAMPLES).map(move |i| c.eval(i as f64 / FIT_SAMPLES as f64));
    let (mut na, mut nb) = if incoming {
        let mut points = vec![target];
        points.extend(sample(a.subsegment(t..1.0)));
        points.extend(sample(b));
        (CubicBez { p3: target, ..a.subsegment(0.0..t) }, fit(&points, direction, unit(b.p3 - b.p2)?)?)
    } else {
        let mut points = vec![a.p0];
        points.extend(sample(a));
        points.extend(sample(b.subsegment(0.0..t)));
        if let Some(last) = points.last_mut() {
            *last = target;
        }
        (fit(&points, unit(a.p1 - a.p0)?, direction)?, CubicBez { p0: target, ..b.subsegment(t..1.0) })
    };
    for p in [&mut na.p1, &mut na.p2, &mut nb.p1, &mut nb.p2] {
        *p = round_to_grid(*p, grid);
    }
    match axis {
        Axis::X => (na.p2.x, nb.p1.x) = (target.x, target.x),
        Axis::Y => (na.p2.y, nb.p1.y) = (target.y, target.y),
    }
    let runs_on = (na.p3 - na.p2).dot(direction) > 0.0 && (nb.p1 - nb.p0).dot(direction) > 0.0;
    let close = within(&[a, b], &[na, nb]) && within(&[na, nb], &[a, b]);
    (runs_on && close).then_some((next, target, na, nb))
}

fn unit(v: Vec2) -> Option<Vec2> {
    (v.hypot() > 1e-9).then(|| v.normalize())
}

/// Do all of `a`'s sampled points lie within [`MAX_EXTREME_SHIFT`] of `b`?
fn within(a: &[CubicBez], b: &[CubicBez]) -> bool {
    let points = |cs: &[CubicBez], n: usize| -> Vec<Point> { cs.iter().flat_map(|c| (0..=n).map(move |i| c.eval(i as f64 / n as f64))).collect() };
    let line = points(b, 2 * FIT_SAMPLES);
    points(a, FIT_SAMPLES).iter().all(|p| line.windows(2).any(|w| matches!(w, [q, r] if dist_to_segment(*p, *q, *r) <= MAX_EXTREME_SHIFT)))
}

/// The cubic from the first of `points` to the last that leaves along `from` and arrives along `to`
/// (unit vectors) and lies closest to the points: least squares over the two handle lengths, with
/// parameters by chord length, improved by Newton steps.
fn fit(points: &[Point], from: Vec2, to: Vec2) -> Option<CubicBez> {
    let (p0, p3) = (*points.first()?, *points.last()?);
    let mut params = Vec::with_capacity(points.len());
    let mut length = 0.0;
    let mut last = p0;
    for p in points {
        length += p.distance(last);
        last = *p;
        params.push(length);
    }
    if !(length.is_finite() && length > 0.0) {
        return None;
    }
    for u in &mut params {
        *u /= length;
    }
    let mut best = None;
    for _ in 0..FIT_ROUNDS {
        let (mut c00, mut c01, mut c11, mut x0, mut x1) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for (p, &u) in points.iter().zip(&params) {
            let v = 1.0 - u;
            let (b0, b1, b2, b3) = (v * v * v, 3.0 * v * v * u, 3.0 * v * u * u, u * u * u);
            let (e0, e1) = (from * b1, to * -b2);
            let rest = p.to_vec2() - p0.to_vec2() * (b0 + b1) - p3.to_vec2() * (b2 + b3);
            c00 += e0.dot(e0);
            c01 += e0.dot(e1);
            c11 += e1.dot(e1);
            x0 += e0.dot(rest);
            x1 += e1.dot(rest);
        }
        let det = c00 * c11 - c01 * c01;
        if !(det.is_finite() && det.abs() > 1e-12) {
            break;
        }
        let (h0, h1) = ((x0 * c11 - x1 * c01) / det, (c00 * x1 - c01 * x0) / det);
        if !(h0.is_finite() && h1.is_finite() && h0 > 0.0 && h1 > 0.0) {
            break;
        }
        let c = CubicBez::new(p0, p0 + from * h0, p3 - to * h1, p3);
        best = Some(c);
        let (qa, qb, qc) = Piece::Cubic(c).deriv_coeffs();
        for (p, u) in points.iter().zip(&mut params) {
            let off = c.eval(*u) - *p;
            let d1 = (qa * (*u * *u) + qb * *u + qc) * 3.0;
            let d2 = (qa * (2.0 * *u) + qb) * 3.0;
            let below = d1.dot(d1) + off.dot(d2);
            if below.abs() > 1e-12 {
                *u = (*u - off.dot(d1) / below).clamp(0.0, 1.0);
            }
        }
    }
    best
}

/// Makes the handles of node `j` exactly horizontal or vertical where one of them is nearly so and
/// its segment has an extreme of that direction close to the node.
fn align_near_extreme(ring: &mut Ring, j: usize) {
    let Some(p) = ring.nodes.get(j).map(|r| r.node.point) else { return };
    let next = ring.next(j);
    let handle_in = ring.seg(j).filter(|s| s.is_curve()).and_then(|s| s.ctrl.last()).map(|h| h.point - p);
    let handle_out = next.and_then(|n| ring.seg(n)).filter(|s| s.is_curve()).and_then(|s| s.ctrl.first()).map(|h| h.point - p);
    let near =
        |v: Option<Vec2>, axis: Axis| v.is_some_and(|v| v.hypot() > 0.0 && (f64::MIN_POSITIVE..=NEAR_EXTREME_DEGREES).contains(&axis.off_by(v)));
    let extreme_near = |seg_end: Option<usize>, axis: Axis| {
        let Some(seg) = seg_end.and_then(|e| ring.seg(e)) else { return false };
        seg.pieces().iter().any(|piece| {
            let (a, b, c) = piece.deriv_coeffs();
            unit_roots(axis.of(a), axis.of(b), axis.of(c)).iter().any(|&t| piece.eval(t).distance(p) <= NEAR_EXTREME_DISTANCE)
        })
    };
    let Some(axis) = [Axis::X, Axis::Y]
        .into_iter()
        .find(|&axis| (near(handle_in, axis) && extreme_near(Some(j), axis)) || (near(handle_out, axis) && extreme_near(next, axis)))
    else {
        return;
    };
    if near(handle_in, axis)
        && let Some(h) = ring.nodes.get_mut(j).and_then(|r| r.ctrl.last_mut())
    {
        axis.align(h, p);
    }
    if near(handle_out, axis)
        && let Some(h) = next.and_then(|n| ring.nodes.get_mut(n)).and_then(|r| r.ctrl.first_mut())
    {
        axis.align(h, p);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    use crate::contour::{NodeKind, contours_from_bez};
    use crate::coverage_difference;
    use fontcraft_geom::BezPath;

    /// A circle of radius 100 around (0, 0) drawn with four cubics whose nodes sit at 45°, so
    /// none of its extremes has a node.
    fn rotated_circle() -> Contour {
        let k = 0.552_284_749_8 * 100.0;
        let dir = |deg: f64| Vec2::new(deg.to_radians().cos(), deg.to_radians().sin());
        let nodes: Vec<Vec2> = (0..4).map(|q| dir(-45.0 + 90.0 * q as f64) * 100.0).collect();
        let mut path = BezPath::new();
        path.move_to(nodes[0].to_point());
        for (q, &pa) in nodes.iter().enumerate() {
            let a = -45.0 + 90.0 * q as f64;
            let pb = nodes[(q + 1) % 4];
            let ta = dir(a + 90.0) * k;
            let tb = dir(a) * k;
            path.curve_to((pa + ta).to_point(), (pb + tb).to_point(), pb.to_point());
        }
        path.close_path();
        contours_from_bez(&path).remove(0)
    }

    #[test]
    fn a_circle_without_extremes_gets_four() {
        let circle = rotated_circle();
        let out = add_extremes(&circle, None, ExtremesOptions { force: false, grid: 1.0 }).unwrap();
        let on: Vec<&Node> = out.nodes.iter().filter(|n| n.on_curve()).collect();
        assert_eq!(on.len(), 8);
        for p in [Point::new(100.0, 0.0), Point::new(0.0, 100.0), Point::new(-100.0, 0.0), Point::new(0.0, -100.0)] {
            let n = on.iter().find(|n| n.point == p).unwrap_or_else(|| panic!("no node at {p:?}: {on:?}"));
            assert!(n.smooth && n.kind == NodeKind::Curve);
        }
        // Each new node's handles are exactly horizontal or vertical.
        let n = out.nodes.len();
        for (i, node) in out.nodes.iter().enumerate() {
            if !node.on_curve() || !(node.point.x == 0.0 || node.point.y == 0.0) {
                continue;
            }
            let before = out.nodes[(i + n - 1) % n].point;
            let after = out.nodes[(i + 1) % n].point;
            if node.point.y == 0.0 {
                assert!(before.x == node.point.x && after.x == node.point.x, "{node:?} {before:?} {after:?}");
            } else {
                assert!(before.y == node.point.y && after.y == node.point.y, "{node:?} {before:?} {after:?}");
            }
        }
        let bez = out.to_bez().unwrap();
        let d = coverage_difference(&[circle.to_bez().unwrap()], &[bez]).unwrap();
        assert!(d.within(0.5, 0.005), "{d:?}");
        // A second run adds nothing.
        assert_eq!(add_extremes(&out, None, ExtremesOptions { force: false, grid: 1.0 }).unwrap(), out);
    }

    #[test]
    fn short_segments_are_skipped_unless_forced() {
        // The curve's top lies within 3 units of its end node: Add Extremes leaves it, Force adds it.
        let path = BezPath::from_svg("M0,0 C0,60 97,102 99,100 L99,0 Z").unwrap();
        let c = contours_from_bez(&path).remove(0);
        let added = add_extremes(&c, None, ExtremesOptions { force: false, grid: 1.0 }).unwrap();
        assert_eq!(added.nodes.iter().filter(|n| n.on_curve()).count(), 3);
        let forced = add_extremes(&c, None, ExtremesOptions { force: true, grid: 0.0 }).unwrap();
        assert_eq!(forced.nodes.iter().filter(|n| n.on_curve()).count(), 4);
        // Less than five units from the end of a 344-unit curve is short; a circle of radius 9 cut at its
        // diagonals gets its extremes, seven units from their neighbours.
        let long = BezPath::from_svg("M0,0 C0,150 270,208 280,200 L280,0 Z").unwrap();
        let c = contours_from_bez(&long).remove(0);
        assert_eq!(add_extremes(&c, None, ExtremesOptions { force: false, grid: 1.0 }).unwrap(), c);
        let mut small = rotated_circle();
        for n in &mut small.nodes {
            n.point = (n.point.to_vec2() * 0.09).to_point();
        }
        let added = add_extremes(&small, None, ExtremesOptions { force: false, grid: 0.0 }).unwrap();
        assert_eq!(added.nodes.iter().filter(|n| n.on_curve()).count(), 8);
    }

    /// An ellipse-like arch whose right half is cut a little before its top, so the node nearest the
    /// top is off it: (0,0) up to a top at (100, 100) and down to (200, 0).
    fn arch_cut_at(t: f64) -> Contour {
        let left = CubicBez::new(Point::new(0.0, 0.0), Point::new(0.0, 55.0), Point::new(45.0, 100.0), Point::new(100.0, 100.0));
        let right = CubicBez::new(Point::new(100.0, 100.0), Point::new(155.0, 100.0), Point::new(200.0, 55.0), Point::new(200.0, 0.0));
        let (short, rest) = (right.subsegment(0.0..t), right.subsegment(t..1.0));
        // The left curve and the first bit of the right one as one curve.
        let mut points = vec![left.p0];
        points.extend((1..=24).map(|i| left.eval(i as f64 / 24.0)));
        points.extend((1..=4).map(|i| short.eval(i as f64 / 4.0)));
        let long = fit(&points, Vec2::new(0.0, 1.0), (short.p3 - short.p2).normalize()).unwrap();
        let mut path = BezPath::new();
        path.move_to(long.p0);
        path.curve_to(long.p1, long.p2, long.p3);
        path.curve_to(rest.p1, rest.p2, rest.p3);
        path.close_path();
        contours_from_bez(&path).remove(0)
    }

    #[test]
    fn a_node_just_off_an_extreme_moves_onto_it() {
        let arch = arch_cut_at(0.01);
        let off = arch.nodes.iter().find(|n| n.on_curve() && n.point.x > 50.0 && n.point.x < 150.0).unwrap().point;
        assert!(off.x > 101.0 && off.x < 103.0, "{off:?}");
        let out = add_extremes(&arch, None, ExtremesOptions { force: false, grid: 1.0 }).unwrap();
        assert_eq!(out.nodes.len(), arch.nodes.len(), "{out:?}");
        let at = out.nodes.iter().position(|n| n.on_curve() && n.point == Point::new(100.0, 100.0)).unwrap_or_else(|| panic!("{out:?}"));
        assert!(out.nodes[at].smooth);
        assert_eq!((out.nodes[at - 1].point.y, out.nodes[at + 1].point.y), (100.0, 100.0));
        let d = coverage_difference(&[arch.to_bez().unwrap()], &[out.to_bez().unwrap()]).unwrap();
        assert!(d.within(0.6, 0.005), "{d:?}");
        // A second run changes nothing.
        assert_eq!(add_extremes(&out, None, ExtremesOptions { force: false, grid: 1.0 }).unwrap(), out);
        // A node further off than 2 units stays, and the extreme gets a node of its own.
        let far = arch_cut_at(0.1);
        let out = add_extremes(&far, None, ExtremesOptions { force: false, grid: 1.0 }).unwrap();
        assert_eq!(out.nodes.len(), far.nodes.len() + 3, "{out:?}");
    }

    #[test]
    fn a_handle_slightly_off_horizontal_is_levelled() {
        // The handle into (100, 100) leans 1° above horizontal, so the curve's top lies just
        // before the node, a corner where a straight segment leaves. The handle is levelled instead
        // of a point being forced in next to the node; the extreme on the right side is added.
        let path = BezPath::from_svg("M0,0 L100,0 C150,0 150,100 100,100 L0,100 Z").unwrap();
        let mut c = contours_from_bez(&path).remove(0);
        // Nodes: (0,0) (100,0) handle handle (100,100) (0,100).
        c.nodes[3].point.y = 100.0 + 1f64.to_radians().tan() * 50.0;
        let out = add_extremes(&c, None, ExtremesOptions { force: true, grid: 1.0 }).unwrap();
        let top = out.nodes.iter().position(|n| n.on_curve() && n.point == Point::new(100.0, 100.0)).unwrap();
        assert_eq!(out.nodes[top - 1].point.y, 100.0);
        assert_eq!(out.nodes.iter().filter(|n| n.on_curve()).count(), 5);
    }
}
