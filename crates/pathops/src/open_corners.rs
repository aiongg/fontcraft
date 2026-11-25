//! Invisible open corners (`openCorner.invisibleWhenSmall`, docs/spec/03a › 13): where a path
//! runs past a corner, turns on a short straight segment and comes back across itself, the small
//! loop beyond the crossing is not part of the shape.
//!
//! The rule and the order of work are ported from glyphsLib's `eraseOpenCorners` filter
//! (`Lib/glyphsLib/filters/eraseOpenCorners.py`, Apache License 2.0), and the crossing of two
//! curves from fontTools' `_curve_curve_intersections_t` (`Lib/fontTools/misc/bezierTools.py`,
//! MIT), whose result the filter cuts at:
//!
//! - The loop is closed by a straight segment, and the path before and after it lies to the right
//!   of that segment, which is the outside of a path drawn in the PostScript direction.
//! - The segments before and after it cross. With the part of each beyond the crossing as a
//!   share of the whole segment, both shares are under a half, or one is under 0.3 and the other
//!   under 0.99. So the loop is small next to what stays visible; a larger one is left as drawn.
//! - Neither share is 0.001 or less: the segments have to cross, not touch at their ends.

use fontcraft_geom::{BezPath, Line, ParamCurve as _, PathEl, PathSeg, Point, Rect, Segment, Shape as _};

use crate::PathOpError;
use crate::contour::{Contour, NodeKind, Piece, RNode, Ring, round_to_grid, split_segment};

/// Two pieces whose boxes are smaller than this, in square units, meet there (fontTools'
/// accuracy).
const MIN_BOX_AREA: f64 = 1e-3;
/// Most pairs of pieces compared for one crossing of two curves; curves that run along each
/// other would otherwise be cut without end.
const MAX_CROSSING_STEPS: usize = 20_000;
/// Most work one call may do, or the calls for one glyph that share a budget: pairs of pieces compared, and what
/// rewriting the path costs. A corner of two curves takes a few hundred; the corners left when it runs out stay.
pub const MAX_CORNER_WORK: usize = 500_000;
/// Segments moved along in the path that count as one step of work.
const SEGMENTS_PER_STEP: usize = 64;
/// Most times a curve is halved for a crossing.
const MAX_HALVINGS: usize = 48;
/// How far apart the two points of a crossing of a curve and a line may be (kurbo reports
/// crossings that are not there: <https://github.com/linebender/kurbo/issues/411>).
const CROSSING_TOLERANCE: f64 = 0.2;
/// Most segments looked at in one path; the rest of the corners of a longer path stay.
const MAX_STEPS: usize = 1_000_000;
/// Samples per piece when looking for the crossing of two segments of any kind.
const CROSSING_SAMPLES: usize = 64;
/// Newton steps that refine a sampled crossing.
const CROSSING_REFINE_STEPS: usize = 12;
/// How close (font units) a refined crossing must bring the two segments to replace the sampled
/// one.
const REFINED_TOLERANCE: f64 = 1e-6;

/// `path` with the invisible open corners of every subpath erased: the two segments end and
/// start at the crossing. `None` when there are none.
pub fn erase_invisible_corners(path: &BezPath) -> Option<BezPath> {
    let mut work = MAX_CORNER_WORK;
    erase_invisible_corners_within(path, &mut work)
}

/// [`erase_invisible_corners`] with the work it may do, which it takes off `work`.
pub fn erase_invisible_corners_within(path: &BezPath, work: &mut usize) -> Option<BezPath> {
    let mut changed = false;
    let mut out = BezPath::new();
    for mut sub in subpaths(path) {
        changed |= erase(&mut sub.segs, sub.closed, work);
        let Some(start) = sub.segs.first().map(PathSeg::start).or(sub.start) else { continue };
        out.move_to(start);
        for seg in &sub.segs {
            out.push(seg.as_path_el());
        }
        if sub.closed {
            out.close_path();
        }
    }
    changed.then_some(out)
}

