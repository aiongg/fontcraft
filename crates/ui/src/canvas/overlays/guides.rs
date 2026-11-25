//! Guides, the grid, the metric lines that parameters set, the CJK grids, measurement guides and the Measurement
//! tool's labels (`docs/spec/03a` § 20, `docs/spec/03b` › Measuring).
//!
//! | Item | Drawn as |
//! |---|---|
//! | local guide | line across the canvas, `guide_local` (or the guide's own colour) |
//! | global guide | the same, `guide_global`; only in the glyphs its filter takes in |
//! | every guide | faded by `appearance.guideOpacity` |
//! | guide knob | ring at the guide's position; filled when selected; a padlock when locked |
//! | guide name | at the guide's left end |
//! | measurement guide | dots where it crosses the outline, the distance between neighbouring dots |
//! | grid | lines every grid step, `grid_line`, once the step is `grid_min_spacing` points on screen |
//! | metric line a parameter sets (shoulder height, small-cap height, per-script values) | like a metric line |
//! | CJK grid, design grid | dotted lines, `cjk_grid`; the CJK guide as a box |
//! | Measurement tool | each point's coordinates, the deltas between on-curve nodes, the ruler's angle and crossings, the overlapped stretches of outline |
//!
//! A global guide with a filter (a glyph predicate) shows only in the glyphs it matches
//! (`fontcraft_glyphquery::guide_in_scope`). A filter that does not read as a predicate leaves the guide shown.
//! The Measurement tool covers the edited glyph, the one glyph the canvas shows.

use std::collections::BTreeMap;

use egui::{Align2, Color32, Pos2, Rect, Shape, Stroke};
use fontcraft_engine::Session;
use fontcraft_engine::model::{Font, Guide};
use fontcraft_tools::{GuideRef, GuideScope};
use serde_json::Value;

use super::tokens::OverlayTokens;
use super::{Drawn, num};
use crate::canvas::layers::PaintCx;
use crate::canvas::tools::Target;

/// Most measurement guides measured.
const MAX_MEASURED_GUIDES: usize = 64;
/// Most labels the Measurement tool draws; a larger glyph shows the first ones.
const MAX_LABELS: usize = 5_000;
/// Most segments of overlapped stretches drawn.
const MAX_OVERLAP_SEGMENTS: usize = 20_000;
/// Most lines one grid draws in each direction.
const MAX_GRID_LINES: usize = 2_000;
/// Most entries read from one list of a query's answer.
const MAX_ITEMS: usize = 20_000;

fn f(v: &Value, key: &str) -> f64 {
    v.get(key).and_then(Value::as_f64).filter(|n| n.is_finite()).unwrap_or(0.0)
}

fn opt_f(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64).filter(|n| n.is_finite())
}

fn index(v: &Value, key: &str) -> usize {
    v.get(key).and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).unwrap_or(0)
}

fn list<'a>(v: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    v.get(key).and_then(Value::as_array).into_iter().flatten().take(MAX_ITEMS)
}

fn numbers<const N: usize>(v: Option<&Value>) -> Option<[f64; N]> {
    let a = v?.as_array().filter(|a| a.len() == N)?;
    let mut out = [0.0; N];
    for (slot, item) in out.iter_mut().zip(a) {
        *slot = item.as_f64().filter(|n| n.is_finite())?;
    }
    Some(out)
}

/// A metric line of `guide.displayLines`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MetricLine {
    pub kind: String,
    pub name: Option<String>,
    pub y: f64,
    pub source: String,
}

/// The CJK grid of `guide.displayLines`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cjk {
    pub bounds: [f64; 4],
    pub rows: Option<f64>,
    pub columns: Option<f64>,
    pub guide: Option<[f64; 4]>,
}

/// `guide.displayLines`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DisplayLines {
    pub width: f64,
    pub grid_step: f64,
    pub metrics: Vec<MetricLine>,
    pub cjk: Option<Cjk>,
    pub design_grid: Option<Vec<f64>>,
}

