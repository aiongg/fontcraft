//! Auto-meter edits and stored measurements, on `fontcraft_pathops::stems`. Rows:
//! `autometer.editStemNumerically`, `font.recalculateMeasurements`, and the engine queries behind
//! `view.showStemWidths`, `view.showCounterWidths`, `view.showCornerAngles` and
//! `view.showQuickMeasurement`.
//!
//! | Command | What it does |
//! |---|---|
//! | `autometer.measure` | the stems of a layer side (components included) with their widths and how they compare with the master's stem values, the counters between neighbouring stems, the angles at sharp nodes |
//! | `autometer.quickMeasure` | the distance across the outline at a point: from the nearest segment through the point to the segment opposite, or along one axis |
//! | `autometer.setStemWidth` | moves the nodes of one measured stem so it gets a new width |
//! | `font.recalculateMeasurements` | measures heights, overshoots and the first two stem values of each direction from the outlines of each master and writes them into the master |
//!
//! The measurements table of the Palette is the master's own metrics and stem values: the same
//! values the hinting and the stem suggestions read.

use std::collections::HashMap;
use std::sync::Arc;

use fontcraft_geom::{BezPath, Line, ParamCurve, ParamCurveNearest, PathSeg, Point, Shape as _, Vec2};
use fontcraft_model::{Font, LayerParts, MAX_COORDINATE, MAX_OUTLINE_ELEMENTS, Master, MetricValue, Node, Shape, metric};
use fontcraft_pathops::stem_filters::{MAX_FILTER_STEM, MIN_STEM_WIDTH};
use fontcraft_pathops::stems::{Stem, StemDirection, StemOptions, detect_stems};
use serde_json::{Map, Value, json};

use super::common::{grid, in_range};
use super::*;
use crate::params::{Params, TargetKind, Targets, kind};

/// Narrowest and widest stem the auto-meter labels by default, in font units.
pub const DEFAULT_MIN_WIDTH: f64 = 20.0;
pub const DEFAULT_MAX_WIDTH: f64 = 250.0;
/// How far, in font units, a stem's width may be from a stem value and still count as a near
/// miss of it (an uncommon stem) by default.
pub const DEFAULT_TOLERANCE: f64 = 5.0;
/// Widest tolerance a caller may ask for.
pub const MAX_TOLERANCE: f64 = 100.0;
/// A width this close to a stem value, in font units, is that value.
const SAME_WIDTH: f64 = 0.5;
/// How far from a stem's edge line, in font units, a node may lie and still carry the edge.
const ON_EDGE: f64 = 1.0;
/// A node at this angle or flatter, in degrees, is not a corner.
const STRAIGHT_ANGLE: f64 = 179.0;
/// Most corners one measurement reports.
pub const MAX_CORNERS: usize = 10_000;
/// Most stems of one direction paired into counters.
const MAX_COUNTER_STEMS: usize = 512;
/// Places along the overlap of two stems where a counter is tried, middle first.
const COUNTER_SAMPLES: [f64; 9] = [0.5, 0.4, 0.6, 0.3, 0.7, 0.2, 0.8, 0.1, 0.9];
/// How far a quick measurement looks for the opposite segment, in font units.
const QUICK_REACH: f64 = 10_000.0;
/// Heights on which a horizontal stem's edge stays when the stem's width changes.
const FIXED_HEIGHT_KINDS: [&str; 5] = [metric::BASELINE, metric::X_HEIGHT, metric::CAP_HEIGHT, metric::ASCENDER, metric::DESCENDER];
/// A horizontal edge this close to a fixed height, in font units, is on it.
const ON_HEIGHT: f64 = 1.0;

/// The fields `font.recalculateMeasurements` writes.
pub const FIELDS: [&str; 9] =
    ["xHeight", "capHeight", "ascender", "descender", "xOvershoot", "capOvershoot", "baselineOvershoot", "verticalStems", "horizontalStems"];

