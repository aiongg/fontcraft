//! Round Corners (`filter.roundCorners`, `filter.roundCorners.selectionRule`,
//! `filter.roundCorners.visualCorrections`, docs/spec/11-filters.md › Round Corners).
//!
//! The worked example of a filter: the schema with its export-string slots, and the operation on
//! one layer's contours.
//!
//! Each rounded corner node becomes two smooth nodes joined by a cubic arc: the first on the
//! incoming segment, the second on the outgoing one, each `d` units (straight-line distance) from
//! the corner, where `d = radius / tan(a / 2)` for a corner of inner angle `a`, so that a straight
//! corner gets a circular arc of the given radius. A curved neighbour is split where the arc
//! meets it and keeps its shape. `d` is at most half of either neighbouring segment, so the
//! radius shrinks where it does not fit; where two roundings meet in the middle of a segment, the
//! segment goes. Of a quadratic spline, the piece next to the corner is the neighbouring segment,
//! and the arc next to a quadratic segment is quadratic.

use std::collections::HashSet;
use std::f64::consts::PI;

use fontcraft_geom::{CubicBez, ParamCurve, Point, QuadBez, Vec2, cubics_to_quadratic_splines};

use super::{Args, Context, Filter, Param, ParamKind, Slot, Value};
use crate::contour::{Contour, Node, NodeKind, RNode, Ring, Seg, round_to_grid};
use crate::direction::correct_direction;
use crate::{MAX_COORDINATE, MAX_NODES, PathOpError};

/// A node whose path turns by less than this many degrees, or by more than 180 minus this, is not
/// a corner.
const MIN_TURN_DEGREES: f64 = 1.0;
/// Visual corrections change the radius by at most this factor either way.
const MAX_VISUAL_FACTOR: f64 = 4.0;
/// A rounding that would start closer than this many units to its corner is left out.
const MIN_TANGENT: f64 = 0.01;
/// Two arc ends closer than this, in curve parameter, are one point.
const SAME_T: f64 = 1e-9;
/// Most distance, in units, between an arc next to a quadratic segment and its cubic.
const ARC_TOLERANCE: f64 = 0.25;
/// An on-curve point this close to the midpoint of the off-curve points around it is the implied
/// point of one spline.
const IMPLIED_EPSILON: f64 = 1e-6;
/// Steps of the search along a curve for the point at a given distance from one end, then
/// bisection steps between the two samples around it.
const SEARCH_STEPS: usize = 64;
const BISECT_STEPS: usize = 48;

pub struct RoundCorners;

const PARAMS: &[Param] = &[
    Param {
        key: "radius",
        label: "Radius",
        kind: ParamKind::Number { min: 0.0, max: MAX_COORDINATE },
        default: Value::Number(20.0),
        required: true,
        slot: Slot::Position(0),
        doc: "radius of the rounding, in units",
    },
    Param {
        key: "visualCorrections",
        label: "Visual corrections",
        kind: ParamKind::Bool,
        default: Value::Bool(false),
        required: false,
        slot: Slot::Position(1),
        doc: "every rounding starts `radius` units from its corner: a smaller radius at sharp corners, a larger one at blunt corners",
    },
];

const DOC: &str = "{glyph: name | glyphs: [names], layer?: id | layers?: [ids] (default: the first master's layer), points?: [[shape, node]], segments?: [[shape, node]] (with one glyph and one layer), radius: units (0 to 1000000), visualCorrections?: bool (default false)} replaces corner nodes with arcs of the given radius. With points or segments, exactly the selected corners are rounded (selected nodes and the end nodes of selected segments), inward or outward; without them, only outward-pointing corners, and every corner of an open path. With visualCorrections every rounding starts `radius` units from its corner, so sharp corners get a smaller radius and blunt corners a larger one. The radius shrinks where it does not fit in half of a neighbouring segment. Next to a quadratic spline that segment is the piece the corner touches, and the arc is quadratic. Smooth nodes stay; components and locked paths are not changed; new points are rounded to the grid. Export string: RoundCorner; radius; visualcorrection → {glyphs: [names], skipped: [{glyph, layer?, reason}]}";