impl DisplayLines {
    pub fn read(v: &Value) -> DisplayLines {
        let metrics = list(v, "metrics")
            .map(|m| MetricLine {
                kind: m.get("kind").and_then(Value::as_str).unwrap_or_default().to_string(),
                name: m.get("name").and_then(Value::as_str).map(str::to_string),
                y: f(m, "y"),
                source: m.get("source").and_then(Value::as_str).unwrap_or_default().to_string(),
            })
            .collect();
        let cjk = v.get("cjk").filter(|c| c.is_object()).map(|c| Cjk {
            bounds: numbers::<4>(c.get("box")).unwrap_or_default(),
            rows: opt_f(c, "rows"),
            columns: opt_f(c, "columns"),
            guide: numbers::<4>(c.get("guide")),
        });
        let design_grid =
            v.get("designGrid").and_then(Value::as_array).map(|a| a.iter().take(2).filter_map(Value::as_f64).filter(|n| n.is_finite()).collect());
        DisplayLines { width: f(v, "width"), grid_step: v.get("grid").map_or(0.0, |g| f(g, "step")), metrics, cjk, design_grid }
    }
}

/// `measure.ruler`: where a line crosses the outline and the gaps between neighbouring crossings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ruler {
    pub crossings: Vec<[f64; 2]>,
    /// From crossing, to crossing, distance.
    pub gaps: Vec<(usize, usize, f64)>,
    pub angle: Option<f64>,
}

impl Ruler {
    pub fn read(v: &Value) -> Ruler {
        Ruler {
            crossings: list(v, "crossings").map(|c| [f(c, "x"), f(c, "y")]).collect(),
            gaps: list(v, "gaps").map(|g| (index(g, "from"), index(g, "to"), f(g, "distance"))).collect(),
            angle: opt_f(v, "angle"),
        }
    }
}

/// A point of `measure.coordinates`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CoordPoint {
    pub shape: usize,
    pub node: usize,
    pub at: [f64; 2],
    pub on_curve: bool,
}

/// `measure.coordinates` of one glyph.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Coordinates {
    pub points: Vec<CoordPoint>,
    /// Shape, from node, to node, italic dx, dy.
    pub deltas: Vec<(usize, usize, usize, f64, f64)>,
    pub anchors: Vec<[f64; 2]>,
    /// Stretches of outline inside an overlap, as polylines.
    pub overlaps: Vec<Vec<[f64; 2]>>,
}

impl Coordinates {
    pub fn read(v: &Value) -> Coordinates {
        Coordinates {
            points: list(v, "points")
                .map(|p| CoordPoint {
                    shape: index(p, "shape"),
                    node: index(p, "node"),
                    at: [f(p, "x"), f(p, "y")],
                    on_curve: p.get("onCurve").and_then(Value::as_bool).unwrap_or(false),
                })
                .collect(),
            deltas: list(v, "deltas").map(|d| (index(d, "shape"), index(d, "from"), index(d, "to"), f(d, "italicDx"), f(d, "dy"))).collect(),
            anchors: list(v, "anchors").map(|a| [f(a, "x"), f(a, "y")]).collect(),
            overlaps: list(v, "overlaps")
                .map(|line| line.as_array().into_iter().flatten().take(MAX_OVERLAP_SEGMENTS).filter_map(|p| numbers::<2>(Some(p))).collect())
                .collect(),
        }
    }
}

/// The global guides of the target's master whose filter leaves the edited glyph out.
pub fn out_of_scope(session: &Session, font: &Font, target: &Target) -> Vec<usize> {
    let Some(glyph) = font.glyph(&target.glyph) else { return Vec::new() };
    let Some(master) = target.layer_in(font).and_then(|l| font.masters.iter().find(|m| m.id == l.master)) else { return Vec::new() };
    if master.guides.iter().all(|g| g.filter.as_deref().is_none_or(|f| f.trim().is_empty())) {
        return Vec::new();
    }
    let Ok(data) = session.glyph_data() else { return Vec::new() };
    let in_scope = |guide: &Guide| {
        let Some(text) = guide.filter.as_deref().map(str::trim).filter(|t| !t.is_empty()) else { return true };
        let Ok(scope) = fontcraft_glyphquery::parse_predicate(text) else { return true };
        let Ok(env) = fontcraft_glyphquery::Env::new(font, &data, Some(&master.id)) else { return true };
        fontcraft_glyphquery::guide_in_scope(Some(&scope), glyph, &env)
    };
    master.guides.iter().enumerate().filter(|(_, g)| !in_scope(g)).map(|(i, _)| i).collect()
}

