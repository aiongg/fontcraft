//! Measuring: the headless forms of the Measurement tool. `measure.ruler` finds where a line
//! crosses a layer's outlines, components included and overlaps merged; `measure.coordinates`
//! lists every point with the deltas between neighbouring on-curve nodes, and the stretches of
//! outline that lie inside an overlap.
//!
//! Both take `placements`, the glyphs of a line with their positions, so the tool covers every
//! glyph shown (`measurement.allVisibleGlyphs`): the ruler then crosses all of them, and the
//! coordinates come per glyph. The placed outlines share one element budget: once it is spent the
//! remaining placements are left out and the answer says `truncated`, so a long line of large
//! glyphs measures its first glyphs instead of failing.
//!
//! X deltas are also given relative to the master's italic angle: `italicDx = dx − dy·tan(angle)`,
//! so a step that follows the italic angle has an `italicDx` of 0.

use std::collections::HashSet;

use fontcraft_geom::{Affine, BezPath, Line, ParamCurve, PathEl, PathSeg, Point, Rect, Shape as _, Vec2};
use fontcraft_model::{Font, Glyph, Layer, LayerParts, LayerSide, MAX_COORDINATE, MAX_OUTLINE_ELEMENTS, Master, NodeKind, Shape, metric};
use serde_json::{Map, Value, json};

use super::*;
use crate::params::{GuideScope, Params, TargetKind, Targets, kind};

/// How close, as a segment parameter, a crossing must be to a segment's end to count as being
/// on the node there.
const AT_NODE: f64 = 1e-6;
/// Crossings closer than this along the line, in font units, are at the same place.
const SAME_PLACE: f64 = 1e-6;
/// How far from the line, in font units, a point must be to be on one side of it.
const OFF_LINE: f64 = 1e-7;
/// How many segments the side test walks along a contour that runs on the line.
const MAX_WALK: f64 = 64.0;
/// Most glyphs one measurement covers (`placements`).
pub const MAX_PLACEMENTS: usize = 200;
/// How far to each side of the outline, in font units, the overlap test looks.
const SIDE_STEP: f64 = 0.01;
/// Points per segment where the overlap test looks; a change shorter than one step can be missed.
const OVERLAP_SAMPLES: usize = 8;
/// Halvings that place the end of an overlapped stretch between two samples.
const OVERLAP_BISECT: usize = 12;
/// Most contour elements and bounding-box tests the overlap test of one query spends, summed over
/// every winding query and every placed glyph. Past it the test stops and keeps the stretches
/// found so far.
const MAX_OVERLAP_WORK: usize = 20_000_000;
/// Most points in the overlapped stretches of one query, over every placed glyph.
const MAX_OVERLAP_POINTS: usize = 50_000;
/// Points a curved piece of an overlapped stretch is drawn with.
const CURVE_STEPS: usize = 8;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "measure.ruler",
            "Measure Along Line",
            [],
            None,
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\", from?: [x, y], to?: [x, y], guide?: {scope: \"local\" | \"global\", index}, placements?: [{glyph, layer?: id, side?, x, y}] (at most 200)} where a line crosses the layer's paths and components: the segment from `from` to `to`, or the whole line of a guide. With `placements`, the line crosses those glyphs instead, each moved by its x and y (positions relative to the target glyph's origin, as in a line of text); once the placed outlines reach the element limit the rest are left out and `truncated` is set. Overlapping paths are measured as merged: crossings inside an overlap are skipped → {glyph, layer, italicAngle, length?, angle?: degrees counter-clockwise, 0 pointing up (with from and to), crossings: [{x, y, distance: along the line from `from` or from the guide's point, placement?: index}], gaps: [{from, to: crossing indices, distance, dx, dy, italicDx, inside: bool (inside the outline)}], truncated?: true}",
            has_doc,
            ruler
        )
        .targets(&[TargetKind::Layer]),
        cmd!(
            query "measure.coordinates",
            "List Point Coordinates",
            [],
            None,
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\", placements?: [{glyph, layer?: id, side?, x, y}] (at most 200)} every node, handle and anchor of the layer with its coordinates, the X and Y deltas between neighbouring on-curve nodes of each path, and the stretches of outline that lie inside an overlap (they vanish when overlaps are removed). With `placements`, the same for each placed glyph, in its own coordinates, and not for the target outside them; once the placed outlines reach the element limit the rest are left out and `truncated` is set. `overlapsTruncated` marks a glyph whose overlap test stopped at the query's work limit → {glyph, layer, italicAngle, points: [{shape, node, x, y, onCurve: bool}], deltas: [{shape, from, to: node indices, dx, dy, italicDx}], anchors: [{name, x, y}], overlaps: [[[x, y]…]], overlapsTruncated?: true (these five without placements), placements?: [{glyph, layer, x, y, italicAngle, points, deltas, anchors, overlaps, overlapsTruncated?}], truncated?: true}",
            has_doc,
            coordinates
        )
        .targets(&[TargetKind::Layer]),
    ]
}

