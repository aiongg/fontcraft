//! Measuring: the headless forms of the Measurement tool. `measure.ruler` finds where a line
//! crosses a layer's outlines, components included and overlaps merged; `measure.coordinates`
//! lists every point with the deltas between neighbouring on-curve nodes.
//!
//! X deltas are also given relative to the master's italic angle: `italicDx = dx − dy·tan(angle)`,
//! so a step that follows the italic angle has an `italicDx` of 0.

use std::collections::HashSet;

use fontcraft_geom::{BezPath, Line, ParamCurve, PathEl, Point, Vec2};
use fontcraft_model::{Font, LayerParts, MAX_COORDINATE, MAX_OUTLINE_ELEMENTS, NodeKind, Shape, metric};
use serde_json::{Value, json};

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

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "measure.ruler",
            "Measure Along Line",
            [],
            None,
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\", from?: [x, y], to?: [x, y], guide?: {scope: \"local\" | \"global\", index}} where a line crosses the layer's paths and components: the segment from `from` to `to`, or the whole line of a guide. Overlapping paths are measured as merged: crossings inside an overlap are skipped → {glyph, layer, italicAngle, length?, angle?: degrees counter-clockwise, 0 pointing up (with from and to), crossings: [{x, y, distance: along the line from `from` or from the guide's point}], gaps: [{from, to: crossing indices, distance, dx, dy, italicDx, inside: bool (inside the outline)}]}",
            has_doc,
            ruler
        )
        .targets(&[TargetKind::Layer]),
        cmd!(
            query "measure.coordinates",
            "List Point Coordinates",
            [],
            None,
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\"} every node, handle and anchor of the layer with its coordinates, and the X and Y deltas between neighbouring on-curve nodes of each path → {glyph, layer, italicAngle, points: [{shape, node, x, y, onCurve: bool}], deltas: [{shape, from, to: node indices, dx, dy, italicDx}], anchors: [{name, x, y}]}",
            has_doc,
            coordinates
        )
        .targets(&[TargetKind::Layer]),
    ]
}

fn italic_tan(font: &Font, master: &fontcraft_model::Master) -> f64 {
    let t = font.metric_pos(master, metric::ITALIC_ANGLE, 0.0).to_radians().tan();
    if t.is_finite() { t } else { 0.0 }
}

/// A point parameter `[x, y]` within the model's limits.
fn point(p: &Params, key: &str) -> Result<Point> {
    let bad = |found: &Value| p.bad(format!("`{key}` must be [x, y] with finite numbers within ±{MAX_COORDINATE}, got {}", kind(found)));
    let v = p.get(key).ok_or_else(|| p.bad(format!("`{key}` is required")))?;
    let coord = |c: &Value| c.as_f64().filter(|n| n.is_finite() && n.abs() <= MAX_COORDINATE);
    match v.as_array().map(Vec::as_slice) {
        Some([x, y]) => match (coord(x), coord(y)) {
            (Some(x), Some(y)) => Ok(Point::new(x, y)),
            _ => Err(bad(v)),
        },
        _ => Err(bad(v)),
    }
}

/// The contours of one side of a layer with components resolved, each with whether it is
/// closed. Paths that do not make a valid outline and components of missing glyphs are left out.
fn contours(font: &Font, parts: LayerParts, master: &str, p: &Params) -> Result<Vec<(BezPath, bool)>> {
    let mut out = Vec::new();
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
                font.component_outline(c, master)?
            }
            Shape::Image(_) | Shape::Raw(_) => continue,
        };
        budget = budget.checked_sub(part.elements().len()).ok_or_else(|| p.bad("the outline is too large to measure"))?;
        split(&part, &mut out);
    }
    Ok(out)
}

/// Splits a path into its contours.
fn split(path: &BezPath, out: &mut Vec<(BezPath, bool)>) {
    let mut current: Vec<PathEl> = Vec::new();
    let mut closed = false;
    let mut flush = |current: &mut Vec<PathEl>, closed: bool| {
        if current.len() > 1 {
            out.push((BezPath::from_vec(std::mem::take(current)), closed));
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
}

/// A crossing that remains when overlaps are merged, with whether the line is inside the
/// outline after it.
#[derive(Clone, Copy, Debug)]
struct Crossing {
    at: f64,
    point: Point,
    inside: bool,
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
fn hits(contours: &[(BezPath, bool)], origin: Point, dir: Vec2) -> Vec<Hit> {
    let mut reach: f64 = 1.0;
    for (contour, _) in contours {
        for el in contour.elements() {
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
    for (contour, closed) in contours {
        let segs: Vec<_> = contour.segments().collect();
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
            } else if *closed {
                Some(0)
            } else {
                None
            };
            !next.is_some_and(|j| starts.contains(&j))
        });
        for (i, t) in found {
            let Some(seg) = segs.get(i) else { continue };
            let pos = i as f64 + t;
            let before = side_from(&segs, *closed, pos, -1.0, &side);
            let after = side_from(&segs, *closed, pos, 1.0, &side);
            if (before == 0.0 && after == 0.0) || before * after > 0.0 {
                continue;
            }
            let sign = if !*closed {
                0
            } else if after > 0.0 || (after == 0.0 && before < 0.0) {
                1
            } else {
                -1
            };
            let point = seg.eval(t);
            out.push(Hit { at: (point - origin).dot(dir), point, sign });
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
            out.push(Crossing { at: h.at, point: h.point, inside });
        }
    }
    out
}

fn ruler(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("measure.ruler", p)?;
    let t = Targets::parse(&p)?;
    if t.has_lists() {
        return Err(p.bad("takes no target lists: it measures the whole layer"));
    }
    let font = s.font()?;
    let r = t.resolve(font)?;
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
    let shapes = contours(font, r.parts, &r.layer.master, &p)?;
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
    let points: Vec<Value> = crossings.iter().map(|c| json!({"x": c.point.x, "y": c.point.y, "distance": c.at})).collect();
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
    Ok(out)
}

fn coordinates(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("measure.coordinates", p)?;
    let t = Targets::parse(&p)?;
    if t.has_lists() {
        return Err(p.bad("takes no target lists: it lists the whole layer"));
    }
    let font = s.font()?;
    let r = t.resolve(font)?;
    let tan = italic_tan(font, r.master);
    let mut points = Vec::new();
    let mut deltas = Vec::new();
    for (si, shape) in r.parts.shapes.iter().enumerate() {
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
    let anchors: Vec<Value> = r.parts.anchors.iter().map(|a| json!({"name": a.name, "x": a.x, "y": a.y})).collect();
    Ok(json!({
        "glyph": r.glyph.name,
        "layer": r.layer.id,
        "italicAngle": font.metric_pos(r.master, metric::ITALIC_ANGLE, 0.0),
        "points": points,
        "deltas": deltas,
        "anchors": anchors,
    }))
}