/// The guides of the target's layer that show their measurement: local, then the global ones in scope.
pub fn measured(session: &Session, target: &Target, out_of_scope: &[usize]) -> Vec<GuideRef> {
    let Ok(font) = session.font() else { return Vec::new() };
    let Some(layer) = target.layer_in(font) else { return Vec::new() };
    let global: &[Guide] = font.masters.iter().find(|m| m.id == layer.master).map(|m| m.guides.as_slice()).unwrap_or_default();
    let local = layer.info.guides.iter().enumerate().map(|(index, g)| (GuideScope::Local, index, g));
    let global = global.iter().enumerate().filter(|(i, _)| !out_of_scope.contains(i)).map(|(index, g)| (GuideScope::Global, index, g));
    local.chain(global).filter(|(_, _, g)| g.show_measurement).map(|(scope, index, _)| GuideRef { scope, index }).take(MAX_MEASURED_GUIDES).collect()
}

fn usable(p: Pos2) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

fn screen(cx: &PaintCx<'_>, x: f64, y: f64) -> Option<Pos2> {
    crate::canvas::paint::screen(cx, [x, y])
}

/// The font-unit rectangle the canvas shows: `[min x, min y, max x, max y]`, whatever the view's rotation.
fn visible(cx: &PaintCx<'_>) -> Option<[f64; 4]> {
    let r = cx.rect;
    let corners = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()].map(|p| cx.camera.to_font(p));
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in corners {
        if !(p.x.is_finite() && p.y.is_finite()) {
            return None;
        }
        (x0, y0, x1, y1) = (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y));
    }
    Some([x0, y0, x1, y1])
}

/// The multiples of `step` from `lo` to `hi`, at most [`MAX_GRID_LINES`] of them.
fn steps(lo: f64, hi: f64, step: f64) -> impl Iterator<Item = f64> {
    let (lo, hi) = (lo.min(hi), lo.max(hi));
    let first = if step > 0.0 && lo.is_finite() { (lo / step).ceil() } else { f64::INFINITY };
    (0..MAX_GRID_LINES).map(move |k| (first + k as f64) * step).take_while(move |v| v.is_finite() && *v <= hi)
}

/// The grid, the metric lines parameters set, the CJK grid and the design grid.
pub fn grid(cx: &PaintCx<'_>) {
    grid_shapes(cx).paint(cx);
}