const LAYER_TARGETS: &[TargetKind] = &[TargetKind::Layer];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "autometer.measure",
            "Measure Stems and Corners",
            [],
            None,
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\", stems?: bool = true, counters?: bool = true, corners?: bool = true, minWidth?: units = 20, maxWidth?: units = 250 (0 to 2000), tolerance?: units = 5 (0 to 100)} measure the layer side's outline, components included: its stems from `minWidth` to `maxWidth` wide, the white space between neighbouring vertical (or horizontal) stems, and the angle at each sharp node of its paths. A stem is `common` when its width is one of the master's stem values of its direction (diagonal stems compare with the vertical values), `uncommon` when it is within `tolerance` of one (`nearest` names it), else `other`; it is `editable` when nodes of the side's paths carry both its edges → {glyph, layer, stems: [{index, direction: \"vertical\" | \"horizontal\" | \"diagonal\", width, round, first: [[x, y], [x, y]], second: [[x, y], [x, y]], class: \"common\" | \"uncommon\" | \"other\", nearest?: number, editable}], counters: [{direction, width, from: [x, y], to: [x, y]}], corners: [{shape, node, x, y, angle: degrees between the two segments, 0 to 180}]}",
            has_doc,
            measure
        )
        .targets(LAYER_TARGETS),
        cmd!(
            query "autometer.quickMeasure",
            "Quick Measurement",
            [],
            None,
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\", at: [x, y], axis?: \"horizontal\" | \"vertical\"} the distance across the layer side's outline (components included) at a point: without `axis`, from the nearest point of the outline through `at` to the next segment beyond; with `axis`, between the nearest crossings of the outline on both sides of `at` along that axis. Refused when nothing lies on the other side → {from: [x, y], to: [x, y], distance, dx, dy}",
            has_doc,
            quick_measure
        )
        .targets(LAYER_TARGETS),
        cmd!(
            "autometer.setStemWidth",
            "Set Stem Width",
            [],
            None,
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\", stem: index (in `autometer.measure`'s stems with the same minWidth and maxWidth), width?: units | by?: units | snap?: true, minWidth?: units = 20, maxWidth?: units = 250} give one stem a new width: `width` sets it (at least 1), `by` adds to it (within ±2000), `snap` sets it to the nearest of the master's stem values of its direction. The nodes on both edges move apart or together by half the change each, their handles with them; a horizontal stem's edge on the baseline, x-height, cap height, ascender or descender stays and the other edge takes the whole change. Moved points land on the font's grid. Refused when the stem's edges are not carried by nodes of the side's paths (a component's stem, a round stem without a node at its extreme) → {glyph, layer, stem, was, width, moved: number of nodes moved}",
            has_doc,
            set_stem_width
        )
        .targets(LAYER_TARGETS),
        cmd!(
            "font.recalculateMeasurements",
            "Recalculate Measurements",
            ["Font"],
            None,
            "{masters?: [id | name] (default: every master), fields?: [\"xHeight\" | \"capHeight\" | \"ascender\" | \"descender\" | \"xOvershoot\" | \"capOvershoot\" | \"baselineOvershoot\" | \"verticalStems\" | \"horizontalStems\"] (default: all)} measure design values from each master's outlines and write them into the master: the x-height from the top of x (else z, v, w), the cap height from H (else I, E, T), the ascender from d (else h, l, k, b), the descender from the bottom of p (else q); the overshoots (the metrics' zones) from o (x-height and baseline) and O (cap height); the first vertical and horizontal stem values from the most frequent straight stem width in n, m, h, l, i, u (vertical) and e, t, f, z (horizontal), the second ones from H, I, N, L, E (vertical) and H, E, F, T, L, Z (horizontal). A field whose glyphs are empty, or a stem value the font defines no stem for, is skipped and listed. All masters change in one undo step; refused when nothing could be measured → {masters: [{master, values: {field: number}}], skipped: [{master, field, reason}]}",
            has_doc,
            recalculate
        ),
    ]
}

// ── Shared ──────────────────────────────────────────────────────────────────────────────────────

/// The outline of a layer side: closed and open paths as they are, components resolved. Paths
/// that do not make a valid outline and components of missing glyphs are left out.
fn side_outline(font: &Font, parts: &LayerParts, master: &str) -> std::result::Result<BezPath, String> {
    let mut out = BezPath::new();
    let mut budget = MAX_OUTLINE_ELEMENTS;
    for shape in parts.shapes {
        let part = match shape {
            Shape::Path(path) => match path.to_bez() {
                Ok(b) => b,
                Err(_) => continue,
            },
            Shape::Component(c) => {
                if font.glyph(&c.glyph).is_none() || c.transform.numbers().iter().any(|v| !v.is_finite()) {
                    continue;
                }
                font.component_outline(c, master).map_err(|e| e.to_string())?
            }
            Shape::Image(_) | Shape::Raw(_) => continue,
        };
        budget = budget.checked_sub(part.elements().len()).ok_or("the outline is too large to measure")?;
        out.extend(part);
    }
    Ok(out)
}

/// What [`measure_parts`] reports, and the stems it reports.
#[derive(Clone, Debug, PartialEq)]
pub struct MeterOptions {
    pub stems: bool,
    pub counters: bool,
    pub corners: bool,
    /// Narrowest and widest stem, in font units (0 to `MAX_FILTER_STEM`, in order).
    pub min_width: f64,
    pub max_width: f64,
    /// How far from a stem value a near miss may be, in font units.
    pub tolerance: f64,
}

impl Default for MeterOptions {
    fn default() -> Self {
        Self { stems: true, counters: true, corners: true, min_width: DEFAULT_MIN_WIDTH, max_width: DEFAULT_MAX_WIDTH, tolerance: DEFAULT_TOLERANCE }
    }
}

/// `minWidth` and `maxWidth`, checked.
fn width_range(p: &Params) -> Result<(f64, f64)> {
    let get = |key: &str, default: f64| -> Result<f64> {
        let v = p.opt_f64(key)?.unwrap_or(default);
        if !(0.0..=MAX_FILTER_STEM).contains(&v) {
            return Err(p.bad(format!("`{key}` must be from 0 to {MAX_FILTER_STEM}, got {v}")));
        }
        Ok(v)
    };
    let (lo, hi) = (get("minWidth", DEFAULT_MIN_WIDTH)?, get("maxWidth", DEFAULT_MAX_WIDTH)?);
    if lo > hi {
        return Err(p.bad(format!("`minWidth` ({lo}) is more than `maxWidth` ({hi})")));
    }
    Ok((lo, hi))
}