/// `contour` with its invisible open corners erased, each a corner node at the crossing, rounded
/// to `grid`. `None` when there are none. The work it does is taken off `work` ([`MAX_CORNER_WORK`] for a glyph).
pub fn erase_invisible_corners_in(contour: &Contour, grid: f64, work: &mut usize) -> Result<Option<Contour>, PathOpError> {
    let mut ring = Ring::from_contour(contour)?;
    let mut changed = false;
    let (mut j, mut steps) = (0, 0);
    while j < ring.len() && ring.len() > 2 && steps < MAX_STEPS && *work > 0 {
        steps += 1;
        let len = ring.len();
        if erase_corner(&mut ring, j, grid, work) {
            spend(work, len);
            j = resume(j, len);
            changed = true;
        } else {
            j += 1;
        }
    }
    Ok(changed.then(|| ring.to_contour()))
}

/// Takes the cost of rewriting a path of `len` segments off `work`.
fn spend(work: &mut usize, len: usize) {
    *work = work.saturating_sub(1 + len / SEGMENTS_PER_STEP);
}

/// Where to look next after the corner at `i` of a path of `len` segments went: at the first
/// segment whose neighbours changed. The last segment's neighbour is the first one.
fn resume(i: usize, len: usize) -> usize {
    if i + 1 >= len { 0 } else { i.saturating_sub(2) }
}

/// A subpath: its segments, the closing one included, and where it starts.
struct Subpath {
    segs: Vec<PathSeg>,
    start: Option<Point>,
    closed: bool,
}

fn subpaths(path: &BezPath) -> Vec<Subpath> {
    let mut out: Vec<Subpath> = Vec::new();
    let mut at = Point::ZERO;
    for el in path.elements() {
        if let PathEl::MoveTo(p) = *el {
            out.push(Subpath { segs: Vec::new(), start: Some(p), closed: false });
            at = p;
            continue;
        }
        if out.last().is_some_and(|s| s.closed) {
            out.push(Subpath { segs: Vec::new(), start: Some(at), closed: false });
        }
        let Some(sub) = out.last_mut() else { continue };
        let seg = match *el {
            PathEl::LineTo(p) => PathSeg::Line(Line::new(at, p)),
            PathEl::QuadTo(c, p) => PathSeg::Quad(fontcraft_geom::QuadBez::new(at, c, p)),
            PathEl::CurveTo(a, b, p) => PathSeg::Cubic(fontcraft_geom::CubicBez::new(at, a, b, p)),
            PathEl::ClosePath => {
                sub.closed = true;
                let Some(start) = sub.start else { continue };
                if at == start {
                    continue;
                }
                PathSeg::Line(Line::new(at, start))
            }
            PathEl::MoveTo(_) => continue,
        };
        at = seg.end();
        sub.segs.push(seg);
    }
    out
}

/// Erases the invisible open corners of one subpath's segments.
fn erase(segs: &mut Vec<PathSeg>, closed: bool, work: &mut usize) -> bool {
    let mut changed = false;
    let (mut i, mut steps) = (0, 0);
    while i < segs.len() && segs.len() > 2 && steps < MAX_STEPS && *work > 0 {
        steps += 1;
        match corner_cut(segs, closed, i, work) {
            Some((before, a, after, b)) => {
                if let Some(s) = segs.get_mut(before) {
                    *s = a;
                }
                if let Some(s) = segs.get_mut(after) {
                    *s = b;
                }
                let next = resume(i, segs.len());
                segs.remove(i);
                spend(work, segs.len());
                changed = true;
                i = next;
            }
            None => i += 1,
        }
    }
    changed
}

