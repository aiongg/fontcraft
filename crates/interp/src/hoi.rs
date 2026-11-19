//! Evaluating higher-order interpolation (HOI) curves (M9.29).
//!
//! Rows: `hoi.defaultLinear`, `hoi.curveTypes`, `hoi.rotationApproximation`,
//! `hoi.appliesToAllExports` (static half), `hoi.timingGraph` (the timing function).
//!
//! A node's curve ([`fontcraft_model::hoi`]) is kept in the node of the lower layer of a span
//! along one axis. Along that span, at the fraction `s` from the lower layer (0) to the upper
//! layer (1), the node is at [`position`]:
//!
//! | Curve | Path at `s` |
//! |---|---|
//! | none | the straight line, at constant speed |
//! | [`HoiCurve::Point`] | the quadratic Bézier through the intermediate point at `s` = 0.5 |
//! | [`HoiCurve::Timing`] | the cubic Bézier with the two handles of [`handles`], at parameter `s` |
//! | [`HoiCurve::Rotation`] | angle and radius about the centre, both linear in `s` |
//!
//! Interpolation ([`apply`]) adds each curve's [`deviation`] from the straight line to the node
//! of the linearly interpolated layer, weighted by the share the span's two layers have in the
//! interpolation. The curve acts only inside its span; outside it, and on every other axis, the
//! node moves in a straight line.
//!
//! [`timing`] is the fraction of the curve's length a node has travelled at `s`, which the timing
//! graph plots. [`point_to_handles`] gives the handles of the cubic equal to a Point curve.

use fontcraft_model::{Hoi, HoiCenter, HoiCurve, HoiDirection, HoiTiming, Layer, Path, Rare, Shape};

use crate::{InterpError, MAX_COORDINATE};

/// A position in glyph coordinates.
pub type Point = (f64, f64);

/// Segments of the polyline [`timing`] measures the curve's length on.
pub const TIMING_SAMPLES: usize = 64;
/// Most points [`timing_graph`] returns.
pub const MAX_GRAPH_SAMPLES: usize = 1024;

/// A fraction this close to 0 or 1 is the end of the span; a weight or a length this small is
/// zero.
const EPSILON: f64 = 1e-9;

/// One node's span along an axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    /// The node on the lower layer, the one that keeps the curve.
    pub lower: Point,
    /// The same node on the upper layer.
    pub upper: Point,
    /// The centre a rotation turns about when its centre is automatic or left out
    /// ([`auto_center`]).
    pub auto_center: Point,
}

impl Span {
    /// The straight-line position at `s`.
    pub fn linear(&self, s: f64) -> Point {
        lerp(self.lower, self.upper, s)
    }

    fn direction(&self) -> Point {
        sub(self.upper, self.lower)
    }
}

/// The node's position at `s` along `curve`. Outside 0 to 1 the node moves in a straight line;
/// an `s` that is not finite gives the lower position.
pub fn position(curve: &HoiCurve, span: &Span, s: f64) -> Point {
    if !s.is_finite() {
        return span.lower;
    }
    if !(0.0..=1.0).contains(&s) {
        return span.linear(s);
    }
    match curve {
        HoiCurve::Point { x, y } => add(span.linear(s), scale(sub((*x, *y), midpoint(span)), 4.0 * s * (1.0 - s))),
        HoiCurve::Timing(t) => {
            let (h1, h2) = handles(span, t);
            cubic(span.lower, h1, h2, span.upper, s)
        }
        HoiCurve::Rotation { center, direction } => rotation(span, rotation_center(*center, span), *direction, s),
    }
}

/// How far `curve` takes the node from the straight line at `s`: zero at both ends of the span.
pub fn deviation(curve: &HoiCurve, span: &Span, s: f64) -> Point {
    if !s.is_finite() || s <= EPSILON || s >= 1.0 - EPSILON {
        return (0.0, 0.0);
    }
    sub(position(curve, span, s), span.linear(s))
}

/// The fraction of the curve's length the node has travelled at `s` (clamped to 0 to 1): `s`
/// itself for a straight line at constant speed, and for a curve of no length.
pub fn timing(curve: &HoiCurve, span: &Span, s: f64) -> f64 {
    let s = if s.is_finite() { s.clamp(0.0, 1.0) } else { 0.0 };
    let lengths = cumulative_lengths(curve, span);
    let total = lengths.last().copied().unwrap_or(0.0);
    if total <= EPSILON {
        return s;
    }
    let at = s * TIMING_SAMPLES as f64;
    let i = (at.floor() as usize).min(TIMING_SAMPLES - 1);
    let (a, b) = (lengths.get(i).copied().unwrap_or(0.0), lengths.get(i + 1).copied().unwrap_or(total));
    ((a + (b - a) * (at - i as f64)) / total).clamp(0.0, 1.0)
}