impl Filter for RoundCorners {
    fn id(&self) -> &'static str {
        "filter.roundCorners"
    }

    fn name(&self) -> &'static str {
        "RoundCorner"
    }

    fn label(&self) -> &'static str {
        "Round Corners"
    }

    fn doc(&self) -> &'static str {
        DOC
    }

    fn params(&self) -> &'static [Param] {
        PARAMS
    }

    fn takes_selection(&self) -> bool {
        true
    }

    fn apply(&self, contours: &[Contour], args: &Args, cx: &Context) -> Result<Vec<Contour>, PathOpError> {
        round_corners(contours, args.number("radius"), args.flag("visualCorrections"), cx)
    }
}

/// Round Corners on one layer's contours. See the module docs for the geometry; the selection
/// rule is in [`Filter::takes_selection`]'s terms: with `cx.selection`, exactly those corners;
/// without, the corners that point out of the filled area, and every corner of an open contour.
pub fn round_corners(contours: &[Contour], radius: f64, visual: bool, cx: &Context) -> Result<Vec<Contour>, PathOpError> {
    if !radius.is_finite() {
        return Err(PathOpError::NotFinite);
    }
    if !(0.0..=MAX_COORDINATE).contains(&radius) {
        return Err(PathOpError::Invalid(format!("the radius ({radius}) must be from 0 to {MAX_COORDINATE} units")));
    }
    let total: usize = contours.iter().map(|c| c.nodes.len()).sum();
    if total > MAX_NODES {
        return Err(PathOpError::TooManyNodes(total));
    }
    let rings = contours.iter().map(Ring::from_contour).collect::<Result<Vec<_>, _>>()?;
    if radius == 0.0 {
        return Ok(contours.to_vec());
    }
    let selected: HashSet<(usize, usize)> = cx.selection.iter().copied().collect();
    let sides = if selected.is_empty() { fill_sides(contours, &rings)? } else { Vec::new() };
    let mut out = Vec::with_capacity(rings.len());
    let mut count = 0usize;
    for (i, ring) in rings.iter().enumerate() {
        let pick = |src: Option<usize>, cross: f64| {
            if selected.is_empty() {
                let side = sides.get(i).copied().unwrap_or(0.0);
                side == 0.0 || cross * side > 0.0
            } else {
                src.is_some_and(|s| selected.contains(&(i, s)))
            }
        };
        let contour = joined(round_ring(&expanded(ring), radius, visual, cx.grid, &pick), cx.grid).to_contour();
        for n in &contour.nodes {
            crate::check(n.point.x)?;
            crate::check(n.point.y)?;
        }
        count = count.saturating_add(contour.nodes.len());
        if count > MAX_NODES {
            return Err(PathOpError::TooManyNodes(count));
        }
        out.push(contour);
    }
    Ok(out)
}

/// For each contour: 1 when the filled area lies to the left of its direction, -1 when it lies to
/// the right, 0 for an open contour or one without area. Correct Path Direction turns every closed
/// contour so the filled area is on its left; a contour it reverses has it on the right.
pub(crate) fn fill_sides(contours: &[Contour], rings: &[Ring]) -> Result<Vec<f64>, PathOpError> {
    let directed = correct_direction(contours)?;
    let mut sides = vec![0.0; rings.len()];
    for (k, &i) in directed.order.iter().enumerate() {
        let (Some(ring), Some(c), Some(side)) = (rings.get(i), directed.contours.get(k), sides.get_mut(i)) else { continue };
        if !ring.closed {
            continue;
        }
        let (before, after) = (ring.area(), Ring::from_contour(c)?.area());
        *side = if before == 0.0 {
            0.0
        } else if (before > 0.0) == (after > 0.0) {
            1.0
        } else {
            -1.0
        };
    }
    Ok(sides)
}

/// Where the arc of one corner meets its two segments.
#[derive(Clone, Copy, Debug)]
struct Corner {
    /// Curve parameter of the arc's start on the incoming segment.
    t_in: f64,
    /// Curve parameter of the arc's end on the outgoing segment.
    t_out: f64,
    p1: Point,
    /// Unit direction of the incoming segment at `p1`.
    d1: Vec2,
    p2: Point,
    /// Unit direction of the outgoing segment at `p2`.
    d2: Vec2,
}

