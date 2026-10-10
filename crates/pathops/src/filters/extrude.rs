//! Extrude (`filter.extrude`, `filter.extrude.offsetIsLength`, `filter.extrude.angleConvention`,
//! `filter.extrude.subtractsOriginalByDefault`, docs/spec/11-filters.md › Extrude).
//!
//! The swept shape is the area the glyph covers while it moves `offset` units in the direction
//! `angle`. It is the original area together with the band (the ribbon) that each piece of the
//! outline sweeps: walk back from a point of the swept shape against the sweep, and either the
//! point is inside the original or the walk meets the outline, whose ribbon holds the point. So one
//! boolean operation gives the result: the ribbons minus the original area (the swept shape minus
//! the original), or the ribbons together with it.
//!
//! A ribbon is a closed path: a run of the outline, a line along the sweep, the same run moved
//! and drawn backwards, and a line back to the start. It encloses its band once only while the run
//! never turns parallel to the sweep, so curves are split where their direction is parallel to it
//! and consecutive pieces that cross the sweep the same way share one ribbon. Each ribbon is drawn
//! counter-clockwise, so the nonzero fill adds them up instead of cancelling them. The original
//! contours are split at the same places, so a ribbon's edge and the outline it came from are the
//! same curve to the bit and the boolean operation leaves no slivers between them.

use std::collections::HashSet;

use fontcraft_geom::{Point, Vec2};

use super::{Args, Context, Filter, Param, ParamKind, Slot, Value};
use crate::boolean::takes_part;
use crate::contour::{Contour, Node, NodeKind, Piece, Ring, round_to_grid};
use crate::{BoolOp, MAX_COORDINATE, MAX_NODES, PathOpError, SMOOTH_MAX_DEGREES, boolean};

/// Largest angle magnitude accepted, in degrees.
const MAX_ANGLE: f64 = 360.0;
/// Split points closer than this, in curve parameter, to a piece's end or to each other are left
/// out: the piece is already parallel to the sweep there.
const ROOT_MARGIN: f64 = 1e-6;
/// Relative size under which a piece counts as parallel to the sweep.
const PARALLEL: f64 = 1e-9;
/// A ribbon or result contour with less area than this, in square units, is dropped.
const MIN_AREA: f64 = 1e-9;

pub struct Extrude;

const PARAMS: &[Param] = &[
    Param {
        key: "offset",
        label: "Offset",
        kind: ParamKind::Number { min: 0.0, max: MAX_COORDINATE },
        default: Value::Number(50.0),
        required: true,
        slot: Slot::None,
        doc: "length of the sweep, in units",
    },
    Param {
        key: "angle",
        label: "Angle",
        kind: ParamKind::Number { min: -MAX_ANGLE, max: MAX_ANGLE },
        default: Value::Number(-45.0),
        required: true,
        slot: Slot::None,
        doc: "direction of the sweep in degrees, counterclockwise, 0 pointing right",
    },
    Param {
        key: "dontSubtract",
        label: "Don't Subtract",
        kind: ParamKind::Bool,
        default: Value::Bool(false),
        required: false,
        slot: Slot::None,
        doc: "keep the original area in the result instead of leaving the shadow only",
    },
];

const DOC: &str = "{glyph: name | glyphs: [names], layer?: id | layers?: [ids] (default: the first master's layer), offset: units (0 to 1000000), angle: degrees (-360 to 360, counterclockwise, 0 pointing right), dontSubtract?: bool (default false)} sweeps the closed paths `offset` units in the direction `angle` and replaces them with the swept shape minus the original, which leaves the shadow only; with dontSubtract the original area stays part of the result. An offset of 0 changes nothing. Open paths, components and locked paths are not changed; new points are rounded to the grid. Extrude has no export string form: filter.apply runs it only with args as an object of parameters → {glyphs: [names], skipped: [{glyph, layer?, reason}]}";

impl Filter for Extrude {
    fn id(&self) -> &'static str {
        "filter.extrude"
    }

    fn name(&self) -> &'static str {
        "Extrude"
    }

    fn label(&self) -> &'static str {
        "Extrude"
    }

    fn doc(&self) -> &'static str {
        DOC
    }

    fn params(&self) -> &'static [Param] {
        PARAMS
    }

    fn has_string_form(&self) -> bool {
        false
    }

    fn apply(&self, contours: &[Contour], args: &Args, cx: &Context) -> Result<Vec<Contour>, PathOpError> {
        extrude(contours, args.number("offset"), args.number("angle"), args.flag("dontSubtract"), cx.grid)
    }
}