/// `samples` points `(s, timing)` of the timing graph, `s` from 0 to 1 in equal steps; at least
/// 2 and at most [`MAX_GRAPH_SAMPLES`].
pub fn timing_graph(curve: &HoiCurve, span: &Span, samples: usize) -> Vec<(f64, f64)> {
    let n = samples.clamp(2, MAX_GRAPH_SAMPLES);
    (0..n)
        .map(|i| {
            let s = i as f64 / (n - 1) as f64;
            (s, timing(curve, span, s))
        })
        .collect()
}

/// The distance travelled at each of the [`TIMING_SAMPLES`] + 1 equally spaced values of `s`.
fn cumulative_lengths(curve: &HoiCurve, span: &Span) -> [f64; TIMING_SAMPLES + 1] {
    let mut out = [0.0; TIMING_SAMPLES + 1];
    let mut previous = span.lower;
    let mut total = 0.0;
    for (i, slot) in out.iter_mut().enumerate().skip(1) {
        let p = position(curve, span, i as f64 / TIMING_SAMPLES as f64);
        let step = (p.0 - previous.0).hypot(p.1 - previous.1);
        if step.is_finite() {
            total += step;
        }
        *slot = total;
        previous = p;
    }
    out
}

/// The two handles of a timing curve: the lower one at its time along the line from the lower
/// node, the upper one at its time back from the upper node, each offset across the line by its
/// amplitude in percent of the line's length. Both offsets are on the left of the direction from
/// the lower to the upper node.
pub fn handles(span: &Span, timing: &HoiTiming) -> (Point, Point) {
    let [lower_time, upper_time, lower_amplitude, upper_amplitude] = timing.values();
    let d = span.direction();
    let across = (-d.1, d.0);
    let h1 = add(add(span.lower, scale(d, lower_time)), scale(across, lower_amplitude / 100.0));
    let h2 = add(sub(span.upper, scale(d, upper_time)), scale(across, upper_amplitude / 100.0));
    (h1, h2)
}

/// The control point of the quadratic a Point curve follows: the curve passes through `ip` at
/// `s` = 0.5.
pub fn point_control(span: &Span, ip: Point) -> Point {
    sub(scale(ip, 2.0), midpoint(span))
}

/// The timing handles of the cubic equal to the Point curve through `ip`; `None` when the two
/// node positions coincide (handles are relative to the line between them) or a value would be
/// out of range.
pub fn point_to_handles(span: &Span, ip: Point) -> Option<HoiTiming> {
    let d = span.direction();
    let length2 = d.0 * d.0 + d.1 * d.1;
    if !length2.is_finite() || length2 <= EPSILON {
        return None;
    }
    let c = point_control(span, ip);
    let v1 = scale(sub(c, span.lower), 2.0 / 3.0);
    let v2 = scale(sub(c, span.upper), 2.0 / 3.0);
    let values = [dot(v1, d) / length2, -dot(v2, d) / length2, 100.0 * cross(d, v1) / length2, 100.0 * cross(d, v2) / length2];
    if !values.iter().all(|v| v.is_finite() && v.abs() <= MAX_COORDINATE) {
        return None;
    }
    let [lower_time, upper_time, lower_amplitude, upper_amplitude] = values;
    Some(HoiTiming {
        lower_time: Some(lower_time),
        upper_time: Some(upper_time),
        lower_amplitude: Some(lower_amplitude),
        upper_amplitude: Some(upper_amplitude),
    })
}

/// The centre a rotation turns about: the given point, or the span's automatic centre.
pub fn rotation_center(center: Option<HoiCenter>, span: &Span) -> Point {
    match center {
        Some(HoiCenter::At { x, y }) => (x, y),
        Some(HoiCenter::Auto) | None => span.auto_center,
    }
}

/// The automatic rotation centre of a node's path: the midpoint of the centres of the path's
/// bounding boxes (of all its nodes) on the lower and the upper layer. `None` for a path without
/// nodes.
pub fn auto_center(lower: &Path, upper: &Path) -> Option<Point> {
    Some(midpoint_of(box_center(lower)?, box_center(upper)?))
}

