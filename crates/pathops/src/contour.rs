//! Contours as node lists, the plain form the node-level operations take and return, and the ring
//! form they work on inside the crate.
//!
//! A [`Contour`] lists its nodes the way the model does: off-curve nodes come before the on-curve
//! node that ends their segment, and in a closed contour the segment that ends at the first
//! on-curve node wraps around from the last nodes. A command converts a model path to a contour
//! and back field by field, and node addresses (`[shape, node]`) are the same in both.

use fontcraft_geom::{BezPath, CubicBez, ParamCurve, PathEl, Point, QuadBez, Rect, Shape, Vec2};

use crate::{PathOpError, check};

/// Most nodes, counted over all contours, that one operation accepts. A segment has at most three
/// nodes, so this matches [`crate::MAX_SEGMENTS`].
pub const MAX_NODES: usize = 3 * crate::MAX_SEGMENTS;
/// Grids finer than this are not applied: rounding to them would change nothing a font can store.
pub const MIN_GRID: f64 = 1e-6;
/// Two points closer than this, in units, give a direction no tangent can be read from.
pub(crate) const DIRECTION_EPSILON: f64 = 1e-9;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NodeKind {
    /// On-curve, ends a straight segment.
    Line,
    /// On-curve, ends a cubic segment (two off-curve nodes before it).
    Curve,
    /// On-curve, ends a quadratic segment (one or more off-curve nodes before it, with implied
    /// on-curve points halfway between neighbours).
    QCurve,
    /// A control point.
    OffCurve,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Node {
    pub point: Point,
    pub kind: NodeKind,
    /// The segments on either side meet without a corner.
    pub smooth: bool,
    /// The caller's mark. It stays with the node while the node survives an operation, moved or
    /// not, so the caller can carry the data the plain form drops (names, locks, user data).
    /// Nodes an operation creates have none.
    pub tag: Option<usize>,
}

impl Node {
    pub fn new(point: Point, kind: NodeKind) -> Self {
        Self { point, kind, smooth: false, tag: None }
    }

    pub fn off(point: Point) -> Self {
        Self::new(point, NodeKind::OffCurve)
    }

    pub fn on_curve(&self) -> bool {
        self.kind != NodeKind::OffCurve
    }

    fn with_tag(mut self, tag: usize) -> Self {
        self.tag = Some(tag);
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Contour {
    pub closed: bool,
    pub nodes: Vec<Node>,
}

impl Contour {
    pub fn new(closed: bool, nodes: Vec<Node>) -> Self {
        Self { closed, nodes }
    }

    /// The contour as Bézier segments, or what is wrong with its node list.
    pub fn to_bez(&self) -> Result<BezPath, PathOpError> {
        Ok(Ring::from_contour(self)?.to_bez())
    }

    /// The same contour drawn the other way. A closed contour keeps its start node; each on-curve
    /// node takes the kind of the segment that now ends at it.
    pub fn reversed(&self) -> Result<Contour, PathOpError> {
        Ok(Ring::from_contour(self)?.reversed().to_contour())
    }

    /// Every node tagged with its own index, so a caller can follow nodes through an operation.
    pub fn tagged(mut self) -> Self {
        for (i, n) in self.nodes.iter_mut().enumerate() {
            *n = n.with_tag(i);
        }
        self
    }
}

/// The subpaths of `path` as contours, node for node: a subpath that ends with a close is
/// closed, and an explicit line back to its start before the close is dropped (the close draws
/// it). No node is smooth.
pub fn contours_from_bez(path: &BezPath) -> Vec<Contour> {
    let mut out = Vec::new();
    let mut current: Option<(Point, Vec<RNode>)> = None;
    let mut finish = |current: &mut Option<(Point, Vec<RNode>)>, closed: bool| {
        if let Some((start, mut nodes)) = current.take() {
            if closed && nodes.len() > 1 && nodes.last().is_some_and(|r| r.node.kind == NodeKind::Line && r.node.point == start) {
                nodes.pop();
            }
            let ring = if closed {
                // The first node is the start; the segment that ends there is the last one drawn
                // when the path comes back to it.
                let back = nodes.last().is_some_and(|r| r.node.point == start) && nodes.len() > 1;
                if back {
                    if let Some(last) = nodes.pop()
                        && let Some(first) = nodes.first_mut()
                    {
                        *first = RNode { node: Node { point: start, ..last.node }, ctrl: last.ctrl, src: None, mark: false };
                    }
                } else if let Some(first) = nodes.first_mut() {
                    first.node.kind = NodeKind::Line;
                }
                Ring { closed: true, nodes }
            } else {
                Ring { closed: false, nodes }
            };
            out.push(ring.to_contour());
        }
    };
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                finish(&mut current, false);
                current = Some((p, vec![RNode::new(Node::new(p, NodeKind::Line), Vec::new())]));
            }
            PathEl::LineTo(p) => {
                if let Some((_, nodes)) = current.as_mut() {
                    nodes.push(RNode::new(Node::new(p, NodeKind::Line), Vec::new()));
                }
            }
            PathEl::QuadTo(c, p) => {
                if let Some((_, nodes)) = current.as_mut() {
                    nodes.push(RNode::new(Node::new(p, NodeKind::QCurve), vec![Node::off(c)]));
                }
            }
            PathEl::CurveTo(a, b, p) => {
                if let Some((_, nodes)) = current.as_mut() {
                    nodes.push(RNode::new(Node::new(p, NodeKind::Curve), vec![Node::off(a), Node::off(b)]));
                }
            }
            PathEl::ClosePath => finish(&mut current, true),
        }
    }
    finish(&mut current, false);
    out
}

