//! Offset Curve (`filter.offsetCurve` and its behaviour rows, docs/spec/11-filters.md › Offset
//! Curve).
//!
//! The pen: a point whose path runs in unit direction `t` moves by `k · (−t.y · h, t.x · v)`, the
//! left normal with its x part scaled by the horizontal distance `h` and its y part by the vertical
//! distance `v`. A vertical edge moves by `h`, a horizontal edge by `v`; with `h = v` this is the
//! usual parallel curve. `k` picks the side and a multiple: positive is left of the path direction,
//! negative right.
//!
//! Two ways to build the result:
//!
//! - Exact (the default). Each segment is offset on its own: a line exactly, a curve as cubics
//!   fitted to the true offset and split until they are within [`TOLERANCE`] of it. Offset pieces
//!   meet in a mitre on the outer side of a corner (a bevel past [`MITER_LIMIT`]) and through the
//!   node on the inner side. A stroke is the union of the two sides of a path. An offset outline is
//!   the union of the outline and the stroke of its paths (positive distances) or the outline less
//!   that stroke (negative ones), so the boolean engine resolves every overlap and loop. Distances
//!   of opposite signs run as two passes, horizontal first.
//! - Keep Compatible. Every node moves to where the offset lines of its two segments meet, and
//!   every handle is refitted to the new ends, so the result has the same nodes as the input and
//!   stays compatible between masters. Nothing is merged or removed.

use fontcraft_geom::{CubicBez, ParamCurve, ParamCurveNearest, Point, Vec2};

use super::round_corners::fill_sides;
use super::{Args, Context, Filter, Input, Item, Param, ParamKind, Slot, Value};
use crate::boolean::takes_part;
use crate::contour::{Contour, DIRECTION_EPSILON, Node, NodeKind, Piece, RNode, Ring, Seg, round_to_grid};
use crate::{BoolOp, MAX_NODES, MAX_SEGMENTS, OpenPaths, OverlapOptions, PathOpError, SMOOTH_MAX_DEGREES, boolean, remove_overlap};

/// Largest distance, in units, either way.
pub const MAX_OFFSET: f64 = 10_000.0;
/// Most distance, in units, between a fitted offset curve and the true offset before the curve is
/// split.
const TOLERANCE: f64 = 0.25;
/// A curve is halved at most this many times over, into at most 64 pieces.
const MAX_DEPTH: u32 = 6;
/// A mitre reaches at most this many times the offset from its node. Beyond that the corner is
/// bevelled, or with Keep Compatible the mitre is shortened.
const MITER_LIMIT: f64 = 4.0;
/// Offset ends closer than this, in units, are one point.
const JOIN_EPSILON: f64 = 1e-6;
/// Points of a curve its fitted offset is compared with.
const SAMPLES: usize = 8;
/// Step, in curve parameter, of a direction read from two nearby points.
const STEP: f64 = 1e-3;
/// Handle length of a quarter ellipse drawn as one cubic, as a fraction of its radius.
const KAPPA: f64 = 0.552_284_749_830_793_4;

const CAP_STYLES: &[&str] = &["flat", "square", "round", "roundInset", "aligned"];

pub struct OffsetCurve;

const PARAMS: &[Param] = &[
    Param {
        key: "horizontal",
        label: "Horizontal",
        kind: ParamKind::Number { min: -MAX_OFFSET, max: MAX_OFFSET },
        default: Value::Number(10.0),
        required: true,
        slot: Slot::Position(0),
        doc: "how far vertical edges move, in units: positive grows the outline, negative shrinks it; with a stroke, the width on each side at position 50",
    },
    Param {
        key: "vertical",
        label: "Vertical",
        kind: ParamKind::Number { min: -MAX_OFFSET, max: MAX_OFFSET },
        default: Value::Number(10.0),
        required: true,
        slot: Slot::Position(1),
        doc: "how far horizontal edges move, in units; the dialog's lock button keeps it equal to horizontal",
    },
    Param {
        key: "makeStroke",
        label: "Make Stroke",
        kind: ParamKind::Bool,
        default: Value::Bool(false),
        required: false,
        slot: Slot::Position(2),
        doc: "turn every path into a stroke outline around it instead of moving the outline",
    },
    Param {
        key: "autoStroke",
        label: "Auto Stroke",
        kind: ParamKind::Bool,
        default: Value::Bool(false),
        required: false,
        slot: Slot::None,
        doc: "make a centred stroke that keeps the layer's height (the paths are first squeezed vertically by the vertical distance at the top and bottom); `auto` in place of the position in the export string",
    },
    Param {
        key: "position",
        label: "Position",
        kind: ParamKind::Number { min: 0.0, max: 100.0 },
        default: Value::Number(50.0),
        required: false,
        slot: Slot::Position(3),
        doc: "where a stroke lies, in percent: 0 all on the left of the path direction, 100 all on the right, 50 centred; a fraction from 0 to 1 in the export string",
    },
    Param {
        key: "keepCompatible",
        label: "Keep Compatible",
        kind: ParamKind::Bool,
        default: Value::Bool(false),
        required: false,
        slot: Slot::None,
        doc: "move every node and handle in place and add or remove none, so masters stay compatible; less accurate on curves",
    },
    Param {
        key: "capStyle",
        label: "Cap Style",
        kind: ParamKind::Choice(CAP_STYLES),
        default: Value::Word("flat"),
        required: false,
        slot: Slot::None,
        doc: "the ends of open paths in a stroke: flat, square (longer by half the width), round, roundInset (round, within the path's length) or aligned (cut horizontally or vertically)",
    },
];

const DOC: &str = "{glyph: name | glyphs: [names], layer?: id | layers?: [ids] (default: the first master's layer), horizontal: units (-10000 to 10000), vertical: units (-10000 to 10000), makeStroke?: bool (default false), autoStroke?: bool (default false), position?: percent (0 to 100, default 50), keepCompatible?: bool (default false), capStyle?: \"flat\" | \"square\" | \"round\" | \"roundInset\" | \"aligned\" (default \"flat\")} moves the outline sideways: vertical edges by horizontal units and horizontal edges by vertical units, outward for positive values and inward for negative ones, with sharp corners; an open path moves to the right of its direction. With makeStroke every path becomes a stroke outline, horizontal and vertical units wide on each side at position 50 (0: all on the left of the path direction, 100: all on the right), with capStyle at the ends of open paths; autoStroke makes a centred stroke that keeps the layer's height. Curves gain nodes where the offset needs them and overlaps are removed, unless keepCompatible: then every node and handle moves in place and only caps add nodes, so masters stay compatible. Components and locked paths are not changed; new points are rounded to the grid. Export string: OffsetCurve; horizontal; vertical; makestroke; position (a fraction from 0 to 1, or auto for autoStroke) → {glyphs: [names], skipped: [{glyph, layer?, reason}]}";