/// The ring with a marked node at each implied on-curve point of its quadratic splines, so every
/// segment is one line, quadratic or cubic.
fn expanded(ring: &Ring) -> Ring {
    let mut nodes = Vec::with_capacity(ring.len());
    let mut tail = Vec::new();
    for (j, r) in ring.nodes.iter().enumerate() {
        let mut implied = Vec::new();
        let mut last = r.clone();
        if r.node.kind == NodeKind::QCurve {
            for pair in r.ctrl.windows(2) {
                let [a, b] = pair else { continue };
                let node = Node { smooth: true, ..Node::new(a.point.midpoint(b.point), NodeKind::QCurve) };
                implied.push(RNode { node, ctrl: vec![*a], src: None, mark: true });
            }
            last.ctrl = r.ctrl.last().copied().into_iter().collect();
        }
        if j == 0 {
            // The start stays first; the points of the closing segment go at the end.
            tail = implied;
        } else {
            nodes.extend(implied);
        }
        nodes.push(last);
    }
    nodes.extend(tail);
    Ring { closed: ring.closed, nodes }
}

/// [`expanded`] undone: a marked node still between two quadratic segments whose off-curve points
/// imply it goes, and any other becomes a node on the grid.
fn joined(mut ring: Ring, grid: f64) -> Ring {
    let mut j = 1;
    while j < ring.len() {
        let len = ring.len();
        let next = ring.nodes.get((j + 1) % len).filter(|n| n.node.kind == NodeKind::QCurve).and_then(|n| n.ctrl.first());
        let Some(r) = ring.nodes.get(j) else { break };
        let implied = r.mark
            && r.node.kind == NodeKind::QCurve
            && matches!((r.ctrl.last(), next), (Some(a), Some(b)) if r.node.point.distance(a.point.midpoint(b.point)) <= IMPLIED_EPSILON);
        if implied {
            let gone = ring.nodes.remove(j);
            let len = ring.len().max(1);
            if let Some(n) = ring.nodes.get_mut(j % len) {
                let mut ctrl = gone.ctrl;
                ctrl.append(&mut n.ctrl);
                n.ctrl = ctrl;
            }
            continue;
        }
        if let Some(r) = ring.nodes.get_mut(j).filter(|r| r.mark) {
            r.node.point = round_to_grid(r.node.point, grid);
            r.mark = false;
        }
        j += 1;
    }
    ring
}

/// `ring` (its splines [`expanded`]) with the corners `pick` takes, by their index in the contour
/// and their turn, rounded.
fn round_ring(ring: &Ring, radius: f64, visual: bool, grid: f64, pick: &impl Fn(Option<usize>, f64) -> bool) -> Ring {
    let m = ring.len();
    let mut corners: Vec<Option<Corner>> = (0..m).map(|j| corner(ring, j, radius, visual, pick)).collect();
    let at = |corners: &[Option<Corner>], j: Option<usize>| j.and_then(|j| corners.get(j).copied().flatten());
    // Two roundings that would overlap on a curve: the second goes. Dropping one only frees room,
    // so one pass is enough.
    for k in ring.segment_ends() {
        let (Some(prev), Some(this)) = (at(&corners, ring.prev(k)), at(&corners, Some(k))) else { continue };
        if this.t_in < prev.t_out - SAME_T
            && let Some(c) = corners.get_mut(k)
        {
            *c = None;
        }
    }
    // Two roundings that meet: the segment between them goes.
    let collapsed: Vec<bool> = (0..m)
        .map(|k| {
            ring.seg(k).is_some()
                && matches!((at(&corners, ring.prev(k)), at(&corners, Some(k))), (Some(prev), Some(this)) if (this.t_in - prev.t_out).abs() <= SAME_T)
        })
        .collect();
    let round = |p: Point| round_to_grid(p, grid);
    let mut nodes: Vec<RNode> = Vec::with_capacity(m + 3 * corners.iter().flatten().count());
    for (k, r) in ring.nodes.iter().enumerate() {
        let Some(seg) = ring.seg(k) else {
            nodes.push(r.clone());
            continue;
        };
        let s = at(&corners, ring.prev(k)).map_or(0.0, |c| c.t_out);
        let this = at(&corners, Some(k));
        let e = this.map_or(1.0, |c| c.t_in);
        let ctrl = match Bez::of(&seg) {
            Some(b) if s > 0.0 || e < 1.0 => b.sub(s, e).ctrl().into_iter().map(|p| Node::off(round(p))).collect(),
            _ => r.ctrl.clone(),
        };
        let Some(c) = this else {
            nodes.push(RNode { ctrl, ..r.clone() });
            continue;
        };
        let gone = collapsed.get(k).copied().unwrap_or(false);
        if !gone {
            nodes.push(RNode { node: Node { point: round(c.p1), smooth: true, ..r.node }, ctrl, src: r.src, mark: false });
        }
        let quadratic = [Some(seg.kind), ring.next(k).and_then(|n| ring.seg(n)).map(|s| s.kind)].contains(&Some(NodeKind::QCurve));
        let (kind, points) = arc(&c, quadratic);
        nodes.push(RNode {
            node: Node { point: round(c.p2), kind, smooth: true, tag: if gone { r.node.tag } else { None } },
            ctrl: points.into_iter().map(|p| Node::off(round(p))).collect(),
            src: None,
            mark: false,
        });
    }
    Ring { closed: ring.closed, nodes }
}

