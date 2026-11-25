//! Union, difference and intersection with the nonzero rule, on Bézier paths and on contours, and
//! the conversion of their results back to nodes (docs/plans/M3.md › M3.6, input rule 7).

use std::collections::{HashMap, HashSet};

use fontcraft_geom::{BezPath, PathEl, Point};

use crate::contour::{Contour, Node, NodeKind, RNode, Ring, dist_to_segment, round_to_grid};
use crate::{MAX_SEGMENTS, PathOpError, merge_close_points, prepare, snap_point};

/// A result node with no input node at its place is smooth when its two directions are within
/// this many degrees (the Tidy up Paths rule).
pub const SMOOTH_MAX_DEGREES: f64 = 1.0;
/// A closed result contour with less area than this, in square units, after rounding is dropped.
const MIN_AREA: f64 = 1e-9;
/// An on-curve point between two quadratic pieces this close to the midpoint of their off-curve
/// points is an implied point of one spline.
const IMPLIED_EPSILON: f64 = 1e-9;
/// A point between two straight segments this close, in units, to the line through its neighbours
/// is on that line. Inputs are rounded to 1 / [`crate::SNAP`] units, which moves a point of one
/// contour off the edge of another by less than this.
const FLAT_DISTANCE: f64 = 2.0 / crate::SNAP;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoolOp {
    Union,
    /// What is in the first set and not in the second.
    Difference,
    Intersection,
}

/// What Remove Overlap does with open paths and with paths of one node, which take no part in
/// the union.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OpenPaths {
    /// Leave them as they are, after the merged contours.
    Keep,
    /// Delete them, as the handbook describes Remove Overlap.
    #[default]
    Remove,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverlapOptions {
    /// Result points are rounded to multiples of this many units; 0 leaves them unrounded.
    pub grid: f64,
    pub open_paths: OpenPaths,
}

impl Default for OverlapOptions {
    fn default() -> Self {
        Self { grid: 1.0, open_paths: OpenPaths::Remove }
    }
}

/// `op` applied to the closed contours of `a` and `b`, each set filled with the nonzero rule.
/// Each result contour is simple and ends with an explicit line back to its start before the
/// close; outer contours run counter-clockwise (y up), counters clockwise.
pub fn binary(a: &[BezPath], b: &[BezPath], op: BoolOp) -> Result<Vec<BezPath>, PathOpError> {
    let pa = prepare(a)?;
    let pb = prepare(b)?;
    let segments = segment_count(&pa) + segment_count(&pb);
    if segments > MAX_SEGMENTS {
        return Err(PathOpError::TooManySegments(segments));
    }
    let empty_a = segment_count(&pa) == 0;
    let empty_b = segment_count(&pb) == 0;
    let nothing = match op {
        BoolOp::Union => empty_a && empty_b,
        BoolOp::Difference => empty_a,
        BoolOp::Intersection => empty_a || empty_b,
    };
    if nothing {
        return Ok(Vec::new());
    }
    let op = match op {
        BoolOp::Union => linesweeper::BinaryOp::Union,
        BoolOp::Difference => linesweeper::BinaryOp::Difference,
        BoolOp::Intersection => linesweeper::BinaryOp::Intersection,
    };
    let contours = linesweeper::binary_op(&pa, &pb, linesweeper::FillRule::NonZero, op).map_err(|e| PathOpError::Engine(e.to_string()))?;
    Ok(contours.into_contours().map(|c| merge_close_points(&c.path)).collect())
}

/// What is in `a` and not in `b`, as [`binary`] gives it.
pub fn difference(a: &[BezPath], b: &[BezPath]) -> Result<Vec<BezPath>, PathOpError> {
    binary(a, b, BoolOp::Difference)
}

/// What is in both `a` and `b`, as [`binary`] gives it.
pub fn intersection(a: &[BezPath], b: &[BezPath]) -> Result<Vec<BezPath>, PathOpError> {
    binary(a, b, BoolOp::Intersection)
}