/// Extrude on one layer's contours: the closed contours swept `offset` units in the direction
/// `angle` (degrees, counterclockwise from the x axis), minus the original area unless
/// `keep_original`. The sweep vector is rounded to `grid`, and so is every new point. Open
/// contours and single nodes follow the result unchanged. See the module docs for the method.
pub fn extrude(contours: &[Contour], offset: f64, angle: f64, keep_original: bool, grid: f64) -> Result<Vec<Contour>, PathOpError> {
    if !(offset.is_finite() && angle.is_finite()) {
        return Err(PathOpError::NotFinite);
    }
    if !(0.0..=MAX_COORDINATE).contains(&offset) {
        return Err(PathOpError::Invalid(format!("the offset ({offset}) must be from 0 to {MAX_COORDINATE} units")));
    }
    if !(-MAX_ANGLE..=MAX_ANGLE).contains(&angle) {
        return Err(PathOpError::Invalid(format!("the angle ({angle}) must be from -{MAX_ANGLE} to {MAX_ANGLE} degrees")));
    }
    let total: usize = contours.iter().map(|c| c.nodes.len()).sum();
    if total > MAX_NODES {
        return Err(PathOpError::TooManyNodes(total));
    }
    let rings = contours.iter().map(Ring::from_contour).collect::<Result<Vec<_>, _>>()?;
    let (sin, cos) = angle.to_radians().sin_cos();
    let v = round_to_grid(Point::new(offset * cos, offset * sin), grid).to_vec2();
    let length = v.hypot();
    if !(length.is_finite() && length > 0.0) || !contours.iter().any(takes_part) {
        return Ok(contours.to_vec());
    }
    let unit = v / length;
    let mut originals = Vec::with_capacity(contours.len());
    let mut ribbons = Vec::new();
    let mut count = 0usize;
    let mut add = |n: usize| {
        count = count.saturating_add(n);
        if count > MAX_NODES { Err(PathOpError::TooManyNodes(count)) } else { Ok(()) }
    };
    for (contour, ring) in contours.iter().zip(&rings) {
        if !takes_part(contour) {
            originals.push(contour.clone());
            continue;
        }
        let steps = steps(ring, unit);
        let split = Contour::new(true, steps.iter().flat_map(|s| s.offs.iter().copied().chain(std::iter::once(s.on))).collect());
        add(split.nodes.len())?;
        originals.push(split);
        for ribbon in ribbons_of(&steps, v)? {
            add(ribbon.nodes.len())?;
            ribbons.push(ribbon);
        }
    }
    let op = if keep_original { BoolOp::Union } else { BoolOp::Difference };
    let result = boolean(&ribbons, &originals, op, grid)?;
    finish(result, contours, grid)
}

/// One piece of a ring after splitting, or a run of whole quadratic pieces of one spline: the
/// off-curve nodes and the on-curve node it ends at.
#[derive(Clone, Debug)]
struct Step {
    offs: Vec<Node>,
    on: Node,
    /// Which way the step crosses the sweep: 1, -1, or 0 when it runs parallel to it.
    sign: i8,
    /// `on` is an implied point between two pieces of a quadratic spline, which the next piece
    /// of the same spline may absorb.
    open: bool,
}

/// The ring's segments as steps, curves split where they run parallel to `unit`. Original nodes
/// stay as they are (tags, smooth flags); split points are new smooth nodes.
fn steps(ring: &Ring, unit: Vec2) -> Vec<Step> {
    let mut out: Vec<Step> = Vec::with_capacity(ring.len());
    for j in ring.segment_ends() {
        let (Some(seg), Some(r)) = (ring.seg(j), ring.nodes.get(j)) else { continue };
        let pieces = seg.pieces();
        let n = pieces.len();
        for (k, piece) in pieces.iter().enumerate() {
            let roots = parallel_roots(piece, unit);
            let whole = roots.is_empty();
            let quad = matches!(piece, Piece::Quad(_));
            let mut t0 = 0.0;
            for (i, t1) in roots.iter().copied().chain(std::iter::once(1.0)).enumerate() {
                let sub = if whole { *piece } else { piece.subsegment(t0, t1) };
                t0 = t1;
                let last = k + 1 == n && i == roots.len();
                let on = if last { r.node } else { Node { smooth: true, ..Node::new(sub.end(), sub.kind()) } };
                let offs = if whole { original_offs(piece, &r.ctrl, k) } else { new_offs(&sub) };
                let sign = crossing(&sub, unit);
                if let Some(prev) = out.last_mut()
                    && prev.open
                    && whole
                    && quad
                    && prev.sign == sign
                {
                    prev.offs.extend(offs);
                    prev.on = on;
                    prev.open = !last;
                    continue;
                }
                out.push(Step { offs, on, sign, open: whole && quad && !last });
            }
        }
    }
    out
}