/// What the neighbours of segment `i` become when it is an invisible open corner: the place and
/// the new form of the one before and of the one after.
fn corner_cut(segs: &[PathSeg], closed: bool, i: usize, work: &mut usize) -> Option<(usize, PathSeg, usize, PathSeg)> {
    let n = segs.len();
    let (before, after) = if closed { ((i + n).checked_sub(1)?.checked_rem(n)?, (i + 1).checked_rem(n)?) } else { (i.checked_sub(1)?, i + 1) };
    let (a, b) = (*segs.get(before)?, *segs.get(after)?);
    if !matches!(segs.get(i)?, PathSeg::Line(_)) {
        return None;
    }
    let (ta, tb) = invisible_corner(a, b, work)?;
    let a = a.subsegment(0.0..ta);
    let (x, end) = (a.end(), b.end());
    let b = match b.subsegment(tb..1.0) {
        PathSeg::Line(_) => PathSeg::Line(Line::new(x, end)),
        PathSeg::Quad(q) => PathSeg::Quad(fontcraft_geom::QuadBez::new(x, q.p1, end)),
        PathSeg::Cubic(c) => PathSeg::Cubic(fontcraft_geom::CubicBez::new(x, c.p1, c.p2, end)),
    };
    Some((before, a, after, b))
}

/// Erases the invisible open corner of `ring` whose straight segment ends at node `j`; false when that is none.
fn erase_corner(ring: &mut Ring, j: usize, grid: f64, work: &mut usize) -> bool {
    let Some(mut changed) = corner_nodes(ring, j, grid, work) else { return false };
    // From the last node to the first, so the places of the ones still to change stay.
    changed.sort_by_key(|(k, _)| std::cmp::Reverse(*k));
    if changed.iter().any(|(k, _)| *k >= ring.nodes.len()) {
        return false;
    }
    for (k, nodes) in changed {
        drop(ring.nodes.splice(k..=k, nodes));
    }
    true
}

/// What the nodes around node `j` become when the straight segment ending there is an invisible open corner: the
/// place of each and the nodes in its stead.
fn corner_nodes(ring: &Ring, j: usize, grid: f64, work: &mut usize) -> Option<[(usize, Vec<RNode>); 3]> {
    let (before, after) = (ring.prev(j)?, ring.next(j)?);
    if ring.nodes.get(j)?.node.kind != NodeKind::Line || ring.seg(j)?.is_curve() || before == after {
        return None;
    }
    let a = ring.seg(before)?.pieces();
    let b = ring.seg(after)?.pieces();
    let last = a.len().checked_sub(1)?;
    // A spline has a piece per off-curve node, all of them made for each look at it.
    *work = work.saturating_sub((a.len() + b.len()) / SEGMENTS_PER_STEP);
    let (ta, tb) = invisible_corner(path_seg(a.last()?), path_seg(b.first()?), work)?;
    let mut head = split_segment(ring, before, &[(last, ta)])?;
    let mut tail = split_segment(ring, after, &[(0, tb)])?;
    if head.len() < 2 || tail.len() < 2 {
        return None;
    }
    head.pop();
    tail.remove(0);
    if let Some(x) = head.last_mut() {
        x.node.point = round_to_grid(x.node.point, grid);
        x.node.smooth = false;
    }
    Some([(before, head), (j, Vec::new()), (after, tail)])
}

fn path_seg(piece: &Piece) -> PathSeg {
    match *piece {
        Piece::Line(a, b) => PathSeg::Line(Line::new(a, b)),
        Piece::Quad(q) => PathSeg::Quad(q),
        Piece::Cubic(c) => PathSeg::Cubic(c),
    }
}

/// Where `a` and `b`, the segments before and after a straight one, cross, as their parameters,
/// when the loop beyond the crossing is an invisible open corner.
fn invisible_corner(a: PathSeg, b: PathSeg, work: &mut usize) -> Option<(f64, f64)> {
    let line = Line::new(a.end(), b.start());
    let before = match a {
        PathSeg::Line(l) => l.p0,
        PathSeg::Quad(q) => q.p1,
        PathSeg::Cubic(c) => c.p2,
    };
    let after = match b {
        PathSeg::Line(l) => l.p1,
        PathSeg::Quad(q) => q.p1,
        PathSeg::Cubic(c) => c.p1,
    };
    let right = |p: Point| (line.p1 - line.p0).cross(p - line.p0) < 0.0;
    if !(right(before) && right(after)) {
        return None;
    }
    let (ta, tb) = crossing(a, b, work)?;
    // The share of each segment beyond the crossing.
    let (beyond_a, beyond_b) = (1.0 - ta, tb);
    let small = (beyond_a < 0.5 && beyond_b < 0.5) || (beyond_a < 0.3 && beyond_b < 0.99) || (beyond_a < 0.99 && beyond_b < 0.3);
    (small && beyond_a > 0.001 && beyond_b > 0.001).then_some((ta, tb))
}