pub fn grid_shapes(cx: &PaintCx<'_>) -> Drawn {
    let mut out = Drawn::default();
    let Some(lines) = &cx.overlays.data.lines else { return out };
    let Some([x0, y0, x1, y1]) = visible(cx) else { return out };
    let t = OverlayTokens::standard();
    let scale = cx.camera.scale();
    let segment = |a: Option<Pos2>, b: Option<Pos2>, stroke: Stroke, out: &mut Drawn| {
        if let (Some(a), Some(b)) = (a, b) {
            out.shapes.push(Shape::line_segment([a, b], stroke));
        }
    };
    let step = lines.grid_step;
    if step.is_finite() && step > 0.0 && step * scale >= f64::from(t.grid_min_spacing) {
        let stroke = Stroke::new(1.0, t.grid_line);
        for x in steps(x0, x1, step) {
            segment(screen(cx, x, y0), screen(cx, x, y1), stroke, &mut out);
        }
        for y in steps(y0, y1, step) {
            segment(screen(cx, x0, y), screen(cx, x1, y), stroke, &mut out);
        }
    }
    if cx.display.metrics {
        let stroke = Stroke::new(cx.tokens.metric_line_width, cx.tokens.metric_line);
        for line in lines.metrics.iter().filter(|l| l.source == "parameter") {
            let (a, b) = (screen(cx, x0, line.y), screen(cx, x1, line.y));
            segment(a, b, stroke, &mut out);
            if cx.display.metrics_names
                && let Some(left) = screen(cx, 0.0, line.y)
            {
                let name = line.name.as_deref().unwrap_or(&line.kind);
                out.label(
                    Pos2::new(left.x + cx.tokens.label_gap, left.y - 2.0),
                    Align2::LEFT_BOTTOM,
                    format!("{name} {}", num(line.y)),
                    cx.tokens.label_size,
                    cx.tokens.metric_name,
                );
            }
        }
    }
    let dotted = |a: Option<Pos2>, b: Option<Pos2>, out: &mut Drawn| {
        if let (Some(a), Some(b)) = (a, b) {
            out.shapes.extend(Shape::dotted_line(&[a, b], t.cjk_grid, t.dot_spacing, t.dot_radius));
        }
    };
    let ys: Vec<f64> = lines.metrics.iter().map(|l| l.y).collect();
    let body = match &lines.cjk {
        Some(cjk) => Some(cjk.bounds),
        None => ys.iter().copied().reduce(f64::min).zip(ys.iter().copied().reduce(f64::max)).map(|(lo, hi)| [0.0, lo, lines.width, hi]),
    };
    if let Some(cjk) = &lines.cjk {
        let [bx0, by0, bx1, by1] = cjk.bounds;
        for (count, vertical) in [(cjk.columns, true), (cjk.rows, false)] {
            let Some(n) = count.filter(|n| *n >= 1.0 && *n <= MAX_GRID_LINES as f64) else { continue };
            for k in 1..(n as usize) {
                let s = k as f64 / n;
                if vertical {
                    let x = bx0 + (bx1 - bx0) * s;
                    dotted(screen(cx, x, by0), screen(cx, x, by1), &mut out);
                } else {
                    let y = by0 + (by1 - by0) * s;
                    dotted(screen(cx, bx0, y), screen(cx, bx1, y), &mut out);
                }
            }
        }
        if let Some([gx0, gy0, gx1, gy1]) = cjk.guide {
            let corners: Option<Vec<Pos2>> = [(gx0, gy0), (gx1, gy0), (gx1, gy1), (gx0, gy1)].iter().map(|&(x, y)| screen(cx, x, y)).collect();
            if let Some(corners) = corners {
                out.shapes.push(Shape::closed_line(corners, Stroke::new(1.0, t.cjk_grid)));
            }
        }
    }
    if let (Some(values), Some([bx0, by0, bx1, by1])) = (&lines.design_grid, body) {
        let h = values.first().copied().unwrap_or(0.0);
        let v = values.get(1).copied().unwrap_or(h);
        let readable = |s: f64| s.is_finite() && s > 0.0 && s * scale >= f64::from(t.grid_min_spacing);
        if readable(h) {
            for x in steps(bx0, bx1, h) {
                dotted(screen(cx, x, by0), screen(cx, x, by1), &mut out);
            }
        }
        if readable(v) {
            for y in steps(by0, by1, v) {
                dotted(screen(cx, bx0, y), screen(cx, bx1, y), &mut out);
            }
        }
    }
    out
}

/// A guide colour written `r,g,b,a` with numbers from 0 to 1.
pub(super) fn parse_color(text: &str) -> Option<Color32> {
    let parts: Vec<f64> = text.split(',').map(|p| p.trim().parse::<f64>().ok().filter(|v| (0.0..=1.0).contains(v))).collect::<Option<_>>()?;
    let byte = |v: f64| (v * 255.0).round() as u8;
    match parts.as_slice() {
        [r, g, b, a] => Some(Color32::from_rgba_unmultiplied(byte(*r), byte(*g), byte(*b), byte(*a))),
        _ => None,
    }
}

