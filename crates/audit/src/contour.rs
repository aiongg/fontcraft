//! Paths read as segments with node addresses, tolerant of malformed node lists, and the
//! geometry the checks share.

use fontcraft_geom::{BezPath, Point, Rect, Shape as _, Vec2};
use fontcraft_model::{Layer, MAX_COORDINATE, NodeKind, Path, Shape};

/// Pieces a curve is cut into where a check needs points along it (length, deviation,
/// crossings).
pub(crate) const CURVE_STEPS: usize = 24;
/// A closed contour whose area is below this many square units encloses nothing.
pub(crate) const ZERO_AREA_TOLERANCE: f64 = 0.01;
/// Most on-curve points of a contour tested for lying inside another contour.
const NESTING_SAMPLES: usize = 16;
/// Most closed contours whose nesting is worked out (pairs grow with the square).
pub(crate) const MAX_NESTING_CONTOURS: usize = 1_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SegKind {
    Line,
    Cubic,
    /// A quadratic spline: one or more off-curve nodes, with implied on-curve points between
    /// neighbours.
    Quad,
    /// The node list does not make a segment (wrong number of off-curve nodes).
    Invalid,
}

/// One segment: from the on-curve node `start` to the on-curve node `end`.
#[derive(Clone, Debug)]
pub(crate) struct Seg {
    pub start: usize,
    pub end: usize,
    pub kind: SegKind,
    pub p0: Point,
    pub p1: Point,
    /// The off-curve nodes between `start` and `end`: node index and position.
    pub ctrl: Vec<(usize, Point)>,
}

/// A Bézier piece of a segment.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Bez {
    Line(Point, Point),
    Quad(Point, Point, Point),
    Cubic(Point, Point, Point, Point),
}

impl Bez {
    pub(crate) fn eval(&self, t: f64) -> Point {
        let mt = 1.0 - t;
        match *self {
            Bez::Line(a, b) => a.lerp(b, t),
            Bez::Quad(a, b, c) => {
                let (wa, wb, wc) = (mt * mt, 2.0 * mt * t, t * t);
                Point::new(wa * a.x + wb * b.x + wc * c.x, wa * a.y + wb * b.y + wc * c.y)
            }
            Bez::Cubic(a, b, c, d) => {
                let (wa, wb, wc, wd) = (mt * mt * mt, 3.0 * mt * mt * t, 3.0 * mt * t * t, t * t * t);
                Point::new(wa * a.x + wb * b.x + wc * c.x + wd * d.x, wa * a.y + wb * b.y + wc * c.y + wd * d.y)
            }
        }
    }

    /// Points along the piece from its start to its end: both ends of a line, `CURVE_STEPS + 1`
    /// points of a curve.
    pub(crate) fn polyline(&self, out: &mut Vec<Point>) {
        match *self {
            Bez::Line(a, b) => {
                out.push(a);
                out.push(b);
            }
            _ => {
                for i in 0..=CURVE_STEPS {
                    out.push(self.eval(i as f64 / CURVE_STEPS as f64));
                }
            }
        }
    }
}

impl Seg {
    /// The segment as Bézier pieces; none for an invalid segment.
    pub(crate) fn beziers(&self) -> Vec<Bez> {
        match self.kind {
            SegKind::Line => vec![Bez::Line(self.p0, self.p1)],
            SegKind::Cubic => match self.ctrl.as_slice() {
                [(_, a), (_, b)] => vec![Bez::Cubic(self.p0, *a, *b, self.p1)],
                _ => Vec::new(),
            },
            SegKind::Quad => {
                let mut out = Vec::with_capacity(self.ctrl.len());
                let mut from = self.p0;
                for (i, (_, c)) in self.ctrl.iter().enumerate() {
                    let to = match self.ctrl.get(i + 1) {
                        Some((_, next)) => c.midpoint(*next),
                        None => self.p1,
                    };
                    out.push(Bez::Quad(from, *c, to));
                    from = to;
                }
                out
            }
            SegKind::Invalid => Vec::new(),
        }
    }

    /// The cubic's four points, for a cubic segment.
    pub(crate) fn cubic(&self) -> Option<(Point, Point, Point, Point)> {
        match (self.kind, self.ctrl.as_slice()) {
            (SegKind::Cubic, [(_, a), (_, b)]) => Some((self.p0, *a, *b, self.p1)),
            _ => None,
        }
    }