/// `p` on the grid of `grid` units, or `p` itself when the grid is off (not finite, or finer than
/// [`MIN_GRID`]).
pub fn round_to_grid(p: Point, grid: f64) -> Point {
    if !(grid.is_finite() && grid >= MIN_GRID) {
        return p;
    }
    // `+ 0.0` turns a negative zero into zero, so equal points compare equal bit for bit.
    Point::new((p.x / grid).round() * grid + 0.0, (p.y / grid).round() * grid + 0.0)
}

/// An on-curve node with the off-curve nodes of the segment that ends at it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RNode {
    pub node: Node,
    pub ctrl: Vec<Node>,
    /// The node's index in the input node list; `None` for a node an operation made.
    pub src: Option<usize>,
    /// Set by an operation on the nodes it wants to report (the selection a command returns).
    pub mark: bool,
}

impl RNode {
    pub fn new(node: Node, ctrl: Vec<Node>) -> Self {
        Self { node, ctrl, src: None, mark: false }
    }
}

/// A contour as on-curve nodes. Segment `j` ends at node `j` and starts at node `j - 1`; in a
/// closed ring segment 0 starts at the last node, and in an open ring there is no segment 0.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Ring {
    pub closed: bool,
    pub nodes: Vec<RNode>,
}

/// One segment of a ring.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Seg<'a> {
    pub from: Point,
    pub ctrl: &'a [Node],
    pub to: Point,
    pub kind: NodeKind,
}

/// A Bézier piece of a segment. A quadratic spline with several off-curve nodes has one piece per
/// off-curve node.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Piece {
    Line(Point, Point),
    Quad(QuadBez),
    Cubic(CubicBez),
}

impl Piece {
    pub fn start(&self) -> Point {
        match self {
            Piece::Line(a, _) => *a,
            Piece::Quad(q) => q.p0,
            Piece::Cubic(c) => c.p0,
        }
    }

    pub fn end(&self) -> Point {
        match self {
            Piece::Line(_, b) => *b,
            Piece::Quad(q) => q.p2,
            Piece::Cubic(c) => c.p3,
        }
    }

    pub fn eval(&self, t: f64) -> Point {
        match self {
            Piece::Line(a, b) => a.lerp(*b, t),
            Piece::Quad(q) => q.eval(t),
            Piece::Cubic(c) => c.eval(t),
        }
    }