fn segment_count(path: &BezPath) -> usize {
    path.elements().iter().filter(|e| !matches!(e, PathEl::MoveTo(_) | PathEl::ClosePath)).count()
}

/// Does this contour take part in a boolean operation? Open paths and paths of a single node
/// (stray nodes) do not.
pub(crate) fn takes_part(c: &Contour) -> bool {
    c.closed && c.nodes.len() > 1
}

/// Remove Overlap on one layer's contours: the union of the closed ones, as nodes (rule 7), then
/// the open paths and stray nodes as `options.open_paths` says.
pub fn remove_overlap(contours: &[Contour], options: OverlapOptions) -> Result<Vec<Contour>, PathOpError> {
    let closed: Vec<&Contour> = contours.iter().filter(|c| takes_part(c)).collect();
    let mut out = combine(&closed, &[], BoolOp::Union, options.grid)?;
    if options.open_paths == OpenPaths::Keep {
        out.extend(contours.iter().filter(|c| !takes_part(c) && !c.nodes.is_empty()).cloned());
    }
    Ok(out)
}

/// `op` on the closed contours of `a` and `b`, as nodes (rule 7). Open paths and stray nodes of
/// both sets follow the result unchanged, those of `a` first.
pub fn boolean(a: &[Contour], b: &[Contour], op: BoolOp, grid: f64) -> Result<Vec<Contour>, PathOpError> {
    let ca: Vec<&Contour> = a.iter().filter(|c| takes_part(c)).collect();
    let cb: Vec<&Contour> = b.iter().filter(|c| takes_part(c)).collect();
    let mut out = combine(&ca, &cb, op, grid)?;
    out.extend(a.iter().chain(b).filter(|c| !takes_part(c) && !c.nodes.is_empty()).cloned());
    Ok(out)
}

fn combine(a: &[&Contour], b: &[&Contour], op: BoolOp, grid: f64) -> Result<Vec<Contour>, PathOpError> {
    let total: usize = a.iter().chain(b).map(|c| c.nodes.len()).sum();
    if total > crate::MAX_NODES {
        return Err(PathOpError::TooManyNodes(total));
    }
    let ra = a.iter().map(|c| Ring::from_contour(c)).collect::<Result<Vec<_>, _>>()?;
    let rb = b.iter().map(|c| Ring::from_contour(c)).collect::<Result<Vec<_>, _>>()?;
    let pa: Vec<BezPath> = ra.iter().map(Ring::to_bez).collect();
    let pb: Vec<BezPath> = rb.iter().map(Ring::to_bez).collect();
    let results = binary(&pa, &pb, op)?;
    let inputs: Vec<&Ring> = ra.iter().chain(&rb).collect();
    Ok(to_contours(&results, &inputs, grid))
}

/// The grid point an input or result point is compared on.
fn key(p: Point, grid: f64) -> (u64, u64) {
    let p = round_to_grid(snap_point(p), grid);
    ((p.x + 0.0).to_bits(), (p.y + 0.0).to_bits())
}

/// Input nodes by place, so result nodes that are input nodes get the input's exact point, tag
/// and smooth flag back.
struct InputNodes {
    on: HashMap<(u64, u64), Node>,
    off: HashMap<(u64, u64), Node>,
    /// The start point of each input contour, in input order.
    starts: Vec<(u64, u64)>,
    /// Where an input contour has a node on a straight line.
    flat: HashSet<(u64, u64)>,
}

impl InputNodes {
    fn new(inputs: &[&Ring], grid: f64) -> Self {
        let mut on = HashMap::new();
        let mut off = HashMap::new();
        let mut starts = Vec::with_capacity(inputs.len());
        let mut flat = HashSet::new();
        for ring in inputs {
            if let Some(first) = ring.nodes.first() {
                starts.push(key(first.node.point, grid));
            }
            for (j, r) in ring.nodes.iter().enumerate() {
                if is_flat(ring, j) {
                    flat.insert(key(r.node.point, grid));
                }
                on.entry(key(r.node.point, grid)).or_insert(r.node);
                for c in &r.ctrl {
                    off.entry(key(c.point, grid)).or_insert(*c);
                }
            }
        }
        Self { on, off, starts, flat }
    }
}