/// The original off-curve nodes of whole piece `k` of a segment with off-curve nodes `ctrl`.
fn original_offs(piece: &Piece, ctrl: &[Node], k: usize) -> Vec<Node> {
    match piece {
        Piece::Line(..) => Vec::new(),
        Piece::Quad(_) => ctrl.get(k).copied().into_iter().collect(),
        Piece::Cubic(_) => ctrl.to_vec(),
    }
}

fn new_offs(piece: &Piece) -> Vec<Node> {
    match piece {
        Piece::Line(..) => Vec::new(),
        Piece::Quad(q) => vec![Node::off(q.p1)],
        Piece::Cubic(c) => vec![Node::off(c.p1), Node::off(c.p2)],
    }
}

/// The curve parameters inside the piece where its direction is parallel to `unit`, in order.
fn parallel_roots(piece: &Piece, unit: Vec2) -> Vec<f64> {
    let (a, b, c) = piece.deriv_coeffs();
    let (qa, qb, qc) = (a.cross(unit), b.cross(unit), c.cross(unit));
    let scale = qa.abs().max(qb.abs()).max(qc.abs());
    if !(scale.is_finite() && scale > 0.0) {
        return Vec::new();
    }
    let (qa, qb, qc) = (qa / scale, qb / scale, qc / scale);
    let mut roots = Vec::with_capacity(2);
    if qa.abs() < PARALLEL {
        if qb.abs() >= PARALLEL {
            roots.push(-qc / qb);
        }
    } else {
        let disc = qb * qb - 4.0 * qa * qc;
        if disc >= 0.0 {
            // The form without cancellation between `qb` and the root of the discriminant.
            let q = -0.5 * (qb + disc.sqrt().copysign(qb));
            roots.push(q / qa);
            if q != 0.0 {
                roots.push(qc / q);
            }
        }
    }
    roots.retain(|t| t.is_finite() && *t > ROOT_MARGIN && *t < 1.0 - ROOT_MARGIN);
    roots.sort_by(f64::total_cmp);
    roots.dedup_by(|x, y| (*x - *y).abs() < ROOT_MARGIN);
    roots
}

/// Which way a piece without inner split points crosses the sweep: the sign of the integral of
/// `direction × unit` along it, 0 when the piece is parallel to the sweep.
fn crossing(piece: &Piece, unit: Vec2) -> i8 {
    let (a, b, c) = piece.deriv_coeffs();
    let s = a.cross(unit) / 3.0 + b.cross(unit) / 2.0 + c.cross(unit);
    let size = a.hypot() / 3.0 + b.hypot() / 2.0 + c.hypot();
    if !(s.is_finite() && s.abs() > PARALLEL * size) {
        0
    } else if s > 0.0 {
        1
    } else {
        -1
    }
}

/// One counter-clockwise ribbon for each run of consecutive steps that cross the sweep the same
/// way. Steps parallel to the sweep sweep no area and get none.
fn ribbons_of(steps: &[Step], v: Vec2) -> Result<Vec<Contour>, PathOpError> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < steps.len() {
        let sign = steps.get(i).map_or(0, |s| s.sign);
        let mut end = i + 1;
        while steps.get(end).is_some_and(|s| s.sign == sign) {
            end += 1;
        }
        // The ring is closed: the first step starts where the last one ends.
        let start = if i == 0 { steps.last() } else { steps.get(i - 1) }.map(|s| s.on);
        if sign != 0
            && let (Some(start), Some(run)) = (start, steps.get(i..end))
            && let Some(ribbon) = ribbon(start, run, v)?
        {
            out.push(ribbon);
        }
        i = end;
    }
    Ok(out)
}

