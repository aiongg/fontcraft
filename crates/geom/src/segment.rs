//! The segments of a node list, with the addresses of their nodes, and what editing commands
//! measure on them: the nearest point, a split, extremes, inflections and bounds.
//!
//! A node list follows the source model: off-curve nodes come before the on-curve node that ends
//! their segment (two before a cubic node, one or more before a quadratic node); in a closed list
//! the segment that ends at the first on-curve node wraps around from the last nodes. A segment is
//! addressed by the index of the on-curve node that ends it.
//!
//! Every function accepts any input, NaN and malformed lists included, and answers without
//! panicking; the work for one segment is bounded whatever its coordinates.

use kurbo::{CubicBez, Line, ParamCurve, ParamCurveExtrema, Point, QuadBez, Rect};

/// Most off-curve nodes one segment may have. A longer run is a malformed list and its segment
/// is left out.
pub const MAX_SEGMENT_HANDLES: usize = 10_000;
/// Evenly spaced samples per piece before the nearest point is refined.
const SAMPLES: usize = 16;
/// Ternary-search steps that refine the nearest point between two samples.
const REFINE_STEPS: usize = 32;

/// What a node is, as far as the segment structure goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeType {
    /// On-curve, ends a straight segment.
    Line,
    /// On-curve, ends a cubic segment (two off-curve nodes before it).
    Curve,
    /// On-curve, ends a quadratic segment (one or more off-curve nodes before it).
    QCurve,
    /// A handle.
    OffCurve,
}

/// One node of a list: where it is and what it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NodePoint {
    pub point: Point,
    pub kind: NodeType,
}

impl NodePoint {
    pub fn new(point: Point, kind: NodeType) -> Self {
        Self { point, kind }
    }
}

/// One piece of a segment's curve. A quadratic segment with several off-curve nodes has one
/// piece per off-curve node; a malformed run of nodes becomes straight pieces through them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Piece {
    Line(Line),
    Quad(QuadBez),
    Cubic(CubicBez),
}

impl Piece {
    pub fn eval(&self, t: f64) -> Point {
        match self {
            Piece::Line(l) => l.eval(t),
            Piece::Quad(q) => q.eval(t),
            Piece::Cubic(c) => c.eval(t),
        }
    }

    fn finite(&self) -> bool {
        let pts: &[Point] = match self {
            Piece::Line(l) => &[l.p0, l.p1],
            Piece::Quad(q) => &[q.p0, q.p1, q.p2],
            Piece::Cubic(c) => &[c.p0, c.p1, c.p2, c.p3],
        };
        pts.iter().all(|p| crate::is_finite(*p))
    }

    /// The nearest point to `p` as `(distance, t)`: sampled, then refined, so the work is fixed
    /// whatever the input. `None` when a coordinate is not finite.
    pub fn nearest(&self, p: Point) -> Option<(f64, f64)> {
        if !self.finite() || !crate::is_finite(p) {
            return None;
        }
        let mut best = (f64::INFINITY, 0.0);
        for i in 0..=SAMPLES {
            let t = i as f64 / SAMPLES as f64;
            let d = self.eval(t).distance(p);
            if d < best.0 {
                best = (d, t);
            }
        }
        let step = 1.0 / SAMPLES as f64;
        let (mut lo, mut hi) = ((best.1 - step).max(0.0), (best.1 + step).min(1.0));
        for _ in 0..REFINE_STEPS {
            let m1 = lo + (hi - lo) / 3.0;
            let m2 = hi - (hi - lo) / 3.0;
            if self.eval(m1).distance(p) < self.eval(m2).distance(p) {
                hi = m2;
            } else {
                lo = m1;
            }
        }
        let t = (lo + hi) / 2.0;
        let d = self.eval(t).distance(p);
        let found = if d < best.0 { (d, t) } else { best };
        found.0.is_finite().then_some(found)
    }