/// Rule 7: engine results as contours. The explicit closing line goes (the close draws it),
/// points are rounded to `grid`, nodes that are input nodes keep the input's point, tag and smooth
/// flag, and each contour starts at the first input start point it passes through. Contours that
/// do are ordered by that input contour; the others follow in engine order. Quadratic pieces that
/// meet at the midpoint of their off-curve points become one spline again. A point the operation
/// left on a straight line goes; a node an input contour already had on a straight line stays, and
/// so does a contour's start.
pub(crate) fn to_contours(results: &[BezPath], inputs: &[&Ring], grid: f64) -> Vec<Contour> {
    let known = InputNodes::new(inputs, grid);
    let mut out: Vec<(usize, Contour)> = Vec::new();
    for path in results {
        for ring in rings_of(path) {
            if let Some((order, ring)) = settle(ring, &known, grid) {
                out.push((order, ring.to_contour()));
            }
        }
    }
    out.sort_by_key(|(order, _)| *order);
    out.into_iter().map(|(_, c)| c).collect()
}

/// The closed subpaths of an engine result as rings whose first node is the subpath's start.
fn rings_of(path: &BezPath) -> Vec<Ring> {
    let mut out = Vec::new();
    let mut start = None::<Point>;
    let mut current = Point::ZERO;
    let mut nodes: Vec<RNode> = Vec::new();
    let mut finish = |start: Point, current: Point, nodes: &mut Vec<RNode>| {
        if nodes.is_empty() {
            return;
        }
        if current != start {
            nodes.push(RNode::new(Node::new(start, NodeKind::Line), Vec::new()));
        }
        // The last segment ends at the start: it becomes the start node's incoming segment.
        nodes.rotate_right(1);
        out.push(Ring { closed: true, nodes: std::mem::take(nodes) });
    };
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                if let Some(s) = start {
                    finish(s, current, &mut nodes);
                }
                nodes.clear();
                start = Some(p);
                current = p;
            }
            PathEl::LineTo(p) => {
                nodes.push(RNode::new(Node::new(p, NodeKind::Line), Vec::new()));
                current = p;
            }
            PathEl::QuadTo(c, p) => {
                nodes.push(RNode::new(Node::new(p, NodeKind::QCurve), vec![Node::off(c)]));
                current = p;
            }
            PathEl::CurveTo(a, b, p) => {
                nodes.push(RNode::new(Node::new(p, NodeKind::Curve), vec![Node::off(a), Node::off(b)]));
                current = p;
            }
            PathEl::ClosePath => {
                if let Some(s) = start.take() {
                    finish(s, current, &mut nodes);
                }
                nodes.clear();
            }
        }
    }
    if let Some(s) = start {
        finish(s, current, &mut nodes);
    }
    out
}