/// The band `run` sweeps along `v`, from `start`, drawn counter-clockwise; `None` when it has no
/// area. The nodes at the outline keep their tags and smooth flags; the moved ones have no tag.
fn ribbon(start: Node, run: &[Step], v: Vec2) -> Result<Option<Contour>, PathOpError> {
    let moved = |n: &Node| Node { point: n.point + v, tag: None, ..*n };
    let size: usize = run.iter().map(|s| s.offs.len() + 1).sum();
    let mut nodes = Vec::with_capacity(2 * size + 2);
    // The closing line from the moved start ends here.
    nodes.push(Node { kind: NodeKind::Line, ..start });
    for s in run {
        nodes.extend(s.offs.iter().copied());
        nodes.push(s.on);
    }
    let last = run.last().map_or(start, |s| s.on);
    nodes.push(Node { kind: NodeKind::Line, ..moved(&last) });
    for (i, s) in run.iter().enumerate().rev() {
        nodes.extend(s.offs.iter().rev().map(moved));
        let before = if i == 0 { Some(start) } else { run.get(i - 1).map(|p| p.on) }.unwrap_or(start);
        nodes.push(Node { kind: s.on.kind, ..moved(&before) });
    }
    let contour = Contour::new(true, nodes);
    let area = Ring::from_contour(&contour)?.area();
    if !(area.is_finite() && area.abs() >= MIN_AREA) {
        return Ok(None);
    }
    if area < 0.0 { contour.reversed().map(Some) } else { Ok(Some(contour)) }
}

