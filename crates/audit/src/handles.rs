//! Checks of handles and curvature: missing extremes, inflections, slanted, retracted, crossed
//! and unbalanced handles, kinks at smooth nodes, smooth-looking corner nodes and curvature
//! breaks.

use fontcraft_geom::{Point, Vec2};

use crate::contour::{Bez, Seg, angle_between, proper_cross, sign_changing_roots};
use crate::{Hit, LayerInput, Nth, Params, Sink, Threshold, num};

/// Parameters this close to a segment's ends count as the ends.
const T_EPSILON: f64 = 1e-3;
/// An inflection closer than this many units to a node is ignored.
pub(crate) const INFLECTION_END_DISTANCE: f64 = 1.0;
/// The `maxOffset` rule of `almostOrthogonalHandle` applies only to handles within this many
/// degrees of an axis, so short diagonal handles are not reported.
pub(crate) const ORTHOGONAL_OFFSET_RULE_MAX_ANGLE: f64 = 10.0;
/// Two directions whose cross product is below this fraction of their lengths' product are
/// parallel.
const PARALLEL_EPSILON: f64 = 1e-9;
/// Curvatures below this (1/units) count as zero.
const CURVATURE_EPSILON: f64 = 1e-9;

pub(crate) const EXTREMES_MIN_DISTANCE: Threshold = Threshold {
    name: "minDistance",
    default: 1.0,
    min: 0.0,
    max: 100.0,
    doc: "Extremes closer than this many units to a node of the segment are ignored.",
};

pub(crate) const HANDLE_MAX_ANGLE: Threshold = Threshold {
    name: "maxAngle",
    default: 2.0,
    min: 0.0,
    max: 45.0,
    doc: "Handles within this many degrees of horizontal or vertical (and not exactly so) are reported.",
};

pub(crate) const HANDLE_MAX_OFFSET: Threshold = Threshold {
    name: "maxOffset",
    default: 2.0,
    min: 0.0,
    max: 100.0,
    doc: "Handles within this many units of horizontal or vertical (and not exactly so) are reported.",
};

pub(crate) const KINK_MAX_ANGLE: Threshold = Threshold {
    name: "maxAngle",
    default: 1.0,
    min: 0.0,
    max: 180.0,
    doc: "A smooth node whose directions differ by more than this many degrees is kinked.",
};

pub(crate) const UNBALANCED_MAX_RATIO: Threshold = Threshold {
    name: "maxRatio",
    default: 4.0,
    min: 1.0,
    max: 1000.0,
    doc: "Largest ratio between the two handles' lengths relative to the distance to their intersection.",
};

pub(crate) const CURVATURE_MAX_RATIO: Threshold = Threshold {
    name: "maxRatio",
    default: 2.0,
    min: 1.0,
    max: 1000.0,
    doc: "Largest ratio between the curvatures on the two sides of a smooth node.",
};

/// The coefficients `p + q t + r t²` of a cubic's derivative divided by three, as vectors.
fn derivative(a: Point, b: Point, c: Point, d: Point) -> (Vec2, Vec2, Vec2) {
    let (u, v, w) = (b - a, c - b, d - c);
    (u, (v - u) * 2.0, w - v * 2.0 + u)
}

/// A curve segment that reaches an extreme in x or y between its nodes.
pub(crate) fn missing_extremes(input: &LayerInput<'_>, p: &Params<'_>, sink: &mut Sink) {
    let min = p.get(&EXTREMES_MIN_DISTANCE);
    for c in input.finite() {
        for seg in c.segs.iter().filter(|s| s.is_curve()) {
            for bez in seg.beziers() {
                for (axis, t) in extremes(&bez) {
                    let x = bez.eval(t);
                    if x.distance(seg.p0) <= min || x.distance(seg.p1) <= min {
                        continue;
                    }
                    sink.push(
                        Hit::new(
                            "missing-extreme",
                            format!(
                                "The segment ending at node {} of path {} has an extreme in {axis} at ({}, {}) between its nodes.",
                                Nth(seg.end),
                                Nth(c.shape),
                                num(x.x),
                                num(x.y)
                            ),
                        )
                        .segments(vec![[c.shape, seg.end]])
                        .at(x)
                        .data("t", t),
                    );
                }
            }
        }
    }
}