fn stems_of(outline: &BezPath, (lo, hi): (f64, f64)) -> Vec<Stem> {
    detect_stems(outline, &StemOptions { min_width: lo, max_width: hi, ..StemOptions::default() })
}

fn direction_name(d: StemDirection) -> &'static str {
    match d {
        StemDirection::Vertical => "vertical",
        StemDirection::Horizontal => "horizontal",
        StemDirection::Diagonal => "diagonal",
    }
}

/// The master's stem values of a direction (diagonal stems use the vertical ones); unset (zero)
/// values left out.
fn common_widths(font: &Font, master: &Master, d: StemDirection) -> Vec<f64> {
    let horizontal = d == StemDirection::Horizontal;
    font.stems
        .iter()
        .zip(master.stem_values.iter())
        .filter(|(def, v)| def.horizontal == horizontal && v.is_finite() && **v > 0.0)
        .map(|(_, v)| *v)
        .collect()
}

/// The stem value nearest to `width`, if any.
fn nearest_common(commons: &[f64], width: f64) -> Option<f64> {
    commons.iter().copied().min_by(|a, b| (a - width).abs().total_cmp(&(b - width).abs()))
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn pt(p: Point) -> Value {
    json!([round2(p.x), round2(p.y)])
}

fn line_json(l: &Line) -> Value {
    json!([pt(l.p0), pt(l.p1)])
}

/// The unit vector from a stem's first edge towards its second, at right angles to the stem.
fn across(stem: &Stem) -> Vec2 {
    match stem.direction {
        StemDirection::Vertical => Vec2::new(1.0, 0.0),
        StemDirection::Horizontal => Vec2::new(0.0, 1.0),
        StemDirection::Diagonal => {
            let u = stem.first.p1 - stem.first.p0;
            let len = u.hypot();
            if len <= 0.0 {
                return Vec2::new(1.0, 0.0);
            }
            let n = Vec2::new(-u.y, u.x) / len;
            if (stem.second.midpoint() - stem.first.midpoint()).dot(n) < 0.0 { -n } else { n }
        }
    }
}

/// The on-curve nodes `(shape, node)` of the side's paths that carry one edge of a stem: for a
/// straight edge, both ends of every straight segment of the paths that lies on the edge's line
/// and overlaps it; for a round stem's edge (a point), the nodes at that point.
fn edge_nodes(parts: &LayerParts, edge: &Line) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let d = edge.p1 - edge.p0;
    let len = d.hypot();
    for (si, shape) in parts.shapes.iter().enumerate() {
        let Shape::Path(path) = shape else { continue };
        if len < ON_EDGE {
            for (ni, node) in path.nodes.iter().enumerate() {
                if node.on_curve() && node.point().distance(edge.p0) <= ON_EDGE {
                    out.push((si, ni));
                }
            }
            continue;
        }
        let u = d / len;
        let n = Vec2::new(-u.y, u.x);
        let off = |q: Point| (q - edge.p0).dot(n).abs();
        let along = |q: Point| (q - edge.p0).dot(u);
        let on: Vec<usize> = (0..path.nodes.len()).filter(|&i| path.nodes.get(i).is_some_and(Node::on_curve)).collect();
        let count = on.len();
        let pairs = if path.closed { count } else { count.saturating_sub(1) };
        for k in 0..pairs {
            let (Some(&a), Some(&b)) = (on.get(k), on.get((k + 1) % count.max(1))) else { continue };
            let (Some(na), Some(nb)) = (path.nodes.get(a), path.nodes.get(b)) else { continue };
            let (pa, pb) = (na.point(), nb.point());
            if off(pa) > ON_EDGE || off(pb) > ON_EDGE {
                continue;
            }
            // The handles between the two nodes lie on the line too: the segment is straight.
            let total = path.nodes.len();
            let mut i = (a + 1) % total.max(1);
            let mut straight = true;
            let mut steps = 0;
            while i != b && steps < total {
                if path.nodes.get(i).is_some_and(|h| off(h.point()) > ON_EDGE) {
                    straight = false;
                    break;
                }
                i = (i + 1) % total.max(1);
                steps += 1;
            }
            let (s0, s1) = (along(pa).min(along(pb)), along(pa).max(along(pb)));
            if straight && s1 > 1e-6 && s0 < len - 1e-6 {
                for node in [a, b] {
                    if !out.contains(&(si, node)) {
                        out.push((si, node));
                    }
                }
            }
        }
    }
    out
}

/// The angle between the segments meeting at an on-curve node, in degrees; `None` at the ends of
/// an open path, where a neighbour coincides all the way round, or for a smooth node.
fn corner_angle(nodes: &[Node], closed: bool, i: usize) -> Option<f64> {
    let node = nodes.get(i)?;
    if !node.on_curve() || node.smooth {
        return None;
    }
    let n = nodes.len();
    let at = node.point();
    let step = |forward: bool| -> Option<Vec2> {
        let mut j = i;
        for _ in 1..n {
            j = if forward {
                if j + 1 == n {
                    if !closed {
                        return None;
                    }
                    0
                } else {
                    j + 1
                }
            } else if j == 0 {
                if !closed {
                    return None;
                }
                n - 1
            } else {
                j - 1
            };
            let v = nodes.get(j)?.point() - at;
            if v.hypot() > 1e-9 {
                return Some(v);
            }
        }
        None
    };
    let (a, b) = (step(false)?, step(true)?);
    let cos = (a.dot(b) / (a.hypot() * b.hypot())).clamp(-1.0, 1.0);
    let angle = cos.acos().to_degrees();
    angle.is_finite().then_some(angle)
}