impl Filter for OffsetCurve {
    fn id(&self) -> &'static str {
        "filter.offsetCurve"
    }

    fn name(&self) -> &'static str {
        "OffsetCurve"
    }

    fn label(&self) -> &'static str {
        "Offset Curve"
    }

    fn doc(&self) -> &'static str {
        DOC
    }

    fn params(&self) -> &'static [Param] {
        PARAMS
    }

    fn apply(&self, contours: &[Contour], args: &Args, cx: &Context) -> Result<Vec<Contour>, PathOpError> {
        let auto = args.flag("autoStroke");
        let options = OffsetOptions {
            horizontal: args.number("horizontal"),
            vertical: args.number("vertical"),
            stroke: args.flag("makeStroke") || auto,
            auto,
            position: if auto { 50.0 } else { args.number("position") },
            compatible: args.flag("keepCompatible"),
            cap: CapStyle::from_word(args.word("capStyle")),
        };
        offset_curve(contours, &options, cx.grid)
    }

    /// `horizontal; vertical; makestroke; position`, the position as a fraction or `auto`.
    fn read_items(&self, items: &[Item]) -> Result<Args, String> {
        if let Some(item) = items.iter().find(|i| i.key.is_some()) {
            return Err(format!("this filter takes no `Key: value` items, got \"{item}\""));
        }
        let mut given: Vec<(&str, Input)> = Vec::with_capacity(items.len());
        for (i, item) in items.iter().enumerate() {
            let value = item.value.trim();
            match i {
                0 => given.push(("horizontal", Input::Text(value))),
                1 => given.push(("vertical", Input::Text(value))),
                2 => given.push(("makeStroke", Input::Text(value))),
                3 if value.eq_ignore_ascii_case("auto") => given.push(("autoStroke", Input::Bool(true))),
                3 => {
                    let fraction = value
                        .parse::<f64>()
                        .ok()
                        .filter(|f| f.is_finite() && (0.0..=1.0).contains(f))
                        .ok_or_else(|| format!("the position must be a fraction from 0 to 1 or `auto`, got \"{value}\""))?;
                    // Rounded so that a percentage written as a fraction reads back as itself.
                    given.push(("position", Input::Number((fraction * 1e8).round() / 1e6)));
                }
                _ => return Err(format!("this filter takes 4 values without a key; \"{value}\" is one too many")),
            }
        }
        Args::new(PARAMS, given)
    }

    fn write_items(&self, args: &Args) -> Vec<Item> {
        let number = |key: &str| Value::Number(args.number(key)).to_string();
        let position =
            if args.flag("autoStroke") { "auto".to_string() } else { Value::Number((args.number("position") * 1e6).round() / 1e8).to_string() };
        vec![
            Item::positional(number("horizontal")),
            Item::positional(number("vertical")),
            Item::positional(Value::Bool(args.flag("makeStroke")).to_string()),
            Item::positional(position),
        ]
    }
}

/// The ends of open paths in a stroke.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CapStyle {
    /// Straight across, at the end.
    #[default]
    Flat,
    /// Straight across, half the stroke width beyond the end.
    Square,
    /// A half ellipse beyond the end.
    Round,
    /// A half ellipse that ends at the end: the stroke is shortened by half its width.
    RoundInset,
    /// Cut along the horizontal or vertical through the end, whichever is closer to square to the
    /// path.
    Aligned,
}

impl CapStyle {
    /// The cap of a `capStyle` word; flat for any other word.
    pub fn from_word(word: &str) -> CapStyle {
        match word {
            "square" => CapStyle::Square,
            "round" => CapStyle::Round,
            "roundInset" => CapStyle::RoundInset,
            "aligned" => CapStyle::Aligned,
            _ => CapStyle::Flat,
        }
    }
}

/// The settings of one Offset Curve run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OffsetOptions {
    /// Units vertical edges move; with a stroke, its width on each side at position 50.
    pub horizontal: f64,
    /// Units horizontal edges move.
    pub vertical: f64,
    /// Turn every path into a stroke outline.
    pub stroke: bool,
    /// With `stroke`: squeeze the paths vertically by `vertical` at the top and bottom first, so
    /// the stroke keeps the layer's height. The caller sets `position` to 50.
    pub auto: bool,
    /// Percent of a stroke on the right of the path direction, 0 to 100.
    pub position: f64,
    /// Move nodes and handles in place; add or remove none (Keep Compatible).
    pub compatible: bool,
    pub cap: CapStyle,
}

/// Offset Curve on one layer's contours; the module docs describe the geometry. Points are rounded
/// to multiples of `grid` units (0: not rounded).
pub fn offset_curve(contours: &[Contour], o: &OffsetOptions, grid: f64) -> Result<Vec<Contour>, PathOpError> {
    for (name, v) in [("horizontal", o.horizontal), ("vertical", o.vertical)] {
        if !v.is_finite() {
            return Err(PathOpError::NotFinite);
        }
        if v.abs() > MAX_OFFSET {
            return Err(PathOpError::Invalid(format!("the {name} distance ({v}) must be from -{MAX_OFFSET} to {MAX_OFFSET} units")));
        }
    }
    if !(o.position.is_finite() && (0.0..=100.0).contains(&o.position)) {
        return Err(PathOpError::Invalid(format!("the position ({}) must be from 0 to 100 percent", o.position)));
    }
    let total: usize = contours.iter().map(|c| c.nodes.len()).sum();
    if total > MAX_NODES {
        return Err(PathOpError::TooManyNodes(total));
    }
    let rings = contours.iter().map(Ring::from_contour).collect::<Result<Vec<_>, _>>()?;
    if o.horizontal == 0.0 && o.vertical == 0.0 {
        return Ok(contours.to_vec());
    }
    let out = if o.stroke {
        let pen = Vec2::new(o.horizontal.abs(), o.vertical.abs());
        let rings = if o.auto { squeeze(rings, pen.y) } else { rings };
        stroke(contours, &rings, pen, o.position / 100.0, o.cap, o.compatible, grid)?
    } else if o.compatible {
        offset_compatible(contours, &rings, Vec2::new(o.horizontal, o.vertical), grid)?
    } else {
        offset_exact(contours, &rings, o.horizontal, o.vertical, grid)?
    };
    let mut count = 0usize;
    for c in &out {
        count = count.saturating_add(c.nodes.len());
        for n in &c.nodes {
            crate::check(n.point.x)?;
            crate::check(n.point.y)?;
        }
    }
    if count > MAX_NODES {
        return Err(PathOpError::TooManyNodes(count));
    }
    Ok(out)
}

// ── The three modes ─────────────────────────────────────────────────────────────────────────────

fn offset_exact(contours: &[Contour], rings: &[Ring], h: f64, v: f64, grid: f64) -> Result<Vec<Contour>, PathOpError> {
    // Each pass: a pen of non-negative distances, and whether the outline grows.
    let passes: Vec<(Vec2, bool)> = if h * v >= 0.0 {
        vec![(Vec2::new(h.abs(), v.abs()), h + v > 0.0)]
    } else {
        vec![(Vec2::new(h.abs(), 0.0), h > 0.0), (Vec2::new(0.0, v.abs()), v > 0.0)]
    };
    let last = passes.len().saturating_sub(1);
    let mut budget = 0usize;
    let mut shape: Vec<Contour> = contours.iter().filter(|c| takes_part(c)).cloned().collect();
    for (i, (pen, grow)) in passes.into_iter().enumerate() {
        let mut sweep = Vec::new();
        for c in &shape {
            let pieces = pieces_of(&Ring::from_contour(c)?);
            if pieces.is_empty() {
                continue;
            }
            let left = ring_of(&side(&pieces, true, pen, 1.0, Joins::Stroke, &mut budget)?, true);
            let right = ring_of(&side(&pieces, true, pen, -1.0, Joins::Stroke, &mut budget)?, true);
            sweep.extend(remove_overlap(&[right.to_contour(), left.reversed().to_contour()], unrounded())?);
        }
        let op = if grow { BoolOp::Union } else { BoolOp::Difference };
        shape = boolean(&shape, &sweep, op, if i == last { grid } else { 0.0 })?;
    }
    let mut out = shape;
    for (contour, ring) in contours.iter().zip(rings) {
        if takes_part(contour) {
            continue;
        }
        let pieces = pieces_of(ring);
        if contour.closed || pieces.is_empty() {
            out.push(contour.clone());
            continue;
        }
        // An open path has no filled side; it moves to the right, as the outward side of a
        // counter-clockwise outer contour.
        let moved = ring_of(&side(&pieces, false, Vec2::new(h, v), -1.0, Joins::Parallel, &mut budget)?, false);
        out.push(on_grid(moved.to_contour(), grid));
    }
    Ok(out)
}