    pub fn subsegment(&self, t0: f64, t1: f64) -> Piece {
        match self {
            Piece::Line(a, b) => Piece::Line(a.lerp(*b, t0), a.lerp(*b, t1)),
            Piece::Quad(q) => Piece::Quad(q.subsegment(t0..t1)),
            Piece::Cubic(c) => Piece::Cubic(c.subsegment(t0..t1)),
        }
    }

    /// `(a, b, c)` such that the derivative is a positive multiple of `a t² + b t + c`.
    pub fn deriv_coeffs(&self) -> (Vec2, Vec2, Vec2) {
        match self {
            Piece::Line(a, b) => (Vec2::ZERO, Vec2::ZERO, *b - *a),
            Piece::Quad(q) => (Vec2::ZERO, (q.p2 - q.p1) - (q.p1 - q.p0), q.p1 - q.p0),
            Piece::Cubic(c) => {
                let (d0, d1, d2) = (c.p1 - c.p0, c.p2 - c.p1, c.p3 - c.p2);
                (d0 - 2.0 * d1 + d2, 2.0 * (d1 - d0), d0)
            }
        }
    }

    pub fn kind(&self) -> NodeKind {
        match self {
            Piece::Line(..) => NodeKind::Line,
            Piece::Quad(_) => NodeKind::QCurve,
            Piece::Cubic(_) => NodeKind::Curve,
        }
    }

    fn ctrl(&self) -> Vec<Node> {
        match self {
            Piece::Line(..) => Vec::new(),
            Piece::Quad(q) => vec![Node::off(q.p1)],
            Piece::Cubic(c) => vec![Node::off(c.p1), Node::off(c.p2)],
        }
    }

    fn push_to(&self, out: &mut BezPath) {
        match self {
            Piece::Line(_, b) => out.line_to(*b),
            Piece::Quad(q) => out.quad_to(q.p1, q.p2),
            Piece::Cubic(c) => out.curve_to(c.p1, c.p2, c.p3),
        }
    }
}

impl Seg<'_> {
    pub fn pieces(&self) -> Vec<Piece> {
        match self.kind {
            NodeKind::Curve => match self.ctrl {
                [a, b] => vec![Piece::Cubic(CubicBez::new(self.from, a.point, b.point, self.to))],
                _ => vec![Piece::Line(self.from, self.to)],
            },
            NodeKind::QCurve if !self.ctrl.is_empty() => {
                let mut out = Vec::with_capacity(self.ctrl.len());
                let mut start = self.from;
                let mut iter = self.ctrl.iter().peekable();
                while let Some(c) = iter.next() {
                    let end = match iter.peek() {
                        Some(next) => c.point.midpoint(next.point),
                        None => self.to,
                    };
                    out.push(Piece::Quad(QuadBez::new(start, c.point, end)));
                    start = end;
                }
                out
            }
            _ => vec![Piece::Line(self.from, self.to)],
        }
    }

    pub fn is_curve(&self) -> bool {
        !self.ctrl.is_empty()
    }
}

