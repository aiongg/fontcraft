//! The Fit Curve section of the Palette (`docs/spec/04-palette.md` § Fit Curve): handle lengths
//! of curve segments as percentages.
//!
//! A handle's percentage is its length over the distance from its node to the point where the
//! two handles' lines cross; when they do not cross ahead of both nodes (parallel handles), over
//! the chord. A segment counts as named when one of its handles is among the points, when both
//! its end nodes are, or when it is in `segments`; both handles change together. Only cubic
//! segments have handles to fit. The eight steps spread evenly between the lower and upper
//! percentage of the range: the `Fit Curve Panel Settings` parameter, else 48 and 74, unless
//! `min` and `max` are given.

use std::collections::BTreeSet;

use fontcraft_geom::{Point, Vec2, round_point};
use fontcraft_model::{Font, LayerParts, NodeKind, Path};
use serde_json::{Value, json};

use super::node::{grid, group};
use super::*;
use crate::params::{Params, TargetKind as K, Targets};

/// The range of the eight steps when the font does not set one (the handbook's example).
const DEFAULT_RANGE: (f64, f64) = (48.0, 74.0);
/// The font parameter that sets the range: two numbers, as text ("52, 78") or a list.
const RANGE_PARAMETER: &str = "Fit Curve Panel Settings";
/// How far a percentage may be from a step and still count as on it, for plus and minus.
const STEP_TOLERANCE: f64 = 0.5;

pub fn specs() -> Vec<CommandSpec> {
    const TARGETS: &[K] = &[K::Layer, K::Points, K::Segments];
    vec![
        cmd!(
            "fitCurve.applyStep",
            "Fit Curve",
            [],
            Some("Ctrl+Alt+1"),
            "{glyph: name, layer?: id, side?, points?: [[shape, node]], segments?: [[shape, node]], step: 1..8, min?: percent, max?: percent} sets both handles of each named curve segment to step 1 to 8 of the range (step 1 is min, step 8 max, the others evenly between; the range defaults to the font's Fit Curve parameter, else 48 to 74) → {segments: count, percent, selection}",
            has_doc,
            apply_step
        )
        .targets(TARGETS)
        .when("editView"),
        cmd!(
            "fitCurve.setHandleLength",
            "Set Handle Length",
            [],
            None,
            "{glyph: name, layer?: id, side?, points?: [[shape, node]], segments?: [[shape, node]], percent: number (clamped to 1..100)} sets both handles of each named curve segment to the percentage → {segments: count, percent, selection}",
            has_doc,
            set_handle_length
        )
        .targets(TARGETS),
        cmd!(
            "fitCurve.increase",
            "Increase Handle Length",
            [],
            None,
            "{glyph: name, layer?: id, side?, points?: [[shape, node]], segments?: [[shape, node]], min?: percent, max?: percent} moves each named curve segment to the next of the eight steps above its mean percentage (at most the top of the range) → {segments: count, selection}",
            has_doc,
            increase
        )
        .targets(TARGETS),
        cmd!(
            "fitCurve.decrease",
            "Decrease Handle Length",
            [],
            None,
            "{glyph: name, layer?: id, side?, points?: [[shape, node]], segments?: [[shape, node]], min?: percent, max?: percent} moves each named curve segment to the step below its mean percentage (at least the bottom of the range) → {segments: count, selection}",
            has_doc,
            decrease
        )
        .targets(TARGETS),
        cmd!(
            "fitCurve.balanceHandles",
            "Balance Handles",
            [],
            None,
            "{glyph: name, layer?: id, side?, points?: [[shape, node]], segments?: [[shape, node]]} sets both handles of each named curve segment to their mean percentage → {segments: count, selection}",
            has_doc,
            balance
        )
        .targets(TARGETS),
        cmd!(
            query "fitCurve.getHandleLength",
            "Get Handle Length",
            [],
            None,
            "{glyph: name, layer?: id, side?, points?: [[shape, node]], segments?: [[shape, node]]} the handle percentages of each named curve segment → {segments: [{segment: [shape, node], percents: [first, second]}], percent: mean of all}",
            has_doc,
            get_handle_length
        )
        .targets(TARGETS),
    ]
}

/// One cubic segment's geometry: its ends, its handles, and for each handle the direction it
/// points in and the length that counts as 100 %.
#[derive(Clone, Copy, Debug)]
struct Frame {
    p0: Point,
    p3: Point,
    u1: Vec2,
    u2: Vec2,
    full1: f64,
    full2: f64,
    h1: Point,
    h2: Point,
}

const EPS: f64 = 1e-9;

