//! Checks of points and segments: coincident and near-coincident points, flat and almost
//! straight curves, collinear lines, short and slanted segments, spikes, edges drawn twice and
//! node lists that do not make segments.

use std::collections::HashMap;

use fontcraft_geom::Point;
use fontcraft_model::NodeKind;

use crate::contour::{SegKind, angle_between, dist_to_line, dist_to_segment};
use crate::{Hit, LayerInput, Nth, Params, Sink, Threshold, num};

pub(crate) const NEAR_COINCIDENT_MIN_DISTANCE: Threshold = Threshold {
    name: "minDistance",
    default: 1.0,
    min: 0.0,
    max: 100.0,
    doc: "Two on-curve points closer than this many units (and not identical) are reported.",
};

pub(crate) const FLAT_CURVE_TOLERANCE: Threshold = Threshold {
    name: "tolerance",
    default: 0.0,
    min: 0.0,
    max: 100.0,
    doc: "How far, in units, a handle may lie from the line between the segment's ends.",
};

pub(crate) const ALMOST_STRAIGHT_MAX_DEVIATION: Threshold = Threshold {
    name: "maxDeviation",
    default: 2.0,
    min: 0.0,
    max: 100.0,
    doc: "A curve that strays less than this many units from its chord is reported.",
};

pub(crate) const COLLINEAR_TOLERANCE: Threshold = Threshold {
    name: "tolerance",
    default: 0.0,
    min: 0.0,
    max: 100.0,
    doc: "How far, in units, the middle node may lie from the line through its neighbours.",
};

pub(crate) const COLLINEAR_MAX_ANGLE: Threshold = Threshold {
    name: "maxAngle",
    default: 0.1,
    min: 0.0,
    max: 1.0,
    doc: "Looser variant: two lines whose directions differ by less than this many radians. Not run on sources with more than one master.",
};

pub(crate) const SHORT_MIN_LENGTH: Threshold =
    Threshold { name: "minLength", default: 3.0, min: 0.0, max: 1000.0, doc: "Segments shorter than this many units are reported." };

pub(crate) const SHORT_MIN_FRACTION: Threshold = Threshold {
    name: "minFraction",
    default: 0.006,
    min: 0.0,
    max: 1.0,
    doc: "Segments shorter than this fraction of their contour's length are reported.",
};

pub(crate) const ORTHOGONAL_LINE_MAX_ANGLE: Threshold = Threshold {
    name: "maxAngle",
    default: 0.5,
    min: 0.0,
    max: 45.0,
    doc: "Lines within this many degrees of horizontal or vertical (and not exactly so) are reported.",
};

pub(crate) const JAGGY_MAX_ANGLE: Threshold = Threshold {
    name: "maxAngle",
    default: 0.25,
    min: 0.0,
    max: std::f64::consts::PI,
    doc: "Segments that meet at an angle sharper than this many radians are reported.",
};

/// Two consecutive on-curve points at the same place: a segment of length zero.
pub(crate) fn coincident_points(input: &LayerInput<'_>, _: &Params<'_>, sink: &mut Sink) {
    for c in input.finite() {
        if c.on_curve_count() < 2 {
            continue;
        }
        for seg in c.segs.iter().filter(|s| s.is_valid()) {
            if seg.points().iter().all(|p| *p == seg.p0) {
                sink.push(
                    Hit::new("coincident", format!("Nodes {} and {} of path {} are at the same place.", Nth(seg.start), Nth(seg.end), Nth(c.shape)))
                        .points(vec![[c.shape, seg.start], [c.shape, seg.end]])
                        .segments(vec![[c.shape, seg.end]])
                        .at(seg.p1),
                );
            }
        }
    }
}

