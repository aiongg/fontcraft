//! Hatch Outline (`filter.hatchOutline`, `filter.hatchOutline.*`, docs/spec/11-filters.md ›
//! Hatch Outline).
//!
//! The filled area of a layer's closed paths (nonzero rule) is replaced by parallel hatch lines.
//! The lines run at `angle` degrees, counter-clockwise from horizontal; one passes through the
//! origin and the others repeat every `stepWidth` units across it.
//!
//! - With Offset Path on, every line becomes a rectangle `strokeWidth` units thick centred on it,
//!   and the rectangles are intersected with the outline: closed strokes whose ends follow the
//!   outline.
//! - With it off, every line is cut where it crosses the outline and each piece inside becomes an
//!   open path of two nodes. The crossings come from a scanline pass over the outline flattened
//!   to straight edges, each refined on its own curve, so an end lies on the outline itself.
//!
//! Open paths and single-node paths of the input stay as they are, after the hatching.

use fontcraft_geom::{Point, Rect, Vec2};

use super::{Args, Context, Filter, Input, Item, Param, ParamKind, Slot, Value};
use crate::contour::{Contour, Node, NodeKind, Piece, Ring, round_to_grid};
use crate::{BoolOp, MAX_COORDINATE, MAX_NODES, PathOpError, boolean};

/// Most hatch lines one layer may get. A finer hatch is an error, not a slow run.
pub const MAX_HATCH_LINES: usize = 10_000;
/// Smallest step width and stroke width, in units.
pub const MIN_HATCH_WIDTH: f64 = 1.0;
/// Most straight edges the flattened outline may have when the lines stay open.
const MAX_EDGES: usize = 500_000;
/// Most crossings of hatch lines with the outline, over all lines.
const MAX_CROSSINGS: usize = 1_000_000;
/// The flattened outline is within this many units of the curves. It decides only which side of
/// the outline a piece of line is on near a curve; the ends are found on the curves.
const FLATTEN_TOLERANCE: f64 = 0.1;
/// Most straight edges one curve is flattened into.
const MAX_PIECE_STEPS: usize = 256;
/// Bisection steps when a crossing is refined on its curve.
const BISECT_STEPS: usize = 48;
/// The stroke rectangles reach this many units past the outline's bounds.
const MARGIN: f64 = 1.0;
/// Two pieces of one line that meet closer than this, along the line, are one piece.
const SAME_U: f64 = 1e-9;

pub struct HatchOutline;

const PARAMS: &[Param] = &[
    Param {
        key: "originX",
        label: "Origin X",
        kind: ParamKind::Number { min: -MAX_COORDINATE, max: MAX_COORDINATE },
        default: Value::Number(0.0),
        required: false,
        slot: Slot::Key("OriginX"),
        doc: "x of a point one hatch line passes through, in units",
    },
    Param {
        key: "originY",
        label: "Origin Y",
        kind: ParamKind::Number { min: -MAX_COORDINATE, max: MAX_COORDINATE },
        default: Value::Number(0.0),
        required: false,
        slot: Slot::Key("OriginY"),
        doc: "y of a point one hatch line passes through, in units",
    },
    Param {
        key: "stepWidth",
        label: "Step Width",
        kind: ParamKind::Number { min: MIN_HATCH_WIDTH, max: MAX_COORDINATE },
        default: Value::Number(20.0),
        required: false,
        slot: Slot::Key("StepWidth"),
        doc: "distance between neighbouring hatch lines, in units",
    },
    Param {
        key: "angle",
        label: "Angle",
        kind: ParamKind::Number { min: -360.0, max: 360.0 },
        default: Value::Number(45.0),
        required: false,
        slot: Slot::Key("Angle"),
        doc: "direction of the lines in degrees, counter-clockwise: 0 horizontal, 90 vertical",
    },
    Param {
        key: "offsetPath",
        label: "Offset Path",
        kind: ParamKind::Bool,
        default: Value::Bool(true),
        required: false,
        // Written as the presence of `Offset` in the export string.
        slot: Slot::None,
        doc: "make every line a closed stroke `strokeWidth` thick; off, the lines stay open paths",
    },
    Param {
        key: "strokeWidth",
        label: "Offset",
        kind: ParamKind::Number { min: MIN_HATCH_WIDTH, max: MAX_COORDINATE },
        default: Value::Number(5.0),
        required: false,
        slot: Slot::Key("Offset"),
        doc: "thickness of each stroke when offsetPath is on, in units",
    },
];