fn box_center(path: &Path) -> Option<Point> {
    let first = path.nodes.first()?;
    let (mut x0, mut y0, mut x1, mut y1) = (first.x, first.y, first.x, first.y);
    for n in &path.nodes {
        x0 = x0.min(n.x);
        y0 = y0.min(n.y);
        x1 = x1.max(n.x);
        y1 = y1.max(n.y);
    }
    Some(((x0 + x1) / 2.0, (y0 + y1) / 2.0))
}

/// Angle and radius about `center`, each linear in `s`. With equal radii this is a circular arc;
/// with unequal ones a spiral between the two positions (`hoi.rotationApproximation`). Without a
/// direction the smaller angle is taken; a half turn goes counterclockwise. A node on the centre
/// has no angle and moves in a straight line.
fn rotation(span: &Span, center: Point, direction: Option<HoiDirection>, s: f64) -> Point {
    let a = sub(span.lower, center);
    let b = sub(span.upper, center);
    let (ra, rb) = (a.0.hypot(a.1), b.0.hypot(b.1));
    if !(ra > EPSILON && rb > EPSILON && ra.is_finite() && rb.is_finite()) {
        return span.linear(s);
    }
    let start = a.1.atan2(a.0);
    let mut turn = b.1.atan2(b.0) - start;
    if turn > std::f64::consts::PI {
        turn -= std::f64::consts::TAU;
    } else if turn <= -std::f64::consts::PI {
        turn += std::f64::consts::TAU;
    }
    match direction {
        Some(HoiDirection::CounterClockwise) if turn < 0.0 => turn += std::f64::consts::TAU,
        Some(HoiDirection::Clockwise) if turn > 0.0 => turn -= std::f64::consts::TAU,
        _ => {}
    }
    let r = ra + (rb - ra) * s;
    let angle = start + turn * s;
    (center.0 + r * angle.cos(), center.1 + r * angle.sin())
}

/// Whether some node of `layer` has a curve.
pub fn has_curves(layer: &Layer) -> bool {
    layer.shapes.iter().any(|s| matches!(s, Shape::Path(p) if p.nodes.iter().any(|n| !n.info.hoi.is_empty())))
}

/// Bends the nodes of `out`, the linear interpolation of `layers` with `weights_x` (horizontal
/// values) and `weights_y` (vertical values), along the curves the layers' nodes keep, then
/// removes the curves from `out`: an interpolated layer keeps none of its own.
///
/// `locations` are the layers' normalized locations and `tags` the axis tags, both in the
/// designspace's axis order. A layer is the lower layer of a span along an axis when another
/// layer has the same coordinates on every other axis and the next greater one on that axis. The
/// span's share is the sum of its two layers' weights, and `s` the upper layer's part of it;
/// spans with no share, and spans the location lies outside, add nothing. Nodes are matched by
/// shape and node index; a span whose shapes do not correspond is skipped.
pub fn apply(
    out: &mut Layer,
    layers: &[&Layer],
    locations: &[Vec<f64>],
    weights_x: &[f64],
    weights_y: &[f64],
    tags: &[&str],
) -> Result<(), InterpError> {
    if !layers.iter().any(|l| has_curves(l)) {
        return Ok(());
    }
    for (axis, tag) in tags.iter().enumerate() {
        for (li, lower) in layers.iter().enumerate() {
            if !has_curves_on(lower, tag) {
                continue;
            }
            let Some(ui) = upper_neighbour(locations, li, axis) else { continue };
            let Some(upper) = layers.get(ui) else { continue };
            let (sx, sy) = (share(weights_x, li, ui), share(weights_y, li, ui));
            if sx.is_some() || sy.is_some() {
                bend(out, lower, upper, tag, sx, sy);
            }
        }
    }
    for shape in &mut out.shapes {
        let Shape::Path(p) = shape else { continue };
        for n in &mut p.nodes {
            let (x, y) = (n.x, n.y);
            if !x.is_finite() || !y.is_finite() || x.abs() > MAX_COORDINATE || y.abs() > MAX_COORDINATE {
                return Err(InterpError::BadNumber { what: format!("the interpolated value ({x}, {y}) on a curve") });
            }
            if !n.info.hoi.is_empty() {
                let mut info = (*n.info).clone();
                info.hoi = Hoi::default();
                n.info = Rare::new(info);
            }
        }
    }
    Ok(())
}