/// One result ring as nodes, with its sort order; `None` when nothing is left after rounding.
fn settle(mut ring: Ring, known: &InputNodes, grid: f64) -> Option<(usize, Ring)> {
    let keys: Vec<(u64, u64)> = ring.nodes.iter().map(|r| key(r.node.point, grid)).collect();
    // Start where an input contour started, the earliest input first.
    let found = known.starts.iter().enumerate().find_map(|(i, s)| keys.iter().position(|k| k == s).map(|at| (i, at)));
    let order = match found {
        Some((i, at)) => {
            ring.nodes.rotate_left(at);
            i
        }
        None => {
            // Otherwise start at a node that is not an implied point of a spline.
            let at = (0..ring.len()).find(|&j| !implied(&ring, j, known, grid)).unwrap_or(0);
            ring.nodes.rotate_left(at);
            usize::MAX
        }
    };
    // Merge implied points of quadratic splines, never the start.
    let mut j = 1;
    while j < ring.len() {
        if implied(&ring, j, known, grid) {
            let gone = ring.nodes.remove(j);
            let len = ring.nodes.len().max(1);
            if let Some(next) = ring.nodes.get_mut(j % len) {
                let mut ctrl = gone.ctrl;
                ctrl.append(&mut next.ctrl);
                next.ctrl = ctrl;
            }
        } else {
            j += 1;
        }
    }
    // Points left on a straight line where contours met go, unless the line had a node there
    // before or the contour starts there.
    let mut j = usize::from(found.is_some());
    while j < ring.len() && ring.len() > 2 {
        let drawn = ring.nodes.get(j).is_some_and(|r| known.flat.contains(&key(r.node.point, grid)));
        if !drawn && is_flat(&ring, j) {
            ring.nodes.remove(j);
        } else {
            j += 1;
        }
    }
    // Input nodes come back as they were; every other point goes on the grid.
    let mut matched = Vec::with_capacity(ring.len());
    for r in &mut ring.nodes {
        let k = key(r.node.point, grid);
        match known.on.get(&k) {
            Some(n) => {
                r.node.point = n.point;
                r.node.smooth = n.smooth;
                r.node.tag = n.tag;
                matched.push(true);
            }
            None => {
                r.node.point = round_to_grid(r.node.point, grid);
                matched.push(false);
            }
        }
        for c in &mut r.ctrl {
            match known.off.get(&key(c.point, grid)) {
                Some(n) => {
                    c.point = n.point;
                    c.tag = n.tag;
                }
                None => c.point = round_to_grid(c.point, grid),
            }
        }
    }
    for (r, m) in ring.nodes.iter_mut().zip(&matched) {
        r.mark = !m;
    }
    ring.remove_zero_length(0.0, |_, _| true);
    for j in 0..ring.len() {
        if ring.nodes.get(j).is_some_and(|r| r.mark) {
            let smooth = ring.looks_smooth(j, SMOOTH_MAX_DEGREES).unwrap_or(false);
            if let Some(r) = ring.nodes.get_mut(j) {
                r.node.smooth = smooth;
            }
        }
    }
    for r in &mut ring.nodes {
        r.mark = false;
    }
    if ring.nodes.is_empty() || ring.area().abs() < MIN_AREA {
        return None;
    }
    Some((order, ring))
}

/// Does node `j` lie on a straight line, between two straight segments that run the same way?
fn is_flat(ring: &Ring, j: usize) -> bool {
    let Some(next) = ring.next(j) else { return false };
    let (Some(into), Some(out)) = (ring.seg(j), ring.seg(next)) else { return false };
    if ring.len() < 3 || into.kind != NodeKind::Line || out.kind != NodeKind::Line || into.is_curve() || out.is_curve() {
        return false;
    }
    (into.to - into.from).dot(out.to - out.from) > 0.0 && dist_to_segment(into.to, into.from, out.to) <= FLAT_DISTANCE
}