/// The first crossing of two segments, as their parameters: of a curve and a line the one
/// nearest the curve's start.
fn crossing(a: PathSeg, b: PathSeg, work: &mut usize) -> Option<(f64, f64)> {
    match (a, b) {
        (PathSeg::Line(a), PathSeg::Line(b)) => line_crossing(a.p0, a.p1, b.p0, b.p1),
        (curve, PathSeg::Line(line)) => curve_line(curve, line),
        (PathSeg::Line(line), curve) => curve_line(curve, line).map(|(t, u)| (u, t)),
        (a, b) => curve_curve(a, b, work),
    }
}

fn curve_line(curve: PathSeg, line: Line) -> Option<(f64, f64)> {
    curve
        .intersect_line(line)
        .iter()
        .map(|hit| (hit.segment_t, hit.line_t))
        .filter(|(t, u)| (0.0..=1.0).contains(t) && (0.0..=1.0).contains(u) && (curve.eval(*t) - line.eval(*u)).hypot() < CROSSING_TOLERANCE)
        .min_by(|x, y| x.0.total_cmp(&y.0))
}

fn curve_curve(a: PathSeg, b: PathSeg, work: &mut usize) -> Option<(f64, f64)> {
    let allowed = MAX_CROSSING_STEPS.min(*work);
    let mut left = allowed;
    let found = meet(a, b, (0.0, 1.0), (0.0, 1.0), 0, &mut left);
    *work = work.saturating_sub(allowed.saturating_sub(left));
    found
}

/// The first place two pieces of curves meet, by halving both until their boxes are tiny; `left` is how many more
/// pairs of pieces may be compared.
fn meet(a: PathSeg, b: PathSeg, ra: (f64, f64), rb: (f64, f64), depth: usize, left: &mut usize) -> Option<(f64, f64)> {
    *left = left.checked_sub(1)?;
    if depth > MAX_HALVINGS {
        return None;
    }
    let (box_a, box_b) = (a.bounding_box(), b.bounding_box());
    if !share_area(box_a, box_b) {
        return None;
    }
    let mid = |r: (f64, f64)| 0.5 * (r.0 + r.1);
    if box_a.area() < MIN_BOX_AREA && box_b.area() < MIN_BOX_AREA {
        return Some((mid(ra), mid(rb)));
    }
    let halves = |s: PathSeg, r: (f64, f64)| [(s.subsegment(0.0..0.5), (r.0, mid(r))), (s.subsegment(0.5..1.0), (mid(r), r.1))];
    let (pa, pb) = (halves(a, ra), halves(b, rb));
    pb.iter().find_map(|(b, rb)| pa.iter().find_map(|(a, ra)| meet(*a, *b, *ra, *rb, depth + 1, &mut *left)))
}

fn share_area(a: Rect, b: Rect) -> bool {
    a.x0.max(b.x0) < a.x1.min(b.x1) && a.y0.max(b.y0) < a.y1.min(b.y1)
}

// ── Open corners of any kind, as the audit and its fix find them ────────────────────────────────

/// An open corner of a path: the segments before, between and after, by place in the segment
/// list, and where the outer two cross.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OpenCorner {
    pub a: usize,
    pub mid: usize,
    pub b: usize,
    pub ta: f64,
    pub tb: f64,
}