/// Adds the deviation of every curve on axis `tag` between `lower` and `upper` to `out`.
/// `sx` and `sy` are the span's share and `s` for horizontal and for vertical values.
fn bend(out: &mut Layer, lower: &Layer, upper: &Layer, tag: &str, sx: Option<(f64, f64)>, sy: Option<(f64, f64)>) {
    for (k, shape) in lower.shapes.iter().enumerate() {
        let Shape::Path(lp) = shape else { continue };
        if !lp.nodes.iter().any(|n| n.info.hoi.get(tag).is_some()) {
            continue;
        }
        let (Some(Shape::Path(up)), Some(Shape::Path(op))) = (upper.shapes.get(k), out.shapes.get_mut(k)) else { continue };
        if up.nodes.len() != lp.nodes.len() || op.nodes.len() != lp.nodes.len() {
            continue;
        }
        let center = auto_center(lp, up);
        for ((n, un), on) in lp.nodes.iter().zip(&up.nodes).zip(op.nodes.iter_mut()) {
            let Some(curve) = n.info.hoi.get(tag) else { continue };
            let span = Span { lower: (n.x, n.y), upper: (un.x, un.y), auto_center: center.unwrap_or_else(|| midpoint_of((n.x, n.y), (un.x, un.y))) };
            if let Some((weight, s)) = sx {
                on.x += weight * deviation(curve, &span, s).0;
            }
            if let Some((weight, s)) = sy {
                on.y += weight * deviation(curve, &span, s).1;
            }
        }
    }
}

fn has_curves_on(layer: &Layer, tag: &str) -> bool {
    layer.shapes.iter().any(|s| matches!(s, Shape::Path(p) if p.nodes.iter().any(|n| n.info.hoi.get(tag).is_some())))
}

/// The layer next above layer `li` along `axis` with the same coordinates on the other axes.
fn upper_neighbour(locations: &[Vec<f64>], li: usize, axis: usize) -> Option<usize> {
    let here = locations.get(li)?;
    let at = *here.get(axis)?;
    let mut best: Option<(usize, f64)> = None;
    for (k, other) in locations.iter().enumerate() {
        if k == li || other.len() != here.len() {
            continue;
        }
        let Some(&v) = other.get(axis) else { continue };
        let same_elsewhere = here.iter().zip(other).enumerate().all(|(b, (p, q))| b == axis || (p - q).abs() <= EPSILON);
        if same_elsewhere && v > at + EPSILON && best.is_none_or(|(_, w)| v < w) {
            best = Some((k, v));
        }
    }
    best.map(|(k, _)| k)
}

/// The share of the span of layers `li` and `ui` and the fraction `s` within it; `None` when the
/// span has no share or the location lies at or beyond its ends.
fn share(weights: &[f64], li: usize, ui: usize) -> Option<(f64, f64)> {
    let (wl, wu) = (*weights.get(li)?, *weights.get(ui)?);
    let total = wl + wu;
    if !total.is_finite() || total <= EPSILON {
        return None;
    }
    let s = wu / total;
    (s > EPSILON && s < 1.0 - EPSILON).then_some((total, s))
}

fn cubic(p0: Point, p1: Point, p2: Point, p3: Point, s: f64) -> Point {
    let t = 1.0 - s;
    let (a, b, c, d) = (t * t * t, 3.0 * t * t * s, 3.0 * t * s * s, s * s * s);
    (a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0, a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1)
}

fn midpoint(span: &Span) -> Point {
    midpoint_of(span.lower, span.upper)
}

fn midpoint_of(a: Point, b: Point) -> Point {
    ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0)
}

fn lerp(a: Point, b: Point, s: f64) -> Point {
    (a.0 + (b.0 - a.0) * s, a.1 + (b.1 - a.1) * s)
}

fn add(a: Point, b: Point) -> Point {
    (a.0 + b.0, a.1 + b.1)
}

fn sub(a: Point, b: Point) -> Point {
    (a.0 - b.0, a.1 - b.1)
}

fn scale(a: Point, k: f64) -> Point {
    (a.0 * k, a.1 * k)
}

fn dot(a: Point, b: Point) -> f64 {
    a.0 * b.0 + a.1 * b.1
}