const DOC: &str = "{glyph: name | glyphs: [names], layer?: id | layers?: [ids] (default: the first master's layer), originX?: units (default 0), originY?: units (default 0), stepWidth?: units (1 to 1000000, default 20), angle?: degrees (-360 to 360, default 45), offsetPath?: bool (default true), strokeWidth?: units (1 to 1000000, default 5)} replaces the filled area of the closed paths with parallel hatch lines. The lines run at `angle` degrees counter-clockwise from horizontal (0 horizontal, 90 vertical); one passes through (originX, originY) and the others repeat every stepWidth units. With offsetPath each line becomes a closed stroke strokeWidth units thick, cut to the outline; without it each piece of line inside the outline becomes an open path of two nodes. At most 10000 lines per layer. Open paths and components stay; locked paths are not changed; new points are rounded to the grid. Export string: HatchOutlineFilter; OriginX: x; OriginY: y; StepWidth: distance; Angle: degrees; Offset: thickness (without Offset, or with Offset: 0, the lines stay open) → {glyphs: [names], skipped: [{glyph, layer?, reason}]}";

impl Filter for HatchOutline {
    fn id(&self) -> &'static str {
        "filter.hatchOutline"
    }

    fn name(&self) -> &'static str {
        "HatchOutlineFilter"
    }

    fn label(&self) -> &'static str {
        "Hatch Outline"
    }

    fn doc(&self) -> &'static str {
        DOC
    }

    fn params(&self) -> &'static [Param] {
        PARAMS
    }

    fn apply(&self, contours: &[Contour], args: &Args, cx: &Context) -> Result<Vec<Contour>, PathOpError> {
        let hatch = Hatch {
            origin: Point::new(args.number("originX"), args.number("originY")),
            step_width: args.number("stepWidth"),
            angle: args.number("angle"),
            stroke_width: args.flag("offsetPath").then(|| args.number("strokeWidth")),
        };
        hatch_outline(contours, &hatch, cx.grid)
    }

    /// `Offset: thickness` turns Offset Path on with that stroke width; without `Offset`, or
    /// with `Offset: 0`, it is off.
    fn read_items(&self, items: &[Item]) -> Result<Args, String> {
        let is_offset = |i: &Item| i.key.as_deref().is_some_and(|k| k.eq_ignore_ascii_case("Offset"));
        let on = items.iter().find(|&i| is_offset(i)).is_some_and(|i| i.value.trim().parse::<f64>().map_or(true, |v| v != 0.0));
        let kept: Vec<Item> = items.iter().filter(|&i| on || !is_offset(i)).cloned().collect();
        let read = super::read_items(PARAMS, &kept)?;
        let given = read.iter().filter(|(k, _)| *k != "offsetPath").map(|(k, v)| (k, input(v))).chain([("offsetPath", Input::Bool(on))]);
        Args::new(PARAMS, given)
    }

    /// Every keyed value, and `Offset` only when Offset Path is on.
    fn write_items(&self, args: &Args) -> Vec<Item> {
        let value = |key: &str| args.get(key).or_else(|| PARAMS.iter().find(|p| p.key == key).map(|p| p.default));
        let mut out: Vec<Item> = PARAMS
            .iter()
            .filter_map(|p| match p.slot {
                Slot::Key(k) if p.key != "strokeWidth" => value(p.key).map(|v| Item::keyed(k, v.to_string())),
                _ => None,
            })
            .collect();
        if args.flag("offsetPath")
            && let Some(v) = value("strokeWidth")
        {
            out.push(Item::keyed("Offset", v.to_string()));
        }
        out
    }
}

fn input(v: Value) -> Input<'static> {
    match v {
        Value::Number(n) => Input::Number(n),
        Value::Bool(b) => Input::Bool(b),
        Value::Word(w) => Input::Text(w),
    }
}