fn italic_tan(font: &Font, master: &Master) -> f64 {
    let t = font.metric_pos(master, metric::ITALIC_ANGLE, 0.0).to_radians().tan();
    if t.is_finite() { t } else { 0.0 }
}

/// A number within the model's coordinate limits.
fn coord(v: &Value) -> Option<f64> {
    v.as_f64().filter(|n| n.is_finite() && n.abs() <= MAX_COORDINATE)
}

/// A point parameter `[x, y]` within the model's limits.
fn point(p: &Params, key: &str) -> Result<Point> {
    let bad = |found: &Value| p.bad(format!("`{key}` must be [x, y] with finite numbers within ±{MAX_COORDINATE}, got {}", kind(found)));
    let v = p.get(key).ok_or_else(|| p.bad(format!("`{key}` is required")))?;
    match v.as_array().map(Vec::as_slice) {
        Some([x, y]) => match (coord(x), coord(y)) {
            (Some(x), Some(y)) => Ok(Point::new(x, y)),
            _ => Err(bad(v)),
        },
        _ => Err(bad(v)),
    }
}

/// One glyph of a line and where it sits relative to the target glyph's origin.
struct Placement<'f> {
    glyph: &'f Glyph,
    layer: &'f Layer,
    master: &'f Master,
    parts: LayerParts<'f>,
    offset: Vec2,
}

/// The `placements` parameter, checked against the font. `None` when it is not given.
fn placements<'f>(p: &Params, font: &'f Font) -> Result<Option<Vec<Placement<'f>>>> {
    let Some(v) = p.get("placements") else { return Ok(None) };
    let list = v.as_array().ok_or_else(|| p.bad(format!("`placements` must be a list of {{glyph, layer?, side?, x, y}}, got {}", kind(v))))?;
    if list.len() > MAX_PLACEMENTS {
        return Err(p.bad(format!("`placements` holds {} glyphs; at most {MAX_PLACEMENTS} are measured at once", list.len())));
    }
    let default_layer = font.default_master()?.id.clone();
    let mut out = Vec::with_capacity(list.len());
    for (i, entry) in list.iter().enumerate() {
        let bad = |what: String| p.bad(format!("`placements` entry {i}: {what}"));
        let o = entry.as_object().ok_or_else(|| bad(format!("must be {{glyph, layer?, side?, x, y}}, got {}", kind(entry))))?;
        let name = o.get("glyph").and_then(Value::as_str).ok_or_else(|| bad("needs `glyph`, a glyph name".into()))?;
        let glyph = font.glyph(name).ok_or_else(|| bad(format!("there is no glyph `{name}`")))?;
        let layer_id = match o.get("layer") {
            None | Some(Value::Null) => default_layer.as_str(),
            Some(Value::String(id)) => id.as_str(),
            Some(other) => return Err(bad(format!("`layer` must be a layer id, got {}", kind(other)))),
        };
        let layer = glyph.layers.iter().find(|l| l.id == layer_id).ok_or_else(|| bad(format!("glyph `{name}` has no layer `{layer_id}`")))?;
        let master = font.master(&layer.master)?;
        let side = match o.get("side") {
            None | Some(Value::Null) => LayerSide::Foreground,
            Some(Value::String(s)) => {
                LayerSide::parse(s).ok_or_else(|| bad(format!("`side` must be \"foreground\" or \"background\", got \"{s}\"")))?
            }
            Some(other) => return Err(bad(format!("`side` must be \"foreground\" or \"background\", got {}", kind(other)))),
        };
        let at = |key: &str| match o.get(key) {
            None | Some(Value::Null) => Ok(0.0),
            Some(v) => coord(v).ok_or_else(|| bad(format!("`{key}` must be a finite number within ±{MAX_COORDINATE}, got {}", kind(v)))),
        };
        out.push(Placement { glyph, layer, master, parts: layer.parts(side), offset: Vec2::new(at("x")?, at("y")?) });
    }
    Ok(Some(out))
}