/// The open corners of a path with the segments `segs`, in segment order: segments of any kind
/// that cross in the later half of the first and the earlier half of the second, whichever side
/// of the shape the loop is on.
pub fn find_open_corners(closed: bool, segs: &[Segment]) -> Vec<OpenCorner> {
    let n = segs.len();
    let enough = if closed { n >= 4 } else { n >= 3 };
    if !enough {
        return Vec::new();
    }
    let last = if closed { n } else { n.saturating_sub(2) };
    let mut out = Vec::new();
    for k in 0..last {
        let (a, mid, b) = (k, (k + 1) % n, (k + 2) % n);
        let (Some(sa), Some(sm), Some(sb)) = (segs.get(a), segs.get(mid), segs.get(b)) else { continue };
        if sa.end != sm.start || sm.end != sb.start {
            continue;
        }
        let Some((ta, tb)) = segment_crossing(sa, sb) else { continue };
        // The loop beyond the crossing is the smaller part of both segments.
        if ta > 0.5 && tb < 0.5 {
            out.push(OpenCorner { a, mid, b, ta, tb });
        }
    }
    out
}

/// Where segments `a` and `b` cross, as their parameters, both strictly inside. Touching ends (a
/// middle segment of length zero) do not count.
fn segment_crossing(a: &Segment, b: &Segment) -> Option<(f64, f64)> {
    let (pa, pb) = (polyline(a), polyline(b));
    for w in pa.windows(2) {
        let (Some(&(t0, p0)), Some(&(t1, p1))) = (w.first(), w.get(1)) else { continue };
        for x in pb.windows(2) {
            let (Some(&(u0, q0)), Some(&(u1, q1))) = (x.first(), x.get(1)) else { continue };
            if !boxes_touch(p0, p1, q0, q1) {
                continue;
            }
            let Some((s, r)) = line_crossing(p0, p1, q0, q1) else { continue };
            let (ta, tb) = (t0 + (t1 - t0) * s, u0 + (u1 - u0) * r);
            let (ta, tb) = refine(a, b, ta, tb).unwrap_or((ta, tb));
            if ta > 0.0 && ta < 1.0 && tb > 0.0 && tb < 1.0 {
                return Some((ta, tb));
            }
        }
    }
    None
}

/// A segment as points with their parameters.
fn polyline(seg: &Segment) -> Vec<(f64, Point)> {
    let steps = CROSSING_SAMPLES.saturating_mul(seg.pieces.len().max(1));
    (0..=steps)
        .filter_map(|i| {
            let t = i as f64 / steps as f64;
            seg.eval(t).filter(|p| p.is_finite()).map(|p| (t, p))
        })
        .collect()
}

fn boxes_touch(p0: Point, p1: Point, q0: Point, q1: Point) -> bool {
    p0.x.max(p1.x) >= q0.x.min(q1.x) && q0.x.max(q1.x) >= p0.x.min(p1.x) && p0.y.max(p1.y) >= q0.y.min(q1.y) && q0.y.max(q1.y) >= p0.y.min(p1.y)
}

/// Where lines p0–p1 and q0–q1 cross, as fractions along each; parallel lines do not cross.
fn line_crossing(p0: Point, p1: Point, q0: Point, q1: Point) -> Option<(f64, f64)> {
    let (d, e) = (p1 - p0, q1 - q0);
    let den = d.cross(e);
    if den == 0.0 || !den.is_finite() {
        return None;
    }
    let w = q0 - p0;
    let (s, r) = (w.cross(e) / den, w.cross(d) / den);
    ((0.0..=1.0).contains(&s) && (0.0..=1.0).contains(&r)).then_some((s, r))
}