/// The white space between neighbouring stems of one direction whose extents overlap, where it
/// is not ink.
fn counters(stems: &[&Stem], direction: StemDirection, outline: &BezPath) -> Vec<Value> {
    // Across: the coordinate the width is measured along; along: the other one.
    type Coord = fn(Point) -> f64;
    let (x, y): (Coord, Coord) = match direction {
        StemDirection::Vertical => (|p| p.x, |p| p.y),
        _ => (|p| p.y, |p| p.x),
    };
    let make = |a: f64, b: f64| match direction {
        StemDirection::Vertical => Point::new(a, b),
        _ => Point::new(b, a),
    };
    let extent = |s: &Stem| {
        let pts = [s.first.p0, s.first.p1, s.second.p0, s.second.p1];
        let lo = |f: fn(Point) -> f64| pts.iter().map(|p| f(*p)).fold(f64::INFINITY, f64::min);
        let hi = |f: fn(Point) -> f64| pts.iter().map(|p| f(*p)).fold(f64::NEG_INFINITY, f64::max);
        // A round stem was measured at one place: give it some extent along the stem.
        let pad = if s.round { s.width / 2.0 } else { 0.0 };
        (lo(x), hi(x), lo(y) - pad, hi(y) + pad)
    };
    let boxes: Vec<(f64, f64, f64, f64)> = stems.iter().take(MAX_COUNTER_STEMS).map(|s| extent(s)).collect();
    let mut out = Vec::new();
    for (i, a) in boxes.iter().enumerate() {
        // The nearest stem beyond this one whose extent along the stem overlaps it.
        let next = boxes
            .iter()
            .enumerate()
            .filter(|(j, b)| *j != i && b.0 > a.1 && b.2.max(a.2) < b.3.min(a.3))
            .min_by(|(_, b), (_, c)| b.0.total_cmp(&c.0));
        let Some((_, b)) = next else { continue };
        let (lo, hi) = (a.2.max(b.2), a.3.min(b.3));
        let width = b.0 - a.1;
        let place = COUNTER_SAMPLES.iter().map(|f| lo + (hi - lo) * f).find(|&at| outline.winding(make((a.1 + b.0) / 2.0, at)) == 0);
        if let Some(at) = place {
            out.push(json!({
                "direction": direction_name(direction),
                "width": round2(width),
                "from": pt(make(a.1, at)),
                "to": pt(make(b.0, at)),
            }));
        }
    }
    out
}

// ── autometer.measure ───────────────────────────────────────────────────────────────────────────

fn measure(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("autometer.measure", v)?;
    let t = Targets::parse(&p)?;
    if t.has_lists() {
        return Err(p.bad("takes no target lists: it measures the whole layer"));
    }
    let range = width_range(&p)?;
    let tolerance = p.opt_f64("tolerance")?.unwrap_or(DEFAULT_TOLERANCE);
    if !(0.0..=MAX_TOLERANCE).contains(&tolerance) {
        return Err(p.bad(format!("`tolerance` must be from 0 to {MAX_TOLERANCE}, got {tolerance}")));
    }
    let options = MeterOptions {
        stems: p.bool_or("stems", true)?,
        counters: p.bool_or("counters", true)?,
        corners: p.bool_or("corners", true)?,
        min_width: range.0,
        max_width: range.1,
        tolerance,
    };
    let font = s.font()?;
    let r = t.resolve(font)?;
    let mut out = measure_parts(font, r.master, &r.parts, &options).map_err(|e| p.bad(e))?;
    if let Some(o) = out.as_object_mut() {
        o.insert("glyph".into(), json!(r.glyph.name));
        o.insert("layer".into(), json!(r.layer.id));
    }
    Ok(out)
}