/// Parameters of the extremes inside a piece, with the axis they are extremes of.
fn extremes(bez: &Bez) -> Vec<(&'static str, f64)> {
    let mut out = Vec::new();
    match *bez {
        Bez::Line(..) => {}
        Bez::Quad(a, b, c) => {
            let u = b - a;
            let v = (c - b) - u;
            for (axis, du, dv) in [("x", u.x, v.x), ("y", u.y, v.y)] {
                for t in sign_changing_roots(0.0, dv, du, T_EPSILON, 1.0 - T_EPSILON) {
                    out.push((axis, t));
                }
            }
        }
        Bez::Cubic(a, b, c, d) => {
            let (dp, dq, dr) = derivative(a, b, c, d);
            for (axis, p, q, r) in [("x", dp.x, dq.x, dr.x), ("y", dp.y, dq.y, dr.y)] {
                for t in sign_changing_roots(r, q, p, T_EPSILON, 1.0 - T_EPSILON) {
                    out.push((axis, t));
                }
            }
        }
    }
    out
}

/// A cubic segment that changes its bending direction between its nodes.
pub(crate) fn inflections(input: &LayerInput<'_>, _: &Params<'_>, sink: &mut Sink) {
    for c in input.finite() {
        for seg in &c.segs {
            let Some((a, b, cc, d)) = seg.cubic() else { continue };
            let (p, q, r) = derivative(a, b, cc, d);
            // The cross product of the first and second derivatives vanishes at an inflection.
            let roots = sign_changing_roots(q.cross(r), 2.0 * p.cross(r), p.cross(q), T_EPSILON, 1.0 - T_EPSILON);
            let bez = Bez::Cubic(a, b, cc, d);
            for t in roots {
                let x = bez.eval(t);
                if x.distance(a) <= INFLECTION_END_DISTANCE || x.distance(d) <= INFLECTION_END_DISTANCE {
                    continue;
                }
                sink.push(
                    Hit::new("inflection", format!("The curve ending at node {} of path {} has an inflection.", Nth(seg.end), Nth(c.shape)))
                        .segments(vec![[c.shape, seg.end]])
                        .at(x)
                        .data("t", t),
                );
            }
        }
    }
}

/// The handles of a cubic segment with the node each belongs to: (handle index, handle, node).
fn handles(seg: &Seg) -> Vec<(usize, Point, Point)> {
    match (seg.cubic(), seg.ctrl.as_slice()) {
        (Some((p0, c1, c2, p1)), [(i1, _), (i2, _)]) => vec![(*i1, c1, p0), (*i2, c2, p1)],
        _ => Vec::new(),
    }
}

/// A handle within `maxAngle` degrees or `maxOffset` units of horizontal or vertical.
pub(crate) fn almost_orthogonal_handle(input: &LayerInput<'_>, p: &Params<'_>, sink: &mut Sink) {
    let max_angle = p.get(&HANDLE_MAX_ANGLE);
    let max_offset = p.get(&HANDLE_MAX_OFFSET);
    for c in input.finite() {
        for seg in &c.segs {
            for (i, h, node) in handles(seg) {
                let v = h - node;
                if v.x == 0.0 || v.y == 0.0 {
                    continue;
                }
                let angle = v.y.abs().atan2(v.x.abs()).to_degrees();
                let off_angle = angle.min(90.0 - angle);
                let offset = v.x.abs().min(v.y.abs());
                if off_angle < max_angle || (offset < max_offset && off_angle < ORTHOGONAL_OFFSET_RULE_MAX_ANGLE) {
                    let axis = if angle < 45.0 { "horizontal" } else { "vertical" };
                    sink.push(
                        Hit::new("almost-orthogonal", format!("Handle {i} of path {} is {}° off {axis}.", Nth(c.shape), num(off_angle), i = Nth(i)))
                            .points(vec![[c.shape, i]])
                            .at(h)
                            .data("angle", off_angle)
                            .data("offset", offset),
                    );
                }
            }
        }
    }
}