fn unit(v: Vec2) -> Option<Vec2> {
    let len = v.hypot();
    (len > EPS && len.is_finite()).then(|| v / len)
}

impl Frame {
    fn new(p0: Point, h1: Point, h2: Point, p3: Point) -> Option<Self> {
        let chord = p3 - p0;
        let u1 = unit(h1 - p0).or_else(|| unit(chord))?;
        let u2 = unit(h2 - p3).or_else(|| unit(-chord))?;
        let cross = u1.cross(u2);
        let (s, r) = if cross.abs() > 1e-9 { (chord.cross(u2) / cross, -u1.cross(chord) / cross) } else { (-1.0, -1.0) };
        let (full1, full2) = if s > EPS && r > EPS && s.is_finite() && r.is_finite() { (s, r) } else { (chord.hypot(), chord.hypot()) };
        (full1 > EPS && full2 > EPS).then_some(Frame { p0, p3, u1, u2, full1, full2, h1, h2 })
    }

    fn percents(&self) -> (f64, f64) {
        ((self.h1 - self.p0).hypot() * 100.0 / self.full1, (self.h2 - self.p3).hypot() * 100.0 / self.full2)
    }

    /// The two handles at the given percentages.
    fn handles(&self, a: f64, b: f64) -> (Point, Point) {
        (self.p0 + self.u1 * (self.full1 * a / 100.0), self.p3 + self.u2 * (self.full2 * b / 100.0))
    }
}

/// A named curve segment: its path, the indices of its handles, and its geometry.
struct Named {
    shape: usize,
    end: usize,
    handles: (usize, usize),
    frame: Frame,
}

/// The cubic segments `t` names in `parts`, in path order.
fn named(parts: LayerParts, t: &Targets) -> Vec<Named> {
    let points = group(&t.points);
    let ends = group(&t.segments);
    let shapes: BTreeSet<usize> = points.keys().chain(ends.keys()).copied().collect();
    let mut out = Vec::new();
    for s in shapes {
        let Some(path) = parts.path(s) else { continue };
        let empty = BTreeSet::new();
        let pts = points.get(&s).unwrap_or(&empty);
        let named_ends = ends.get(&s).unwrap_or(&empty);
        for seg in path.segments() {
            let &[a, b] = seg.handles.as_slice() else { continue };
            if path.nodes.get(seg.end).map(|n| n.kind) != Some(NodeKind::Curve) {
                continue;
            }
            let chosen =
                pts.contains(&a) || pts.contains(&b) || (pts.contains(&seg.start) && pts.contains(&seg.end)) || named_ends.contains(&seg.end);
            if !chosen {
                continue;
            }
            let at = |i: usize| path.nodes.get(i).map(|n| n.point());
            let (Some(p0), Some(h1), Some(h2), Some(p3)) = (at(seg.start), at(a), at(b), at(seg.end)) else { continue };
            if let Some(frame) = Frame::new(p0, h1, h2, p3) {
                out.push(Named { shape: s, end: seg.end, handles: (a, b), frame });
            }
        }
    }
    out
}

/// A percentage from 1 to 100.
fn percent_value(v: f64) -> f64 {
    v.clamp(1.0, 100.0)
}

/// The range of the eight steps: `min` and `max` when given, else the font's parameter, else
/// the default.
fn range(p: &Params, font: &Font) -> Result<(f64, f64)> {
    let (dmin, dmax) = font_range(font).unwrap_or(DEFAULT_RANGE);
    let min = p.opt_f64("min")?.map_or(dmin, percent_value);
    let max = p.opt_f64("max")?.map_or(dmax, percent_value);
    Ok(if min <= max { (min, max) } else { (max, min) })
}

fn font_range(font: &Font) -> Option<(f64, f64)> {
    use fontcraft_model::plist::Value as Plist;
    let v = font.info.params.get(RANGE_PARAMETER)?;
    let numbers: Vec<f64> = match v {
        Plist::String(_) => v.as_str()?.split(',').filter_map(|s| s.trim().parse::<f64>().ok()).collect(),
        Plist::Array(a) => {
            a.items.iter().filter_map(|i| i.as_num().map(|n| n.as_f64()).or_else(|| i.as_str().and_then(|s| s.trim().parse().ok()))).collect()
        }
        _ => return None,
    };
    let &[a, b] = numbers.as_slice() else { return None };
    if !(a.is_finite() && b.is_finite()) {
        return None;
    }
    let (a, b) = (percent_value(a), percent_value(b));
    Some(if a <= b { (a, b) } else { (b, a) })
}

/// The percentage of step `k` (1 to 8) in `range`.
fn step_value((min, max): (f64, f64), k: u8) -> f64 {
    min + f64::from(k.saturating_sub(1)) * (max - min) / 7.0
}