fn offset_compatible(contours: &[Contour], rings: &[Ring], pen: Vec2, grid: f64) -> Result<Vec<Contour>, PathOpError> {
    let sides = fill_sides(contours, rings)?;
    let mut out = Vec::with_capacity(rings.len());
    for (i, (contour, ring)) in contours.iter().zip(rings).enumerate() {
        // Outward is the side away from the fill; an open path moves to its right.
        let k = if ring.closed { -sides.get(i).copied().unwrap_or(0.0) } else { -1.0 };
        if k == 0.0 {
            out.push(contour.clone());
        } else {
            out.push(on_grid(compatible_ring(ring, pen, k, true).to_contour(), grid));
        }
    }
    Ok(out)
}

/// `position` is the fraction of the stroke on the right.
fn stroke(
    contours: &[Contour],
    rings: &[Ring],
    pen: Vec2,
    position: f64,
    cap: CapStyle,
    compatible: bool,
    grid: f64,
) -> Result<Vec<Contour>, PathOpError> {
    let (kl, kr) = (2.0 * (1.0 - position), -2.0 * position);
    let mut budget = 0usize;
    let mut parts = Vec::new();
    let mut kept = Vec::new();
    for (contour, ring) in contours.iter().zip(rings) {
        let pieces = pieces_of(ring);
        let (Some(first), Some(last)) = (pieces.first(), pieces.last()) else {
            kept.push(contour.clone());
            continue;
        };
        let (left, right) = if compatible {
            (compatible_ring(ring, pen, kl, false), compatible_ring(ring, pen, kr, false))
        } else {
            (
                ring_of(&side(&pieces, ring.closed, pen, kl, Joins::Stroke, &mut budget)?, ring.closed),
                ring_of(&side(&pieces, ring.closed, pen, kr, Joins::Stroke, &mut budget)?, ring.closed),
            )
        };
        // Outer contours run counter-clockwise: the right side forward and the left side
        // backward.
        let path: Vec<Contour> = if ring.closed {
            vec![right.to_contour(), left.reversed().to_contour()]
        } else {
            let ends = Ends { start: first.start(), start_dir: first.start_dir(), end: last.end(), end_dir: last.end_dir() };
            open_stroke(left, right, &ends, cap).map(|r| r.reversed().to_contour()).into_iter().collect()
        };
        if compatible {
            parts.extend(path.into_iter().map(|c| on_grid(c, grid)));
        } else {
            parts.extend(remove_overlap(&path, unrounded())?);
        }
    }
    let mut out = if compatible { parts } else { remove_overlap(&parts, OverlapOptions { grid, open_paths: OpenPaths::Remove })? };
    out.extend(kept);
    Ok(out)
}

fn unrounded() -> OverlapOptions {
    OverlapOptions { grid: 0.0, open_paths: OpenPaths::Remove }
}

fn on_grid(mut c: Contour, grid: f64) -> Contour {
    for n in &mut c.nodes {
        n.point = round_to_grid(n.point, grid);
    }
    c
}

/// The rings squeezed vertically about the middle of their bounds, so that they are `2 · v` units
/// less tall (at least flat).
fn squeeze(mut rings: Vec<Ring>, v: f64) -> Vec<Ring> {
    let Some(b) = rings.iter().filter_map(Ring::bounds).reduce(|a, b| a.union(b)) else { return rings };
    let height = b.height();
    if height <= 0.0 {
        return rings;
    }
    let factor = ((height - 2.0 * v) / height).max(0.0);
    let middle = (b.y0 + b.y1) / 2.0;
    for ring in &mut rings {
        for r in &mut ring.nodes {
            for n in std::iter::once(&mut r.node).chain(r.ctrl.iter_mut()) {
                n.point.y = middle + (n.point.y - middle) * factor;
            }
        }
    }
    rings
}

// ── The pen and the offset of a curve ───────────────────────────────────────────────────────────

/// How far side `k` of the pen moves a point whose path runs in unit direction `t`.
fn reach(t: Vec2, pen: Vec2, k: f64) -> Vec2 {
    Vec2::new(-t.y * pen.x * k, t.x * pen.y * k)
}

fn unit(v: Vec2) -> Option<Vec2> {
    (v.is_finite() && v.hypot() > DIRECTION_EPSILON).then(|| v.normalize())
}

fn deriv(c: &CubicBez, t: f64) -> Vec2 {
    let mt = 1.0 - t;
    (c.p1 - c.p0) * (3.0 * mt * mt) + (c.p2 - c.p1) * (6.0 * mt * t) + (c.p3 - c.p2) * (3.0 * t * t)
}

fn deriv2(c: &CubicBez, t: f64) -> Vec2 {
    ((c.p2 - c.p1) - (c.p1 - c.p0)) * (6.0 * (1.0 - t)) + ((c.p3 - c.p2) - (c.p2 - c.p1)) * (6.0 * t)
}

/// The unit direction of `c` at `t`. Where a handle is retracted, it is read from two nearby
/// points.
fn tangent(c: &CubicBez, t: f64) -> Option<Vec2> {
    unit(deriv(c, t)).or_else(|| unit(c.eval((t + STEP).min(1.0)) - c.eval((t - STEP).max(0.0))))
}

/// The point of side `k`'s offset of `c` at `t`.
fn offset_at(c: &CubicBez, t: f64, pen: Vec2, k: f64) -> Option<Point> {
    Some(c.eval(t) + reach(tangent(c, t)?, pen, k))
}

/// The unit direction of side `k`'s offset of `c` at `t`.
fn offset_dir(c: &CubicBez, t: f64, pen: Vec2, k: f64) -> Option<Vec2> {
    let d1 = deriv(c, t);
    let speed = d1.hypot();
    if speed > DIRECTION_EPSILON {
        // The pen is linear in the direction, so the offset's derivative times the speed is
        // c′·|c′| plus the pen applied to the part of c″ square to the direction.
        let along = d1 / speed;
        let d2 = deriv2(c, t);
        if let Some(u) = unit(d1 * speed + reach(d2 - along * along.dot(d2), pen, k)) {
            return Some(u);
        }
    }
    let (a, b) = ((t - STEP).max(0.0), (t + STEP).min(1.0));
    unit(offset_at(c, b, pen, k)? - offset_at(c, a, pen, k)?)
}