/// Newton steps on a(ta) = b(tb), from the sampled crossing; `None` when they do not settle.
fn refine(a: &Segment, b: &Segment, mut ta: f64, mut tb: f64) -> Option<(f64, f64)> {
    const H: f64 = 1e-7;
    for _ in 0..CROSSING_REFINE_STEPS {
        let f = a.eval(ta)? - b.eval(tb)?;
        if f.hypot() <= REFINED_TOLERANCE {
            return Some((ta, tb));
        }
        let da = (a.eval((ta + H).min(1.0))? - a.eval((ta - H).max(0.0))?) / (2.0 * H);
        let db = (b.eval((tb + H).min(1.0))? - b.eval((tb - H).max(0.0))?) / (2.0 * H);
        // Solve [da, -db] · [dta, dtb] = -f.
        let det = da.cross(-db);
        if det == 0.0 || !det.is_finite() {
            return None;
        }
        let dta = (-f).cross(-db) / det;
        let dtb = da.cross(-f) / det;
        ta += dta;
        tb += dtb;
        if !(ta.is_finite() && tb.is_finite()) || !(0.0..=1.0).contains(&ta) || !(0.0..=1.0).contains(&tb) {
            return None;
        }
    }
    let f = a.eval(ta)? - b.eval(tb)?;
    (f.hypot() <= REFINED_TOLERANCE).then_some((ta, tb))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    use crate::contour::{Node, contours_from_bez};

    /// A closed path through `points`, counter-clockwise.
    fn polygon(points: &[(f64, f64)]) -> BezPath {
        let mut p = BezPath::new();
        for (i, q) in points.iter().enumerate() {
            if i == 0 { p.move_to(*q) } else { p.line_to(*q) }
        }
        p.close_path();
        p
    }

    /// The on-curve points of `path`, rounded.
    fn corners(path: &BezPath) -> Vec<(i32, i32)> {
        path.elements().iter().filter_map(PathEl::end_point).map(|p| (p.x.round() as i32, p.y.round() as i32)).collect()
    }

    /// A 100 × 100 square whose top right corner is open by `ear` units on both sides.
    fn eared(ear: f64) -> BezPath {
        polygon(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0 + ear), (100.0 + ear, 100.0), (0.0, 100.0)])
    }

    #[test]
    fn a_small_ear_goes_and_a_large_one_stays() {
        let out = erase_invisible_corners(&eared(40.0)).unwrap();
        assert_eq!(corners(&out), [(0, 0), (100, 0), (100, 100), (0, 100), (0, 0)]);
        assert_eq!(out.elements().last(), Some(&PathEl::ClosePath));
        // The loop's legs are longer than the visible sides.
        assert!(erase_invisible_corners(&eared(150.0)).is_none());
        // A loop on the inside (the path turned round) is no outward ear.
        assert!(erase_invisible_corners(&eared(40.0).reverse_subpaths()).is_none());
        // A plain square has none, and open paths and empty ones pass.
        assert!(erase_invisible_corners(&polygon(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)])).is_none());
        assert!(erase_invisible_corners(&BezPath::new()).is_none());

        // The same as nodes: the two corner nodes become one, the others keep their tags.
        let c = contours_from_bez(&eared(40.0)).remove(0).tagged();
        let out = erase_invisible_corners_in(&c, 1.0, &mut { MAX_CORNER_WORK }).unwrap().unwrap();
        let points: Vec<(f64, f64, Option<usize>)> = out.nodes.iter().map(|n| (n.point.x, n.point.y, n.tag)).collect();
        assert_eq!(points, [(0.0, 0.0, Some(0)), (100.0, 0.0, Some(1)), (100.0, 100.0, None), (0.0, 100.0, Some(4))]);
    }

    #[test]
    fn corners_on_curves_and_at_the_start_are_found() {
        // Two curves joined by a straight segment.
        let mut p = BezPath::new();
        p.move_to((400.0, 0.0));
        p.curve_to((400.0, 100.0), (450.0, 300.0), (300.0, 300.0));
        p.line_to((200.0, 100.0));
        p.curve_to((250.0, 100.0), (450.0, 150.0), (450.0, 50.0));
        p.line_to((400.0, 0.0));
        p.close_path();
        let out = erase_invisible_corners(&p).unwrap();
        // As the filter cuts it: the closing segment goes, and the path starts at the crossing.
        assert_eq!(corners(&out), [(406, 105), (300, 300), (200, 100), (406, 105)]);

        // The straight segment is the first one: the path then starts at the crossing.
        let first = polygon(&[(100.0, 140.0), (140.0, 100.0), (0.0, 100.0), (0.0, 0.0), (100.0, 0.0)]);
        let out = erase_invisible_corners(&first).unwrap();
        assert_eq!(corners(&out), [(100, 100), (0, 100), (0, 0), (100, 0), (100, 100)]);

        // Two off-curve nodes before the ear: the spline keeps its shape up to the crossing.
        let on = |x: f64, y: f64, kind| Node::new(Point::new(x, y), kind);
        let off = |x: f64, y: f64| Node::off(Point::new(x, y));
        let c = Contour::new(
            true,
            vec![
                on(0.0, 0.0, NodeKind::Line),
                on(60.0, 0.0, NodeKind::Line),
                off(100.0, 0.0),
                off(100.0, 60.0),
                on(100.0, 140.0, NodeKind::QCurve),
                on(140.0, 100.0, NodeKind::Line),
                on(0.0, 100.0, NodeKind::Line),
            ],
        );
        let out = erase_invisible_corners_in(&c, 1.0, &mut { MAX_CORNER_WORK }).unwrap().unwrap();
        let on_curve: Vec<Point> = out.nodes.iter().filter(|n| n.on_curve()).map(|n| n.point).collect();
        assert!(on_curve.contains(&Point::new(100.0, 100.0)) && on_curve.contains(&Point::new(100.0, 30.0)), "{on_curve:?}");
        assert!(!on_curve.contains(&Point::new(140.0, 100.0)));
    }

    #[test]
    fn curves_along_each_other_and_wild_coordinates_end() {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.curve_to((300_000.0, 900_000.0), (600_000.0, 900_000.0), (900_000.0, 0.0));
        p.line_to((900_000.0, 0.5));
        p.curve_to((600_000.0, 900_000.0), (300_000.0, 900_000.0), (0.0, 0.25));
        p.close_path();
        let _ = erase_invisible_corners(&p);
        let mut nan = eared(40.0);
        nan.line_to((f64::NAN, 1.0));
        nan.close_path();
        nan.move_to((f64::INFINITY, 0.0));
        nan.curve_to((1.0, f64::NAN), (2.0, 3.0), (4.0, 5.0));
        let _ = erase_invisible_corners(&nan);
    }

    #[test]
    fn the_work_for_one_call_is_limited_as_a_whole() {
        // Each pair of curves runs along the other without meeting it.
        let mut p = BezPath::new();
        for _ in 0..100 {
            p.move_to((0.0, 0.25));
            p.curve_to((300_000.0, 900_000.0), (600_000.0, 900_000.0), (900_000.0, 0.5));
            p.line_to((900_000.0, 0.0));
            p.curve_to((600_000.0, 900_000.0), (300_000.0, 900_000.0), (0.0, 0.0));
            p.close_path();
        }
        let mut work = MAX_CORNER_WORK;
        assert!(erase_invisible_corners_within(&p, &mut work).is_none());
        assert_eq!(work, 0);
        // Nothing is left for a corner after them; on its own it goes.
        p.extend(eared(40.0).elements().iter().copied());
        assert!(erase_invisible_corners(&p).is_none());
        assert!(erase_invisible_corners(&eared(40.0)).is_some());
        // Many corners on one contour, as nodes.
        let mut comb = vec![(0.0, -500.0)];
        for i in 0..2_000 {
            let x = f64::from(i) * 300.0;
            comb.extend([(x, 0.0), (x + 100.0, 0.0), (x + 100.0, 140.0), (x + 140.0, 100.0), (x, 100.0)]);
        }
        comb.push((600_000.0, -500.0));
        let contour = contours_from_bez(&polygon(&comb)).remove(0);
        let mut work = MAX_CORNER_WORK;
        let out = erase_invisible_corners_in(&contour, 1.0, &mut work).unwrap().unwrap();
        assert_eq!(out.nodes.len(), contour.nodes.len() - 2_000);
        assert!(work > 0);
    }
}