impl Ring {
    pub fn from_contour(c: &Contour) -> Result<Ring, PathOpError> {
        let n = c.nodes.len();
        if n > MAX_NODES {
            return Err(PathOpError::TooManyNodes(n));
        }
        for node in &c.nodes {
            check(node.point.x)?;
            check(node.point.y)?;
        }
        let Some(first) = c.nodes.iter().position(Node::on_curve) else {
            if n == 0 {
                return Ok(Ring { closed: c.closed, nodes: Vec::new() });
            }
            // A closed quadratic spline of off-curve nodes only (legal in TrueType outlines):
            // it starts at the implied point between its last and first nodes, which becomes a
            // node.
            if let (true, Some(last), Some(head)) = (c.closed && n >= 2, c.nodes.last(), c.nodes.first()) {
                let start = Node::new(last.point.midpoint(head.point), NodeKind::QCurve);
                return Ok(Ring { closed: true, nodes: vec![RNode::new(start, c.nodes.clone())] });
            }
            return Err(PathOpError::Malformed("a path has no on-curve node".into()));
        };
        if !c.closed && first != 0 {
            return Err(PathOpError::Malformed("an open path starts with an off-curve node".into()));
        }
        let mut nodes: Vec<RNode> = Vec::new();
        let mut pending: Vec<Node> = Vec::new();
        let steps = if c.closed { n } else { n - first };
        for step in 0..steps {
            let index = (first + step) % n;
            let Some(node) = c.nodes.get(index) else { continue };
            if node.on_curve() {
                nodes.push(RNode { node: *node, ctrl: std::mem::take(&mut pending), src: Some(index), mark: false });
            } else {
                pending.push(*node);
            }
        }
        if c.closed {
            if let Some(r) = nodes.first_mut() {
                r.ctrl = pending;
            }
        } else if !pending.is_empty() {
            return Err(PathOpError::Malformed("an open path ends with off-curve nodes".into()));
        }
        for (i, r) in nodes.iter().enumerate() {
            if !c.closed && i == 0 {
                continue;
            }
            match (r.node.kind, r.ctrl.len()) {
                (NodeKind::Line, 0) | (NodeKind::Curve, 2) | (NodeKind::OffCurve, _) => {}
                (NodeKind::QCurve, k) if k >= 1 => {}
                (NodeKind::Line, _) => return Err(PathOpError::Malformed("a line node follows off-curve nodes".into())),
                (NodeKind::Curve, k) => {
                    return Err(PathOpError::Malformed(format!("a curve node needs two off-curve nodes before it, found {k}")));
                }
                (NodeKind::QCurve, _) => return Err(PathOpError::Malformed("a qcurve node needs an off-curve node before it".into())),
            }
        }
        Ok(Ring { closed: c.closed, nodes })
    }

    pub fn to_contour(&self) -> Contour {
        let mut nodes = Vec::with_capacity(self.nodes.iter().map(|r| r.ctrl.len() + 1).sum());
        for (i, r) in self.nodes.iter().enumerate() {
            if i > 0 {
                nodes.extend(r.ctrl.iter().copied());
            }
            nodes.push(r.node);
        }
        if self.closed
            && let Some(first) = self.nodes.first()
        {
            nodes.extend(first.ctrl.iter().copied());
        }
        Contour { closed: self.closed, nodes }
    }

    /// The index in [`Ring::to_contour`]'s node list of ring node `k`.
    pub fn list_index(&self, k: usize) -> usize {
        self.nodes.iter().take(k + 1).skip(1).map(|r| r.ctrl.len() + 1).sum()
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// The node after `j`, if any.
    pub fn next(&self, j: usize) -> Option<usize> {
        let m = self.nodes.len();
        if j + 1 < m {
            Some(j + 1)
        } else if self.closed && m > 0 {
            Some(0)
        } else {
            None
        }
    }

    /// The node before `j`, if any.
    pub fn prev(&self, j: usize) -> Option<usize> {
        if j > 0 {
            Some(j - 1)
        } else if self.closed {
            self.nodes.len().checked_sub(1)
        } else {
            None
        }
    }

    /// The segment that ends at node `j`.
    pub fn seg(&self, j: usize) -> Option<Seg<'_>> {
        let to = self.nodes.get(j)?;
        let from = self.nodes.get(self.prev(j)?)?;
        Some(Seg { from: from.node.point, ctrl: &to.ctrl, to: to.node.point, kind: to.node.kind })
    }

    /// Indices of the nodes that end a segment, in path order.
    pub fn segment_ends(&self) -> std::ops::Range<usize> {
        let first = usize::from(!self.closed);
        first.min(self.nodes.len())..self.nodes.len()
    }

    pub fn to_bez(&self) -> BezPath {
        let mut out = BezPath::new();
        let Some(first) = self.nodes.first() else { return out };
        out.move_to(first.node.point);
        let closing = if self.closed { Some(0) } else { None };
        for j in (1..self.nodes.len()).chain(closing) {
            let Some(seg) = self.seg(j) else { continue };
            // The close draws the closing line.
            if j == 0 && !seg.is_curve() {
                continue;
            }
            for piece in seg.pieces() {
                piece.push_to(&mut out);
            }
        }
        if self.closed {
            out.close_path();
        }
        out
    }