    /// The piece cut at `t` (clamped to 0..1) into the part before and the part after.
    pub fn split(&self, t: f64) -> (Piece, Piece) {
        let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.5 };
        match self {
            Piece::Line(l) => (Piece::Line(l.subsegment(0.0..t)), Piece::Line(l.subsegment(t..1.0))),
            Piece::Quad(q) => (Piece::Quad(q.subsegment(0.0..t)), Piece::Quad(q.subsegment(t..1.0))),
            Piece::Cubic(c) => (Piece::Cubic(c.subsegment(0.0..t)), Piece::Cubic(c.subsegment(t..1.0))),
        }
    }

    /// The parameters strictly inside the piece where its tangent is horizontal or vertical,
    /// in increasing order.
    pub fn extremes(&self) -> Vec<f64> {
        if !self.finite() {
            return Vec::new();
        }
        let mut out: Vec<f64> = match self {
            Piece::Line(_) => Vec::new(),
            Piece::Quad(q) => q.extrema().into_iter().collect(),
            Piece::Cubic(c) => c.extrema().into_iter().collect(),
        };
        out.retain(|t| *t > 0.0 && *t < 1.0);
        out.sort_by(f64::total_cmp);
        out.dedup();
        out
    }

    /// The parameters strictly inside the piece where its bend changes direction. Only a cubic
    /// piece has any.
    pub fn inflections(&self) -> Vec<f64> {
        let Piece::Cubic(c) = self else { return Vec::new() };
        if !self.finite() {
            return Vec::new();
        }
        let mut out: Vec<f64> = c.inflections().into_iter().filter(|t| *t > 0.0 && *t < 1.0).collect();
        out.sort_by(f64::total_cmp);
        out.dedup();
        out
    }

    /// The tight box around the curve. `None` when a coordinate is not finite.
    pub fn bounds(&self) -> Option<Rect> {
        if !self.finite() {
            return None;
        }
        Some(match self {
            Piece::Line(l) => l.bounding_box(),
            Piece::Quad(q) => q.bounding_box(),
            Piece::Cubic(c) => c.bounding_box(),
        })
    }
}

/// A segment: from the on-curve node `start` to the on-curve node `end`, through the off-curve
/// nodes `handles`, all as indices into the node list.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub start: usize,
    pub end: usize,
    pub handles: Vec<usize>,
    pub pieces: Vec<Piece>,
}

/// The nearest point of a segment to a given point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Nearest {
    pub distance: f64,
    /// From 0 at `start` to 1 at `end`, across all pieces.
    pub t: f64,
    pub point: Point,
}

impl Segment {
    /// A straight segment (no off-curve nodes).
    pub fn is_line(&self) -> bool {
        self.handles.is_empty()
    }

    /// Which piece holds the segment parameter `t`, and the parameter inside it.
    fn locate(&self, t: f64) -> Option<(usize, f64)> {
        let n = self.pieces.len();
        if n == 0 {
            return None;
        }
        let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { return None };
        let scaled = t * n as f64;
        let i = (scaled.floor() as usize).min(n - 1);
        Some((i, scaled - i as f64))
    }

    /// The point at `t`, 0 at `start` and 1 at `end`.
    pub fn eval(&self, t: f64) -> Option<Point> {
        let (i, local) = self.locate(t)?;
        self.pieces.get(i).map(|p| p.eval(local))
    }

    /// The point of the segment nearest to `p`.
    pub fn nearest(&self, p: Point) -> Option<Nearest> {
        let count = self.pieces.len() as f64;
        let mut best: Option<Nearest> = None;
        for (i, piece) in self.pieces.iter().enumerate() {
            let Some((d, t)) = piece.nearest(p) else { continue };
            if best.is_none_or(|b| d < b.distance) {
                best = Some(Nearest { distance: d, t: (i as f64 + t) / count, point: piece.eval(t) });
            }
        }
        best
    }

    /// The segment cut at `t`: the pieces before and the pieces after.
    pub fn split(&self, t: f64) -> Option<(Vec<Piece>, Vec<Piece>)> {
        let (i, local) = self.locate(t)?;
        let piece = self.pieces.get(i)?;
        let (a, b) = piece.split(local);
        let mut before: Vec<Piece> = self.pieces.iter().take(i).copied().collect();
        before.push(a);
        let mut after = vec![b];
        after.extend(self.pieces.iter().skip(i + 1).copied());
        Some((before, after))
    }

    /// Segment parameters where the tangent is horizontal or vertical, inside the segment.
    pub fn extremes(&self) -> Vec<f64> {
        self.across(Piece::extremes)
    }

    /// Segment parameters where the bend changes direction, inside the segment.
    pub fn inflections(&self) -> Vec<f64> {
        self.across(Piece::inflections)
    }

    fn across(&self, f: fn(&Piece) -> Vec<f64>) -> Vec<f64> {
        let n = self.pieces.len() as f64;
        self.pieces.iter().enumerate().flat_map(|(i, p)| f(p).into_iter().map(move |t| (i as f64 + t) / n)).collect()
    }

    /// The tight box around the segment's curve.
    pub fn bounds(&self) -> Option<Rect> {
        let mut out: Option<Rect> = None;
        for p in &self.pieces {
            let b = p.bounds()?;
            out = Some(out.map_or(b, |o| o.union(b)));
        }
        out
    }
}