/// One contour of a measured outline: whether it is closed, and which placement it belongs to
/// (0 without placements).
#[derive(Clone, Debug)]
struct Contour {
    path: BezPath,
    closed: bool,
    owner: usize,
}

/// The contours of one side of a layer with components resolved. Paths that do not make a
/// valid outline and components of missing glyphs are left out. `budget` is the number of path
/// elements still allowed; it shrinks by what this layer adds. Returns `false`, adding nothing,
/// when the layer does not fit in it.
fn contours(font: &Font, parts: LayerParts, master: &str, owner: usize, budget: &mut usize, out: &mut Vec<Contour>) -> Result<bool> {
    let start = out.len();
    let mut left = *budget;
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
                font.component_outline(c, master)?
            }
            Shape::Image(_) | Shape::Raw(_) => continue,
        };
        let Some(rest) = left.checked_sub(part.elements().len()) else {
            out.truncate(start);
            return Ok(false);
        };
        left = rest;
        split(&part, owner, out);
    }
    *budget = left;
    Ok(true)
}

/// The error of a target layer whose outline alone is over the element budget.
fn too_large(p: &Params) -> crate::EngineError {
    p.bad("the outline is too large to measure")
}

/// Splits a path into its contours.
fn split(path: &BezPath, owner: usize, out: &mut Vec<Contour>) {
    let mut current: Vec<PathEl> = Vec::new();
    let mut closed = false;
    let mut flush = |current: &mut Vec<PathEl>, closed: bool| {
        if current.len() > 1 {
            out.push(Contour { path: BezPath::from_vec(std::mem::take(current)), closed, owner });
        }
        current.clear();
    };
    for el in path.elements() {
        match el {
            PathEl::MoveTo(_) => {
                flush(&mut current, closed);
                closed = false;
                current.push(*el);
            }
            PathEl::ClosePath => {
                current.push(*el);
                closed = true;
            }
            other => current.push(*other),
        }
    }
    flush(&mut current, closed);
}

/// A place where the line crosses a contour. `at` is the signed distance along the line from its
/// origin; `sign` is +1 or −1 for the direction a closed contour crosses in, 0 for an open one.
#[derive(Clone, Copy, Debug)]
struct Hit {
    at: f64,
    point: Point,
    sign: i64,
    owner: usize,
}

/// A crossing that remains when overlaps are merged, with whether the line is inside the
/// outline after it.
#[derive(Clone, Copy, Debug)]
struct Crossing {
    at: f64,
    point: Point,
    inside: bool,
    owner: usize,
}

/// Which side of the line (positive: left) the contour is on just before (`step` −1) or after
/// (`step` +1) position `pos` (segment index plus parameter), walking along the contour until a
/// point is off the line. 0 when none is found.
fn side_from<S: ParamCurve>(segs: &[S], closed: bool, pos: f64, step: f64, side: &impl Fn(Point) -> f64) -> f64 {
    let n = segs.len() as f64;
    let limit = n.min(MAX_WALK);
    let mut d = 1e-4;
    while d <= limit {
        let mut q = pos + step * d;
        if closed {
            q = q.rem_euclid(n);
        } else if !(0.0..=n).contains(&q) {
            return 0.0;
        }
        let i = (q.floor().max(0.0) as usize).min(segs.len().saturating_sub(1));
        let t = (q - i as f64).clamp(0.0, 1.0);
        if let Some(seg) = segs.get(i) {
            let v = side(seg.eval(t));
            if v.abs() > OFF_LINE {
                return v.signum();
            }
        }
        d = if d < 0.5 { d * 10.0 } else { d + 0.5 };
    }
    0.0
}

