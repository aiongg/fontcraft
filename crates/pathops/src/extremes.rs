//! Add Extremes and Force Extremes (`path.addExtremes`, `path.forceExtremes`,
//! `extremes.definitions`, docs/spec/03a › question 17).

use std::collections::HashSet;

use fontcraft_geom::{Point, Vec2};

use crate::PathOpError;
use crate::contour::{Contour, Node, Piece, RNode, Ring, round_to_grid, split_segment, unit_roots};

/// Add Extremes skips an extreme that would leave a segment shorter than this many units (the
/// `shortSegments` threshold).
pub const MIN_EXTREME_SEGMENT: f64 = 3.0;
/// A handle within this many degrees of horizontal or vertical, at a node within
/// [`NEAR_EXTREME_DISTANCE`] of an extreme, is made exactly horizontal or vertical (the
/// `almostOrthogonalHandle` thresholds).
pub const NEAR_EXTREME_DEGREES: f64 = 2.0;
pub const NEAR_EXTREME_DISTANCE: f64 = 2.0;
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
/// First, a node whose handle is within 2° of horizontal or vertical, and that has an extreme of
/// that direction within 2 units on the handle's segment, has its handles that are within 2° made
/// exactly horizontal or vertical: the extreme moves onto the node and no point is added next to
/// it. Then a node goes at each point inside a curve where the tangent is horizontal or vertical.
/// Add Extremes skips an extreme closer than 3 units to an end of its segment or to the extreme
/// before it (distances along the chord); Force Extremes adds every extreme that is a new point.
/// New nodes are smooth and rounded to `grid`, and their handles are lined up with them so the
/// tangent stays exactly horizontal or vertical. A quadratic spline that gets a node has each of
/// its implied on-curve points made a node.
pub fn add_extremes(contour: &Contour, selected: Option<&[usize]>, options: ExtremesOptions) -> Result<Contour, PathOpError> {
    let mut ring = Ring::from_contour(contour)?;
    let set: Option<HashSet<usize>> = selected.map(|s| s.iter().copied().collect());
    let chosen = |r: &RNode| set.as_ref().is_none_or(|s| r.src.is_some_and(|i| s.contains(&i)));

    for j in 0..ring.len() {
        if ring.nodes.get(j).is_some_and(chosen) {
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
    let min = if options.force { SAME_POINT } else { MIN_EXTREME_SEGMENT };
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
    }

    #[test]
    fn a_handle_slightly_off_horizontal_is_levelled() {
        // The handle into (100, 100) leans 1° above horizontal, so the curve's top lies just
        // before the node. The handle is levelled instead of a point being forced in next to the
        // node; the extreme on the right side is added.
        let path = BezPath::from_svg("M0,0 L100,0 C150,0 150,100 100,100 C50,100 0,50 0,0 Z").unwrap();
        let mut c = contours_from_bez(&path).remove(0);
        // Nodes: (0,0) (100,0) handle handle (100,100) handle handle.
        c.nodes[3].point.y = 100.0 + 1f64.to_radians().tan() * 50.0;
        let out = add_extremes(&c, None, ExtremesOptions { force: true, grid: 1.0 }).unwrap();
        let top = out.nodes.iter().position(|n| n.on_curve() && n.point == Point::new(100.0, 100.0)).unwrap();
        assert_eq!(out.nodes[top - 1].point.y, 100.0);
        assert_eq!(out.nodes.iter().filter(|n| n.on_curve()).count(), 4);
    }
}