/// The settings of one hatch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hatch {
    /// A point one line passes through.
    pub origin: Point,
    /// Distance between neighbouring lines, in units.
    pub step_width: f64,
    /// Degrees, counter-clockwise from horizontal.
    pub angle: f64,
    /// The thickness of each closed stroke; `None` leaves the lines open.
    pub stroke_width: Option<f64>,
}

/// Hatch Outline on one layer's contours; see the module docs. New points are rounded to `grid`
/// units (0: not rounded).
pub fn hatch_outline(contours: &[Contour], hatch: &Hatch, grid: f64) -> Result<Vec<Contour>, PathOpError> {
    let Hatch { origin, step_width, angle, stroke_width } = *hatch;
    for v in [origin.x, origin.y, step_width, angle].into_iter().chain(stroke_width) {
        if !v.is_finite() {
            return Err(PathOpError::NotFinite);
        }
    }
    crate::check(origin.x)?;
    crate::check(origin.y)?;
    for (name, w) in [("step width", Some(step_width)), ("stroke width", stroke_width)] {
        if let Some(w) = w
            && !(MIN_HATCH_WIDTH..=MAX_COORDINATE).contains(&w)
        {
            return Err(PathOpError::Invalid(format!("the {name} ({w}) must be from {MIN_HATCH_WIDTH} to {MAX_COORDINATE} units")));
        }
    }
    let total: usize = contours.iter().map(|c| c.nodes.len()).sum();
    if total > MAX_NODES {
        return Err(PathOpError::TooManyNodes(total));
    }
    let rings = contours.iter().map(Ring::from_contour).collect::<Result<Vec<_>, _>>()?;
    let takes_part = |c: &Contour| c.closed && c.nodes.len() > 1;
    let kept: Vec<Contour> = contours.iter().filter(|&c| !takes_part(c) && !c.nodes.is_empty()).cloned().collect();
    let filled: Vec<&Ring> = contours.iter().zip(&rings).filter(|&(c, _)| takes_part(c)).map(|(_, r)| r).collect();
    let Some(bounds) = filled.iter().filter_map(|r| r.bounds()).reduce(|a, b| a.union(b)) else {
        return Ok(contours.to_vec());
    };
    let frame = Frame::new(angle);
    let (u_range, v_range) = frame.extent(bounds);
    let half = stroke_width.map_or(0.0, |w| w / 2.0);
    let lines = Lines::new(origin, step_width, &frame, (v_range.0 - half, v_range.1 + half))?;
    let mut out = match stroke_width {
        Some(_) => strokes(contours, &frame, &lines, half, u_range, v_range, grid)?,
        None => open_lines(&filled, &frame, &lines, grid)?,
    };
    out.extend(kept);
    let count: usize = out.iter().map(|c| c.nodes.len()).sum();
    if count > MAX_NODES {
        return Err(PathOpError::TooManyNodes(count));
    }
    for n in out.iter().flat_map(|c| &c.nodes) {
        crate::check(n.point.x)?;
        crate::check(n.point.y)?;
    }
    Ok(out)
}

/// Coordinates along the lines (`u`) and across them (`v`).
struct Frame {
    /// Unit direction of the lines.
    d: Vec2,
    /// `d` turned 90 degrees counter-clockwise, so (u, v) keeps the orientation of (x, y).
    n: Vec2,
}

impl Frame {
    fn new(degrees: f64) -> Frame {
        let (sin, cos) = degrees.to_radians().sin_cos();
        Frame { d: Vec2::new(cos, sin), n: Vec2::new(-sin, cos) }
    }

    fn u(&self, p: Point) -> f64 {
        p.to_vec2().dot(self.d)
    }

    fn v(&self, p: Point) -> f64 {
        p.to_vec2().dot(self.n)
    }

    fn point(&self, u: f64, v: f64) -> Point {
        Point::ZERO + self.d * u + self.n * v
    }

    /// The (min, max) of u and of v over the corners of `r`.
    fn extent(&self, r: Rect) -> ((f64, f64), (f64, f64)) {
        let corners = [Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)];
        let range = |f: &dyn Fn(Point) -> f64| corners.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| (lo.min(f(*p)), hi.max(f(*p))));
        (range(&|p: Point| self.u(p)), range(&|p: Point| self.v(p)))
    }
}

/// The lines whose `v` falls in a range: `v = first + i * step` for `i` in `0..count`.
struct Lines {
    first: f64,
    step: f64,
    count: usize,
}