    pub(crate) fn is_curve(&self) -> bool {
        matches!(self.kind, SegKind::Cubic | SegKind::Quad)
    }

    pub(crate) fn is_valid(&self) -> bool {
        self.kind != SegKind::Invalid
    }

    /// The start, the off-curve points and the end.
    pub(crate) fn points(&self) -> Vec<Point> {
        let mut v = Vec::with_capacity(self.ctrl.len() + 2);
        v.push(self.p0);
        v.extend(self.ctrl.iter().map(|(_, p)| *p));
        v.push(self.p1);
        v
    }

    /// The direction leaving the start: towards the first point that is not on it.
    pub(crate) fn start_dir(&self) -> Option<Vec2> {
        self.points().into_iter().skip(1).map(|p| p - self.p0).find(|v| v.hypot2() > 0.0)
    }

    /// The direction arriving at the end: from the last point that is not on it.
    pub(crate) fn end_dir(&self) -> Option<Vec2> {
        self.points().into_iter().rev().skip(1).map(|p| self.p1 - p).find(|v| v.hypot2() > 0.0)
    }

    /// Points along the segment from start to end.
    pub(crate) fn polyline(&self) -> Vec<Point> {
        let mut out = Vec::new();
        for b in self.beziers() {
            let mut piece = Vec::new();
            b.polyline(&mut piece);
            // Each piece starts where the previous one ended.
            let skip = usize::from(!out.is_empty());
            out.extend(piece.into_iter().skip(skip));
        }
        out
    }

    /// Length along the segment (curves measured along their polyline).
    pub(crate) fn length(&self) -> f64 {
        polyline_length(&self.polyline())
    }

    /// Length of the control polygon: start, off-curve points, end.
    pub(crate) fn control_length(&self) -> f64 {
        polyline_length(&self.points())
    }

    /// The box of the segment's points (the control box).
    pub(crate) fn control_box(&self) -> Rect {
        points_box(&self.points()).unwrap_or(Rect::new(self.p0.x, self.p0.y, self.p0.x, self.p0.y))
    }
}

/// One path of a layer.
pub(crate) struct Contour<'a> {
    pub shape: usize,
    pub path: &'a Path,
    pub segs: Vec<Seg>,
    /// Every coordinate is a number within the model's range.
    pub finite: bool,
    /// Per node: the segment ending at it and the segment starting at it.
    incoming: Vec<Option<usize>>,
    outgoing: Vec<Option<usize>>,
}

impl<'a> Contour<'a> {
    pub(crate) fn new(shape: usize, path: &'a Path) -> Self {
        let nodes = &path.nodes;
        let n = nodes.len();
        let finite = nodes.iter().all(|nd| in_range(nd.x) && in_range(nd.y));
        let on: Vec<usize> = nodes.iter().enumerate().filter(|(_, nd)| nd.on_curve()).map(|(i, _)| i).collect();
        let mut segs = Vec::with_capacity(on.len());
        let mut incoming = vec![None; n];
        let mut outgoing = vec![None; n];
        for (k, &end) in on.iter().enumerate() {
            let start = if k == 0 {
                if !path.closed {
                    continue;
                }
                match on.last() {
                    Some(&s) => s,
                    None => continue,
                }
            } else {
                match on.get(k - 1) {
                    Some(&s) => s,
                    None => continue,
                }
            };
            let mut ctrl = Vec::new();
            let mut i = (start + 1) % n;
            let mut steps = 0;
            // In a closed path with one on-curve node, start equals end and the walk goes once
            // around the other nodes.
            while i != end && steps < n {
                if let Some(nd) = nodes.get(i) {
                    ctrl.push((i, nd.point()));
                }
                i = (i + 1) % n;
                steps += 1;
            }
            let (Some(a), Some(b)) = (nodes.get(start), nodes.get(end)) else { continue };
            let kind = match b.kind {
                NodeKind::Line if ctrl.is_empty() => SegKind::Line,
                NodeKind::Curve if ctrl.len() == 2 => SegKind::Cubic,
                NodeKind::QCurve if !ctrl.is_empty() => SegKind::Quad,
                _ => SegKind::Invalid,
            };
            let idx = segs.len();
            if let Some(slot) = incoming.get_mut(end) {
                *slot = Some(idx);
            }
            if let Some(slot) = outgoing.get_mut(start) {
                *slot = Some(idx);
            }
            segs.push(Seg { start, end, kind, p0: a.point(), p1: b.point(), ctrl });
        }
        Self { shape, path, segs, finite, incoming, outgoing }
    }