/// The auto-meter's measurements of one layer side (`parts`) of a layer on `master`: what
/// `autometer.measure` returns, without the glyph and layer. `options` must hold a width range in
/// order within `0..=MAX_FILTER_STEM` and a finite tolerance.
pub fn measure_parts(font: &Font, master: &Master, parts: &LayerParts, options: &MeterOptions) -> std::result::Result<Value, String> {
    let (want_stems, want_counters, want_corners) = (options.stems, options.counters, options.corners);
    let range = (options.min_width, options.max_width);
    let tolerance = options.tolerance;
    let outline = if want_stems || want_counters { side_outline(font, parts, &master.id)? } else { BezPath::new() };
    let found = if want_stems || want_counters { stems_of(&outline, range) } else { Vec::new() };
    let mut stems = Vec::new();
    if want_stems {
        for (index, stem) in found.iter().enumerate() {
            let commons = common_widths(font, master, stem.direction);
            let nearest = nearest_common(&commons, stem.width);
            let class = match nearest.map(|c| (c - stem.width).abs()) {
                Some(d) if d <= SAME_WIDTH => "common",
                Some(d) if d <= tolerance => "uncommon",
                _ => "other",
            };
            let editable = !edge_nodes(parts, &stem.first).is_empty() && !edge_nodes(parts, &stem.second).is_empty();
            let mut o = Map::new();
            o.insert("index".into(), json!(index));
            o.insert("direction".into(), json!(direction_name(stem.direction)));
            o.insert("width".into(), json!(round2(stem.width)));
            o.insert("round".into(), json!(stem.round));
            o.insert("first".into(), line_json(&stem.first));
            o.insert("second".into(), line_json(&stem.second));
            o.insert("class".into(), json!(class));
            if class == "uncommon"
                && let Some(c) = nearest
            {
                o.insert("nearest".into(), json!(round2(c)));
            }
            o.insert("editable".into(), json!(editable));
            stems.push(Value::Object(o));
        }
    }
    let mut counter_list = Vec::new();
    if want_counters {
        for d in [StemDirection::Vertical, StemDirection::Horizontal] {
            let of: Vec<&Stem> = found.iter().filter(|s| s.direction == d).collect();
            counter_list.extend(counters(&of, d, &outline));
        }
    }
    let mut corners = Vec::new();
    if want_corners {
        'shapes: for (si, shape) in parts.shapes.iter().enumerate() {
            let Shape::Path(path) = shape else { continue };
            for (ni, node) in path.nodes.iter().enumerate() {
                if corners.len() >= MAX_CORNERS {
                    break 'shapes;
                }
                if let Some(angle) = corner_angle(&path.nodes, path.closed, ni).filter(|a| *a < STRAIGHT_ANGLE) {
                    corners.push(json!({"shape": si, "node": ni, "x": round2(node.x), "y": round2(node.y), "angle": (angle * 10.0).round() / 10.0}));
                }
            }
        }
    }
    Ok(json!({"stems": stems, "counters": counter_list, "corners": corners}))
}

// ── autometer.quickMeasure ──────────────────────────────────────────────────────────────────────

/// Where the ray from `from` along the unit vector `dir` first meets the outline, beyond a
/// hair's breadth: its distance along the ray.
fn first_hit(outline: &BezPath, from: Point, dir: Vec2) -> Option<f64> {
    let ray = Line::new(from, from + dir * QUICK_REACH);
    let mut best: Option<f64> = None;
    for seg in outline.segments() {
        for hit in seg.intersect_line(ray) {
            let d = hit.line_t * QUICK_REACH;
            if d > 1e-6 && d.is_finite() && best.is_none_or(|b| d < b) {
                best = Some(d);
            }
        }
    }
    best
}

fn quick_measure(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("autometer.quickMeasure", v)?;
    let t = Targets::parse(&p)?;
    if t.has_lists() {
        return Err(p.bad("takes no target lists: it measures the whole layer"));
    }
    let at = {
        let bad = |found: &Value| p.bad(format!("`at` must be [x, y] with finite numbers within ±{MAX_COORDINATE}, got {}", kind(found)));
        let v = p.get("at").ok_or_else(|| p.bad("`at` is required"))?;
        let coord = |c: &Value| c.as_f64().filter(|n| n.is_finite() && n.abs() <= MAX_COORDINATE);
        match v.as_array().map(Vec::as_slice) {
            Some([x, y]) => match (coord(x), coord(y)) {
                (Some(x), Some(y)) => Point::new(x, y),
                _ => return Err(bad(v)),
            },
            _ => return Err(bad(v)),
        }
    };
    let axis = match p.opt_str("axis")? {
        None => None,
        Some("horizontal") => Some(Vec2::new(1.0, 0.0)),
        Some("vertical") => Some(Vec2::new(0.0, 1.0)),
        Some(other) => return Err(p.bad(format!("`axis` must be \"horizontal\" or \"vertical\", got \"{other}\""))),
    };
    let font = s.font()?;
    let r = t.resolve(font)?;
    quick_measure_parts(font, &r.layer.master, &r.parts, at, axis).map_err(|e| p.bad(e))
}

/// What `autometer.quickMeasure` returns for one layer side (`parts`) of a layer on the master
/// `master`: the measurement at `at`, along the unit vector `axis` when given.
pub fn quick_measure_parts(font: &Font, master: &str, parts: &LayerParts, at: Point, axis: Option<Vec2>) -> std::result::Result<Value, String> {
    if !(at.x.is_finite() && at.y.is_finite()) {
        return Err("`at` must be finite".into());
    }
    let outline = side_outline(font, parts, master)?;
    let nothing = || "nothing of the outline lies on the other side".to_string();
    let (from, to) = match axis {
        Some(dir) => {
            let back = first_hit(&outline, at, -dir).ok_or_else(nothing)?;
            let ahead = first_hit(&outline, at, dir).ok_or_else(nothing)?;
            (at - dir * back, at + dir * ahead)
        }
        None => {
            let near = outline
                .segments()
                .map(|seg: PathSeg| {
                    let n = seg.nearest(at, 1e-6);
                    (n.distance_sq, seg.eval(n.t))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, q)| q)
                .ok_or("the layer has no outline to measure")?;
            let d = at - near;
            let len = d.hypot();
            if len <= 1e-9 {
                return Err("`at` lies on the outline: point between two segments".into());
            }
            let dir = d / len;
            let ahead = first_hit(&outline, near, dir).ok_or_else(nothing)?;
            (near, near + dir * ahead)
        }
    };
    let d = to - from;
    Ok(json!({"from": pt(from), "to": pt(to), "distance": round2(d.hypot()), "dx": round2(d.x), "dy": round2(d.y)}))
}