impl Lines {
    fn new(origin: Point, step: f64, frame: &Frame, (lo, hi): (f64, f64)) -> Result<Lines, PathOpError> {
        let base = frame.v(origin);
        let k_lo = ((lo - base) / step).ceil();
        let k_hi = ((hi - base) / step).floor();
        if !(k_lo.is_finite() && k_hi.is_finite()) {
            return Err(PathOpError::NotFinite);
        }
        let count = k_hi - k_lo + 1.0;
        if count > MAX_HATCH_LINES as f64 {
            return Err(PathOpError::Invalid(format!(
                "the hatch needs {count} lines; at most {MAX_HATCH_LINES} are allowed, so use a larger step width"
            )));
        }
        // `count` is whole and at most MAX_HATCH_LINES here.
        let count = if count >= 1.0 { count as usize } else { 0 };
        Ok(Lines { first: base + k_lo * step, step, count })
    }

    fn iter(&self) -> impl Iterator<Item = f64> + '_ {
        (0..self.count).map(|i| self.first + i as f64 * self.step)
    }
}

/// Offset Path on: a rectangle `2 * half` thick around each line, intersected with the closed
/// paths. The rectangles stop just past the outline's bounds, so their corners stay near it.
fn strokes(
    contours: &[Contour],
    frame: &Frame,
    lines: &Lines,
    half: f64,
    (u0, u1): (f64, f64),
    (v_lo, v_hi): (f64, f64),
    grid: f64,
) -> Result<Vec<Contour>, PathOpError> {
    let (u0, u1) = (u0 - MARGIN, u1 + MARGIN);
    let mut rects = Vec::with_capacity(lines.count);
    for s in lines.iter() {
        let (v0, v1) = ((s - half).max(v_lo - MARGIN), (s + half).min(v_hi + MARGIN));
        if v1 <= v0 {
            continue;
        }
        let corner = |u: f64, v: f64| Node::new(frame.point(u, v), NodeKind::Line);
        rects.push(Contour::new(true, vec![corner(u0, v0), corner(u1, v0), corner(u1, v1), corner(u0, v1)]));
    }
    let closed: Vec<Contour> = contours.iter().filter(|c| c.closed && c.nodes.len() > 1).cloned().collect();
    boolean(&closed, &rects, BoolOp::Intersection, grid)
}

/// A straight edge of the flattened outline, in frame coordinates, and the part of its curve
/// (`piece`, from `t0` to `t1`) it stands for.
#[derive(Clone, Copy, Debug)]
struct Edge {
    a: (f64, f64),
    b: (f64, f64),
    piece: usize,
    t0: f64,
    t1: f64,
}

impl Edge {
    fn v_min(&self) -> f64 {
        self.a.1.min(self.b.1)
    }

    fn v_max(&self) -> f64 {
        self.a.1.max(self.b.1)
    }
}

/// How many straight edges `piece` is flattened into: enough that each is within
/// [`FLATTEN_TOLERANCE`] of the curve (from the bound on the second derivative), at most
/// [`MAX_PIECE_STEPS`].
fn steps(piece: &Piece) -> usize {
    let (dd, factor) = match piece {
        Piece::Line(..) => return 1,
        Piece::Quad(q) => (((q.p0 - q.p1) + (q.p2 - q.p1)).hypot(), 0.25),
        Piece::Cubic(c) => (((c.p0 - c.p1) + (c.p2 - c.p1)).hypot().max(((c.p1 - c.p2) + (c.p3 - c.p2)).hypot()), 0.75),
    };
    let n = (factor * dd / FLATTEN_TOLERANCE).sqrt().ceil();
    if n.is_finite() && n >= 1.0 { (n as usize).min(MAX_PIECE_STEPS) } else { 1 }
}