    /// Signed area (positive for counter-clockwise with y up); zero for an open ring.
    pub fn area(&self) -> f64 {
        if self.closed { self.to_bez().area() } else { 0.0 }
    }

    pub fn bounds(&self) -> Option<Rect> {
        if self.nodes.is_empty() {
            return None;
        }
        let path = self.to_bez();
        if path.elements().iter().any(|e| !matches!(e, PathEl::MoveTo(_) | PathEl::ClosePath)) {
            Some(path.bounding_box())
        } else {
            self.nodes.first().map(|r| Rect::from_points(r.node.point, r.node.point))
        }
    }

    pub fn reversed(&self) -> Ring {
        let m = self.nodes.len();
        let mut out = Vec::with_capacity(m);
        if self.closed {
            // In the reversed ring node j is reached from its old successor, along the old
            // segment that ended there, drawn backwards.
            for k in 0..m {
                let j = (m - k) % m;
                let (Some(node), Some(src)) = (self.nodes.get(j), self.nodes.get((j + 1) % m)) else { continue };
                out.push(RNode {
                    node: Node { kind: src.node.kind, ..node.node },
                    ctrl: src.ctrl.iter().rev().copied().collect(),
                    src: node.src,
                    mark: node.mark,
                });
            }
        } else {
            for k in 0..m {
                let j = m - 1 - k;
                let Some(node) = self.nodes.get(j) else { continue };
                let (kind, ctrl) = match self.nodes.get(j + 1) {
                    Some(src) if k > 0 => (src.node.kind, src.ctrl.iter().rev().copied().collect()),
                    _ => (NodeKind::Line, Vec::new()),
                };
                out.push(RNode { node: Node { kind, ..node.node }, ctrl, src: node.src, mark: node.mark });
            }
        }
        Ring { closed: self.closed, nodes: out }
    }

    /// The unit direction in which the path arrives at node `j`, read from the nearest point of
    /// its segment that is not on the node.
    pub fn tangent_in(&self, j: usize) -> Option<Vec2> {
        let seg = self.seg(j)?;
        let q = seg.ctrl.iter().rev().map(|n| n.point).chain(std::iter::once(seg.from)).find(|q| q.distance(seg.to) > DIRECTION_EPSILON)?;
        Some((seg.to - q).normalize())
    }

    /// The unit direction in which the path leaves node `j`.
    pub fn tangent_out(&self, j: usize) -> Option<Vec2> {
        let seg = self.seg(self.next(j)?)?;
        let q = seg.ctrl.iter().map(|n| n.point).chain(std::iter::once(seg.to)).find(|q| q.distance(seg.from) > DIRECTION_EPSILON)?;
        Some((q - seg.from).normalize())
    }

    /// Whether node `j` looks smooth: a curve on at least one side and the two directions within
    /// `max_degrees` of each other. `None` where a direction cannot be read or `j` ends an open
    /// ring.
    pub fn looks_smooth(&self, j: usize, max_degrees: f64) -> Option<bool> {
        let curve_in = self.seg(j)?.is_curve();
        let curve_out = self.seg(self.next(j)?)?.is_curve();
        if !curve_in && !curve_out {
            return Some(false);
        }
        let (a, b) = (self.tangent_in(j)?, self.tangent_out(j)?);
        Some(angle_degrees(a, b) <= max_degrees)
    }