/// Every place where the infinite line through `origin` along unit vector `dir` crosses the
/// contours. A line that only touches a contour does not cross it.
fn hits(contours: &[Contour], origin: Point, dir: Vec2) -> Vec<Hit> {
    let mut reach: f64 = 1.0;
    for contour in contours {
        for el in contour.path.elements() {
            let pts = match *el {
                PathEl::MoveTo(a) | PathEl::LineTo(a) => [Some(a), None, None],
                PathEl::QuadTo(a, b) => [Some(a), Some(b), None],
                PathEl::CurveTo(a, b, c) => [Some(a), Some(b), Some(c)],
                PathEl::ClosePath => [None, None, None],
            };
            for q in pts.into_iter().flatten() {
                reach = reach.max((q - origin).hypot() + 1.0);
            }
        }
    }
    let line = Line::new(origin - dir * reach, origin + dir * reach);
    let side = |q: Point| dir.cross(q - origin);
    let mut out = Vec::new();
    for contour in contours {
        let closed = contour.closed;
        let segs: Vec<_> = contour.path.segments().collect();
        let n = segs.len();
        let mut found: Vec<(usize, f64)> = Vec::new();
        for (i, seg) in segs.iter().enumerate() {
            let mut ts: Vec<f64> = seg.intersect_line(line).iter().map(|x| x.segment_t.clamp(0.0, 1.0)).collect();
            ts.sort_by(f64::total_cmp);
            // A tangent can give the same root twice.
            ts.dedup_by(|a, b| (*a - *b).abs() < AT_NODE);
            found.extend(ts.into_iter().map(|t| (i, t)));
        }
        // A crossing at the node between two segments is found at the end of one and the start of
        // the next: keep the second.
        let starts: HashSet<usize> = found.iter().filter(|(_, t)| *t < AT_NODE).map(|(i, _)| *i).collect();
        found.retain(|&(i, t)| {
            if t <= 1.0 - AT_NODE {
                return true;
            }
            let next = if i.saturating_add(1) < n {
                Some(i.saturating_add(1))
            } else if closed {
                Some(0)
            } else {
                None
            };
            !next.is_some_and(|j| starts.contains(&j))
        });
        for (i, t) in found {
            let Some(seg) = segs.get(i) else { continue };
            let pos = i as f64 + t;
            let before = side_from(&segs, closed, pos, -1.0, &side);
            let after = side_from(&segs, closed, pos, 1.0, &side);
            if (before == 0.0 && after == 0.0) || before * after > 0.0 {
                continue;
            }
            let sign = if !closed {
                0
            } else if after > 0.0 || (after == 0.0 && before < 0.0) {
                1
            } else {
                -1
            };
            let point = seg.eval(t);
            out.push(Hit { at: (point - origin).dot(dir), point, sign, owner: contour.owner });
        }
    }
    out
}

/// The crossings of the merged outline: a crossing of a closed contour counts only where the
/// line goes from outside every contour to inside one, or back (non-zero winding). Crossings of
/// open contours are all kept.
fn merged(mut hits: Vec<Hit>) -> Vec<Crossing> {
    hits.sort_by(|a, b| a.at.total_cmp(&b.at));
    let mut groups: Vec<Vec<Hit>> = Vec::new();
    for h in hits {
        match groups.last_mut() {
            Some(g) if g.last().is_some_and(|l| (h.at - l.at).abs() < SAME_PLACE) => g.push(h),
            _ => groups.push(vec![h]),
        }
    }
    let mut out = Vec::new();
    let mut winding: i64 = 0;
    for g in groups {
        let was_inside = winding != 0;
        winding = winding.saturating_add(g.iter().map(|h| h.sign).sum::<i64>());
        let inside = winding != 0;
        let keep = if was_inside != inside { g.first() } else { g.iter().find(|h| h.sign == 0) };
        if let Some(h) = keep {
            out.push(Crossing { at: h.at, point: h.point, inside, owner: h.owner });
        }
    }
    out
}