/// Offset Path off: each line cut by the outline. Along each line the crossings are sorted and
/// summed by direction (the nonzero rule); a piece where the sum is not zero is inside.
fn open_lines(filled: &[&Ring], frame: &Frame, lines: &Lines, grid: f64) -> Result<Vec<Contour>, PathOpError> {
    let mut pieces: Vec<Piece> = Vec::new();
    for ring in filled {
        for j in ring.segment_ends() {
            if let Some(seg) = ring.seg(j) {
                pieces.extend(seg.pieces());
            }
        }
    }
    let total = pieces.iter().map(steps).fold(0usize, usize::saturating_add);
    if total > MAX_EDGES {
        return Err(PathOpError::Invalid(format!("the outline is too detailed to hatch without Offset Path ({total} edges; at most {MAX_EDGES})")));
    }
    let uv = |p: Point| (frame.u(p), frame.v(p));
    let mut edges: Vec<Edge> = Vec::with_capacity(total);
    for (i, piece) in pieces.iter().enumerate() {
        let n = steps(piece);
        let mut t0 = 0.0;
        let mut a = uv(piece.start());
        for k in 1..=n {
            let t1 = k as f64 / n as f64;
            let b = if k == n { uv(piece.end()) } else { uv(piece.eval(t1)) };
            edges.push(Edge { a, b, piece: i, t0, t1 });
            (a, t0) = (b, t1);
        }
    }
    edges.retain(|e| e.a.1 != e.b.1);
    edges.sort_by(|x, y| x.v_min().total_cmp(&y.v_min()));

    let mut out = Vec::new();
    let mut next = 0usize;
    let mut active: Vec<usize> = Vec::new();
    let mut crossings: Vec<(f64, i32)> = Vec::new();
    let mut found = 0usize;
    for s in lines.iter() {
        while let Some(e) = edges.get(next)
            && e.v_min() <= s
        {
            active.push(next);
            next += 1;
        }
        // An edge counts from its lower end up to, not including, its upper end, so a line
        // through a node meets one of the node's two edges, not both.
        active.retain(|&i| edges.get(i).is_some_and(|e| e.v_max() > s));
        crossings.clear();
        for e in active.iter().filter_map(|&i| edges.get(i)) {
            let Some(piece) = pieces.get(e.piece) else { continue };
            crossings.push((crossing(piece, e, s, frame), if e.b.1 > e.a.1 { 1 } else { -1 }));
        }
        found = found.saturating_add(crossings.len());
        if found > MAX_CROSSINGS {
            return Err(PathOpError::Invalid(format!("the hatch crosses the outline more than {MAX_CROSSINGS} times; use a larger step width")));
        }
        crossings.sort_by(|x, y| x.0.total_cmp(&y.0));
        let mut spans: Vec<(f64, f64)> = Vec::new();
        let (mut winding, mut start) = (0i32, 0.0);
        for &(u, w) in &crossings {
            let before = winding;
            winding = winding.saturating_add(w);
            if before == 0 && winding != 0 {
                start = u;
            } else if before != 0 && winding == 0 {
                match spans.last_mut() {
                    Some(last) if start - last.1 <= SAME_U => last.1 = u,
                    _ => spans.push((start, u)),
                }
            }
        }
        for (u0, u1) in spans {
            let (p0, p1) = (round_to_grid(frame.point(u0, s), grid), round_to_grid(frame.point(u1, s), grid));
            if p0 != p1 {
                out.push(Contour::new(false, vec![Node::new(p0, NodeKind::Line), Node::new(p1, NodeKind::Line)]));
            }
        }
        if out.len().saturating_mul(2) > MAX_NODES {
            return Err(PathOpError::TooManyNodes(out.len().saturating_mul(2)));
        }
    }
    Ok(out)
}