/// Applies `new` (the two percentages a segment gets from its current ones) to every segment `t`
/// names, as one undo step. Returns the result with the count.
fn change(s: &mut Session, p: &Params, label: &str, t: &Targets, new: impl Fn(f64, f64) -> (f64, f64)) -> Result<Value> {
    let step = grid(s.font()?);
    let count = s.edit(label, |font| {
        t.edit_layer(font, |layer| {
            let list = named(layer.parts(t.side), t);
            if list.is_empty() {
                return Err(p.bad("none of the targets is on a curve segment"));
            }
            let mut parts = layer.parts_mut(t.side);
            for n in &list {
                let (a, b) = n.frame.percents();
                let (a, b) = new(a, b);
                let (h1, h2) = n.frame.handles(percent_value(a), percent_value(b));
                let Some(path) = parts.path_mut(n.shape) else { continue };
                set(path, n.handles.0, round_point(h1, step));
                set(path, n.handles.1, round_point(h2, step));
            }
            Ok(list.len())
        })
    })?;
    Ok(json!({ "segments": count, "selection": t.to_json() }))
}

fn set(path: &mut Path, i: usize, p: Point) {
    if let Some(n) = path.nodes.get_mut(i) {
        (n.x, n.y) = (p.x, p.y);
    }
}

fn targets(p: &Params) -> Result<Targets> {
    let t = Targets::parse(p)?;
    if t.points.is_empty() && t.segments.is_empty() {
        return Err(p.bad("give the curve segments: `points` (their handles or both end nodes) or `segments`"));
    }
    Ok(t)
}

fn with_percent(mut v: Value, percent: f64) -> Value {
    if let Some(o) = v.as_object_mut() {
        o.insert("percent".into(), json!(percent));
    }
    v
}

fn apply_step(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("fitCurve.applyStep", v)?;
    let k = p.opt_int("step", 1, 8)?.ok_or_else(|| p.bad("`step` is required"))?;
    let k = u8::try_from(k).map_err(|_| p.bad("`step` must be 1 to 8"))?;
    let t = targets(&p)?;
    let value = step_value(range(&p, s.font()?)?, k);
    let out = change(s, &p, "Fit Curve", &t, |_, _| (value, value))?;
    Ok(with_percent(out, value))
}

fn set_handle_length(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("fitCurve.setHandleLength", v)?;
    let value = percent_value(p.f64("percent")?);
    let t = targets(&p)?;
    let out = change(s, &p, "Set Handle Length", &t, |_, _| (value, value))?;
    Ok(with_percent(out, value))
}

fn increase(s: &mut Session, v: &Value) -> Result<Value> {
    shift_step(s, v, "fitCurve.increase", true)
}

fn decrease(s: &mut Session, v: &Value) -> Result<Value> {
    shift_step(s, v, "fitCurve.decrease", false)
}

fn shift_step(s: &mut Session, v: &Value, command: &'static str, up: bool) -> Result<Value> {
    let p = Params::new(command, v)?;
    let t = targets(&p)?;
    let r = range(&p, s.font()?)?;
    let steps: Vec<f64> = (1..=8).map(|k| step_value(r, k)).collect();
    let next = |a: f64, b: f64| {
        let mean = (a + b) / 2.0;
        let to = if up {
            steps.iter().copied().find(|v| *v > mean + STEP_TOLERANCE).unwrap_or(r.1)
        } else {
            steps.iter().rev().copied().find(|v| *v < mean - STEP_TOLERANCE).unwrap_or(r.0)
        };
        (to, to)
    };
    change(s, &p, if up { "Increase Handle Length" } else { "Decrease Handle Length" }, &t, next)
}

fn balance(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("fitCurve.balanceHandles", v)?;
    let t = targets(&p)?;
    change(s, &p, "Balance Handles", &t, |a, b| ((a + b) / 2.0, (a + b) / 2.0))
}

fn get_handle_length(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("fitCurve.getHandleLength", v)?;
    let t = targets(&p)?;
    let r = t.resolve(s.font()?)?;
    let list = named(r.parts, &t);
    if list.is_empty() {
        return Err(p.bad("none of the targets is on a curve segment"));
    }
    let mut sum = 0.0;
    let segments: Vec<Value> = list
        .iter()
        .map(|n| {
            let (a, b) = n.frame.percents();
            sum += a + b;
            json!({"segment": [n.shape, n.end], "percents": [a, b]})
        })
        .collect();
    let mean = sum / (2.0 * list.len() as f64);
    Ok(json!({ "segments": segments, "percent": mean }))
}