    /// Removes the nodes that `merge(previous, node)` accepts and that end a straight segment no
    /// longer than `epsilon`, or a curve whose points all lie within `epsilon` of its start. The
    /// start of a closed ring stays: there the node before it goes, and the start takes over its
    /// incoming segment. At least one node stays.
    pub fn remove_zero_length(&mut self, epsilon: f64, merge: impl Fn(&RNode, &RNode) -> bool) {
        let mut j = usize::from(!self.closed);
        // Each pass of the loop either removes a node or moves on, so it ends.
        while j < self.nodes.len() && self.nodes.len() > 1 {
            let Some(prev) = self.prev(j) else { break };
            let (Some(a), Some(b)) = (self.nodes.get(prev), self.nodes.get(j)) else { break };
            let near = |p: Point| p.distance(a.node.point) <= epsilon;
            let zero = near(b.node.point) && b.ctrl.iter().all(|c| near(c.point));
            if !(zero && merge(a, b)) {
                j += 1;
                continue;
            }
            if j == 0 {
                // The node before the start goes; the start takes over its incoming segment.
                if let Some(gone) = self.nodes.pop()
                    && let Some(start) = self.nodes.first_mut()
                {
                    start.ctrl = gone.ctrl;
                    start.node.kind = gone.node.kind;
                    start.node.tag = start.node.tag.or(gone.node.tag);
                    start.mark |= gone.mark;
                }
            } else {
                let gone = self.nodes.remove(j);
                if let Some(kept) = self.nodes.get_mut(prev) {
                    kept.node.tag = kept.node.tag.or(gone.node.tag);
                    kept.mark |= gone.mark;
                }
            }
        }
    }
}

/// The angle between two directions, in degrees, from 0 to 180.
pub(crate) fn angle_degrees(a: Vec2, b: Vec2) -> f64 {
    a.cross(b).atan2(a.dot(b)).abs().to_degrees()
}