/// A handle at the same place as its node. With both handles of a segment retracted the
/// segment is a line in disguise, which Tidy up fixes; a single one is reported only.
pub(crate) fn zero_length_handle(input: &LayerInput<'_>, _: &Params<'_>, sink: &mut Sink) {
    for c in input.finite() {
        for seg in &c.segs {
            let hs = handles(seg);
            let retracted: Vec<(usize, Point)> = hs.iter().filter(|(_, h, n)| h == n).map(|(i, h, _)| (*i, *h)).collect();
            match retracted.as_slice() {
                [] => {}
                [(i, h)] => sink.push(
                    Hit::new("retracted", format!("Handle {i} of path {} sits on its node.", Nth(c.shape), i = Nth(*i)))
                        .points(vec![[c.shape, *i]])
                        .at(*h)
                        .without_fix(),
                ),
                _ => sink.push(
                    Hit::new(
                        "both-retracted",
                        format!("Both handles of the curve ending at node {} of path {} sit on their nodes.", Nth(seg.end), Nth(c.shape)),
                    )
                    .points(retracted.iter().map(|(i, _)| [c.shape, *i]).collect())
                    .segments(vec![[c.shape, seg.end]])
                    .at(seg.p0.midpoint(seg.p1)),
                ),
            }
        }
    }
}

/// A smooth node whose incoming and outgoing directions differ by more than `maxAngle`.
pub(crate) fn kinked_smooth_node(input: &LayerInput<'_>, p: &Params<'_>, sink: &mut Sink) {
    let max = p.get(&KINK_MAX_ANGLE);
    for c in input.finite() {
        for (node, n) in c.path.nodes.iter().enumerate() {
            if !n.smooth || !n.on_curve() {
                continue;
            }
            let Some((a, b)) = c.joint(node) else { continue };
            let (Some(arrive), Some(leave)) = (a.end_dir(), b.start_dir()) else { continue };
            let angle = angle_between(arrive, leave).to_degrees();
            if angle > max {
                sink.push(
                    Hit::new("kinked", format!("Smooth node {node} of path {} has a kink of {}°.", Nth(c.shape), num(angle), node = Nth(node)))
                        .points(vec![[c.shape, node]])
                        .at(n.point())
                        .data("angle", angle),
                );
            }
        }
    }
}

/// A corner node between two curves whose handles are exactly on one line.
pub(crate) fn unflagged_smooth_node(input: &LayerInput<'_>, _: &Params<'_>, sink: &mut Sink) {
    for c in input.finite() {
        for (node, n) in c.path.nodes.iter().enumerate() {
            if n.smooth || !n.on_curve() {
                continue;
            }
            let Some((a, b)) = c.joint(node) else { continue };
            if !(a.is_curve() && b.is_curve()) {
                continue;
            }
            let here = n.point();
            let (Some((_, h_in)), Some((_, h_out))) = (a.ctrl.last(), b.ctrl.first()) else { continue };
            if *h_in == here || *h_out == here {
                continue;
            }
            let (u, v) = (here - *h_in, *h_out - here);
            if u.cross(v).abs() <= PARALLEL_EPSILON * u.hypot() * v.hypot() && u.dot(v) > 0.0 {
                sink.push(
                    Hit::new("collinear-handles", format!("Corner node {node} of path {} has handles on one line.", Nth(c.shape), node = Nth(node)))
                        .points(vec![[c.shape, node]])
                        .at(here),
                );
            }
        }
    }
}

/// How long each handle of a cubic is relative to the distance from its node to where the two
/// handle lines meet (or to the chord, for parallel handles). `None` when a handle is retracted
/// or the handles point away from each other.
fn handle_fractions(p0: Point, c1: Point, c2: Point, p1: Point) -> Option<(f64, f64)> {
    let (h1, h2) = (c1 - p0, c2 - p1);
    let (l1, l2) = (h1.hypot(), h2.hypot());
    if l1 == 0.0 || l2 == 0.0 {
        return None;
    }
    let cross = h1.cross(h2);
    if cross.abs() <= PARALLEL_EPSILON * l1 * l2 {
        let chord = p0.distance(p1);
        return (chord > 0.0).then_some((l1 / chord, l2 / chord));
    }
    let d = p1 - p0;
    let s = d.cross(h2) / cross;
    let u = d.cross(h1) / cross;
    (s > 0.0 && u > 0.0).then_some((1.0 / s, 1.0 / u))
}