/// A cubic from `p0` to `p3` that follows side `k`'s offset of `c`: it leaves and arrives in the
/// offset's directions, with handle lengths fitted (least squares) to points of the true offset.
/// A handle marked in `retracted` stays on its node.
fn fit(c: &CubicBez, p0: Point, p3: Point, pen: Vec2, k: f64, retracted: [bool; 2]) -> CubicBez {
    let d0 = offset_dir(c, 0.0, pen, k).or_else(|| tangent(c, 0.0)).unwrap_or(Vec2::ZERO);
    let d1 = offset_dir(c, 1.0, pen, k).or_else(|| tangent(c, 1.0)).unwrap_or(Vec2::ZERO);
    let (mut m11, mut m12, mut m22, mut r1, mut r2) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for i in 1..=SAMPLES {
        let t = i as f64 / (SAMPLES + 1) as f64;
        let Some(target) = offset_at(c, t, pen, k) else { continue };
        let mt = 1.0 - t;
        let base = p0.to_vec2() * (mt * mt * mt + 3.0 * mt * mt * t) + p3.to_vec2() * (3.0 * mt * t * t + t * t * t);
        let r = target.to_vec2() - base;
        let (u, w) = (d0 * (3.0 * mt * mt * t), d1 * (-3.0 * mt * t * t));
        m11 += u.dot(u);
        m12 += u.dot(w);
        m22 += w.dot(w);
        r1 += u.dot(r);
        r2 += w.dot(r);
    }
    let chord = p0.distance(p3);
    let fallback = chord / 3.0;
    let (a, b) = match retracted {
        [true, true] => (0.0, 0.0),
        [true, false] => (0.0, if m22 > 0.0 { r2 / m22 } else { fallback }),
        [false, true] => (if m11 > 0.0 { r1 / m11 } else { fallback }, 0.0),
        [false, false] => {
            let det = m11 * m22 - m12 * m12;
            // Never negative (Cauchy–Schwarz); near zero when the samples cannot tell the two
            // lengths apart.
            if det > 0.0 && det > 1e-12 * m11 * m22 { ((r1 * m22 - r2 * m12) / det, (r2 * m11 - r1 * m12) / det) } else { (fallback, fallback) }
        }
    };
    // A handle longer than this is a fit gone wrong.
    let limit = 2.0 * (chord + c.p0.distance(c.p1) + c.p1.distance(c.p2) + c.p2.distance(c.p3));
    let sane = |l: f64, keep: bool| {
        if keep {
            0.0
        } else if l.is_finite() && (0.0..=limit).contains(&l) {
            l
        } else {
            fallback
        }
    };
    let [keep0, keep1] = retracted;
    CubicBez::new(p0, p0 + d0 * sane(a, keep0), p3 - d1 * sane(b, keep1), p3)
}

/// The largest distance from points of the true offset to `fitted`.
fn fit_error(c: &CubicBez, fitted: &CubicBez, pen: Vec2, k: f64) -> f64 {
    let mut worst: f64 = 0.0;
    for i in 0..SAMPLES {
        let t = (i as f64 + 0.5) / SAMPLES as f64;
        if let Some(p) = offset_at(c, t, pen, k) {
            worst = worst.max(fitted.nearest(p, 1e-6).distance_sq);
        }
    }
    worst.sqrt()
}

/// Side `k`'s offset of `c` as cubics within [`TOLERANCE`], appended to `out`.
fn offset_cubic(c: CubicBez, pen: Vec2, k: f64, depth: u32, out: &mut Vec<Bez>, budget: &mut usize) -> Result<(), PathOpError> {
    let (Some(p0), Some(p3)) = (offset_at(&c, 0.0, pen, k), offset_at(&c, 1.0, pen, k)) else { return Ok(()) };
    let fitted = fit(&c, p0, p3, pen, k, [false, false]);
    if depth >= MAX_DEPTH || fit_error(&c, &fitted, pen, k) <= TOLERANCE {
        return push(out, Bez::Cubic(fitted), budget);
    }
    offset_cubic(c.subsegment(0.0..0.5), pen, k, depth + 1, out, budget)?;
    offset_cubic(c.subsegment(0.5..1.0), pen, k, depth + 1, out, budget)
}

fn push(out: &mut Vec<Bez>, b: Bez, budget: &mut usize) -> Result<(), PathOpError> {
    *budget += 1;
    if *budget > MAX_SEGMENTS {
        return Err(PathOpError::TooManySegments(*budget));
    }
    out.push(b);
    Ok(())
}

/// Where the line through `p` along `a` meets the line through `q` along `b`, with the distances
/// along each: `p + a·s = q + b·u`. `None` for parallel lines.
fn intersect(p: Point, a: Vec2, q: Point, b: Vec2) -> Option<(Point, f64, f64)> {
    let cross = a.cross(b);
    if cross.abs() < 1e-9 {
        return None;
    }
    let d = q - p;
    let (s, u) = (d.cross(b) / cross, d.cross(a) / cross);
    Some((p + a * s, s, u))
}

// ── Exact sides ─────────────────────────────────────────────────────────────────────────────────

/// A straight or cubic piece of a path.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Bez {
    Line(Point, Point),
    Cubic(CubicBez),
}

impl Bez {
    /// A quadratic piece becomes the cubic that draws the same curve.
    fn of(piece: Piece) -> Bez {
        match piece {
            Piece::Line(a, b) => Bez::Line(a, b),
            Piece::Quad(q) => Bez::Cubic(CubicBez::new(q.p0, q.p0 + (q.p1 - q.p0) * (2.0 / 3.0), q.p2 + (q.p1 - q.p2) * (2.0 / 3.0), q.p2)),
            Piece::Cubic(c) => Bez::Cubic(c),
        }
    }

    fn start(&self) -> Point {
        match self {
            Bez::Line(a, _) => *a,
            Bez::Cubic(c) => c.p0,
        }
    }

    fn end(&self) -> Point {
        match self {
            Bez::Line(_, b) => *b,
            Bez::Cubic(c) => c.p3,
        }
    }

    fn is_line(&self) -> bool {
        matches!(self, Bez::Line(..))
    }

    fn degenerate(&self) -> bool {
        match self {
            Bez::Line(a, b) => a.distance(*b) <= JOIN_EPSILON,
            Bez::Cubic(c) => [c.p1, c.p2, c.p3].iter().all(|p| p.distance(c.p0) <= JOIN_EPSILON),
        }
    }

    fn start_dir(&self) -> Option<Vec2> {
        match self {
            Bez::Line(a, b) => unit(*b - *a),
            Bez::Cubic(c) => tangent(c, 0.0),
        }
    }

    fn end_dir(&self) -> Option<Vec2> {
        match self {
            Bez::Line(a, b) => unit(*b - *a),
            Bez::Cubic(c) => tangent(c, 1.0),
        }
    }

    /// Moves the start; a handle moves with its node.
    fn set_start(&mut self, p: Point) {
        match self {
            Bez::Line(a, _) => *a = p,
            Bez::Cubic(c) => {
                let d = p - c.p0;
                c.p0 = p;
                c.p1 += d;
            }
        }
    }

    fn set_end(&mut self, p: Point) {
        match self {
            Bez::Line(_, b) => *b = p,
            Bez::Cubic(c) => {
                let d = p - c.p3;
                c.p3 = p;
                c.p2 += d;
            }
        }
    }

    /// The ring node that ends this piece.
    fn rnode(&self) -> RNode {
        match self {
            Bez::Line(_, b) => RNode::new(Node::new(*b, NodeKind::Line), Vec::new()),
            Bez::Cubic(c) => RNode::new(Node::new(c.p3, NodeKind::Curve), vec![Node::off(c.p1), Node::off(c.p2)]),
        }
    }
}