/// The `u` where line `v = s` crosses edge `e`: exact on a straight piece; on a curve, found by
/// bisection between the edge's ends, which lie on the curve on either side of the line.
fn crossing(piece: &Piece, e: &Edge, s: f64, frame: &Frame) -> f64 {
    if matches!(piece, Piece::Line(..)) {
        let f = (s - e.a.1) / (e.b.1 - e.a.1);
        return e.a.0 + f * (e.b.0 - e.a.0);
    }
    let (mut below, mut above) = if e.a.1 <= s { (e.t0, e.t1) } else { (e.t1, e.t0) };
    for _ in 0..BISECT_STEPS {
        let mid = (below + above) / 2.0;
        if frame.v(piece.eval(mid)) <= s {
            below = mid;
        } else {
            above = mid;
        }
    }
    frame.u(piece.eval((below + above) / 2.0))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    use crate::filters::FilterString;
    use crate::fixtures::rect;

    fn hatch(step: f64, angle: f64, stroke: Option<f64>) -> Hatch {
        Hatch { origin: Point::new(5.0, 5.0), step_width: step, angle, stroke_width: stroke }
    }

    fn ends(c: &Contour) -> ((f64, f64), (f64, f64)) {
        let p = |n: &Node| (n.point.x, n.point.y);
        (p(&c.nodes[0]), p(&c.nodes[1]))
    }

    #[test]
    fn open_lines_are_cut_at_the_outline_and_skip_counters() {
        // A 100-unit square with a counter from 40 to 60; horizontal lines at y = 5, 25, 45, 65, 85.
        let outer = rect(0.0, 0.0, 100.0, 100.0);
        let counter = rect(40.0, 40.0, 60.0, 60.0).reversed().unwrap();
        let out = hatch_outline(&[outer, counter], &hatch(20.0, 0.0, None), 1.0).unwrap();
        let lines: Vec<_> = out.iter().map(ends).collect();
        assert!(out.iter().all(|c| !c.closed && c.nodes.len() == 2));
        assert_eq!(
            lines,
            vec![
                ((0.0, 5.0), (100.0, 5.0)),
                ((0.0, 25.0), (100.0, 25.0)),
                ((0.0, 45.0), (40.0, 45.0)),
                ((60.0, 45.0), (100.0, 45.0)),
                ((0.0, 65.0), (100.0, 65.0)),
                ((0.0, 85.0), (100.0, 85.0)),
            ]
        );
        // Vertical lines at 90 degrees, at x = 5 + 20k, in the order of the frame's v = -x.
        let out = hatch_outline(&[rect(0.0, 0.0, 100.0, 100.0)], &hatch(20.0, 90.0, None), 1.0).unwrap();
        assert_eq!(out.iter().map(|c| ends(c).0.0).collect::<Vec<_>>(), vec![85.0, 65.0, 45.0, 25.0, 5.0]);
        assert_eq!(ends(&out[0]), ((85.0, 0.0), (85.0, 100.0)));
    }

    #[test]
    fn an_end_on_a_curve_lies_on_the_curve() {
        // A circle of radius 100 at the origin, from four cubic quarters.
        let k = 55.228_474_983;
        let on = |x: f64, y: f64| Node::new(Point::new(x, y), NodeKind::Curve);
        let off = |x: f64, y: f64| Node::off(Point::new(x, y));
        let circle = Contour::new(
            true,
            vec![
                on(100.0, 0.0),
                off(100.0, k),
                off(k, 100.0),
                on(0.0, 100.0),
                off(-k, 100.0),
                off(-100.0, k),
                on(-100.0, 0.0),
                off(-100.0, -k),
                off(-k, -100.0),
                on(0.0, -100.0),
                off(k, -100.0),
                off(100.0, -k),
            ],
        );
        let h = Hatch { origin: Point::new(0.0, 0.0), step_width: 30.0, angle: 0.0, stroke_width: None };
        let out = hatch_outline(&[circle], &h, 0.0).unwrap();
        assert_eq!(out.len(), 7);
        for c in &out {
            let (a, b) = ends(c);
            let y: f64 = a.1;
            let x = (100.0f64 * 100.0 - y * y).sqrt();
            // The cubic circle is within 0.03 units of a true one, which moves an end near the top
            // by up to 0.07 along the line.
            assert!((a.0 + x).abs() < 0.15 && (b.0 - x).abs() < 0.15, "{a:?} {b:?}");
        }
    }

    #[test]
    fn strokes_are_closed_and_cut_to_the_outline() {
        let out = hatch_outline(&[rect(0.0, 0.0, 100.0, 100.0)], &hatch(20.0, 0.0, Some(4.0)), 1.0).unwrap();
        assert_eq!(out.len(), 5);
        let mut areas: Vec<f64> = out.iter().map(|c| Ring::from_contour(c).unwrap().area()).collect();
        areas.sort_by(f64::total_cmp);
        assert!(areas.iter().all(|a| (a - 400.0).abs() < 1e-6), "{areas:?}");
        assert!(out.iter().all(|c| c.closed));
        let bounds = out.iter().map(|c| Ring::from_contour(c).unwrap().bounds().unwrap()).reduce(|a, b| a.union(b)).unwrap();
        assert_eq!((bounds.x0, bounds.y0, bounds.x1, bounds.y1), (0.0, 3.0, 100.0, 87.0));
        // Strokes as wide as the step merge into the whole square.
        let out = hatch_outline(&[rect(0.0, 0.0, 100.0, 100.0)], &hatch(20.0, 30.0, Some(25.0)), 1.0).unwrap();
        let area: f64 = out.iter().map(|c| Ring::from_contour(c).unwrap().area()).sum();
        assert!((area - 10_000.0).abs() < 1.0, "{area}");
    }

    #[test]
    fn open_paths_stay_and_a_layer_without_closed_paths_is_unchanged() {
        let open = Contour::new(false, vec![Node::new(Point::new(0.0, 0.0), NodeKind::Line), Node::new(Point::new(10.0, 300.0), NodeKind::Line)]);
        assert_eq!(hatch_outline(std::slice::from_ref(&open), &hatch(20.0, 0.0, Some(4.0)), 1.0).unwrap(), vec![open.clone()]);
        let out = hatch_outline(&[rect(0.0, 0.0, 100.0, 100.0), open.clone()], &hatch(20.0, 0.0, None), 1.0).unwrap();
        assert_eq!(out.len(), 6);
        assert_eq!(out.last(), Some(&open));
    }

    #[test]
    fn the_string_form_switches_offset_path_with_offset() {
        let f = HatchOutline;
        let read = |text: &str| f.read_items(&FilterString::parse(text).unwrap().items);
        let on = read("HatchOutlineFilter; OriginX: 10; OriginY: 20; StepWidth: 30; Angle: 90; Offset: 6").unwrap();
        assert!(on.flag("offsetPath"));
        assert_eq!(
            (on.number("originX"), on.number("originY"), on.number("stepWidth"), on.number("angle"), on.number("strokeWidth")),
            (10.0, 20.0, 30.0, 90.0, 6.0)
        );
        assert_eq!(FilterString::write(&f, &on, &[], &[]), "HatchOutlineFilter;OriginX:10;OriginY:20;StepWidth:30;Angle:90;Offset:6;");
        for off in ["HatchOutlineFilter; StepWidth: 30", "HatchOutlineFilter; StepWidth: 30; offset: 0"] {
            let args = read(off).unwrap();
            assert!(!args.flag("offsetPath"), "{off}");
            assert_eq!(args.number("strokeWidth"), 5.0);
            assert_eq!(FilterString::write(&f, &args, &[], &[]), "HatchOutlineFilter;OriginX:0;OriginY:0;StepWidth:30;Angle:45;");
        }
        for bad in [
            "HatchOutlineFilter; 20",
            "HatchOutlineFilter; StepWidth: 0",
            "HatchOutlineFilter; StepWidth: NaN",
            "HatchOutlineFilter; Offset: -3",
            "HatchOutlineFilter; Offset: thick",
            "HatchOutlineFilter; Angle: 1e9",
            "HatchOutlineFilter; OffsetPath: 1",
        ] {
            assert!(read(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn hostile_settings_are_errors() {
        let square = [rect(0.0, 0.0, 100.0, 100.0)];
        for h in [
            hatch(f64::NAN, 0.0, None),
            hatch(20.0, f64::INFINITY, None),
            hatch(0.0, 0.0, None),
            hatch(-5.0, 0.0, None),
            hatch(20.0, 0.0, Some(0.0)),
            hatch(20.0, 0.0, Some(f64::NAN)),
            Hatch { origin: Point::new(1e300, 0.0), ..hatch(20.0, 0.0, None) },
        ] {
            assert!(hatch_outline(&square, &h, 1.0).is_err(), "{h:?}");
        }
        // Far too many lines for the outline is an error before anything is built.
        let huge = [rect(-500_000.0, -500_000.0, 500_000.0, 500_000.0)];
        assert!(matches!(hatch_outline(&huge, &hatch(1.0, 0.0, None), 1.0), Err(PathOpError::Invalid(_))));
        assert!(matches!(hatch_outline(&huge, &hatch(1.0, 0.0, Some(1.0)), 1.0), Err(PathOpError::Invalid(_))));
    }
}