// ── autometer.setStemWidth ──────────────────────────────────────────────────────────────────────

fn snap(v: f64, grid: f64) -> f64 {
    if grid > 0.0 { (v / grid).round() * grid } else { v }
}

fn set_stem_width(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("autometer.setStemWidth", v)?;
    let t = Targets::parse(&p)?;
    if t.has_lists() {
        return Err(p.bad("takes no target lists: give `stem`"));
    }
    let range = width_range(&p)?;
    let index = p.opt_int("stem", 0, i64::from(u32::MAX))?.ok_or_else(|| p.bad("`stem` is required"))?;
    let index = usize::try_from(index).map_err(|_| p.bad("`stem` is out of range"))?;
    let font = s.font()?;
    let step = grid(font);
    let (moves, was, width) = {
        let r = t.resolve(font)?;
        let outline = side_outline(font, &r.parts, &r.layer.master).map_err(|e| p.bad(e))?;
        let found = stems_of(&outline, range);
        let stem = found.get(index).ok_or_else(|| p.bad(format!("there is no stem {index}: the layer has {} in this width range", found.len())))?;
        let was = stem.width;
        let width = match (p.opt_f64("width")?, p.opt_f64("by")?, p.bool_or("snap", false)?) {
            (Some(w), None, false) => w,
            (None, Some(by), false) => {
                if by.abs() > MAX_FILTER_STEM {
                    return Err(p.bad(format!("`by` must be within ±{MAX_FILTER_STEM}, got {by}")));
                }
                was + by
            }
            (None, None, true) => nearest_common(&common_widths(font, r.master, stem.direction), was)
                .ok_or_else(|| p.bad(format!("the master has no {} stem value to snap to", direction_name(stem.direction))))?,
            _ => return Err(p.bad("give one of `width`, `by` and `snap`")),
        };
        if !(MIN_STEM_WIDTH..=MAX_FILTER_STEM).contains(&width) {
            return Err(p.bad(format!("the new width must be from {MIN_STEM_WIDTH} to {MAX_FILTER_STEM}, got {}", round2(width))));
        }
        let change = width - was;
        let sides = [edge_nodes(&r.parts, &stem.first), edge_nodes(&r.parts, &stem.second)];
        if sides.iter().any(Vec::is_empty) {
            return Err(p.bad(
                "no nodes of the layer's paths carry this stem's edges (it may come from a component, or a curve without a node at its extreme)",
            ));
        }
        let fixed: Vec<f64> = FIXED_HEIGHT_KINDS.iter().filter_map(|k| font.metric(r.master, k).map(|m| m.pos)).collect();
        let on_fixed = |l: &Line| {
            let y = (l.p0.y + l.p1.y) / 2.0;
            stem.direction == StemDirection::Horizontal && fixed.iter().any(|h| (h - y).abs() <= ON_HEIGHT)
        };
        // Half the change on each side, the first side's share on the grid, so an even outline
        // keeps landing on whole units and gets exactly the width asked for.
        let (a, b) = match (on_fixed(&stem.first), on_fixed(&stem.second)) {
            (true, true) => return Err(p.bad("both edges of this stem sit on metric lines; neither can move")),
            (true, false) => (0.0, change),
            (false, true) => (change, 0.0),
            (false, false) => {
                let a = snap(change / 2.0, step);
                (a, change - a)
            }
        };
        let n = across(stem);
        let mut moves: HashMap<(usize, usize), Vec2> = HashMap::new();
        for (list, by) in sides.iter().zip([-n * a, n * b]) {
            for &key in list {
                *moves.entry(key).or_insert(Vec2::ZERO) += by;
            }
        }
        (moves, was, width)
    };
    let moved = moves.len();
    let label = "Set Stem Width";
    let (glyph, layer) = s.edit(label, |font| {
        t.edit_layer(font, |layer| {
            let id = layer.id.clone();
            let mut parts = layer.parts_mut(t.side);
            let mut by_shape: HashMap<usize, Vec<(usize, Vec2)>> = HashMap::new();
            for (&(shape, node), &d) in &moves {
                by_shape.entry(shape).or_default().push((node, d));
            }
            for (shape, list) in by_shape {
                let Some(path) = parts.path_mut(shape) else { continue };
                let n = path.nodes.len();
                let closed = path.closed;
                let mut shift: Vec<(Vec2, u32)> = vec![(Vec2::ZERO, 0); n];
                let mut add = |i: usize, d: Vec2| {
                    if let Some(slot) = shift.get_mut(i) {
                        slot.0 += d;
                        slot.1 += 1;
                    }
                };
                for &(node, d) in &list {
                    add(node, d);
                    // The handles next to the node follow it.
                    let prev = if node > 0 { Some(node - 1) } else { closed.then(|| n.saturating_sub(1)) };
                    let next = if node + 1 < n { Some(node + 1) } else { closed.then_some(0) };
                    for k in [prev, next].into_iter().flatten() {
                        if k != node && path.nodes.get(k).is_some_and(|h| !h.on_curve()) {
                            add(k, d);
                        }
                    }
                }
                for (node, (sum, count)) in path.nodes.iter_mut().zip(shift) {
                    if count == 0 {
                        continue;
                    }
                    let d = sum / f64::from(count);
                    let (x, y) = (snap(node.x + d.x, step), snap(node.y + d.y, step));
                    if !(in_range(x) && in_range(y)) {
                        return Err(p.bad(format!("a node would move beyond ±{MAX_COORDINATE}")));
                    }
                    node.x = x;
                    node.y = y;
                }
            }
            Ok((t.glyph.clone(), id))
        })
    })?;
    Ok(json!({"glyph": glyph, "layer": layer, "stem": index, "was": round2(was), "width": round2(width), "moved": moved}))
}