/// The rounding of node `j`, or `None` when it is not a corner to round.
fn corner(ring: &Ring, j: usize, radius: f64, visual: bool, pick: &impl Fn(Option<usize>, f64) -> bool) -> Option<Corner> {
    if ring.len() < 2 || ring.nodes.get(j)?.node.smooth {
        return None;
    }
    let incoming = Bez::of(&ring.seg(j)?)?;
    let outgoing = Bez::of(&ring.seg(ring.next(j)?)?)?;
    let (tin, tout) = (ring.tangent_in(j)?, ring.tangent_out(j)?);
    let cross = tin.cross(tout);
    let turn = cross.abs().atan2(tin.dot(tout));
    let min = MIN_TURN_DEGREES.to_radians();
    if turn < min || turn > PI - min || !pick(ring.nodes.get(j)?.src, cross) {
        return None;
    }
    // Half the corner's inner angle.
    let half = (PI - turn) / 2.0;
    let factor = if visual { half.tan().clamp(1.0 / MAX_VISUAL_FACTOR, MAX_VISUAL_FACTOR) } else { 1.0 };
    let d = (radius * factor / half.tan()).min(incoming.chord() / 2.0).min(outgoing.chord() / 2.0);
    if !(d.is_finite() && d >= MIN_TANGENT) {
        return None;
    }
    let t_in = incoming.param_at_distance(d, End::Last)?;
    let t_out = outgoing.param_at_distance(d, End::First)?;
    Some(Corner {
        t_in,
        t_out,
        p1: incoming.eval(t_in),
        d1: incoming.direction(t_in).unwrap_or(tin),
        p2: outgoing.eval(t_out),
        d2: outgoing.direction(t_out).unwrap_or(tout),
    })
}

/// The arc of a corner: the kind of its end node and its off-curve points.
fn arc(c: &Corner, quadratic: bool) -> (NodeKind, Vec<Point>) {
    let (h1, h2) = arc_handles(c);
    let spline = quadratic.then(|| cubics_to_quadratic_splines(&[CubicBez::new(c.p1, h1, h2, c.p2)], ARC_TOLERANCE)).flatten();
    match spline.as_deref().and_then(<[_]>::first).map(|s| s.points()) {
        Some([_, off @ .., _]) if !off.is_empty() => (NodeKind::QCurve, off.to_vec()),
        _ => (NodeKind::Curve, vec![h1, h2]),
    }
}