/// The pieces of a ring, quadratic ones as cubics, without pieces of no length. A closed ring
/// starts with the segment that closes it.
fn pieces_of(ring: &Ring) -> Vec<Bez> {
    ring.segment_ends().filter_map(|j| ring.seg(j)).flat_map(|s| s.pieces()).map(Bez::of).filter(|b| !b.degenerate()).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Joins {
    /// Mitre or bevel on the outer side of a corner, through the node on the inner side: one side
    /// of a stroke, filled with the nonzero rule.
    Stroke,
    /// Mitre on the outer side, trimmed (lines) or joined straight on the inner side: an offset
    /// open path.
    Parallel,
}

/// The pieces between two offset pieces, and where the pieces themselves now end and start.
#[derive(Debug, Default)]
struct Join {
    end: Option<Point>,
    start: Option<Point>,
    between: Vec<Bez>,
}

/// Side `k`'s offset of a chain of pieces, joined at every node.
fn side(pieces: &[Bez], closed: bool, pen: Vec2, k: f64, joins: Joins, budget: &mut usize) -> Result<Vec<Bez>, PathOpError> {
    let mut spans: Vec<Bez> = Vec::new();
    let mut prev: Option<&Bez> = None;
    let mut first: Option<&Bez> = None;
    for piece in pieces {
        let mut offs = Vec::new();
        match piece {
            Bez::Line(a, b) => {
                let Some(t) = unit(*b - *a) else { continue };
                let w = reach(t, pen, k);
                push(&mut offs, Bez::Line(*a + w, *b + w), budget)?;
            }
            Bez::Cubic(c) => offset_cubic(*c, pen, k, 0, &mut offs, budget)?,
        }
        let Some(head) = offs.first() else { continue };
        if let (Some(p), Some(last)) = (prev, spans.last()) {
            let j = join(last, head, p, piece, k, joins);
            apply_join(&mut spans, &mut offs, j);
        }
        spans.append(&mut offs);
        prev = Some(piece);
        first = first.or(Some(piece));
    }
    if closed && let (Some(p), Some(f), Some(last), Some(head)) = (prev, first, spans.last(), spans.first()) {
        let j = join(last, head, p, f, k, joins);
        if let Some(e) = j.end
            && let Some(l) = spans.last_mut()
        {
            l.set_end(e);
        }
        if let Some(s) = j.start
            && let Some(h) = spans.first_mut()
        {
            h.set_start(s);
        }
        spans.extend(j.between);
    }
    Ok(spans)
}

/// Applies a join between the last of `spans` and the first of `next`.
fn apply_join(spans: &mut Vec<Bez>, next: &mut [Bez], j: Join) {
    if let Some(e) = j.end
        && let Some(l) = spans.last_mut()
    {
        l.set_end(e);
    }
    if let Some(s) = j.start
        && let Some(h) = next.first_mut()
    {
        h.set_start(s);
    }
    spans.extend(j.between);
}

/// How offset piece `a` (of original piece `pa`) meets offset piece `b` (of `pb`).
fn join(a: &Bez, b: &Bez, pa: &Bez, pb: &Bez, k: f64, joins: Joins) -> Join {
    let (ae, bs) = (a.end(), b.start());
    if ae.distance(bs) <= JOIN_EPSILON {
        return Join { start: Some(ae), ..Join::default() };
    }
    let node = pa.end();
    let (Some(tin), Some(tout)) = (pa.end_dir(), pb.start_dir()) else { return straight(&[ae, bs]) };
    let (da, db) = (a.end_dir().unwrap_or(tin), b.start_dir().unwrap_or(tout));
    let hit = intersect(ae, da, bs, db);
    let width = ae.distance(node).max(bs.distance(node));
    let outer = tin.cross(tout) * k < 0.0;
    if outer || joins == Joins::Parallel {
        // The offset ends move apart: their lines meet ahead of `a` and behind `b`.
        if let Some((m, s, u)) = hit
            && s >= 0.0
            && u <= 0.0
            && m.distance(node) <= MITER_LIMIT * width
        {
            let mut j = Join::default();
            if a.is_line() {
                j.end = Some(m);
            } else {
                j.between.extend(straight(&[ae, m]).between);
            }
            if b.is_line() {
                j.start = Some(m);
            } else {
                j.between.extend(straight(&[m, bs]).between);
            }
            return j;
        }
        // Two offset lines that cross: both end where they meet.
        if joins == Joins::Parallel
            && let (Bez::Line(a0, _), Bez::Line(_, b1)) = (a, b)
            && let Some((m, s, u)) = hit
            && s <= 0.0
            && u >= 0.0
            && -s <= a0.distance(ae)
            && u <= bs.distance(*b1)
        {
            return Join { end: Some(m), start: Some(m), between: Vec::new() };
        }
        return straight(&[ae, bs]);
    }
    straight(&[ae, node, bs])
}

/// Lines through `points`, leaving out lines of no length.
fn straight(points: &[Point]) -> Join {
    let between = points.windows(2).filter_map(|w| if let [p, q] = w { Some((*p, *q)) } else { None }).filter(|(p, q)| p.distance(*q) > JOIN_EPSILON);
    Join { between: between.map(|(p, q)| Bez::Line(p, q)).collect(), ..Join::default() }
}

/// A ring of the pieces in `spans`, which follow each other end to start (and come back to the
/// start when `closed`). Nodes whose two directions agree are smooth.
fn ring_of(spans: &[Bez], closed: bool) -> Ring {
    let mut nodes: Vec<RNode> = Vec::with_capacity(spans.len() + 1);
    if closed {
        nodes.extend(spans.last().map(Bez::rnode));
        nodes.extend(spans.iter().take(spans.len().saturating_sub(1)).map(Bez::rnode));
    } else {
        nodes.extend(spans.first().map(|s| RNode::new(Node::new(s.start(), NodeKind::Line), Vec::new())));
        nodes.extend(spans.iter().map(Bez::rnode));
    }
    let mut ring = Ring { closed, nodes };
    let smooth: Vec<bool> = (0..ring.len()).map(|j| ring.looks_smooth(j, SMOOTH_MAX_DEGREES).unwrap_or(false)).collect();
    for (r, s) in ring.nodes.iter_mut().zip(smooth) {
        r.node.smooth = s;
    }
    ring
}

// ── Compatible sides ────────────────────────────────────────────────────────────────────────────

/// Side `k`'s offset of `ring` with the same nodes: each node where the offset lines of its two
/// segments meet, each handle refitted.
fn compatible_ring(ring: &Ring, pen: Vec2, k: f64, keep_tags: bool) -> Ring {
    let moved: Vec<Point> = (0..ring.len()).map(|j| moved_node(ring, j, pen, k)).collect();
    let tag = |n: &Node| Node { tag: if keep_tags { n.tag } else { None }, ..*n };
    let nodes = ring
        .nodes
        .iter()
        .enumerate()
        .map(|(j, r)| {
            let to = moved.get(j).copied().unwrap_or(r.node.point);
            let from = ring.prev(j).and_then(|i| moved.get(i).copied());
            let ctrl = match (ring.seg(j), from) {
                (Some(seg), Some(from)) if seg.is_curve() => moved_handles(&seg, from, to, pen, k),
                _ => r.ctrl.clone(),
            };
            RNode { node: tag(&Node { point: to, ..r.node }), ctrl: ctrl.iter().map(&tag).collect(), src: r.src, mark: false }
        })
        .collect();
    Ring { closed: ring.closed, nodes }
}

fn moved_node(ring: &Ring, j: usize, pen: Vec2, k: f64) -> Point {
    let Some(p) = ring.nodes.get(j).map(|r| r.node.point) else { return Point::ZERO };
    let tin = ring.tangent_in(j);
    let tout = ring.tangent_out(j);
    match (tin, tout) {
        (Some(a), Some(b)) => {
            let (wa, wb) = (reach(a, pen, k), reach(b, pen, k));
            let (ae, bs) = (p + wa, p + wb);
            if ae.distance(bs) <= JOIN_EPSILON {
                return ae.midpoint(bs);
            }
            let limit = MITER_LIMIT * wa.hypot().max(wb.hypot());
            match intersect(ae, a, bs, b) {
                Some((m, _, _)) if m.distance(p) <= limit => m,
                Some((m, _, _)) => p + (m - p).normalize() * limit,
                None => ae.midpoint(bs),
            }
        }
        (Some(t), None) | (None, Some(t)) => p + reach(t, pen, k),
        (None, None) => p,
    }
}

/// The handles of `seg` for its new ends `from` and `to`.
fn moved_handles(seg: &Seg, from: Point, to: Point, pen: Vec2, k: f64) -> Vec<Node> {
    if let (NodeKind::Curve, [c1, c2]) = (seg.kind, seg.ctrl) {
        let c = CubicBez::new(seg.from, c1.point, c2.point, seg.to);
        let retracted = [c1.point.distance(seg.from) <= JOIN_EPSILON, c2.point.distance(seg.to) <= JOIN_EPSILON];
        let f = fit(&c, from, to, pen, k, retracted);
        return vec![Node { point: f.p1, ..*c1 }, Node { point: f.p2, ..*c2 }];
    }
    // A quadratic spline: each off-curve point goes where the offset lines of its two legs of the
    // control polygon meet; the first and last legs pass through the moved ends.
    let points: Vec<Point> = std::iter::once(seg.from).chain(seg.ctrl.iter().map(|n| n.point)).chain(std::iter::once(seg.to)).collect();
    let last = seg.ctrl.len().saturating_sub(1);
    let leg = |a: Option<&Point>, b: Option<&Point>, anchor: Option<Point>| -> Option<(Point, Vec2, Vec2)> {
        let (a, b) = (*a?, *b?);
        let d = unit(b - a)?;
        let w = reach(d, pen, k);
        Some((anchor.unwrap_or(a + w), d, w))
    };
    seg.ctrl
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let before = leg(points.get(i), points.get(i + 1), (i == 0).then_some(from));
            let after = leg(points.get(i + 1), points.get(i + 2), (i == last).then_some(to));
            let here = n.point;
            let point = match (before, after) {
                (Some((p, a, wa)), Some((q, b, wb))) => {
                    let limit = MITER_LIMIT * wa.hypot().max(wb.hypot());
                    match intersect(p, a, q, b) {
                        Some((m, _, _)) if m.distance(here) <= limit => m,
                        _ => here + (wa + wb) * 0.5,
                    }
                }
                (Some((_, _, w)), None) | (None, Some((_, _, w))) => here + w,
                (None, None) => here,
            };
            Node { point, ..*n }
        })
        .collect()
}