/// A cubic whose handles differ a lot in relative length, or with a handle reaching past the
/// handles' intersection.
pub(crate) fn unbalanced_handles(input: &LayerInput<'_>, p: &Params<'_>, sink: &mut Sink) {
    let max = p.get(&UNBALANCED_MAX_RATIO);
    for c in input.finite() {
        for seg in &c.segs {
            let Some((p0, c1, c2, p1)) = seg.cubic() else { continue };
            let Some((f1, f2)) = handle_fractions(p0, c1, c2, p1) else { continue };
            let ratio = f1.max(f2) / f1.min(f2);
            let (code, message) = if f1 > 1.0 || f2 > 1.0 {
                (
                    "beyond-intersection",
                    format!("A handle of the curve ending at node {} of path {} reaches past where the handles meet.", Nth(seg.end), Nth(c.shape)),
                )
            } else if ratio > max {
                (
                    "unbalanced",
                    format!(
                        "The handles of the curve ending at node {} of path {} differ {}-fold in length.",
                        Nth(seg.end),
                        Nth(c.shape),
                        num(ratio)
                    ),
                )
            } else {
                continue;
            };
            let pts = seg.ctrl.iter().map(|(i, _)| [c.shape, *i]).collect();
            sink.push(
                Hit::new(code, message)
                    .points(pts)
                    .segments(vec![[c.shape, seg.end]])
                    .at(p0.midpoint(p1))
                    .data("ratio", ratio)
                    .data("fraction1", f1)
                    .data("fraction2", f2),
            );
        }
    }
}

/// The two handles of a cubic cross each other.
pub(crate) fn crossed_handles(input: &LayerInput<'_>, _: &Params<'_>, sink: &mut Sink) {
    for c in input.finite() {
        for seg in &c.segs {
            let Some((p0, c1, c2, p1)) = seg.cubic() else { continue };
            if let Some(x) = proper_cross(p0, c1, p1, c2) {
                let pts = seg.ctrl.iter().map(|(i, _)| [c.shape, *i]).collect();
                sink.push(
                    Hit::new("crossed", format!("The handles of the curve ending at node {} of path {} cross.", Nth(seg.end), Nth(c.shape)))
                        .points(pts)
                        .segments(vec![[c.shape, seg.end]])
                        .at(x),
                );
            }
        }
    }
}

/// Curvature of a cubic at its start: ⅔ |(b − a) × (c − b)| / |b − a|³.
fn start_curvature(a: Point, b: Point, c: Point) -> Option<f64> {
    let u = b - a;
    let len = u.hypot();
    (len > 0.0).then_some(2.0 / 3.0 * u.cross(c - b).abs() / (len * len * len))
}

/// At a smooth node between two cubics, the curvatures on the two sides differ more than
/// `maxRatio`-fold.
pub(crate) fn curvature_break(input: &LayerInput<'_>, p: &Params<'_>, sink: &mut Sink) {
    let max = p.get(&CURVATURE_MAX_RATIO);
    for c in input.finite() {
        for (node, n) in c.path.nodes.iter().enumerate() {
            if !n.smooth || !n.on_curve() {
                continue;
            }
            let Some((a, b)) = c.joint(node) else { continue };
            let (Some((_, a1, a2, a3)), Some((b0, b1, b2, _))) = (a.cubic(), b.cubic()) else { continue };
            // The end of the incoming curve is the start of the same curve run backwards.
            let (Some(k_in), Some(k_out)) = (start_curvature(a3, a2, a1), start_curvature(b0, b1, b2)) else { continue };
            let (lo, hi) = (k_in.min(k_out), k_in.max(k_out));
            if lo <= CURVATURE_EPSILON {
                continue;
            }
            let ratio = hi / lo;
            if ratio > max {
                sink.push(
                    Hit::new(
                        "curvature-break",
                        format!("At smooth node {node} of path {} the curvature changes {}-fold.", Nth(c.shape), num(ratio), node = Nth(node)),
                    )
                    .points(vec![[c.shape, node]])
                    .at(n.point())
                    .data("ratio", ratio)
                    .data("curvatureIn", k_in)
                    .data("curvatureOut", k_out),
                );
            }
        }
    }
}