/// The boolean result tidied: every point that is not an original point goes on the grid, nodes
/// that end segments of no length go, and a node is smooth when it looks smooth and, for an
/// original node, was smooth before. Open paths and single nodes pass unchanged.
fn finish(result: Vec<Contour>, originals: &[Contour], grid: f64) -> Result<Vec<Contour>, PathOpError> {
    let bits = |p: Point| ((p.x + 0.0).to_bits(), (p.y + 0.0).to_bits());
    let known: HashSet<(u64, u64)> = originals.iter().flat_map(|c| &c.nodes).map(|n| bits(n.point)).collect();
    let place = |p: Point| if known.contains(&bits(p)) { p } else { round_to_grid(p, grid) };
    let mut out = Vec::with_capacity(result.len());
    let mut count = 0usize;
    for c in result {
        let contour = if takes_part(&c) {
            let mut ring = Ring::from_contour(&c)?;
            for r in &mut ring.nodes {
                r.node.point = place(r.node.point);
                for o in &mut r.ctrl {
                    o.point = place(o.point);
                }
            }
            ring.remove_zero_length(0.0, |_, _| true);
            for j in 0..ring.len() {
                let looks = ring.looks_smooth(j, SMOOTH_MAX_DEGREES).unwrap_or(false);
                if let Some(r) = ring.nodes.get_mut(j) {
                    r.node.smooth = looks && (r.node.tag.is_none() || r.node.smooth);
                }
            }
            if ring.area().abs() < MIN_AREA {
                continue;
            }
            ring.to_contour()
        } else {
            c
        };
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    use crate::fixtures::rect;

    /// The summed signed area of closed contours (counters count negative) and their bounds.
    fn measure(cs: &[Contour]) -> (f64, (f64, f64, f64, f64)) {
        let rings: Vec<Ring> = cs.iter().map(|c| Ring::from_contour(c).unwrap()).collect();
        let area = rings.iter().map(Ring::area).sum();
        let b = rings.iter().filter_map(Ring::bounds).reduce(|a, b| a.union(b)).unwrap();
        (area, (b.x0, b.y0, b.x1, b.y1))
    }

    /// A circle of four cubics around the origin, counter-clockwise from (r, 0).
    fn circle(r: f64) -> Contour {
        let k = 0.5523 * r;
        let on = |x: f64, y: f64| Node { smooth: true, ..Node::new(Point::new(x, y), NodeKind::Curve) };
        let off = |x: f64, y: f64| Node::off(Point::new(x, y));
        Contour::new(
            true,
            vec![
                on(r, 0.0),
                off(r, k),
                off(k, r),
                on(0.0, r),
                off(-k, r),
                off(-r, k),
                on(-r, 0.0),
                off(-r, -k),
                off(-k, -r),
                on(0.0, -r),
                off(k, -r),
                off(r, -k),
            ],
        )
    }

    #[test]
    fn a_square_leaves_its_shadow_or_the_whole_swept_shape() {
        let square = [rect(0.0, 0.0, 100.0, 100.0)];
        let shadow = extrude(&square, 50.0, 0.0, false, 0.0).unwrap();
        let (area, bounds) = measure(&shadow);
        assert!((area - 5_000.0).abs() < 1e-6, "{area}");
        assert_eq!(bounds, (100.0, 0.0, 150.0, 100.0));
        let whole = extrude(&square, 50.0, 0.0, true, 0.0).unwrap();
        let (area, bounds) = measure(&whole);
        assert!((area - 15_000.0).abs() < 1e-6, "{area}");
        assert_eq!(bounds, (0.0, 0.0, 150.0, 100.0));
    }

    #[test]
    fn a_counter_shrinks_by_the_sweep() {
        // A 300 square with a 100 counter, swept 50 to the right: the counter keeps its right half.
        let shapes = [rect(0.0, 0.0, 300.0, 300.0), rect(100.0, 100.0, 200.0, 200.0).reversed().unwrap()];
        let (whole, _) = measure(&extrude(&shapes, 50.0, 0.0, true, 0.0).unwrap());
        assert!((whole - (350.0 * 300.0 - 50.0 * 100.0)).abs() < 1e-6, "{whole}");
        let (shadow, _) = measure(&extrude(&shapes, 50.0, 0.0, false, 0.0).unwrap());
        assert!((shadow - 20_000.0).abs() < 1e-6, "{shadow}");
    }

    #[test]
    fn curves_parallel_to_the_sweep_on_the_way_are_split() {
        // At 45 degrees two of the four arcs turn parallel to the sweep in their middle. The swept
        // circle is a stadium: the circle plus a 200 by 100 band.
        let circle = [circle(100.0)];
        let (round, _) = measure(&circle);
        let (shadow, _) = measure(&extrude(&circle, 100.0, 45.0, false, 0.0).unwrap());
        assert!((shadow - 20_000.0).abs() < 100.0, "{shadow}");
        let (whole, _) = measure(&extrude(&circle, 100.0, 45.0, true, 0.0).unwrap());
        assert!((whole - (round + 20_000.0)).abs() < 100.0, "{whole}");
        // The same with the circle drawn the other way round.
        let reversed = [circle[0].reversed().unwrap()];
        let (again, _) = measure(&extrude(&reversed, 100.0, 45.0, true, 0.0).unwrap());
        assert!((again - whole).abs() < 1.0, "{again} {whole}");
    }

    #[test]
    fn new_points_go_on_the_grid_and_original_nodes_keep_their_tags() {
        let square = [rect(0.0, 0.0, 100.0, 100.0).tagged()];
        let out = extrude(&square, 50.0, 30.0, true, 1.0).unwrap();
        assert!(out.iter().flat_map(|c| &c.nodes).all(|n| n.point.x.fract() == 0.0 && n.point.y.fract() == 0.0));
        let origin = out.iter().flat_map(|c| &c.nodes).find(|n| n.point == Point::ZERO).unwrap();
        assert_eq!(origin.tag, Some(0));
        // The sweep is rounded too: (43.3, 25) becomes (43, 25).
        assert_eq!(measure(&out).1, (0.0, 0.0, 143.0, 125.0));
        // The circle's own off-curve points (at 55.23) stay where they are.
        let circle = [circle(100.0)];
        let out = extrude(&circle, 80.0, -45.0, false, 1.0).unwrap();
        assert!(!out.is_empty());
        let original: Vec<Point> = circle[0].nodes.iter().map(|n| n.point).collect();
        let new = out.iter().flat_map(|c| &c.nodes).filter(|n| !original.contains(&n.point));
        assert!(new.clone().count() > 0);
        assert!(new.into_iter().all(|n| n.point.x.fract() == 0.0 && n.point.y.fract() == 0.0));
    }

    #[test]
    fn nothing_to_sweep_changes_nothing() {
        let square = [rect(0.0, 0.0, 100.0, 100.0)];
        assert_eq!(extrude(&square, 0.0, 30.0, false, 1.0).unwrap(), square);
        // Shorter than half a grid step.
        assert_eq!(extrude(&square, 0.4, 0.0, false, 1.0).unwrap(), square);
        let open = Contour { closed: false, ..rect(0.0, 0.0, 100.0, 100.0) };
        assert_eq!(extrude(std::slice::from_ref(&open), 50.0, 0.0, false, 1.0).unwrap(), vec![open.clone()]);
        // An open path next to a closed one follows the result unchanged.
        let out = extrude(&[rect(0.0, 0.0, 100.0, 100.0), open.clone()], 50.0, 0.0, false, 1.0).unwrap();
        assert_eq!(out.last(), Some(&open));
    }

    #[test]
    fn hostile_parameters_are_errors() {
        let square = [rect(0.0, 0.0, 100.0, 100.0)];
        for (offset, angle) in [(f64::NAN, 0.0), (f64::INFINITY, 0.0), (-1.0, 0.0), (MAX_COORDINATE * 2.0, 0.0), (50.0, f64::NAN), (50.0, 361.0)] {
            assert!(extrude(&square, offset, angle, false, 1.0).is_err(), "{offset} {angle}");
        }
        // The swept shape would reach beyond the coordinate limit.
        assert!(extrude(&square, MAX_COORDINATE, 0.0, true, 1.0).is_err());
    }
}