// ── Open strokes and caps ───────────────────────────────────────────────────────────────────────

/// The ends of an open path and its directions there (both along the path).
struct Ends {
    start: Point,
    start_dir: Option<Vec2>,
    end: Point,
    end_dir: Option<Vec2>,
}

/// The stroke outline of an open path from its two sides: the left side forward, the end cap,
/// the right side backward, the start cap.
fn open_stroke(mut left: Ring, mut right: Ring, ends: &Ends, cap: CapStyle) -> Option<Ring> {
    let start_out = ends.start_dir.map(|t| -t);
    for (at_end, out, end) in [(true, ends.end_dir, ends.end), (false, start_out, ends.start)] {
        let Some(t) = out else { continue };
        let (Some(l), Some(r)) = (end_point(&left, at_end), end_point(&right, at_end)) else { continue };
        for (ring, p) in [(&mut left, l), (&mut right, r)] {
            let shift = match cap {
                CapStyle::RoundInset => -(l.distance(r) / 2.0),
                CapStyle::Aligned if t.x.abs() >= t.y.abs() => (end.x - p.x) / t.x,
                CapStyle::Aligned => (end.y - p.y) / t.y,
                _ => 0.0,
            };
            if shift.is_finite() && shift != 0.0 {
                shift_end(ring, at_end, t * shift);
            }
        }
    }
    let back = right.reversed();
    let (l0, l1, r1, r0) = (left.nodes.first()?, left.nodes.last()?, back.nodes.first()?, back.nodes.last()?);
    let (start_mid, start_kind, start_ctrl) = cap_nodes(r0.node.point, l0.node.point, start_out, cap);
    let (end_mid, end_kind, end_ctrl) = cap_nodes(l1.node.point, r1.node.point, ends.end_dir, cap);
    let mut nodes = Vec::with_capacity(left.len() + back.len() + 4);
    nodes.push(RNode { node: Node { kind: start_kind, ..l0.node }, ctrl: start_ctrl, src: None, mark: false });
    nodes.extend(left.nodes.iter().skip(1).cloned());
    nodes.extend(end_mid);
    nodes.push(RNode { node: Node { kind: end_kind, ..r1.node }, ctrl: end_ctrl, src: None, mark: false });
    nodes.extend(back.nodes.iter().skip(1).cloned());
    nodes.extend(start_mid);
    Some(Ring { closed: true, nodes })
}

fn end_point(ring: &Ring, at_end: bool) -> Option<Point> {
    let r = if at_end { ring.nodes.last() } else { ring.nodes.first() };
    r.map(|r| r.node.point)
}

/// Moves the first or last node of an open ring with the handle next to it.
fn shift_end(ring: &mut Ring, at_end: bool, delta: Vec2) {
    if at_end {
        if let Some(r) = ring.nodes.last_mut() {
            r.node.point += delta;
            if let Some(h) = r.ctrl.last_mut() {
                h.point += delta;
            }
        }
    } else {
        if let Some(r) = ring.nodes.first_mut() {
            r.node.point += delta;
        }
        if let Some(h) = ring.nodes.get_mut(1).and_then(|r| r.ctrl.first_mut()) {
            h.point += delta;
        }
    }
}