    pub(crate) fn closed(&self) -> bool {
        self.path.closed
    }

    pub(crate) fn on_curve_count(&self) -> usize {
        self.path.nodes.iter().filter(|n| n.on_curve()).count()
    }

    /// The segment ending at `node`.
    pub(crate) fn incoming(&self, node: usize) -> Option<&Seg> {
        self.incoming.get(node).copied().flatten().and_then(|i| self.segs.get(i))
    }

    /// The segments arriving at and leaving `node`, when they are two different valid segments.
    pub(crate) fn joint(&self, node: usize) -> Option<(&Seg, &Seg)> {
        let (i, o) = (self.incoming.get(node).copied().flatten()?, self.outgoing.get(node).copied().flatten()?);
        if i == o {
            return None;
        }
        let (a, b) = (self.segs.get(i)?, self.segs.get(o)?);
        (a.is_valid() && b.is_valid()).then_some((a, b))
    }

    /// The first on-curve node, which is the start of a closed path.
    pub(crate) fn start_node(&self) -> Option<usize> {
        self.path.nodes.iter().position(|n| n.on_curve())
    }

    /// The path as a Bézier path, when it is usable.
    pub(crate) fn bez(&self) -> Option<BezPath> {
        if !self.finite {
            return None;
        }
        self.path.to_bez().ok()
    }

    /// The bounding box: exact for a usable path, else the box of its nodes.
    pub(crate) fn bbox(&self) -> Option<Rect> {
        if !self.finite || self.path.nodes.is_empty() {
            return None;
        }
        match self.bez() {
            Some(b) if !b.elements().is_empty() => Some(b.bounding_box()),
            _ => points_box(&self.path.nodes.iter().map(|n| n.point()).collect::<Vec<_>>()),
        }
    }
}

/// The paths of a layer, with their shape indices.
pub(crate) fn contours(layer: &Layer) -> Vec<Contour<'_>> {
    layer
        .shapes
        .iter()
        .enumerate()
        .filter_map(|(i, s)| match s {
            Shape::Path(p) => Some(Contour::new(i, p)),
            _ => None,
        })
        .collect()
}

pub(crate) fn in_range(v: f64) -> bool {
    v.is_finite() && v.abs() <= MAX_COORDINATE
}

pub(crate) fn polyline_length(points: &[Point]) -> f64 {
    points.windows(2).map(|w| if let [a, b] = w { a.distance(*b) } else { 0.0 }).sum()
}

pub(crate) fn points_box(points: &[Point]) -> Option<Rect> {
    let first = points.first()?;
    Some(points.iter().fold(Rect::new(first.x, first.y, first.x, first.y), |r, p| r.union_pt(*p)))
}