/// The non-zero winding number of a set of closed contours, counting the contour elements it
/// reads.
struct Winding<'a> {
    closed: Vec<(&'a BezPath, Rect, usize)>,
    work: usize,
}

impl<'a> Winding<'a> {
    fn new(contours: &'a [Contour], work: usize) -> Self {
        let closed =
            contours.iter().filter(|c| c.closed).map(|c| (&c.path, c.path.bounding_box().inflate(1.0, 1.0), c.path.elements().len())).collect();
        Self { closed, work }
    }

    /// The winding number at `q`; `None` once the work limit is reached. Each bounding box it
    /// tests counts as work, so many small contours cannot make it slow either.
    fn at(&mut self, q: Point) -> Option<i64> {
        self.work = self.work.checked_add(self.closed.len()).filter(|w| *w <= MAX_OVERLAP_WORK)?;
        let mut w: i64 = 0;
        for (path, bbox, elements) in &self.closed {
            if !bbox.contains(q) {
                continue;
            }
            self.work = self.work.checked_add(*elements).filter(|w| *w <= MAX_OVERLAP_WORK)?;
            w = w.saturating_add(i64::from(path.winding(q)));
        }
        Some(w)
    }

    /// Whether the outline at parameter `t` of `seg` lies inside an overlap: the outline is
    /// filled on both sides of it there.
    fn overlapped(&mut self, seg: &PathSeg, t: f64) -> Option<bool> {
        let q = seg.eval(t);
        let h = 1e-4;
        let mut d = seg.eval((t + h).min(1.0)) - seg.eval((t - h).max(0.0));
        if d.hypot().is_nan() || d.hypot() <= 1e-12 {
            d = seg.end() - seg.start();
        }
        let len = d.hypot();
        if !len.is_finite() || len <= 1e-12 {
            return Some(false);
        }
        let n = Vec2::new(-d.y, d.x) * (SIDE_STEP / len);
        Some(self.at(q + n)? != 0 && self.at(q - n)? != 0)
    }
}