// ── font.recalculateMeasurements ────────────────────────────────────────────────────────────────

const X_HEIGHT_GLYPHS: &[&str] = &["x", "z", "v", "w"];
const CAP_HEIGHT_GLYPHS: &[&str] = &["H", "I", "E", "T"];
const ASCENDER_GLYPHS: &[&str] = &["d", "h", "l", "k", "b"];
const DESCENDER_GLYPHS: &[&str] = &["p", "q"];
const X_OVERSHOOT_GLYPHS: &[&str] = &["o"];
const CAP_OVERSHOOT_GLYPHS: &[&str] = &["O"];
const LOWER_VERTICAL: &[&str] = &["n", "m", "h", "l", "i", "u"];
const UPPER_VERTICAL: &[&str] = &["H", "I", "N", "L", "E"];
const LOWER_HORIZONTAL: &[&str] = &["e", "t", "f", "z"];
const UPPER_HORIZONTAL: &[&str] = &["H", "E", "F", "T", "L", "Z"];

/// The bounds of the first of `names` that has an outline on the master: (bottom, top).
fn first_bounds(font: &Font, master: &str, names: &[&str]) -> Option<(f64, f64)> {
    names.iter().find_map(|name| {
        let outline = font.outline(name, master).ok()?;
        if outline.elements().is_empty() {
            return None;
        }
        let b = outline.bounding_box();
        (b.y0.is_finite() && b.y1.is_finite()).then_some((b.y0, b.y1))
    })
}

/// The most frequent straight stem width of `direction`, in whole units, across `names`; ties go
/// to the narrower width.
fn frequent_width(font: &Font, master: &str, names: &[&str], direction: StemDirection) -> Option<f64> {
    let mut counts: HashMap<i64, usize> = HashMap::new();
    for name in names {
        let Ok(outline) = font.outline(name, master) else { continue };
        let options = StemOptions { direction: Some(direction), max_width: MAX_FILTER_STEM, ..StemOptions::default() };
        for stem in detect_stems(&outline, &options).iter().filter(|s| !s.round) {
            *counts.entry(stem.width.round() as i64).or_default() += 1;
        }
    }
    counts.into_iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0))).map(|(w, _)| w as f64)
}