/// The segments of a node list, in path order. A closed list's segment that wraps around comes
/// where its end node is in the walk from the first on-curve node. Segments with more than
/// [`MAX_SEGMENT_HANDLES`] handles are left out; a list without an on-curve node has none.
pub fn segments(nodes: &[NodePoint], closed: bool) -> Vec<Segment> {
    let count = nodes.len();
    let mut out = Vec::new();
    let Some(first) = nodes.iter().position(|n| n.kind != NodeType::OffCurve) else { return out };
    let steps = if closed { count } else { count.saturating_sub(first + 1) };
    let mut prev = first;
    let mut pending: Vec<usize> = Vec::new();
    let mut overflow = false;
    for step in 1..=steps {
        let idx = (first + step) % count;
        let Some(node) = nodes.get(idx) else { continue };
        if node.kind == NodeType::OffCurve {
            if pending.len() < MAX_SEGMENT_HANDLES {
                pending.push(idx);
            } else {
                overflow = true;
            }
            continue;
        }
        if !overflow && let Some(seg) = build(nodes, prev, idx, &pending) {
            out.push(seg);
        }
        pending.clear();
        overflow = false;
        prev = idx;
    }
    out
}

/// The segment that ends at node `end`, if that node ends one.
pub fn segment_ending_at(nodes: &[NodePoint], closed: bool, end: usize) -> Option<Segment> {
    if nodes.get(end)?.kind == NodeType::OffCurve {
        return None;
    }
    segments(nodes, closed).into_iter().find(|s| s.end == end)
}

fn build(nodes: &[NodePoint], start: usize, end: usize, handles: &[usize]) -> Option<Segment> {
    let s = nodes.get(start)?.point;
    let end_node = nodes.get(end)?;
    let e = end_node.point;
    let hs: Vec<Point> = handles.iter().filter_map(|&i| nodes.get(i).map(|n| n.point)).collect();
    let mut pieces = Vec::new();
    match (end_node.kind, hs.as_slice()) {
        (_, []) => pieces.push(Piece::Line(Line::new(s, e))),
        (NodeType::Curve, [a, b]) => pieces.push(Piece::Cubic(CubicBez::new(s, *a, *b, e))),
        (NodeType::QCurve, _) => {
            let mut from = s;
            for (k, c) in hs.iter().enumerate() {
                let to = match hs.get(k + 1) {
                    Some(next) => c.midpoint(*next),
                    None => e,
                };
                pieces.push(Piece::Quad(QuadBez::new(from, *c, to)));
                from = to;
            }
        }
        _ => {
            let mut from = s;
            for p in hs.iter().chain(std::iter::once(&e)) {
                pieces.push(Piece::Line(Line::new(from, *p)));
                from = *p;
            }
        }
    }
    Some(Segment { start, end, handles: handles.to_vec(), pieces })
}

/// The box around a set of points (a selection's nodes and handles, anchors…). Points that are
/// not finite are left out; `None` when none is left.
pub fn bounds(points: impl IntoIterator<Item = Point>) -> Option<Rect> {
    let mut out: Option<Rect> = None;
    for p in points.into_iter().filter(|p| crate::is_finite(*p)) {
        out = Some(match out {
            None => Rect::from_points(p, p),
            Some(r) => r.union_pt(p),
        });
    }
    out
}

/// The grid step for a grid spacing and its subdivision: spacing ÷ subdivision. 0 means no
/// rounding: a spacing of 0, or a value that is not a positive finite number. A subdivision that
/// is not a positive finite number counts as 1.
pub fn grid_step(spacing: f64, subdivision: f64) -> f64 {
    if !(spacing.is_finite() && spacing > 0.0) {
        return 0.0;
    }
    let sub = if subdivision.is_finite() && subdivision >= 1.0 { subdivision } else { 1.0 };
    let step = spacing / sub;
    if step.is_finite() && step > 0.0 { step } else { 0.0 }
}

/// `v` rounded to the nearest multiple of `step` (see [`grid_step`]). A step of 0, or a value
/// that is not finite, leaves `v` as it is.
pub fn round_to_grid(v: f64, step: f64) -> f64 {
    if !(step.is_finite() && step > 0.0) || !v.is_finite() {
        return v;
    }
    let r = (v / step).round() * step;
    // Keep a step like 0.1 from leaving 0.30000000000000004 behind.
    let r = if step < 1.0 { (r * 1e9).round() / 1e9 } else { r };
    if r.is_finite() { r } else { v }
}