/// The parameter ranges of `seg` and whether each lies inside an overlap. `None` once the work
/// limit is reached.
fn classify(w: &mut Winding<'_>, seg: &PathSeg) -> Option<Vec<(f64, f64, bool)>> {
    let mut out: Vec<(f64, f64, bool)> = Vec::new();
    let mut push = |a: f64, b: f64, inside: bool| match out.last_mut() {
        Some(last) if last.2 == inside => last.1 = b,
        _ => out.push((a, b, inside)),
    };
    let mut prev_t = 0.0;
    let mut prev = w.overlapped(seg, 0.0)?;
    for i in 1..=OVERLAP_SAMPLES {
        let t = i as f64 / OVERLAP_SAMPLES as f64;
        let now = w.overlapped(seg, t)?;
        if now == prev {
            push(prev_t, t, prev);
        } else {
            let (mut lo, mut hi) = (prev_t, t);
            for _ in 0..OVERLAP_BISECT {
                let mid = (lo + hi) / 2.0;
                if w.overlapped(seg, mid)? == prev {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            let switch = (lo + hi) / 2.0;
            push(prev_t, switch, prev);
            push(switch, t, now);
        }
        prev_t = t;
        prev = now;
    }
    Some(out)
}

/// What the overlap tests of one query have spent so far.
#[derive(Clone, Copy, Debug, Default)]
struct Spent {
    /// Contour elements read and bounding boxes tested ([`MAX_OVERLAP_WORK`]).
    work: usize,
    /// Points of overlapped stretches returned ([`MAX_OVERLAP_POINTS`]).
    points: usize,
}

/// The stretches of closed contours that lie inside an overlap of the outline (of other
/// contours, or of the contour with itself), as polylines, and whether the test finished. Work
/// and size are capped for the whole query (`spent`): past the caps the stretches found so far
/// are returned.
fn overlaps(contours: &[Contour], spent: &mut Spent) -> (Vec<Vec<Point>>, bool) {
    let mut w = Winding::new(contours, spent.work);
    let mut out: Vec<Vec<Point>> = Vec::new();
    let mut complete = true;
    'contours: for contour in contours.iter().filter(|c| c.closed) {
        let mut open: Vec<Point> = Vec::new();
        for seg in contour.path.segments() {
            let Some(ranges) = classify(&mut w, &seg) else {
                complete = false;
                break 'contours;
            };
            for (a, b, inside) in ranges {
                if !inside || b - a <= 0.0 {
                    if open.len() > 1 {
                        out.push(std::mem::take(&mut open));
                    }
                    open.clear();
                    continue;
                }
                let piece = seg.subsegment(a..b);
                let steps = if matches!(piece, PathSeg::Line(_)) { 1 } else { CURVE_STEPS };
                let joins = open.last().is_some_and(|l| (*l - piece.start()).hypot() < 1e-6);
                if !joins {
                    if open.len() > 1 {
                        out.push(std::mem::take(&mut open));
                    }
                    open.clear();
                }
                for k in usize::from(joins)..=steps {
                    open.push(piece.eval(k as f64 / steps as f64));
                }
                spent.points = spent.points.saturating_add(steps.saturating_add(1));
                if spent.points > MAX_OVERLAP_POINTS {
                    complete = false;
                    break 'contours;
                }
            }
        }
        if open.len() > 1 {
            out.push(open);
        }
    }
    spent.work = w.work.max(spent.work);
    (out, complete)
}

fn ruler(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("measure.ruler", p)?;
    let t = Targets::parse(&p)?;
    if t.has_lists() {
        return Err(p.bad("takes no target lists: it measures the whole layer"));
    }
    let font = s.font()?;
    let r = t.resolve(font)?;
    let placed = placements(&p, font)?;
    let (origin, dir, length) = match (p.get("guide"), p.get("from"), p.get("to")) {
        (Some(_), None, None) => {
            let g = super::guide::guide_ref(&p, "guide")?;
            let list = match g.scope {
                GuideScope::Local => r.parts.guides,
                GuideScope::Global => r.master.guides.as_slice(),
            };
            let guide = list.get(g.index).ok_or_else(|| p.bad(format!("{} guide {} does not resolve", g.scope.as_str(), g.index)))?;
            let a = guide.angle.to_radians();
            if !(guide.x.is_finite() && guide.y.is_finite() && a.is_finite()) {
                return Err(p.bad("the guide's position or angle is not a number"));
            }
            (Point::new(guide.x, guide.y), Vec2::new(a.cos(), a.sin()), None)
        }
        (None, Some(_), Some(_)) => {
            let (from, to) = (point(&p, "from")?, point(&p, "to")?);
            let d = to - from;
            let len = d.hypot();
            if !len.is_finite() || len <= 1e-9 {
                return Err(p.bad("`from` and `to` are the same point"));
            }
            (from, d * (1.0 / len), Some(len))
        }
        _ => return Err(p.bad("give `from` and `to`, or `guide`")),
    };
    let tan = italic_tan(font, r.master);
    let mut shapes = Vec::new();
    let mut budget = MAX_OUTLINE_ELEMENTS;
    let mut truncated = false;
    match &placed {
        None => {
            if !contours(font, r.parts, &r.layer.master, 0, &mut budget, &mut shapes)? {
                return Err(too_large(&p));
            }
        }
        Some(list) => {
            for (i, pl) in list.iter().enumerate() {
                let start = shapes.len();
                if !contours(font, pl.parts, &pl.layer.master, i, &mut budget, &mut shapes)? {
                    truncated = true;
                    break;
                }
                let shift = Affine::translate(pl.offset);
                for c in shapes.iter_mut().skip(start) {
                    c.path.apply_affine(shift);
                }
            }
        }
    }
    let crossings: Vec<Crossing> =
        merged(hits(&shapes, origin, dir)).into_iter().filter(|c| length.is_none_or(|len| c.at >= -SAME_PLACE && c.at <= len + SAME_PLACE)).collect();
    let gaps: Vec<Value> = crossings
        .iter()
        .zip(crossings.iter().skip(1))
        .enumerate()
        .map(|(i, (a, b))| {
            let d = b.point - a.point;
            json!({"from": i, "to": i.saturating_add(1), "distance": d.hypot(), "dx": d.x, "dy": d.y, "italicDx": d.x - d.y * tan, "inside": a.inside})
        })
        .collect();
    let points: Vec<Value> = crossings
        .iter()
        .map(|c| {
            let mut v = json!({"x": c.point.x, "y": c.point.y, "distance": c.at});
            if let (Some(_), Some(o)) = (&placed, v.as_object_mut()) {
                o.insert("placement".into(), json!(c.owner));
            }
            v
        })
        .collect();
    let mut out = json!({
        "glyph": r.glyph.name,
        "layer": r.layer.id,
        "italicAngle": font.metric_pos(r.master, metric::ITALIC_ANGLE, 0.0),
        "crossings": points,
        "gaps": gaps,
    });
    if let (Some(len), Some(o)) = (length, out.as_object_mut()) {
        o.insert("length".into(), json!(len));
        o.insert("angle".into(), json!((-dir.x).atan2(dir.y).to_degrees().rem_euclid(360.0)));
    }
    if let (true, Some(o)) = (truncated, out.as_object_mut()) {
        o.insert("truncated".into(), json!(true));
    }
    Ok(out)
}

/// The coordinates, deltas, anchors and overlapped stretches of one side of a layer. `budget`
/// holds the path elements still allowed in this query, and `spent` what its overlap tests have
/// used. `None` when the layer's outline does not fit in the budget.
fn layer_coordinates(
    font: &Font,
    master: &Master,
    layer: &Layer,
    parts: LayerParts,
    budget: &mut usize,
    spent: &mut Spent,
) -> Result<Option<Map<String, Value>>> {
    let mut shapes = Vec::new();
    if !contours(font, parts, &layer.master, 0, budget, &mut shapes)? {
        return Ok(None);
    }
    let tan = italic_tan(font, master);
    let mut points = Vec::new();
    let mut deltas = Vec::new();
    for (si, shape) in parts.shapes.iter().enumerate() {
        let Shape::Path(path) = shape else { continue };
        let mut on_curve = Vec::new();
        for (ni, n) in path.nodes.iter().enumerate() {
            let on = n.kind != NodeKind::OffCurve;
            points.push(json!({"shape": si, "node": ni, "x": n.x, "y": n.y, "onCurve": on}));
            if on {
                on_curve.push((ni, n.x, n.y));
            }
        }
        let pairs: Vec<_> = on_curve.iter().zip(on_curve.iter().skip(1)).collect();
        let wrap = if path.closed && on_curve.len() > 1 { on_curve.last().zip(on_curve.first()) } else { None };
        for (a, b) in pairs.into_iter().chain(wrap) {
            let (dx, dy) = (b.1 - a.1, b.2 - a.2);
            deltas.push(json!({"shape": si, "from": a.0, "to": b.0, "dx": dx, "dy": dy, "italicDx": dx - dy * tan}));
        }
    }
    let anchors: Vec<Value> = parts.anchors.iter().map(|a| json!({"name": a.name, "x": a.x, "y": a.y})).collect();
    let (lines, complete) = overlaps(&shapes, spent);
    let overlapped: Vec<Value> = lines.into_iter().map(|line| json!(line.iter().map(|q| [q.x, q.y]).collect::<Vec<_>>())).collect();
    let mut out = Map::new();
    out.insert("italicAngle".into(), json!(font.metric_pos(master, metric::ITALIC_ANGLE, 0.0)));
    out.insert("points".into(), Value::Array(points));
    out.insert("deltas".into(), Value::Array(deltas));
    out.insert("anchors".into(), Value::Array(anchors));
    out.insert("overlaps".into(), Value::Array(overlapped));
    if !complete {
        out.insert("overlapsTruncated".into(), json!(true));
    }
    Ok(Some(out))
}

fn coordinates(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("measure.coordinates", p)?;
    let t = Targets::parse(&p)?;
    if t.has_lists() {
        return Err(p.bad("takes no target lists: it lists the whole layer"));
    }
    let font = s.font()?;
    let r = t.resolve(font)?;
    let placed = placements(&p, font)?;
    let mut budget = MAX_OUTLINE_ELEMENTS;
    let mut spent = Spent::default();
    // With placements the target is measured as one of them, if it is placed at all.
    let mut out = match &placed {
        None => layer_coordinates(font, r.master, r.layer, r.parts, &mut budget, &mut spent)?.ok_or_else(|| too_large(&p))?,
        Some(_) => {
            let mut m = Map::new();
            m.insert("italicAngle".into(), json!(font.metric_pos(r.master, metric::ITALIC_ANGLE, 0.0)));
            m
        }
    };
    out.insert("glyph".into(), json!(r.glyph.name));
    out.insert("layer".into(), json!(r.layer.id));
    if let Some(list) = placed {
        let mut each = Vec::with_capacity(list.len());
        for pl in &list {
            let Some(mut one) = layer_coordinates(font, pl.master, pl.layer, pl.parts, &mut budget, &mut spent)? else {
                out.insert("truncated".into(), json!(true));
                break;
            };
            one.insert("glyph".into(), json!(pl.glyph.name));
            one.insert("layer".into(), json!(pl.layer.id));
            one.insert("x".into(), json!(pl.offset.x));
            one.insert("y".into(), json!(pl.offset.y));
            each.push(Value::Object(one));
        }
        out.insert("placements".into(), Value::Array(each));
    }
    Ok(Value::Object(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Contour {
        let mut path = BezPath::new();
        path.move_to((x0, y0));
        path.line_to((x1, y0));
        path.line_to((x1, y1));
        path.line_to((x0, y1));
        path.close_path();
        Contour { path, closed: true, owner: 0 }
    }

    #[test]
    fn overlapped_stretches_are_the_parts_inside_another_contour() {
        // Two squares that overlap in 50,0–100,100: the right edge of the first and the left
        // edge of the second lie inside the other.
        let found = overlaps(&[rect(0.0, 0.0, 100.0, 100.0), rect(50.0, 0.0, 150.0, 100.0)], &mut Spent::default()).0;
        let mut xs: Vec<(f64, f64)> = found
            .iter()
            .map(|line| {
                let (a, b) = (line.first().copied().unwrap_or_default(), line.last().copied().unwrap_or_default());
                ((a.x + b.x) / 2.0, (a.y - b.y).abs())
            })
            .collect();
        xs.sort_by(|a, b| a.0.total_cmp(&b.0));
        assert_eq!(xs.len(), 2, "{found:?}");
        assert!((xs[0].0 - 50.0).abs() < 0.01 && (xs[1].0 - 100.0).abs() < 0.01, "{found:?}");
        assert!(xs.iter().all(|(_, h)| (h - 100.0).abs() < 0.1), "whole edges: {found:?}");
        // Apart, nothing is overlapped.
        assert!(overlaps(&[rect(0.0, 0.0, 100.0, 100.0), rect(200.0, 0.0, 300.0, 100.0)], &mut Spent::default()).0.is_empty());
        // A counter is not an overlap.
        let mut counter = rect(25.0, 25.0, 75.0, 75.0);
        counter.path = counter.path.reverse_subpaths();
        assert!(overlaps(&[rect(0.0, 0.0, 100.0, 100.0), counter], &mut Spent::default()).0.is_empty());
    }

    #[test]
    fn many_small_contours_stop_at_the_work_limit() {
        // Every winding query tests the bounding box of each of the 20,000 squares: without
        // counting those tests the overlap test would make about 1e10 of them.
        let squares: Vec<Contour> = (0..20_000)
            .map(|i| {
                let (x, y) = (f64::from(i % 200) * 20.0, f64::from(i / 200) * 20.0);
                rect(x, y, x + 10.0, y + 10.0)
            })
            .collect();
        let mut spent = Spent::default();
        let (found, complete) = overlaps(&squares, &mut spent);
        assert!(found.is_empty() && !complete);
        assert!(spent.work <= MAX_OVERLAP_WORK, "{spent:?}");
    }
}