/// The part of the line through `at` along `dir` inside `rect`.
pub(super) fn clip(at: Pos2, dir: egui::Vec2, rect: Rect) -> Option<(Pos2, Pos2)> {
    let (mut lo, mut hi) = (f32::NEG_INFINITY, f32::INFINITY);
    for (d, p, min, max) in [(dir.x, at.x, rect.left(), rect.right()), (dir.y, at.y, rect.top(), rect.bottom())] {
        if d.abs() < 1e-9 {
            if p < min || p > max {
                return None;
            }
            continue;
        }
        let (a, b) = ((min - p) / d, (max - p) / d);
        lo = lo.max(a.min(b));
        hi = hi.min(a.max(b));
    }
    (lo.is_finite() && hi.is_finite() && lo <= hi).then(|| (at + dir * lo, at + dir * hi))
}

/// The guide's line across the canvas, its left end first, and its knob.
fn guide_line(cx: &PaintCx<'_>, g: &Guide) -> Option<(Pos2, Pos2, Pos2)> {
    let a = if g.angle.is_finite() { g.angle.to_radians() } else { 0.0 };
    let at = screen(cx, g.x, g.y)?;
    let ahead = screen(cx, g.x + a.cos(), g.y + a.sin())?;
    let dir = (ahead - at).normalized();
    if !(dir.x.is_finite() && dir.y.is_finite()) {
        return None;
    }
    let (p, q) = clip(at, dir, cx.rect)?;
    Some(if (p.x, -p.y) <= (q.x, -q.y) { (p, q, at) } else { (q, p, at) })
}

fn padlock(at: Pos2, size: f32, color: Color32) -> Vec<Shape> {
    let body = Rect::from_center_size(Pos2::new(at.x, at.y + size * 0.35), egui::vec2(size * 1.6, size * 1.2));
    let shackle: Vec<Pos2> = (0..=8)
        .map(|k| {
            let t = std::f32::consts::PI * k as f32 / 8.0;
            Pos2::new(at.x + size * 0.55 * t.cos(), body.top() - size * 0.7 * t.sin())
        })
        .collect();
    vec![Shape::rect_filled(body, 1.0, color), Shape::line(shackle, Stroke::new(1.5, color))]
}

/// Local and global guides with their knobs and names, and the crossings of measurement guides.
pub fn guides(cx: &PaintCx<'_>) {
    guide_shapes(cx).paint(cx);
}

pub fn guide_shapes(cx: &PaintCx<'_>) -> Drawn {
    let mut out = Drawn::default();
    let Some(glyph) = cx.glyph() else { return out };
    if !cx.display.guides {
        return out;
    }
    let t = OverlayTokens::standard();
    let data = cx.overlays.data;
    let local = glyph.layer.info.guides.iter().enumerate().map(|(index, g)| (GuideRef { scope: GuideScope::Local, index }, g));
    let global = data
        .global_guides
        .iter()
        .enumerate()
        .filter(|(index, _)| !data.out_of_scope.contains(index))
        .map(|(index, g)| (GuideRef { scope: GuideScope::Global, index }, g));
    let mut colors = BTreeMap::new();
    for (r, g) in local.chain(global) {
        let color = g
            .color
            .as_deref()
            .and_then(parse_color)
            .unwrap_or(match r.scope {
                GuideScope::Local => t.guide_local,
                GuideScope::Global => t.guide_global,
            })
            .gamma_multiply(cx.tokens.guide_opacity);
        colors.insert(r, color);
        let Some((left, right, knob)) = guide_line(cx, g) else { continue };
        out.shapes.push(Shape::line_segment([left, right], Stroke::new(t.guide_width, color)));
        if usable(knob) && cx.rect.contains(knob) {
            if g.locked {
                out.shapes.extend(padlock(knob, t.guide_knob_radius, color));
            } else if cx.tools.selection.guides.contains(&r) {
                out.shapes.push(Shape::circle_filled(knob, t.guide_knob_radius, color));
            } else {
                out.shapes.push(Shape::circle_stroke(knob, t.guide_knob_radius, Stroke::new(t.guide_width, color)));
            }
        }
        if let Some(name) = g.name.as_deref().filter(|n| !n.is_empty()) {
            out.label(Pos2::new(left.x + t.label_offset, left.y - 2.0), Align2::LEFT_BOTTOM, name.to_string(), t.guide_name_size, color);
        }
    }
    for (r, ruler) in &data.guide_rulers {
        let color = colors.get(r).copied().unwrap_or(t.measure_ruler);
        crossings(cx, ruler, color, &mut out);
    }
    out
}