/// Two consecutive on-curve points closer than `minDistance` without being identical.
pub(crate) fn near_coincident_points(input: &LayerInput<'_>, p: &Params<'_>, sink: &mut Sink) {
    let min = p.get(&NEAR_COINCIDENT_MIN_DISTANCE);
    for c in input.finite() {
        if c.on_curve_count() < 2 {
            continue;
        }
        for seg in c.segs.iter().filter(|s| s.is_valid()) {
            let len = seg.control_length();
            if len > 0.0 && len < min {
                let d = seg.p0.distance(seg.p1);
                sink.push(
                    Hit::new(
                        "near-coincident",
                        format!("Nodes {} and {} of path {} are {} units apart.", Nth(seg.start), Nth(seg.end), Nth(c.shape), num(d)),
                    )
                    .points(vec![[c.shape, seg.start], [c.shape, seg.end]])
                    .segments(vec![[c.shape, seg.end]])
                    .at(seg.p1)
                    .data("distance", d),
                );
            }
        }
    }
}

/// A cubic segment whose handles lie on the line between its ends.
pub(crate) fn flat_curve(input: &LayerInput<'_>, p: &Params<'_>, sink: &mut Sink) {
    let tol = p.get(&FLAT_CURVE_TOLERANCE);
    for c in input.finite() {
        for seg in &c.segs {
            let Some((p0, c1, c2, p1)) = seg.cubic() else { continue };
            if p0 == p1 {
                continue;
            }
            if dist_to_segment(c1, p0, p1) <= tol && dist_to_segment(c2, p0, p1) <= tol {
                let handles: Vec<[usize; 2]> = seg.ctrl.iter().map(|(i, _)| [c.shape, *i]).collect();
                sink.push(
                    Hit::new("flat-curve", format!("The curve ending at node {} of path {} is straight.", Nth(seg.end), Nth(c.shape)))
                        .points(handles)
                        .segments(vec![[c.shape, seg.end]])
                        .at(p0.midpoint(p1)),
                );
            }
        }
    }
}

/// A curve that strays less than `maxDeviation` from its chord without being flat.
pub(crate) fn almost_straight_curve(input: &LayerInput<'_>, p: &Params<'_>, sink: &mut Sink) {
    let max = p.get(&ALMOST_STRAIGHT_MAX_DEVIATION);
    for c in input.finite() {
        for seg in c.segs.iter().filter(|s| s.is_curve()) {
            let (p0, p1) = (seg.p0, seg.p1);
            if p0 == p1 {
                continue;
            }
            let handle_dev = seg.ctrl.iter().map(|(_, h)| dist_to_segment(*h, p0, p1)).fold(0.0, f64::max);
            if handle_dev == 0.0 {
                continue;
            }
            let dev = seg.polyline().iter().map(|q| dist_to_segment(*q, p0, p1)).fold(0.0, f64::max);
            if dev < max {
                sink.push(
                    Hit::new(
                        "almost-straight",
                        format!(
                            "The curve ending at node {} of path {} strays only {} units from a straight line.",
                            Nth(seg.end),
                            Nth(c.shape),
                            num(dev)
                        ),
                    )
                    .segments(vec![[c.shape, seg.end]])
                    .at(p0.midpoint(p1))
                    .data("deviation", dev),
                );
            }
        }
    }
}

/// Two consecutive lines on one straight line: the node between them is redundant.
pub(crate) fn collinear_lines(input: &LayerInput<'_>, p: &Params<'_>, sink: &mut Sink) {
    let tol = p.get(&COLLINEAR_TOLERANCE);
    let max_angle = p.get(&COLLINEAR_MAX_ANGLE);
    let loose = !input.ctx.variable;
    for c in input.finite() {
        for node in 0..c.path.nodes.len() {
            let Some((a, b)) = c.joint(node) else { continue };
            if a.kind != SegKind::Line || b.kind != SegKind::Line {
                continue;
            }
            let (prev, mid, next) = (a.p0, a.p1, b.p1);
            if prev == mid || mid == next || prev == next {
                continue;
            }
            let span = next - prev;
            let t = (mid - prev).dot(span) / span.hypot2();
            let between = t > 0.0 && t < 1.0;
            let hit = if between && dist_to_line(mid, prev, next) <= tol {
                Some(("collinear", "lies"))
            } else if loose && angle_between(mid - prev, next - mid) < max_angle {
                Some(("nearly-collinear", "almost lies"))
            } else {
                None
            };
            if let Some((code, verb)) = hit {
                sink.push(
                    Hit::new(code, format!("Node {node} of path {} {verb} on the line between its neighbours.", Nth(c.shape), node = Nth(node)))
                        .points(vec![[c.shape, node]])
                        .at(mid)
                        .data("angle", angle_between(mid - prev, next - mid)),
                );
            }
        }
    }
}