/// One value to write.
enum NewValue {
    Pos(&'static str, f64),
    Over(&'static str, f64),
    Stem(usize, f64),
}

fn recalculate(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.recalculateMeasurements", v)?;
    let font = s.font()?;
    let masters: Vec<&Master> = match p.opt_strings("masters")? {
        None => font.masters.iter().collect(),
        Some(list) => {
            let mut out: Vec<&Master> = Vec::new();
            for name in list {
                let m = font.masters.iter().find(|m| m.id == name || m.name == name).ok_or_else(|| p.bad(format!("there is no master `{name}`")))?;
                if !out.iter().any(|o| o.id == m.id) {
                    out.push(m);
                }
            }
            out
        }
    };
    if masters.is_empty() {
        return Err(p.bad("give at least one master"));
    }
    let fields: Vec<&str> = match p.opt_strings("fields")? {
        None => FIELDS.to_vec(),
        Some(list) => {
            for f in &list {
                if !FIELDS.contains(f) {
                    return Err(p.bad(format!("unknown field `{f}`; fields are {}", FIELDS.join(", "))));
                }
            }
            list
        }
    };
    let wants = |f: &str| fields.contains(&f);
    let mut writes: Vec<(String, Vec<(&'static str, NewValue)>)> = Vec::new();
    let mut skipped = Vec::new();
    for m in &masters {
        let mut out: Vec<(&'static str, NewValue)> = Vec::new();
        let mut skip = |field: &str, reason: &str| skipped.push(json!({"master": m.id, "field": field, "reason": reason}));
        let heights: [(&'static str, &'static str, &[&str], bool); 4] = [
            ("xHeight", metric::X_HEIGHT, X_HEIGHT_GLYPHS, true),
            ("capHeight", metric::CAP_HEIGHT, CAP_HEIGHT_GLYPHS, true),
            ("ascender", metric::ASCENDER, ASCENDER_GLYPHS, true),
            ("descender", metric::DESCENDER, DESCENDER_GLYPHS, false),
        ];
        let mut measured: HashMap<&str, f64> = HashMap::new();
        for (field, kind_name, names, top) in heights {
            if !wants(field) {
                continue;
            }
            match first_bounds(font, &m.id, names) {
                Some((bottom, high)) => {
                    let v = (if top { high } else { bottom }).round();
                    measured.insert(kind_name, v);
                    out.push((field, NewValue::Pos(kind_name, v)));
                }
                None => skip(field, &format!("none of {} has an outline", names.join(", "))),
            }
        }
        let overs: [(&'static str, &'static str, &[&str], bool); 3] = [
            ("xOvershoot", metric::X_HEIGHT, X_OVERSHOOT_GLYPHS, true),
            ("capOvershoot", metric::CAP_HEIGHT, CAP_OVERSHOOT_GLYPHS, true),
            ("baselineOvershoot", metric::BASELINE, X_OVERSHOOT_GLYPHS, false),
        ];
        for (field, kind_name, names, top) in overs {
            if !wants(field) {
                continue;
            }
            let Some(current) = font.metric(m, kind_name).map(|v: MetricValue| v.pos) else {
                skip(field, &format!("the font has no plain {kind_name} metric"));
                continue;
            };
            let base = measured.get(kind_name).copied().unwrap_or(current);
            match first_bounds(font, &m.id, names) {
                Some((bottom, high)) => out.push((field, NewValue::Over(kind_name, ((if top { high } else { bottom }) - base).round()))),
                None => skip(field, &format!("none of {} has an outline", names.join(", "))),
            }
        }
        for (field, horizontal, lower, upper) in
            [("verticalStems", false, LOWER_VERTICAL, UPPER_VERTICAL), ("horizontalStems", true, LOWER_HORIZONTAL, UPPER_HORIZONTAL)]
        {
            if !wants(field) {
                continue;
            }
            let direction = if horizontal { StemDirection::Horizontal } else { StemDirection::Vertical };
            let defs: Vec<usize> = font.stems.iter().enumerate().filter(|(_, d)| d.horizontal == horizontal).map(|(i, _)| i).take(2).collect();
            if defs.is_empty() {
                skip(field, "the font defines no stem of this direction");
                continue;
            }
            let lower_w = frequent_width(font, &m.id, lower, direction);
            let upper_w = frequent_width(font, &m.id, upper, direction);
            // The first stem value stands for the lowercase, the second for the capitals; a font
            // without lowercase stems gives its first value the capitals' width.
            let values = [lower_w.or(upper_w), upper_w];
            for (def, value) in defs.iter().zip(values) {
                match value {
                    Some(w) => out.push((field, NewValue::Stem(*def, w))),
                    None => skip(field, "its sample glyphs have no straight stems"),
                }
            }
        }
        if !out.is_empty() {
            writes.push((m.id.clone(), out));
        }
    }
    if writes.is_empty() {
        return Err(p.bad("nothing could be measured: the sample glyphs are empty or the font lacks the metrics and stems to fill"));
    }
    let mut report = Vec::new();
    for (master, list) in &writes {
        let mut values = Map::new();
        for (field, w) in list {
            let v = match w {
                NewValue::Pos(_, v) | NewValue::Over(_, v) | NewValue::Stem(_, v) => *v,
            };
            match values.get_mut(*field) {
                // Two stem values of one direction: the field lists both.
                Some(Value::Array(a)) => a.push(json!(v)),
                Some(existing) => *existing = json!([existing.clone(), v]),
                None => {
                    values.insert((*field).into(), json!(v));
                }
            }
        }
        report.push(json!({"master": master, "values": values}));
    }
    s.edit("Recalculate Measurements", |font| {
        for (master, list) in &writes {
            for (_, w) in list {
                match w {
                    NewValue::Pos(k, v) => font.set_metric_pos(master, k, *v).map_err(|e| p.bad(e.to_string()))?,
                    NewValue::Over(k, v) => {
                        let Some(i) = font.metrics.iter().position(|d| d.kind == *k && d.name.is_none() && d.filter.is_none()) else { continue };
                        let Some(m) = Arc::make_mut(&mut font.masters).iter_mut().find(|m| m.id == *master) else { continue };
                        if m.metric_values.len() <= i {
                            m.metric_values.resize(i + 1, MetricValue::default());
                        }
                        if let Some(slot) = m.metric_values.get_mut(i) {
                            slot.over = *v;
                        }
                    }
                    NewValue::Stem(i, v) => {
                        let count = font.stems.len();
                        let Some(m) = Arc::make_mut(&mut font.masters).iter_mut().find(|m| m.id == *master) else { continue };
                        if m.stem_values.len() < count {
                            m.stem_values.resize(count, 0.0);
                        }
                        if let Some(slot) = m.stem_values.get_mut(*i) {
                            *slot = *v;
                        }
                    }
                }
            }
        }
        Ok(())
    })?;
    Ok(json!({"masters": report, "skipped": skipped}))
}