fn crossings(cx: &PaintCx<'_>, ruler: &Ruler, color: Color32, out: &mut Drawn) {
    let t = OverlayTokens::standard();
    let points: Vec<Option<Pos2>> = ruler.crossings.iter().map(|[x, y]| screen(cx, *x, *y)).collect();
    for p in points.iter().flatten() {
        out.shapes.push(Shape::circle_filled(*p, t.measure_crossing_radius, color));
    }
    for (from, to, distance) in ruler.gaps.iter().take(MAX_LABELS) {
        let (Some(Some(a)), Some(Some(b))) = (points.get(*from), points.get(*to)) else { continue };
        let mid = a.lerp(*b, 0.5);
        out.label(Pos2::new(mid.x, mid.y - t.label_offset), Align2::CENTER_BOTTOM, num(*distance), t.measure_label_size, color);
    }
}

/// The Measurement tool's labels: overlapped stretches, coordinates, anchors, deltas, and the ruler.
pub fn measurement(cx: &PaintCx<'_>) {
    measurement_shapes(cx).paint(cx);
}

pub fn measurement_shapes(cx: &PaintCx<'_>) -> Drawn {
    let mut out = Drawn::default();
    let t = OverlayTokens::standard();
    let data = cx.overlays.data;
    if let Some(coords) = &data.coordinates {
        let overlap = Stroke::new(cx.tokens.outline_width, t.measure_overlap);
        let mut segments = 0usize;
        'lines: for line in &coords.overlaps {
            let pts: Vec<Option<Pos2>> = line.iter().map(|[x, y]| screen(cx, *x, *y)).collect();
            for pair in pts.windows(2) {
                segments = segments.saturating_add(1);
                if segments > MAX_OVERLAP_SEGMENTS {
                    break 'lines;
                }
                if let [Some(a), Some(b)] = pair {
                    out.shapes.push(Shape::line_segment([*a, *b], overlap));
                }
            }
        }
        let mut at: BTreeMap<(usize, usize), Pos2> = BTreeMap::new();
        let off = egui::vec2(t.label_offset, -t.label_offset);
        let mut labels = 0usize;
        for p in coords.points.iter().take(MAX_LABELS) {
            let [x, y] = p.at;
            let Some(s) = screen(cx, x, y) else { continue };
            at.insert((p.shape, p.node), s);
            let color = if p.on_curve { t.measure_on_curve } else { t.measure_off_curve };
            let [x, y] = p.at;
            out.label(s + off, Align2::LEFT_BOTTOM, format!("{}, {}", num(x), num(y)), t.measure_label_size, color);
            labels += 1;
        }
        for [x, y] in coords.anchors.iter().take(MAX_LABELS.saturating_sub(labels)) {
            if let Some(s) = screen(cx, *x, *y) {
                out.label(s + off, Align2::LEFT_BOTTOM, format!("{}, {}", num(*x), num(*y)), t.measure_label_size, t.measure_on_curve);
                labels += 1;
            }
        }
        for (shape, from, to, dx, dy) in coords.deltas.iter().take(MAX_LABELS.saturating_sub(labels)) {
            let (Some(a), Some(b)) = (at.get(&(*shape, *from)), at.get(&(*shape, *to))) else { continue };
            out.label(a.lerp(*b, 0.5), Align2::CENTER_CENTER, format!("{}, {}", num(*dx), num(*dy)), t.measure_label_size, t.measure_delta);
        }
    }
    if let Some((_, to, ruler)) = &data.ruler {
        if let (Some(b), Some(angle)) = (screen(cx, to.x, to.y), ruler.angle) {
            let at = Pos2::new(b.x + t.label_offset, b.y - t.label_offset);
            out.label(at, Align2::LEFT_BOTTOM, format!("{}°", num(angle)), t.measure_label_size, t.measure_ruler);
        }
        crossings(cx, ruler, t.measure_ruler, &mut out);
    }
    out
}