/// A segment shorter than `minLength` or than `minFraction` of its contour; a line after a line
/// does not count.
pub(crate) fn short_segments(input: &LayerInput<'_>, p: &Params<'_>, sink: &mut Sink) {
    let min_len = p.get(&SHORT_MIN_LENGTH);
    let min_frac = p.get(&SHORT_MIN_FRACTION);
    for c in input.finite() {
        let lengths: Vec<f64> = c.segs.iter().map(|s| if s.is_valid() { s.length() } else { 0.0 }).collect();
        let total: f64 = lengths.iter().sum();
        for (seg, len) in c.segs.iter().zip(lengths) {
            if !seg.is_valid() || len <= 0.0 {
                continue;
            }
            if seg.kind == SegKind::Line && c.incoming(seg.start).is_some_and(|prev| prev.kind == SegKind::Line) {
                continue;
            }
            if len < min_len || len < min_frac * total {
                sink.push(
                    Hit::new(
                        "short-segment",
                        format!("The segment ending at node {} of path {} is {} units long.", Nth(seg.end), Nth(c.shape), num(len)),
                    )
                    .segments(vec![[c.shape, seg.end]])
                    .at(seg.p1)
                    .data("length", len),
                );
            }
        }
    }
}

/// A line within `maxAngle` degrees of horizontal or vertical. Skipped on italic masters.
pub(crate) fn almost_orthogonal_line(input: &LayerInput<'_>, p: &Params<'_>, sink: &mut Sink) {
    if crate::metrics::italic_angle(input) != 0.0 {
        sink.skip("the master is italic");
        return;
    }
    let max = p.get(&ORTHOGONAL_LINE_MAX_ANGLE);
    for c in input.finite() {
        for seg in c.segs.iter().filter(|s| s.kind == SegKind::Line) {
            let v = seg.p1 - seg.p0;
            if v.x == 0.0 || v.y == 0.0 {
                continue;
            }
            let angle = v.y.abs().atan2(v.x.abs()).to_degrees();
            let off = angle.min(90.0 - angle);
            if off < max {
                let axis = if angle < 45.0 { "horizontal" } else { "vertical" };
                sink.push(
                    Hit::new(
                        "almost-orthogonal",
                        format!("The line ending at node {} of path {} is {}° off {axis}.", Nth(seg.end), Nth(c.shape), num(off)),
                    )
                    .points(vec![[c.shape, seg.start], [c.shape, seg.end]])
                    .segments(vec![[c.shape, seg.end]])
                    .at(seg.p0.midpoint(seg.p1))
                    .data("angle", off),
                );
            }
        }
    }
}

/// Two segments that meet at a very sharp angle: a spike or a folded-back node.
pub(crate) fn jaggy_segments(input: &LayerInput<'_>, p: &Params<'_>, sink: &mut Sink) {
    let max = p.get(&JAGGY_MAX_ANGLE);
    for c in input.finite() {
        for node in 0..c.path.nodes.len() {
            let Some((a, b)) = c.joint(node) else { continue };
            let (Some(arrive), Some(leave)) = (a.end_dir(), b.start_dir()) else { continue };
            let angle = angle_between(-arrive, leave);
            if angle > 0.0 && angle < max {
                sink.push(
                    Hit::new(
                        "jaggy",
                        format!(
                            "Node {node} of path {} is a spike: its segments meet at {}°.",
                            Nth(c.shape),
                            num(angle.to_degrees()),
                            node = Nth(node)
                        ),
                    )
                    .points(vec![[c.shape, node]])
                    .at(a.p1)
                    .data("angle", angle),
                );
            }
        }
    }
}

/// The points of a segment as exact bit patterns, with -0 read as 0.
type SegKey = Vec<(u64, u64)>;

fn key(points: &[Point]) -> SegKey {
    points.iter().map(|p| ((p.x + 0.0).to_bits(), (p.y + 0.0).to_bits())).collect()
}