/// The z component of `a × b`.
fn cross(a: Point, b: Point) -> f64 {
    a.0 * b.1 - a.1 * b.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Point, b: Point) -> bool {
        (a.0 - b.0).abs() <= 1e-9 && (a.1 - b.1).abs() <= 1e-9
    }

    fn span(lower: Point, upper: Point) -> Span {
        Span { lower, upper, auto_center: midpoint_of(lower, upper) }
    }

    fn timing_curve(lower_time: f64, upper_time: f64, lower_amplitude: f64, upper_amplitude: f64) -> HoiCurve {
        HoiCurve::Timing(HoiTiming {
            lower_time: Some(lower_time),
            upper_time: Some(upper_time),
            lower_amplitude: Some(lower_amplitude),
            upper_amplitude: Some(upper_amplitude),
        })
    }

    #[test]
    fn each_curve_type_at_the_ends_and_halfway() {
        let line = span((0.0, 0.0), (100.0, 0.0));
        let rotation = |center: Point, direction| HoiCurve::Rotation { center: Some(HoiCenter::At { x: center.0, y: center.1 }), direction };
        let cases = [
            ("point", HoiCurve::Point { x: 50.0, y: 40.0 }, line, (50.0, 40.0)),
            ("handles", timing_curve(1.0 / 3.0, 1.0 / 3.0, 30.0, 30.0), line, (50.0, 22.5)),
            ("half turn, automatic: counterclockwise", rotation((50.0, 0.0), None), line, (50.0, -50.0)),
            ("half turn, clockwise", rotation((50.0, 0.0), Some(HoiDirection::Clockwise)), line, (50.0, 50.0)),
            ("unequal radii", rotation((0.0, 0.0), None), span((100.0, 0.0), (0.0, 200.0)), (150.0 * 0.5f64.sqrt(), 150.0 * 0.5f64.sqrt())),
        ];
        for (what, curve, span, halfway) in cases {
            assert!(close(position(&curve, &span, 0.0), span.lower), "{what} at 0");
            assert!(close(position(&curve, &span, 1.0), span.upper), "{what} at 1");
            assert!(close(position(&curve, &span, 0.5), halfway), "{what} at 0.5: {:?}", position(&curve, &span, 0.5));
            assert_eq!(deviation(&curve, &span, 0.0), (0.0, 0.0), "{what}: no deviation at the lower layer");
            assert_eq!(deviation(&curve, &span, 1.0), (0.0, 0.0), "{what}: no deviation at the upper layer");
        }
        let auto = HoiCurve::Rotation { center: Some(HoiCenter::Auto), direction: Some(HoiDirection::CounterClockwise) };
        let around = Span { lower: (0.0, 0.0), upper: (100.0, 0.0), auto_center: (50.0, 0.0) };
        assert!(close(position(&auto, &around, 0.5), (50.0, -50.0)), "an automatic centre is the span's");
        assert!(close(position(&HoiCurve::Point { x: 50.0, y: 40.0 }, &line, 1.5), (150.0, 0.0)), "straight beyond the span");
    }

    #[test]
    fn default_handles_are_linear_in_path_and_timing() {
        let line = span((10.0, 20.0), (210.0, 70.0));
        let default = HoiCurve::Timing(HoiTiming::default());
        for i in 0..=10 {
            let s = f64::from(i) / 10.0;
            assert!(close(position(&default, &line, s), line.linear(s)), "default handles at {s}");
            assert!((timing(&default, &line, s) - s).abs() <= 1e-9, "default timing at {s}");
        }
        let graph = timing_graph(&default, &line, 5);
        assert_eq!(graph.len(), 5);
        assert!(graph.iter().all(|(s, t)| (s - t).abs() <= 1e-9), "the diagonal: {graph:?}");
        // Handles close to their nodes start and end slowly.
        let eased = timing_curve(0.1, 0.1, 0.0, 0.0);
        assert!(timing(&eased, &line, 0.25) < 0.25, "slow at first");
        assert!((timing(&eased, &line, 0.5) - 0.5).abs() <= 1e-6, "halfway at the middle");
    }

    #[test]
    fn the_handles_of_a_point_curve_give_the_same_curve() {
        let s0 = span((10.0, 20.0), (210.0, 70.0));
        let ip = (80.0, 150.0);
        let point = HoiCurve::Point { x: ip.0, y: ip.1 };
        let handles = HoiCurve::Timing(point_to_handles(&s0, ip).unwrap());
        for i in 0..=8 {
            let s = f64::from(i) / 8.0;
            assert!(close(position(&point, &s0, s), position(&handles, &s0, s)), "at {s}");
        }
        let on_line = point_to_handles(&s0, midpoint(&s0)).unwrap().values();
        assert!(on_line.iter().zip([1.0 / 3.0, 1.0 / 3.0, 0.0, 0.0]).all(|(a, b)| (a - b).abs() <= 1e-9), "{on_line:?}");
        assert_eq!(point_to_handles(&span((5.0, 5.0), (5.0, 5.0)), (9.0, 9.0)), None, "no line to measure handles on");
    }
}