/// The handles of a cubic from `p1` to `p2` that leaves along `d1` and arrives along `d2`, sized
/// so that it is close to a circular arc.
fn arc_handles(c: &Corner) -> (Point, Point) {
    let chord = c.p1.distance(c.p2);
    let sweep = c.d1.cross(c.d2).abs().atan2(c.d1.dot(c.d2));
    let k = if sweep > 1e-9 {
        let r = chord / (2.0 * (sweep / 2.0).sin());
        4.0 / 3.0 * (sweep / 4.0).tan() * r
    } else {
        chord / 3.0
    };
    (c.p1 + c.d1 * k, c.p2 - c.d2 * k)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum End {
    First,
    Last,
}

/// A straight, quadratic or cubic segment.
#[derive(Clone, Copy, Debug)]
enum Bez {
    Line(Point, Point),
    Quad(QuadBez),
    Cubic(CubicBez),
}

impl Bez {
    /// `None` for a quadratic spline of several pieces.
    fn of(seg: &Seg) -> Option<Bez> {
        match (seg.kind, seg.ctrl) {
            (NodeKind::Line, []) => Some(Bez::Line(seg.from, seg.to)),
            (NodeKind::QCurve, [c]) => Some(Bez::Quad(QuadBez::new(seg.from, c.point, seg.to))),
            (NodeKind::Curve, [a, b]) => Some(Bez::Cubic(CubicBez::new(seg.from, a.point, b.point, seg.to))),
            _ => None,
        }
    }

    fn eval(&self, t: f64) -> Point {
        match self {
            Bez::Line(a, b) => a.lerp(*b, t),
            Bez::Quad(q) => q.eval(t),
            Bez::Cubic(c) => c.eval(t),
        }
    }

    fn chord(&self) -> f64 {
        self.eval(0.0).distance(self.eval(1.0))
    }

    fn direction(&self, t: f64) -> Option<Vec2> {
        let v = match self {
            Bez::Line(a, b) => *b - *a,
            Bez::Quad(q) => (q.p1 - q.p0) * (1.0 - t) + (q.p2 - q.p1) * t,
            Bez::Cubic(c) => {
                let mt = 1.0 - t;
                (c.p1 - c.p0) * (3.0 * mt * mt) + (c.p2 - c.p1) * (6.0 * mt * t) + (c.p3 - c.p2) * (3.0 * t * t)
            }
        };
        (v.hypot() > 1e-9).then(|| v.normalize())
    }

    fn sub(&self, s: f64, e: f64) -> Bez {
        match self {
            Bez::Line(..) => Bez::Line(self.eval(s), self.eval(e)),
            Bez::Quad(q) => Bez::Quad(q.subsegment(s..e)),
            Bez::Cubic(c) => Bez::Cubic(c.subsegment(s..e)),
        }
    }

    /// The off-curve points.
    fn ctrl(&self) -> Vec<Point> {
        match self {
            Bez::Line(..) => Vec::new(),
            Bez::Quad(q) => vec![q.p1],
            Bez::Cubic(c) => vec![c.p1, c.p2],
        }
    }

    /// The parameter of the point at straight-line distance `d` from one end: the nearest such
    /// point to that end. `None` when no point of the segment is that far.
    fn param_at_distance(&self, d: f64, from: End) -> Option<f64> {
        let at = |u: f64| if from == End::First { u } else { 1.0 - u };
        let anchor = self.eval(at(0.0));
        match self {
            Bez::Line(a, b) => {
                let len = a.distance(*b);
                (len > 0.0 && d <= len).then(|| at(d / len))
            }
            Bez::Quad(_) | Bez::Cubic(_) => {
                let far = |u: f64| self.eval(at(u)).distance(anchor) >= d;
                let mut lo = 0.0;
                for i in 1..=SEARCH_STEPS {
                    let mut hi = i as f64 / SEARCH_STEPS as f64;
                    if far(hi) {
                        for _ in 0..BISECT_STEPS {
                            let mid = (lo + hi) / 2.0;
                            if far(mid) {
                                hi = mid;
                            } else {
                                lo = mid;
                            }
                        }
                        return Some(at(hi));
                    }
                    lo = hi;
                }
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    use crate::fixtures::rect;

    fn poly(points: &[(f64, f64)]) -> Contour {
        Contour::new(true, points.iter().map(|&(x, y)| Node::new(Point::new(x, y), NodeKind::Line)).collect())
    }

    fn on_curve(c: &Contour) -> Vec<(f64, f64)> {
        c.nodes.iter().filter(|n| n.on_curve()).map(|n| (n.point.x, n.point.y)).collect()
    }

    fn close(a: Point, b: (f64, f64)) -> bool {
        a.distance(Point::new(b.0, b.1)) < 1e-6
    }

    #[test]
    fn a_square_gets_four_quarter_circles() {
        let out = round_corners(&[rect(0.0, 0.0, 100.0, 100.0)], 10.0, false, &Context::default()).unwrap();
        let c = &out[0];
        assert_eq!(c.nodes.len(), 16);
        // The corner at the start becomes the end of the closing edge and the arc's end.
        assert!(close(c.nodes[0].point, (0.0, 10.0)) && c.nodes[0].kind == NodeKind::Line && c.nodes[0].smooth);
        let k = 4.0 / 3.0 * (PI / 8.0).tan() * 10.0;
        assert!(close(c.nodes[1].point, (0.0, 10.0 - k)) && c.nodes[1].kind == NodeKind::OffCurve);
        assert!(close(c.nodes[2].point, (10.0 - k, 0.0)));
        assert!(close(c.nodes[3].point, (10.0, 0.0)) && c.nodes[3].kind == NodeKind::Curve);
        assert!(close(c.nodes[4].point, (90.0, 0.0)));
        // A circle of radius 10 cut in four, less its tiny cubic error.
        let area = Ring::from_contour(c).unwrap().area();
        assert!((area - (10_000.0 - (400.0 - 100.0 * PI))).abs() < 0.1, "{area}");
        // On the grid every new point is whole.
        let grid = Context { grid: 1.0, ..Context::default() };
        let out = round_corners(&[rect(0.0, 0.0, 100.0, 100.0)], 10.0, false, &grid).unwrap();
        assert!(out[0].nodes.iter().all(|n| n.point.x.fract() == 0.0 && n.point.y.fract() == 0.0));
    }

    #[test]
    fn only_outward_corners_without_a_selection_and_exactly_the_selected_ones_with_one() {
        // An L, counter-clockwise; node 3 at (50, 50) is the inward corner.
        let l = poly(&[(0.0, 0.0), (200.0, 0.0), (200.0, 50.0), (50.0, 50.0), (50.0, 200.0), (0.0, 200.0)]).tagged();
        let out = round_corners(std::slice::from_ref(&l), 10.0, false, &Context::default()).unwrap();
        let points = on_curve(&out[0]);
        assert_eq!(points.len(), 11);
        assert!(points.contains(&(50.0, 50.0)));
        // Drawn the other way round the same corners are outward.
        let reversed = round_corners(&[l.reversed().unwrap()], 10.0, false, &Context::default()).unwrap();
        assert!(on_curve(&reversed[0]).contains(&(50.0, 50.0)));
        assert_eq!(on_curve(&reversed[0]).len(), 11);

        let cx = Context { selection: &[(0, 3)], ..Context::default() };
        let out = round_corners(std::slice::from_ref(&l), 10.0, false, &cx).unwrap();
        assert_eq!(on_curve(&out[0]), vec![(0.0, 0.0), (200.0, 0.0), (200.0, 50.0), (60.0, 50.0), (50.0, 60.0), (50.0, 200.0), (0.0, 200.0)]);
        // The node keeps its tag on the arc's first point.
        assert_eq!(out[0].nodes[3].tag, Some(3));
    }

    #[test]
    fn a_counter_keeps_its_corners() {
        let outer = rect(0.0, 0.0, 300.0, 300.0);
        let counter = rect(100.0, 100.0, 200.0, 200.0).reversed().unwrap();
        let out = round_corners(&[outer, counter.clone()], 10.0, false, &Context::default()).unwrap();
        assert_eq!(out[0].nodes.len(), 16);
        assert_eq!(out[1], counter);
    }

    #[test]
    fn visual_corrections_start_every_rounding_radius_units_from_its_corner() {
        // Corners of 90, 45 and 45 degrees.
        let t = poly(&[(0.0, 0.0), (100.0, 0.0), (0.0, 100.0)]);
        let plain = round_corners(std::slice::from_ref(&t), 10.0, false, &Context::default()).unwrap();
        let visual = round_corners(std::slice::from_ref(&t), 10.0, true, &Context::default()).unwrap();
        let first_after_origin = |c: &Contour| on_curve(c).into_iter().find(|p| p.1 == 0.0 && p.0 > 50.0).unwrap();
        // Without corrections the 45-degree corner at (100, 0) starts 10 / tan(22.5°) away.
        assert!((first_after_origin(&plain[0]).0 - (100.0 - 10.0 / (PI / 8.0).tan())).abs() < 1e-6);
        assert!((first_after_origin(&visual[0]).0 - 90.0).abs() < 1e-6);
    }

    #[test]
    fn a_radius_too_large_for_a_side_shrinks_and_the_side_goes() {
        // A 40-wide bar: both roundings of each short side meet in its middle.
        let out = round_corners(&[rect(0.0, 0.0, 40.0, 200.0)], 50.0, false, &Context::default()).unwrap();
        let points = on_curve(&out[0]);
        assert_eq!(points.len(), 6, "{points:?}");
        assert!(points.contains(&(20.0, 0.0)) && points.contains(&(20.0, 200.0)));
    }

    #[test]
    fn a_curved_neighbour_is_split_on_its_own_curve() {
        let c = Contour::new(
            true,
            vec![
                Node::new(Point::new(0.0, 0.0), NodeKind::Line),
                Node::new(Point::new(100.0, 0.0), NodeKind::Line),
                Node::off(Point::new(100.0, 55.0)),
                Node::off(Point::new(55.0, 100.0)),
                Node::new(Point::new(0.0, 100.0), NodeKind::Curve),
            ],
        );
        let cx = Context { selection: &[(0, 1)], ..Context::default() };
        let out = round_corners(&[c], 10.0, false, &cx).unwrap();
        let arc_end = out[0].nodes.iter().filter(|n| n.on_curve()).nth(2).unwrap();
        assert!((arc_end.point.distance(Point::new(100.0, 0.0)) - 10.0).abs() < 1e-6);
        let original = CubicBez::new((100.0, 0.0), (100.0, 55.0), (55.0, 100.0), (0.0, 100.0));
        let on_original = (0..=1000).any(|i| original.eval(f64::from(i) / 1000.0).distance(arc_end.point) < 0.2);
        assert!(on_original);
        // A smooth node is not a corner, selected or not.
        let mut smooth = rect(0.0, 0.0, 100.0, 100.0);
        smooth.nodes[1].smooth = true;
        let cx = Context { selection: &[(0, 1)], ..Context::default() };
        assert_eq!(round_corners(std::slice::from_ref(&smooth), 10.0, false, &cx).unwrap()[0], smooth);
    }

    #[test]
    fn a_corner_next_to_a_quadratic_spline_gets_a_quadratic_arc() {
        let o = |x: f64, y: f64| Node::off(Point::new(x, y));
        // A quarter of a round shape: a spline with one implied point, (80, 80), from (100, 0) to
        // (0, 100).
        let c = Contour::new(
            true,
            vec![
                Node::new(Point::new(0.0, 0.0), NodeKind::Line),
                Node::new(Point::new(100.0, 0.0), NodeKind::Line),
                o(100.0, 60.0),
                o(60.0, 100.0),
                Node::new(Point::new(0.0, 100.0), NodeKind::QCurve),
            ],
        );
        let cx = Context { selection: &[(0, 1)], ..Context::default() };
        let out = round_corners(std::slice::from_ref(&c), 10.0, false, &cx).unwrap().remove(0);
        assert!(out.nodes.iter().all(|n| n.kind != NodeKind::Curve), "{out:?}");
        let points = on_curve(&out);
        assert!(points.contains(&(90.0, 0.0)) && points.contains(&(80.0, 80.0)), "{points:?}");
        let spline = Ring::from_contour(&c).unwrap().to_bez();
        let d = crate::coverage_difference(&[spline], &[out.to_bez().unwrap()]).unwrap();
        // Only the corner goes: a square of 10 less a quarter circle, give or take the bend.
        assert!(d.total < 25.0, "{d:?}");
        // Corners away from the spline leave it as it is, implied point and all.
        let cx = Context { selection: &[(0, 0)], ..Context::default() };
        let out = round_corners(std::slice::from_ref(&c), 10.0, false, &cx).unwrap().remove(0);
        assert!(out.nodes.windows(3).any(|w| w == &c.nodes[2..5]), "{out:?}");
    }

    #[test]
    fn hostile_radii_are_errors() {
        for r in [f64::NAN, f64::INFINITY, -1.0, MAX_COORDINATE * 2.0] {
            assert!(round_corners(&[rect(0.0, 0.0, 10.0, 10.0)], r, false, &Context::default()).is_err());
        }
    }
}