/// Two segments of the layer with the same end points and the same handles, in either
/// direction: one edge drawn twice.
pub(crate) fn overlapping_segments(input: &LayerInput<'_>, _: &Params<'_>, sink: &mut Sink) {
    let mut seen: HashMap<SegKey, [usize; 2]> = HashMap::new();
    for c in input.finite() {
        for seg in c.segs.iter().filter(|s| s.is_valid()) {
            let pts = seg.points();
            if pts.iter().all(|q| *q == seg.p0) {
                continue;
            }
            let forward = key(&pts);
            let mut rev = pts.clone();
            rev.reverse();
            let backward = key(&rev);
            let k = if forward <= backward { forward } else { backward };
            let here = [c.shape, seg.end];
            match seen.get(&k) {
                Some(&first) => {
                    let [first_shape, first_node] = first;
                    let mut shapes = vec![first_shape];
                    if c.shape != first_shape {
                        shapes.push(c.shape);
                    }
                    sink.push(
                        Hit::new(
                            "overlapping-segments",
                            format!(
                                "The segment ending at node {} of path {} repeats the one ending at node {} of path {}.",
                                Nth(seg.end),
                                Nth(c.shape),
                                Nth(first_node),
                                Nth(first_shape)
                            ),
                        )
                        .shapes(shapes)
                        .segments(vec![first, here])
                        .at(seg.p0.midpoint(seg.p1)),
                    );
                }
                None => {
                    seen.insert(k, here);
                }
            }
        }
    }
}

/// Node lists that do not make segments: a curve node without exactly two off-curve nodes
/// before it, a line node after off-curve nodes, a quadratic node with none, open paths that
/// start or end with off-curve nodes.
pub(crate) fn invalid_segment_structure(input: &LayerInput<'_>, _: &Params<'_>, sink: &mut Sink) {
    for c in &input.contours {
        let nodes = &c.path.nodes;
        let Some(last_on) = nodes.iter().rposition(|n| n.on_curve()) else {
            if !c.closed() && !nodes.is_empty() {
                sink.push(Hit::new("no-on-curve", format!("Open path {} has only off-curve nodes.", Nth(c.shape))).shapes(vec![c.shape]));
            }
            continue;
        };
        if !c.closed() {
            if nodes.first().is_some_and(|n| !n.on_curve()) {
                sink.push(
                    Hit::new("open-starts-off-curve", format!("Open path {} starts with an off-curve node.", Nth(c.shape)))
                        .shapes(vec![c.shape])
                        .points(vec![[c.shape, 0]]),
                );
            }
            if last_on + 1 < nodes.len() {
                sink.push(
                    Hit::new("open-ends-off-curve", format!("Open path {} ends with off-curve nodes.", Nth(c.shape)))
                        .shapes(vec![c.shape])
                        .points((last_on + 1..nodes.len()).map(|i| [c.shape, i]).collect()),
                );
            }
        }
        for seg in c.segs.iter().filter(|s| s.kind == SegKind::Invalid) {
            let count = seg.ctrl.len();
            let (code, message) = match nodes.get(seg.end).map(|n| n.kind) {
                Some(NodeKind::Line) => (
                    "line-after-off-curve",
                    format!("Node {} of path {} is a line node with {count} off-curve nodes before it.", Nth(seg.end), Nth(c.shape)),
                ),
                Some(NodeKind::Curve) => (
                    "curve-off-curve-count",
                    format!("Node {} of path {} is a curve node with {count} off-curve nodes before it; it needs 2.", Nth(seg.end), Nth(c.shape)),
                ),
                _ => (
                    "qcurve-without-off-curve",
                    format!("Node {} of path {} is a quadratic curve node with no off-curve node before it.", Nth(seg.end), Nth(c.shape)),
                ),
            };
            sink.push(
                Hit::new(code, message)
                    .shapes(vec![c.shape])
                    .points(vec![[c.shape, seg.end]])
                    .segments(vec![[c.shape, seg.end]])
                    .data("offCurveNodes", count as f64),
            );
        }
    }
}