/// Distance from `p` to the segment from `a` to `b`.
pub(crate) fn dist_to_segment(p: Point, a: Point, b: Point) -> f64 {
    let ab = b - a;
    let len2 = ab.hypot2();
    if len2 <= 0.0 {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// The real roots of `a t² + b t + c` in the open interval (0, 1), sorted, without repeats.
pub(crate) fn unit_roots(a: f64, b: f64, c: f64) -> Vec<f64> {
    let scale = a.abs().max(b.abs()).max(c.abs());
    if !(scale.is_finite() && scale > 0.0) {
        return Vec::new();
    }
    let (a, b, c) = (a / scale, b / scale, c / scale);
    let mut roots = Vec::with_capacity(2);
    if a.abs() < 1e-12 {
        if b.abs() >= 1e-12 {
            roots.push(-c / b);
        }
    } else {
        let disc = b * b - 4.0 * a * c;
        if disc >= 0.0 {
            let q = -0.5 * (b + b.signum() * disc.sqrt());
            roots.push(q / a);
            if q != 0.0 {
                roots.push(c / q);
            }
        }
    }
    roots.retain(|t| t.is_finite() && *t > 0.0 && *t < 1.0);
    roots.sort_by(f64::total_cmp);
    roots.dedup_by(|x, y| (*x - *y).abs() < 1e-12);
    roots
}

/// The nodes that replace node `j` of `ring` when its segment is cut at `cuts` (piece index and
/// parameter, sorted): one new on-curve node per cut, each with the off-curve nodes before it,
/// then node `j` itself with the off-curve nodes of the last piece. A quadratic spline that is cut
/// has each of its implied on-curve points made a node. `None` when node `j` ends no segment.
pub(crate) fn split_segment(ring: &Ring, j: usize, cuts: &[(usize, f64)]) -> Option<Vec<RNode>> {
    let seg = ring.seg(j)?;
    let end = ring.nodes.get(j)?;
    if cuts.is_empty() {
        return Some(vec![end.clone()]);
    }
    let pieces = seg.pieces();
    let mut subs: Vec<Piece> = Vec::new();
    for (i, piece) in pieces.iter().enumerate() {
        let mut t0 = 0.0;
        for &(_, t) in cuts.iter().filter(|(p, _)| *p == i) {
            if t > t0 && t < 1.0 {
                subs.push(piece.subsegment(t0, t));
                t0 = t;
            }
        }
        subs.push(piece.subsegment(t0, 1.0));
    }
    let last = subs.len().saturating_sub(1);
    let mut out = Vec::with_capacity(subs.len());
    for (i, sub) in subs.iter().enumerate() {
        if i == last {
            out.push(RNode { ctrl: sub.ctrl(), ..end.clone() });
        } else {
            let mut node = Node::new(sub.end(), sub.kind());
            node.smooth = sub.kind() != NodeKind::Line;
            out.push(RNode::new(node, sub.ctrl()));
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    fn on(x: f64, y: f64, kind: NodeKind) -> Node {
        Node::new(Point::new(x, y), kind)
    }

    fn off(x: f64, y: f64) -> Node {
        Node::off(Point::new(x, y))
    }

    #[test]
    fn a_closed_contour_that_ends_with_off_curve_nodes_round_trips() {
        let c = Contour::new(
            true,
            vec![
                on(0.0, 0.0, NodeKind::Curve),
                on(100.0, 0.0, NodeKind::Line),
                off(100.0, 50.0),
                off(50.0, 100.0),
                on(0.0, 100.0, NodeKind::Curve),
                off(-20.0, 60.0),
                off(-20.0, 20.0),
            ],
        );
        let ring = Ring::from_contour(&c).unwrap();
        assert_eq!(ring.len(), 3);
        assert_eq!(ring.to_contour(), c);
        assert_eq!(ring.list_index(2), 4);
        // Reversing twice gives the contour back, and once keeps the start and flips the area.
        let r = ring.reversed();
        assert_eq!(r.reversed().to_contour(), c);
        assert_eq!(r.nodes[0].node.point, Point::new(0.0, 0.0));
        assert!((r.area() + ring.area()).abs() < 1e-9);
        assert_eq!(c.reversed().unwrap().nodes.len(), 7);
    }

    #[test]
    fn malformed_node_lists_are_refused() {
        let c = Contour::new(true, vec![on(0.0, 0.0, NodeKind::Line), off(5.0, 5.0), on(10.0, 0.0, NodeKind::Curve)]);
        assert!(matches!(Ring::from_contour(&c), Err(PathOpError::Malformed(_))));
        let c = Contour::new(false, vec![off(5.0, 5.0), on(10.0, 0.0, NodeKind::Line)]);
        assert!(matches!(Ring::from_contour(&c), Err(PathOpError::Malformed(_))));
        let c = Contour::new(true, vec![on(f64::NAN, 0.0, NodeKind::Line)]);
        assert_eq!(Ring::from_contour(&c), Err(PathOpError::NotFinite));
    }

    #[test]
    fn bez_paths_become_node_lists_without_the_closing_line() {
        let path = BezPath::from_svg("M0,0 L100,0 L100,100 L0,0 Z M0,0 C0,50 50,100 100,100 L200,0").unwrap();
        let cs = contours_from_bez(&path);
        assert_eq!(cs.len(), 2);
        assert!(cs[0].closed && !cs[1].closed);
        assert_eq!(cs[0].nodes.len(), 3);
        assert_eq!(cs[1].nodes.len(), 5);
        let back = cs[1].to_bez().unwrap();
        assert_eq!(back.elements().len(), 3);
    }

    #[test]
    fn a_cut_quadratic_spline_keeps_its_shape() {
        let c = Contour::new(false, vec![on(0.0, 0.0, NodeKind::Line), off(0.0, 100.0), off(100.0, 100.0), on(100.0, 0.0, NodeKind::QCurve)]);
        let ring = Ring::from_contour(&c).unwrap();
        let nodes = split_segment(&ring, 1, &[(0, 0.5), (1, 0.25)]).unwrap();
        // Two cuts and the implied point between the pieces become nodes.
        assert_eq!(nodes.len(), 4);
        let mut split = ring.clone();
        split.nodes.truncate(1);
        split.nodes.extend(nodes);
        let a = ring.to_bez();
        let b = split.to_bez();
        assert!((a.area() - b.area()).abs() < 1e-6);
        assert_eq!(split.nodes.last().unwrap().node.point, Point::new(100.0, 0.0));
    }
}