/// Both coordinates of `p` rounded to the grid.
pub fn round_point(p: Point, step: f64) -> Point {
    Point::new(round_to_grid(p.x, step), round_to_grid(p.y, step))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(x: f64, y: f64, kind: NodeType) -> NodePoint {
        NodePoint::new(Point::new(x, y), kind)
    }

    /// A closed path: a line, a cubic, a two-handle quadratic, and the wrapping line.
    fn sample() -> Vec<NodePoint> {
        use NodeType::*;
        vec![
            n(0.0, 0.0, Line),
            n(100.0, 0.0, Line),
            n(150.0, 0.0, OffCurve),
            n(200.0, 50.0, OffCurve),
            n(200.0, 100.0, Curve),
            n(200.0, 150.0, OffCurve),
            n(100.0, 200.0, OffCurve),
            n(0.0, 200.0, QCurve),
        ]
    }

    #[test]
    fn segments_carry_their_addresses() {
        let segs = segments(&sample(), true);
        let ends: Vec<(usize, usize, Vec<usize>, usize)> = segs.iter().map(|s| (s.start, s.end, s.handles.clone(), s.pieces.len())).collect();
        assert_eq!(ends, vec![(0, 1, vec![], 1), (1, 4, vec![2, 3], 1), (4, 7, vec![5, 6], 2), (7, 0, vec![], 1)]);
        // Open: no wrapping segment, and the first node ends nothing.
        assert_eq!(segments(&sample(), false).len(), 3);
        assert!(segment_ending_at(&sample(), false, 0).is_none());
        assert!(segment_ending_at(&sample(), true, 0).is_some());
        assert!(segment_ending_at(&sample(), true, 2).is_none());
        // Malformed and hostile lists answer without panicking.
        assert!(segments(&[], true).is_empty());
        assert!(segments(&[n(1.0, 1.0, NodeType::OffCurve)], true).is_empty());
        let bad = vec![n(f64::NAN, 0.0, NodeType::Line), n(1.0, f64::INFINITY, NodeType::Curve)];
        for s in segments(&bad, true) {
            assert!(s.nearest(Point::new(0.0, 0.0)).is_none());
            assert!(s.bounds().is_none());
            assert!(s.extremes().is_empty());
        }
    }

    #[test]
    fn nearest_split_extremes_and_inflections() {
        let segs = segments(&sample(), true);
        let cubic = &segs[1];
        let near = cubic.nearest(Point::new(250.0, 100.0)).unwrap();
        assert!(near.point.distance(Point::new(200.0, 100.0)) < 1e-3, "{near:?}");
        assert!((near.t - 1.0).abs() < 1e-3);
        // A split keeps both ends and meets in the middle.
        let (a, b) = cubic.split(0.5).unwrap();
        let (Piece::Cubic(a), Piece::Cubic(b)) = (a[0], b[0]) else { panic!("cubic halves") };
        assert_eq!((a.p0, b.p3), (Point::new(100.0, 0.0), Point::new(200.0, 100.0)));
        assert!(a.p3.distance(b.p0) < 1e-9);
        assert!(a.p3.distance(cubic.eval(0.5).unwrap()) < 1e-9);
        // The two-piece quadratic splits inside its second piece.
        let (before, after) = segs[2].split(0.75).unwrap();
        assert_eq!((before.len(), after.len()), (2, 1));
        // An S curve has one inflection and its extremes lie inside the segment.
        let s = segments(
            &[n(0.0, 0.0, NodeType::Line), n(100.0, 100.0, NodeType::OffCurve), n(0.0, 100.0, NodeType::OffCurve), n(100.0, 200.0, NodeType::Curve)],
            false,
        );
        let inf = s[0].inflections();
        assert_eq!(inf.len(), 1);
        assert!((inf[0] - 0.5).abs() < 1e-9);
        assert!(s[0].extremes().iter().all(|t| *t > 0.0 && *t < 1.0));
        assert!(segs[0].extremes().is_empty() && segs[0].inflections().is_empty());
        // Tight bounds of the cubic stay within its control box.
        let b = cubic.bounds().unwrap();
        assert!(b.x0 >= 100.0 - 1e-9 && b.x1 <= 200.0 + 1e-9 && b.y0 >= -1e-9 && b.y1 <= 100.0 + 1e-9);
    }

    #[test]
    fn bounds_and_grid_rounding() {
        let r = bounds([Point::new(10.0, 5.0), Point::new(f64::NAN, 0.0), Point::new(-2.0, 7.0)]).unwrap();
        assert_eq!((r.x0, r.y0, r.x1, r.y1), (-2.0, 5.0, 10.0, 7.0));
        assert!(bounds([Point::new(f64::NAN, 0.0)]).is_none());
        assert_eq!(grid_step(1.0, 1.0), 1.0);
        assert_eq!(grid_step(100.0, 5.0), 20.0);
        assert_eq!(grid_step(0.0, 5.0), 0.0);
        assert_eq!(grid_step(f64::NAN, 1.0), 0.0);
        assert_eq!(grid_step(10.0, 0.0), 10.0);
        assert_eq!(round_to_grid(12.4, 1.0), 12.0);
        assert_eq!(round_to_grid(12.4, 0.0), 12.4);
        assert_eq!(round_to_grid(31.0, 20.0), 40.0);
        assert_eq!(round_to_grid(0.3, grid_step(1.0, 10.0)), 0.3);
        assert!(round_to_grid(f64::NAN, 1.0).is_nan());
        assert_eq!(round_point(Point::new(1.6, -1.6), 1.0), Point::new(2.0, -2.0));
    }
}