/// A cap from `a` to `b`, bulging along `t`: the nodes between them, and the kind and handles of
/// the segment that ends at `b`.
fn cap_nodes(a: Point, b: Point, t: Option<Vec2>, cap: CapStyle) -> (Vec<RNode>, NodeKind, Vec<Node>) {
    let half = a.distance(b) / 2.0;
    let Some(t) = t.filter(|_| half > JOIN_EPSILON) else { return (Vec::new(), NodeKind::Line, Vec::new()) };
    let ext = t * half;
    match cap {
        CapStyle::Square => {
            let line = |p: Point| RNode::new(Node::new(p, NodeKind::Line), Vec::new());
            (vec![line(a + ext), line(b + ext)], NodeKind::Line, Vec::new())
        }
        CapStyle::Round | CapStyle::RoundInset => {
            // Two quarter ellipses with conjugate radii `a - c` and `ext`.
            let c = a.midpoint(b);
            let r = a - c;
            let top = c + ext;
            let mid =
                RNode::new(Node { smooth: true, ..Node::new(top, NodeKind::Curve) }, vec![Node::off(a + ext * KAPPA), Node::off(top + r * KAPPA)]);
            (vec![mid], NodeKind::Curve, vec![Node::off(top - r * KAPPA), Node::off(b + ext * KAPPA)])
        }
        CapStyle::Flat | CapStyle::Aligned => (Vec::new(), NodeKind::Line, Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    use crate::filters::FilterString;
    use crate::fixtures::rect;
    use fontcraft_geom::Rect;

    fn options(h: f64, v: f64) -> OffsetOptions {
        OffsetOptions { horizontal: h, vertical: v, stroke: false, auto: false, position: 50.0, compatible: false, cap: CapStyle::Flat }
    }

    fn stroked(h: f64, cap: CapStyle) -> OffsetOptions {
        OffsetOptions { stroke: true, cap, ..options(h, h) }
    }

    fn poly(closed: bool, points: &[(f64, f64)]) -> Contour {
        Contour::new(closed, points.iter().map(|&(x, y)| Node::new(Point::new(x, y), NodeKind::Line)).collect())
    }

    /// A circle of four cubics, counter-clockwise.
    fn circle(r: f64) -> Contour {
        let k = r * KAPPA;
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

    /// Signed area: outer contours count positive, counters negative.
    fn area(cs: &[Contour]) -> f64 {
        cs.iter().map(|c| Ring::from_contour(c).unwrap().area()).sum::<f64>().abs()
    }

    fn bounds(cs: &[Contour]) -> Rect {
        cs.iter().filter_map(|c| Ring::from_contour(c).unwrap().bounds()).reduce(|a, b| a.union(b)).unwrap()
    }

    fn near(r: Rect, x0: f64, y0: f64, x1: f64, y1: f64) -> bool {
        [r.x0 - x0, r.y0 - y0, r.x1 - x1, r.y1 - y1].iter().all(|d| d.abs() < 1e-3)
    }

    fn on_curve(c: &Contour) -> Vec<(f64, f64)> {
        let mut v: Vec<(f64, f64)> = c.nodes.iter().filter(|n| n.on_curve()).map(|n| (n.point.x, n.point.y)).collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v
    }

    #[test]
    fn a_square_grows_and_shrinks_with_sharp_corners() {
        let out = offset_curve(&[rect(0.0, 0.0, 100.0, 100.0)], &options(10.0, 10.0), 1.0).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(on_curve(&out[0]), vec![(-10.0, -10.0), (-10.0, 110.0), (110.0, -10.0), (110.0, 110.0)]);
        let out = offset_curve(&[rect(0.0, 0.0, 100.0, 100.0)], &options(-10.0, -10.0), 1.0).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(on_curve(&out[0]), vec![(10.0, 10.0), (10.0, 90.0), (90.0, 10.0), (90.0, 90.0)]);
        // A stem thinner than twice the offset disappears.
        assert!(offset_curve(&[rect(0.0, 0.0, 15.0, 100.0)], &options(-10.0, -10.0), 1.0).unwrap().is_empty());
    }

    #[test]
    fn a_counter_shrinks_when_the_outline_grows() {
        let outer = rect(0.0, 0.0, 300.0, 300.0);
        let counter = rect(100.0, 100.0, 200.0, 200.0).reversed().unwrap();
        let out = offset_curve(&[outer, counter], &options(10.0, 10.0), 1.0).unwrap();
        assert_eq!(out.len(), 2);
        assert!((area(&out) - (320.0 * 320.0 - 80.0 * 80.0)).abs() < 1e-6, "{}", area(&out));
    }

    #[test]
    fn the_two_distances_are_separate_and_may_have_opposite_signs() {
        let square = [rect(0.0, 0.0, 100.0, 100.0)];
        assert!(near(bounds(&offset_curve(&square, &options(20.0, 0.0), 0.0).unwrap()), -20.0, 0.0, 120.0, 100.0));
        assert!(near(bounds(&offset_curve(&square, &options(10.0, -10.0), 0.0).unwrap()), -10.0, 10.0, 110.0, 90.0));
        let compatible = OffsetOptions { compatible: true, ..options(10.0, -10.0) };
        assert_eq!(on_curve(&offset_curve(&square, &compatible, 0.0).unwrap()[0]), vec![(-10.0, 10.0), (-10.0, 90.0), (110.0, 10.0), (110.0, 90.0)]);
    }

    #[test]
    fn a_circle_stays_a_circle() {
        for (d, r) in [(10.0, 110.0), (-10.0, 90.0)] {
            let out = offset_curve(&[circle(100.0)], &options(d, d), 0.0).unwrap();
            let expected = std::f64::consts::PI * r * r;
            assert!((area(&out) - expected).abs() < expected * 0.002, "{d}: {} against {expected}", area(&out));
            for n in out.iter().flat_map(|c| &c.nodes).filter(|n| n.on_curve()) {
                assert!((n.point.distance(Point::ZERO) - r).abs() < 0.5, "{d}: {:?}", n.point);
            }
        }
        // Keep Compatible: the same twelve nodes, each node exactly 10 units out.
        let compatible = OffsetOptions { compatible: true, ..options(10.0, 10.0) };
        let out = offset_curve(&[circle(100.0)], &compatible, 0.0).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].nodes.len(), 12);
        assert!(out[0].nodes.iter().zip(&circle(100.0).nodes).all(|(a, b)| a.kind == b.kind && a.smooth == b.smooth));
        for n in out[0].nodes.iter().filter(|n| n.on_curve()) {
            assert!((n.point.distance(Point::ZERO) - 110.0).abs() < 1e-9, "{:?}", n.point);
        }
        let expected = std::f64::consts::PI * 110.0 * 110.0;
        assert!((area(&out) - expected).abs() < expected * 0.002, "{}", area(&out));
    }

    #[test]
    fn keep_compatible_moves_every_node_in_place() {
        // An L, counter-clockwise, with an inward corner at (50, 50).
        let l = poly(true, &[(0.0, 0.0), (200.0, 0.0), (200.0, 50.0), (50.0, 50.0), (50.0, 200.0), (0.0, 200.0)]).tagged();
        let compatible = OffsetOptions { compatible: true, ..options(10.0, 10.0) };
        let out = offset_curve(std::slice::from_ref(&l), &compatible, 1.0).unwrap();
        let points: Vec<(f64, f64)> = out[0].nodes.iter().map(|n| (n.point.x, n.point.y)).collect();
        assert_eq!(points, vec![(-10.0, -10.0), (210.0, -10.0), (210.0, 60.0), (60.0, 60.0), (60.0, 210.0), (-10.0, 210.0)]);
        assert!(out[0].nodes.iter().enumerate().all(|(i, n)| n.tag == Some(i)));
        // Drawn the other way round it grows the same way.
        let reversed = offset_curve(&[l.reversed().unwrap()], &compatible, 1.0).unwrap();
        assert_eq!(on_curve(&reversed[0]), on_curve(&out[0]));
        // The exact form gives the same outline here.
        let exact = offset_curve(std::slice::from_ref(&l), &options(10.0, 10.0), 1.0).unwrap();
        assert_eq!(on_curve(&exact[0]), on_curve(&out[0]));
    }

    #[test]
    fn an_open_line_strokes_with_each_cap() {
        let line = [poly(false, &[(0.0, 0.0), (100.0, 0.0)])];
        // Two half discs of radius 10.
        let disc = std::f64::consts::PI * 100.0;
        for (cap, b, a) in [
            (CapStyle::Flat, [0.0, -10.0, 100.0, 10.0], 2000.0),
            (CapStyle::Square, [-10.0, -10.0, 110.0, 10.0], 2400.0),
            (CapStyle::Round, [-10.0, -10.0, 110.0, 10.0], 2000.0 + disc),
            (CapStyle::RoundInset, [0.0, -10.0, 100.0, 10.0], 1600.0 + disc),
            (CapStyle::Aligned, [0.0, -10.0, 100.0, 10.0], 2000.0),
        ] {
            for compatible in [false, true] {
                let o = OffsetOptions { compatible, ..stroked(10.0, cap) };
                let out = offset_curve(&line, &o, 0.0).unwrap();
                assert_eq!(out.len(), 1, "{cap:?}");
                assert!(out[0].closed);
                assert!(near(bounds(&out), b[0], b[1], b[2], b[3]), "{cap:?} {compatible}: {:?}", bounds(&out));
                assert!((area(&out) - a).abs() < 1.0, "{cap:?} {compatible}: {}", area(&out));
            }
        }
        // Aligned caps of a diagonal end on verticals through the ends.
        let diagonal = [poly(false, &[(0.0, 0.0), (100.0, 100.0)])];
        let out = offset_curve(&diagonal, &stroked(10.0, CapStyle::Aligned), 0.0).unwrap();
        let b = bounds(&out);
        assert!(b.x0.abs() < 1e-6 && (b.x1 - 100.0).abs() < 1e-6, "{b:?}");
    }

    #[test]
    fn position_puts_the_stroke_left_or_right_of_the_path() {
        let line = [poly(false, &[(0.0, 0.0), (100.0, 0.0)])];
        for (position, y0, y1) in [(0.0, 0.0, 20.0), (100.0, -20.0, 0.0), (25.0, -5.0, 15.0)] {
            let out = offset_curve(&line, &OffsetOptions { position, ..stroked(10.0, CapStyle::Flat) }, 0.0).unwrap();
            assert!(near(bounds(&out), 0.0, y0, 100.0, y1), "{position}: {:?}", bounds(&out));
        }
    }

    #[test]
    fn a_closed_path_strokes_into_a_frame() {
        let square = [rect(0.0, 0.0, 100.0, 100.0)];
        for compatible in [false, true] {
            let out = offset_curve(&square, &OffsetOptions { compatible, ..stroked(10.0, CapStyle::Flat) }, 1.0).unwrap();
            assert_eq!(out.len(), 2, "{compatible}");
            let signed: f64 = out.iter().map(|c| Ring::from_contour(c).unwrap().area()).sum();
            assert!((signed - 8000.0).abs() < 1e-6, "{compatible}: {signed}");
            assert!(near(bounds(&out), -10.0, -10.0, 110.0, 110.0));
        }
    }

    #[test]
    fn auto_stroke_keeps_the_height() {
        let square = [rect(0.0, 0.0, 100.0, 100.0)];
        let auto = OffsetOptions { auto: true, ..stroked(10.0, CapStyle::Flat) };
        let out = offset_curve(&square, &auto, 1.0).unwrap();
        assert!(near(bounds(&out), -10.0, 0.0, 110.0, 100.0), "{:?}", bounds(&out));
    }

    #[test]
    fn an_open_path_moves_to_its_right() {
        let line = [poly(false, &[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)])];
        let out = offset_curve(&line, &options(10.0, 10.0), 0.0).unwrap();
        assert!(!out[0].closed);
        let points: Vec<(f64, f64)> = out[0].nodes.iter().map(|n| (n.point.x, n.point.y)).collect();
        assert_eq!(points, vec![(0.0, -10.0), (110.0, -10.0), (110.0, 100.0)]);
    }

    #[test]
    fn the_export_string_holds_the_position_as_a_fraction_or_auto() {
        let f = OffsetCurve;
        let read = |s: &str| f.read_items(&FilterString::parse(s).unwrap().items);
        let args = read("OffsetCurve; 10; 5; 1; 0.25").unwrap();
        assert_eq!((args.number("horizontal"), args.number("vertical"), args.number("position")), (10.0, 5.0, 25.0));
        assert!(args.flag("makeStroke") && !args.flag("autoStroke"));
        assert_eq!(FilterString::write(&f, &args, &[], &[]), "OffsetCurve;10;5;1;0.25;");
        let auto = read("OffsetCurve;10;10;0;AUTO").unwrap();
        assert!(auto.flag("autoStroke") && auto.number("position") == 50.0);
        assert_eq!(FilterString::write(&f, &auto, &[], &[]), "OffsetCurve;10;10;0;auto;");
        let short = read("OffsetCurve;-5;-5").unwrap();
        assert_eq!(FilterString::write(&f, &short, &[], &[]), "OffsetCurve;-5;-5;0;0.5;");
        let odd =
            Args::new(PARAMS, [("horizontal", Input::Number(1.0)), ("vertical", Input::Number(1.0)), ("position", Input::Number(33.3))]).unwrap();
        let s = FilterString::write(&f, &odd, &[], &[]);
        assert_eq!(s, "OffsetCurve;1;1;0;0.333;");
        assert_eq!(read(&s).unwrap(), odd);
        for bad in [
            "OffsetCurve",
            "OffsetCurve;10",
            "OffsetCurve;10;10;1;1.5",
            "OffsetCurve;10;10;1;-0.1",
            "OffsetCurve;10;10;1;NaN",
            "OffsetCurve;10;10;1;0.5;9",
            "OffsetCurve;10;10;2",
            "OffsetCurve;NaN;10",
            "OffsetCurve;10;1e9",
            "OffsetCurve;10;10;Position:0.5",
        ] {
            assert!(read(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn hostile_settings_and_outlines_are_errors() {
        let square = [rect(0.0, 0.0, 100.0, 100.0)];
        for o in [
            options(f64::NAN, 1.0),
            options(1.0, f64::INFINITY),
            options(MAX_OFFSET * 2.0, 1.0),
            OffsetOptions { position: 101.0, ..stroked(10.0, CapStyle::Flat) },
            OffsetOptions { position: f64::NAN, ..stroked(10.0, CapStyle::Flat) },
        ] {
            assert!(offset_curve(&square, &o, 1.0).is_err(), "{o:?}");
        }
        let nan = poly(true, &[(0.0, 0.0), (f64::NAN, 0.0), (10.0, 10.0)]);
        assert!(offset_curve(&[nan], &options(10.0, 10.0), 1.0).is_err());
        // Near the coordinate limit the result would leave it.
        let edge = [rect(crate::MAX_COORDINATE - 10.0, 0.0, crate::MAX_COORDINATE, 10.0)];
        assert!(offset_curve(&edge, &OffsetOptions { compatible: true, ..options(100.0, 100.0) }, 1.0).is_err());
        // Zero distances change nothing.
        assert_eq!(offset_curve(&square, &options(0.0, 0.0), 1.0).unwrap(), square.to_vec());
    }
}