/// Is node `j` an implied on-curve point between two quadratic pieces, and not an input node?
fn implied(ring: &Ring, j: usize, known: &InputNodes, grid: f64) -> bool {
    let Some(r) = ring.nodes.get(j) else { return false };
    let Some(next) = ring.next(j).and_then(|n| ring.nodes.get(n)) else { return false };
    if ring.len() < 2 || r.node.kind != NodeKind::QCurve || next.node.kind != NodeKind::QCurve {
        return false;
    }
    let (Some(a), Some(b)) = (r.ctrl.last(), next.ctrl.first()) else { return false };
    r.node.point.distance(a.point.midpoint(b.point)) < IMPLIED_EPSILON && !known.on.contains_key(&key(r.node.point, grid))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    use crate::coverage_difference;
    use crate::fixtures::rect;
    use fontcraft_geom::Shape;

    fn bez(cs: &[Contour]) -> Vec<BezPath> {
        cs.iter().map(|c| c.to_bez().unwrap()).collect()
    }

    #[test]
    fn two_overlapping_squares_merge_and_keep_the_first_start() {
        let a = rect(0.0, 0.0, 100.0, 100.0).tagged();
        let b = rect(50.0, 50.0, 150.0, 150.0);
        let out = remove_overlap(&[a.clone(), b.clone()], OverlapOptions::default()).unwrap();
        assert_eq!(out.len(), 1);
        let c = &out[0];
        assert!(c.closed);
        // Six corners, no node repeated at the start, starting where the first square started.
        assert_eq!(c.nodes.len(), 8);
        assert_eq!(c.nodes[0].point, Point::new(0.0, 0.0));
        assert_eq!(c.nodes[0].tag, Some(0));
        assert!(c.nodes.iter().all(|n| n.kind == NodeKind::Line && !n.smooth));
        assert!((c.to_bez().unwrap().area() - 17_500.0).abs() < 1e-6);
        // The intersections are on the grid.
        assert!(c.nodes.iter().any(|n| n.point == Point::new(100.0, 50.0)));
    }

    #[test]
    fn points_a_merge_leaves_on_a_straight_line_go() {
        // A descender under a stem of the same width: its top corners end up on the stem's sides.
        // The node drawn on the stem's right side stays.
        let mut stem = rect(0.0, 0.0, 100.0, 500.0);
        stem.nodes.insert(2, Node::new(Point::new(100.0, 300.0), NodeKind::Line));
        let tail = rect(0.0, -200.0, 100.0, 50.0);
        let out = remove_overlap(&[tail, stem], OverlapOptions::default()).unwrap();
        let points: Vec<(f64, f64)> = out.iter().flat_map(|c| &c.nodes).map(|n| (n.point.x, n.point.y)).collect();
        assert_eq!(points, vec![(0.0, -200.0), (100.0, -200.0), (100.0, 300.0), (100.0, 500.0), (0.0, 500.0)]);
        // A slanted edge, whose meeting points are not on the grid: nothing is left on it, and
        // the contour keeps its start although the merge puts that on a straight line.
        let lower = Contour::new(true, rect(0.0, 0.0, 100.0, 100.0).nodes.into_iter().map(|n| slant(n, 0.1)).collect());
        let upper = Contour::new(true, rect(0.0, 100.0, 100.0, 333.0).nodes.into_iter().map(|n| slant(n, 0.1)).collect());
        let out = remove_overlap(&[lower.clone(), upper.clone()], OverlapOptions { grid: 0.0, ..OverlapOptions::default() }).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].nodes.len(), 4, "{out:?}");
        // The upper contour starts at a corner that ends up on the left edge, and keeps that start.
        let out = remove_overlap(&[upper, lower], OverlapOptions { grid: 0.0, ..OverlapOptions::default() }).unwrap();
        assert_eq!(out[0].nodes[0].point, Point::new(10.0, 100.0));
        assert_eq!(out[0].nodes.len(), 5, "{out:?}");
    }

    fn slant(mut n: Node, by: f64) -> Node {
        n.point.x += by * n.point.y;
        n
    }

    #[test]
    fn subtract_and_intersect_two_squares() {
        let a = rect(0.0, 0.0, 100.0, 100.0);
        let b = rect(50.0, 50.0, 150.0, 150.0);
        let d = boolean(std::slice::from_ref(&a), std::slice::from_ref(&b), BoolOp::Difference, 1.0).unwrap();
        assert_eq!(d.len(), 1);
        assert!((d[0].to_bez().unwrap().area() - 7_500.0).abs() < 1e-6);
        let i = boolean(&[a], &[b], BoolOp::Intersection, 1.0).unwrap();
        assert_eq!(i.len(), 1);
        assert!((i[0].to_bez().unwrap().area() - 2_500.0).abs() < 1e-6);
        // A square inside another, subtracted, punches a clockwise counter.
        let outer = rect(0.0, 0.0, 300.0, 300.0);
        let inner = rect(100.0, 100.0, 200.0, 200.0);
        let o = boolean(&[outer], &[inner], BoolOp::Difference, 1.0).unwrap();
        assert_eq!(o.len(), 2);
        let mut areas: Vec<f64> = o.iter().map(|c| c.to_bez().unwrap().area()).collect();
        areas.sort_by(f64::total_cmp);
        assert_eq!(areas, vec![-10_000.0, 90_000.0]);
    }

    #[test]
    fn a_figure_eight_becomes_two_lobes() {
        // One contour crossing itself once: with the nonzero rule both lobes are filled, one
        // drawn each way.
        let eight = crate::contour::contours_from_bez(&BezPath::from_svg("M0,0 L100,100 L100,0 L0,100 Z").unwrap());
        let out = remove_overlap(&eight, OverlapOptions::default()).unwrap();
        assert_eq!(out.iter().map(|c| c.nodes.len()).sum::<usize>(), 6);
        for c in &out {
            assert!(c.to_bez().unwrap().area() > 0.0, "every lobe is outer, so counter-clockwise");
        }
        let d = coverage_difference(&bez(&eight), &bez(&out)).unwrap();
        assert!(d.max_pixel < 1e-9, "{d:?}");
    }

    #[test]
    fn open_paths_and_stray_nodes_follow_the_option() {
        let open = Contour::new(false, vec![Node::new(Point::new(0.0, 0.0), NodeKind::Line), Node::new(Point::new(10.0, 0.0), NodeKind::Line)]);
        let stray = Contour::new(true, vec![Node::new(Point::new(5.0, 5.0), NodeKind::Line)]);
        let square = rect(0.0, 0.0, 10.0, 10.0);
        let input = [open.clone(), square, stray.clone()];
        let kept = remove_overlap(&input, OverlapOptions { grid: 1.0, open_paths: OpenPaths::Keep }).unwrap();
        assert_eq!(kept.len(), 3);
        assert_eq!(kept[1], open);
        assert_eq!(kept[2], stray);
        let removed = remove_overlap(&input, OverlapOptions::default()).unwrap();
        assert_eq!(removed.len(), 1);
        assert!(removed[0].closed);
    }

    #[test]
    fn a_contour_without_overlaps_comes_back_unchanged() {
        let mut c = crate::contour::contours_from_bez(&BezPath::from_svg("M0,0 L100,0 C150,0 200,50 200,100 L0,100 Z").unwrap()).remove(0);
        c.nodes[1].smooth = true;
        let c = c.tagged();
        let out = remove_overlap(std::slice::from_ref(&c), OverlapOptions::default()).unwrap();
        assert_eq!(out, vec![c]);
        // The same without a grid: input points are restored exactly.
        let mut d = rect(0.25, 0.0, 10.0, 10.0);
        d.nodes[1].point.x = 10.000_01;
        let out = remove_overlap(std::slice::from_ref(&d), OverlapOptions { grid: 0.0, open_paths: OpenPaths::Keep }).unwrap();
        assert_eq!(out, vec![d]);
    }

    #[test]
    fn quadratic_splines_stay_splines() {
        let o = |x, y| Node::off(Point::new(x, y));
        let q = Node::new(Point::new(0.0, 0.0), NodeKind::QCurve);
        // One on-curve node and four off-curve nodes: three implied on-curve points.
        let spline = Contour::new(true, vec![q, o(100.0, 0.0), o(100.0, 100.0), o(-100.0, 100.0), o(-100.0, 0.0)]);
        let out = remove_overlap(std::slice::from_ref(&spline), OverlapOptions::default()).unwrap();
        assert_eq!(out, vec![spline]);
    }
}