/// Distance from `p` to the line segment from `a` to `b`.
pub(crate) fn dist_to_segment(p: Point, a: Point, b: Point) -> f64 {
    let ab = b - a;
    let len2 = ab.hypot2();
    if len2 == 0.0 {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// Distance from `p` to the infinite line through `a` and `b`.
pub(crate) fn dist_to_line(p: Point, a: Point, b: Point) -> f64 {
    let ab = b - a;
    let len = ab.hypot();
    if len == 0.0 {
        return p.distance(a);
    }
    (ab.cross(p - a) / len).abs()
}

/// The angle between two directions, in radians from 0 to π.
pub(crate) fn angle_between(a: Vec2, b: Vec2) -> f64 {
    a.cross(b).abs().atan2(a.dot(b))
}

/// Where the segments `a`–`b` and `c`–`d` cross, when each passes strictly through the other.
/// Touching at an end and lying on one line do not count.
pub(crate) fn proper_cross(a: Point, b: Point, c: Point, d: Point) -> Option<Point> {
    let d1 = (b - a).cross(c - a);
    let d2 = (b - a).cross(d - a);
    let d3 = (d - c).cross(a - c);
    let d4 = (d - c).cross(b - c);
    let opposite = |x: f64, y: f64| (x > 0.0 && y < 0.0) || (x < 0.0 && y > 0.0);
    if opposite(d1, d2) && opposite(d3, d4) {
        let s = d1 / (d1 - d2);
        Some(c.lerp(d, s))
    } else {
        None
    }
}

/// The first crossing of two polylines.
pub(crate) fn polylines_cross(a: &[Point], b: &[Point]) -> Option<Point> {
    for wa in a.windows(2) {
        let [p, q] = wa else { continue };
        let ra = Rect::from_points(*p, *q);
        for wb in b.windows(2) {
            let [r, s] = wb else { continue };
            if !overlaps(ra, Rect::from_points(*r, *s)) {
                continue;
            }
            if let Some(x) = proper_cross(*p, *q, *r, *s) {
                return Some(x);
            }
        }
    }
    None
}

/// Do two boxes share any point (edges included)?
pub(crate) fn overlaps(a: Rect, b: Rect) -> bool {
    a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1
}

/// The real roots of `a t² + b t + c` strictly inside (`lo`, `hi`) at which the polynomial
/// changes sign. Coefficients near zero relative to the largest one count as zero.
pub(crate) fn sign_changing_roots(a: f64, b: f64, c: f64, lo: f64, hi: f64) -> Vec<f64> {
    let scale = a.abs().max(b.abs()).max(c.abs());
    if !(scale.is_finite() && scale > 0.0) {
        return Vec::new();
    }
    let eps = scale * 1e-12;
    let mut roots = Vec::new();
    if a.abs() <= eps {
        if b.abs() > eps {
            roots.push(-c / b);
        }
    } else {
        let disc = b * b - 4.0 * a * c;
        if disc > 0.0 {
            let sq = disc.sqrt();
            // The stable form: no cancellation between `b` and the square root.
            let q = -0.5 * (b + b.signum() * sq);
            roots.push(q / a);
            if q != 0.0 {
                roots.push(c / q);
            }
        }
    }
    roots.retain(|t| t.is_finite() && *t > lo && *t < hi);
    roots.sort_by(f64::total_cmp);
    roots.dedup();
    roots
}

/// A closed path with an area, for nesting questions.
pub(crate) struct Region {
    pub shape: usize,
    pub bez: BezPath,
    pub bbox: Rect,
    pub area: f64,
    samples: Vec<Point>,
}

/// The closed, usable paths that enclose some area.
pub(crate) fn regions(contours: &[Contour<'_>]) -> Vec<Region> {
    let mut out = Vec::new();
    for c in contours.iter().filter(|c| c.closed() && c.finite) {
        let Some(bez) = c.bez() else { continue };
        if bez.elements().is_empty() {
            continue;
        }
        let area = bez.area();
        if !area.is_finite() || area.abs() < ZERO_AREA_TOLERANCE {
            continue;
        }
        let on: Vec<Point> = c.path.nodes.iter().filter(|n| n.on_curve()).map(|n| n.point()).collect();
        let step = (on.len() / NESTING_SAMPLES).max(1);
        let samples = on.into_iter().step_by(step).take(NESTING_SAMPLES).collect();
        out.push(Region { shape: c.shape, bbox: bez.bounding_box(), bez, area, samples });
    }
    out
}

/// Does `outer` enclose `inner`: `inner`'s box lies within `outer`'s, and every sampled
/// on-curve point of `inner` is inside `outer`?
pub(crate) fn encloses(outer: &Region, inner: &Region) -> bool {
    if outer.shape == inner.shape || outer.bbox == inner.bbox || !outer.bbox.contains_rect(inner.bbox) {
        return false;
    }
    !inner.samples.is_empty() && inner.samples.iter().all(|p| outer.bez.winding(*p) != 0)
}

/// For every region: how many regions enclose it, and the smallest of them.
pub(crate) fn nesting(regions: &[Region]) -> Vec<(usize, Option<usize>)> {
    regions
        .iter()
        .map(|inner| {
            let mut depth = 0;
            let mut parent: Option<(usize, f64)> = None;
            for (j, outer) in regions.iter().enumerate() {
                if encloses(outer, inner) {
                    depth += 1;
                    let a = outer.area.abs();
                    if parent.is_none_or(|(_, best)| a < best) {
                        parent = Some((j, a));
                    }
                }
            }
            (depth, parent.map(|(j, _)| j))
        })
        .collect()
}
