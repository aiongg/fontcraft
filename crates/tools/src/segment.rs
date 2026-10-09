//! The segments of a path with the addresses of their nodes.

use fontcraft_geom::{CubicBez, Line, ParamCurve, Point, QuadBez};
use fontcraft_model::{Node, NodeKind, Path};

use crate::geometry::{bounds, dist, usable};

/// Most off-curve nodes one segment may have; a longer run is a malformed path and its segment
/// is left out.
const MAX_SEGMENT_HANDLES: usize = 10_000;
/// Evenly spaced samples per curve piece before the nearest point is refined.
const SAMPLES: usize = 16;
/// Ternary-search steps that refine the nearest point between two samples.
const REFINE_STEPS: usize = 24;

/// One piece of a segment's curve. A quadratic segment with several off-curve nodes has one
/// piece per off-curve node.
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

    /// The nearest point to `p` as `(distance, t)`. Sampled and then refined, so the work is
    /// fixed whatever the input.
    fn nearest(&self, p: Point) -> (f64, f64) {
        let mut best = (f64::INFINITY, 0.0);
        for i in 0..=SAMPLES {
            let t = i as f64 / SAMPLES as f64;
            let d = dist(self.eval(t), p);
            if d < best.0 {
                best = (d, t);
            }
        }
        let step = 1.0 / SAMPLES as f64;
        let (mut lo, mut hi) = ((best.1 - step).max(0.0), (best.1 + step).min(1.0));
        for _ in 0..REFINE_STEPS {
            let m1 = lo + (hi - lo) / 3.0;
            let m2 = hi - (hi - lo) / 3.0;
            if dist(self.eval(m1), p) < dist(self.eval(m2), p) {
                hi = m2;
            } else {
                lo = m1;
            }
        }
        let t = (lo + hi) / 2.0;
        let d = dist(self.eval(t), p);
        if d < best.0 { (d, t) } else { best }
    }
}

/// A segment of a path: from the on-curve node `start` to the on-curve node `end`, through the
/// off-curve nodes `handles`. A segment is addressed by `[shape, end]` (decision 0004).
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub shape: usize,
    pub start: usize,
    pub end: usize,
    pub handles: Vec<usize>,
    pub pieces: Vec<Piece>,
    /// The box around every point that defines the segment (the curve lies inside it).
    pub bounds: (Point, Point),
}

impl Segment {
    /// A straight segment (no off-curve nodes).
    pub fn is_line(&self) -> bool {
        self.handles.is_empty()
    }

    /// The nearest point to `p` as `(distance, t, point)`, `t` running from 0 at `start` to 1 at
    /// `end` across all pieces.
    pub fn nearest(&self, p: Point) -> Option<(f64, f64, Point)> {
        let count = self.pieces.len();
        let mut best: Option<(f64, f64, Point)> = None;
        for (i, piece) in self.pieces.iter().enumerate() {
            let (d, t) = piece.nearest(p);
            if best.is_none_or(|b| d < b.0) {
                best = Some((d, (i as f64 + t) / count as f64, piece.eval(t)));
            }
        }
        best.filter(|b| b.0.is_finite())
    }
}

/// The segments of `path`, the shape at index `shape`. Segments with unusable coordinates
/// (not numbers, beyond the model's limit) or too many handles are left out; a malformed run of
/// nodes becomes straight pieces through its points, so it can still be hit.
pub fn segments(shape: usize, path: &Path) -> Vec<Segment> {
    let nodes = &path.nodes;
    let count = nodes.len();
    let mut out = Vec::new();
    let Some(first) = nodes.iter().position(Node::on_curve) else { return out };
    let steps = if path.closed { count } else { count.saturating_sub(first + 1) };
    let mut prev = first;
    let mut pending: Vec<usize> = Vec::new();
    let mut overflow = false;
    for step in 1..=steps {
        let idx = (first + step) % count;
        let Some(node) = nodes.get(idx) else { continue };
        if node.kind == NodeKind::OffCurve {
            if pending.len() < MAX_SEGMENT_HANDLES {
                pending.push(idx);
            } else {
                overflow = true;
            }
            continue;
        }
        if !overflow && let Some(seg) = build(shape, nodes, prev, idx, &pending) {
            out.push(seg);
        }
        pending.clear();
        overflow = false;
        prev = idx;
    }
    out
}

fn build(shape: usize, nodes: &[Node], start: usize, end: usize, handles: &[usize]) -> Option<Segment> {
    let s = nodes.get(start)?.point();
    let end_node = nodes.get(end)?;
    let e = end_node.point();
    let hs: Vec<Point> = handles.iter().filter_map(|&i| nodes.get(i).map(Node::point)).collect();
    if !(usable(s) && usable(e) && hs.iter().all(|p| usable(*p))) {
        return None;
    }
    let mut pieces = Vec::new();
    match (end_node.kind, hs.as_slice()) {
        (_, []) => pieces.push(Piece::Line(Line::new(s, e))),
        (NodeKind::Curve, [a, b]) => pieces.push(Piece::Cubic(CubicBez::new(s, *a, *b, e))),
        (NodeKind::QCurve, _) => {
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
    let bounds = bounds(std::iter::once(s).chain(hs.iter().copied()).chain(std::iter::once(e)))?;
    Some(Segment { shape, start, end, handles: handles.to_vec(), pieces, bounds })
}
