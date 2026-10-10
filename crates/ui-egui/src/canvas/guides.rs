//! Guides, the grid, the metric lines that parameters set, the CJK grids, measurement guides
//! and the Measurement tool's labels (`docs/spec/03a` § 20, `docs/spec/03b` › Measuring).
//!
//! | Item | Drawn as |
//! |---|---|
//! | local guide | line across the canvas, `guide_local` (or the guide's own colour) |
//! | global guide | the same, `guide_global` |
//! | guide knob | ring at the guide's position; filled when selected; a padlock when locked |
//! | guide name | at the guide's left end |
//! | measurement guide | dots where it crosses the outline, the distance between neighbouring dots |
//! | grid | lines every grid step, `grid_line`, once the step is `grid_min_spacing` points on screen |
//! | metric line a parameter sets (shoulder height, small-cap height, per-script values) | like a metric line |
//! | CJK grid, design grid | dotted lines, `cjk_grid`; the CJK guide as a box |
//! | Measurement tool | each point's coordinates, the deltas between on-curve nodes, the ruler |
//!
//! The lines, crossings and coordinates come from engine queries (`guide.displayLines`,
//! `measure.ruler`, `measure.coordinates`), run once per frame by [`Extras::query`] before the
//! frame borrows the font.

use std::collections::BTreeMap;

use egui::{Align2, Color32, FontId, Pos2, Rect, Shape, Stroke};
use fontcraft_engine::Session;
use fontcraft_geom::Point;
use fontcraft_model::{Font, Guide, Layer};
use fontcraft_tools::{GuideRef, GuideScope, Overlay, OverlayKind, Selection};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use super::Painting;
use crate::state::{DisplayOptions, Tab};

/// Most measurement guides measured per frame.
const MAX_MEASURED_GUIDES: usize = 64;
/// Most labels the Measurement tool draws per frame; a larger glyph shows the first ones.
const MAX_LABELS: usize = 5_000;
/// Most lines one grid draws in each direction.
const MAX_GRID_LINES: usize = 2_000;
/// Gap between the dots of a dotted line, and their radius, in points.
const DOT_SPACING: f32 = 4.0;
const DOT_RADIUS: f32 = 0.75;
/// Distance of a label from the point it names, in points.
const LABEL_OFFSET: f32 = 5.0;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Grid {
    step: f64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct MetricLine {
    kind: String,
    name: Option<String>,
    y: f64,
    source: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Cjk {
    #[serde(rename = "box")]
    bounds: [f64; 4],
    rows: Option<f64>,
    columns: Option<f64>,
    guide: Option<[f64; 4]>,
}

/// `guide.displayLines`.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct DisplayLines {
    width: f64,
    grid: Grid,
    metrics: Vec<MetricLine>,
    cjk: Option<Cjk>,
    #[serde(rename = "designGrid")]
    design_grid: Option<Vec<f64>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Crossing {
    x: f64,
    y: f64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Gap {
    from: usize,
    to: usize,
    distance: f64,
}

/// `measure.ruler`.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Ruler {
    crossings: Vec<Crossing>,
    gaps: Vec<Gap>,
    angle: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct CoordPoint {
    shape: usize,
    node: usize,
    x: f64,
    y: f64,
    #[serde(rename = "onCurve")]
    on_curve: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Delta {
    shape: usize,
    from: usize,
    to: usize,
    dy: f64,
    #[serde(rename = "italicDx")]
    italic_dx: f64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct AnchorPoint {
    x: f64,
    y: f64,
}

/// `measure.coordinates`.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Coordinates {
    points: Vec<CoordPoint>,
    deltas: Vec<Delta>,
    anchors: Vec<AnchorPoint>,
}

/// What this module draws, read from the engine once per frame.
#[derive(Debug, Default)]
pub struct Extras {
    lines: Option<DisplayLines>,
    /// Measurement guides and where they cross the outline.
    guide_rulers: Vec<(GuideRef, Ruler)>,
    /// The Measurement tool's labels, while it is the active tool.
    coordinates: Option<Coordinates>,
    /// The Measurement tool's ruler and where it crosses the outline.
    ruler: Option<(Point, Point, Ruler)>,
}

fn query_as<T: DeserializeOwned>(session: &mut Session, id: &str, params: Value) -> Option<T> {
    session.execute(id, &params).ok().and_then(|v| serde_json::from_value(v).ok())
}

/// `target` with `key` set to `value`.
fn with(target: &Map<String, Value>, key: &str, value: Value) -> Value {
    let mut m = target.clone();
    m.insert(key.into(), value);
    Value::Object(m)
}

impl Extras {
    /// Runs the queries the frame draws from: the display lines of the edited layer, the
    /// crossings of its measurement guides (while guides show), and, while `tool` is the
    /// Measurement tool, the coordinates and the crossings of its ruler (from `overlays`).
    pub fn query(session: &mut Session, tab: &Tab, display: &DisplayOptions, tool: &str, overlays: &[Overlay]) -> Extras {
        let mut out = Extras::default();
        let Some(glyph) = tab.edited_glyph() else { return out };
        let (layer, measured) = {
            let Ok(font) = session.font() else { return out };
            let Some(master) = super::view::active_master(font) else { return out };
            let Some(layer) = font.glyph(glyph).and_then(|g| g.layer(&master.id)) else { return out };
            let mut measured = Vec::new();
            if display.guides {
                let local = layer.info.guides.iter().enumerate().map(|(index, g)| (GuideScope::Local, index, g));
                let global = master.guides.iter().enumerate().map(|(index, g)| (GuideScope::Global, index, g));
                measured = local
                    .chain(global)
                    .filter(|(_, _, g)| g.show_measurement)
                    .map(|(scope, index, _)| GuideRef { scope, index })
                    .take(MAX_MEASURED_GUIDES)
                    .collect();
            }
            (layer.id.clone(), measured)
        };
        let mut target = Map::new();
        target.insert("glyph".into(), json!(glyph));
        target.insert("layer".into(), json!(layer));
        out.lines = query_as(session, "guide.displayLines", Value::Object(target.clone()));
        for g in measured {
            let params = with(&target, "guide", json!({"scope": g.scope.as_str(), "index": g.index}));
            if let Some(r) = query_as(session, "measure.ruler", params) {
                out.guide_rulers.push((g, r));
            }
        }
        if tool == "measurement" {
            out.coordinates = query_as(session, "measure.coordinates", Value::Object(target.clone()));
            let ruler = overlays.iter().find_map(|o| match o {
                Overlay::Line { kind: OverlayKind::Ruler, from, to } => Some((*from, *to)),
                _ => None,
            });
            if let Some((from, to)) = ruler {
                let mut params = target.clone();
                params.insert("from".into(), json!([from.x, from.y]));
                params.insert("to".into(), json!([to.x, to.y]));
                if let Some(r) = query_as(session, "measure.ruler", Value::Object(params)) {
                    out.ruler = Some((from, to, r));
                }
            }
        }
        out
    }
}

/// A number as a label: at most two decimals, without trailing zeros.
pub fn num(v: f64) -> String {
    if !v.is_finite() {
        return "?".into();
    }
    let r = (v * 100.0).round() / 100.0;
    let text = format!("{r:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" { "0".into() } else { text.to_string() }
}

fn pos(p: Point) -> Pos2 {
    Pos2::new(p.x as f32, p.y as f32)
}

fn usable(p: Pos2) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

fn screen(c: &Painting, x: f64, y: f64) -> Pos2 {
    pos(c.view.to_screen(Point::new(x, y)))
}

/// A colour from a guide's `r,g,b,a` text (each from 0 to 1).
fn parse_color(text: &str) -> Option<Color32> {
    let parts: Vec<f64> = text.split(',').map(|p| p.trim().parse::<f64>().ok().filter(|v| (0.0..=1.0).contains(v))).collect::<Option<_>>()?;
    let byte = |v: f64| (v * 255.0).round() as u8;
    match parts.as_slice() {
        [r, g, b, a] => Some(Color32::from_rgba_unmultiplied(byte(*r), byte(*g), byte(*b), byte(*a))),
        _ => None,
    }
}

/// The part of the line through `at` with direction `dir` (screen points) inside `rect`
/// (Liang–Barsky clipping), or `None` when it misses the rectangle.
fn clip(at: Pos2, dir: (f32, f32), rect: Rect) -> Option<(Pos2, Pos2)> {
    let (mut lo, mut hi) = (f32::NEG_INFINITY, f32::INFINITY);
    for (d, p, min, max) in [(dir.0, at.x, rect.left(), rect.right()), (dir.1, at.y, rect.top(), rect.bottom())] {
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
    (lo.is_finite() && hi.is_finite() && lo <= hi)
        .then(|| (Pos2::new(at.x + lo * dir.0, at.y + lo * dir.1), Pos2::new(at.x + hi * dir.0, at.y + hi * dir.1)))
}

/// A guide's line on the canvas, its left end first (the lower end of a vertical guide).
fn guide_line(c: &Painting, g: &Guide) -> Option<(Pos2, Pos2)> {
    let at = screen(c, g.x, g.y);
    let a = if g.angle.is_finite() { g.angle.to_radians() } else { 0.0 };
    if !usable(at) {
        return None;
    }
    // Screen y grows downwards.
    let (a, b) = clip(at, (a.cos() as f32, -(a.sin() as f32)), c.rect)?;
    Some(if (a.x, -a.y) <= (b.x, -b.y) { (a, b) } else { (b, a) })
}

/// A padlock: a body and a shackle, centred on `at`.
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

/// Behind the outline: the grid, the metric lines parameters set, the CJK and design grids.
pub fn draw_lines(c: &Painting, extras: &Extras, display: &DisplayOptions) {
    let Some(lines) = &extras.lines else { return };
    let tokens = c.tokens;
    let rect = c.rect;
    let (top_left, bottom_right) = (
        c.view.to_font(Point::new(f64::from(rect.left()), f64::from(rect.top()))),
        c.view.to_font(Point::new(f64::from(rect.right()), f64::from(rect.bottom()))),
    );
    let scale = c.view.scale();

    // The grid, once its step is wide enough on screen to read.
    let step = lines.grid.step;
    if step.is_finite() && step > 0.0 && step * scale >= f64::from(tokens.grid_min_spacing) {
        let stroke = Stroke::new(1.0, tokens.grid_line);
        for x in steps(top_left.x, bottom_right.x, step) {
            let sx = screen(c, x, 0.0).x;
            c.painter.line_segment([Pos2::new(sx, rect.top()), Pos2::new(sx, rect.bottom())], stroke);
        }
        for y in steps(bottom_right.y, top_left.y, step) {
            let sy = screen(c, 0.0, y).y;
            c.painter.line_segment([Pos2::new(rect.left(), sy), Pos2::new(rect.right(), sy)], stroke);
        }
    }

    if display.metrics {
        let stroke = Stroke::new(tokens.metric_line_width, tokens.metric_line);
        for line in lines.metrics.iter().filter(|l| l.source == "parameter" && l.y.is_finite()) {
            let y = screen(c, 0.0, line.y).y;
            c.painter.line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)], stroke);
            if display.metrics_names {
                let name = line.name.as_deref().unwrap_or(&line.kind);
                let font = FontId::proportional(tokens.metric_name_size);
                c.painter.text(
                    Pos2::new(rect.left() + 4.0, y - 2.0),
                    Align2::LEFT_BOTTOM,
                    format!("{name} {}", num(line.y)),
                    font,
                    tokens.metric_name,
                );
            }
        }
    }

    let dotted = |a: Pos2, b: Pos2| {
        if usable(a) && usable(b) {
            c.painter.extend(Shape::dotted_line(&[a, b], tokens.cjk_grid, DOT_SPACING, DOT_RADIUS));
        }
    };
    let ys: Vec<f64> = lines.metrics.iter().map(|l| l.y).filter(|y| y.is_finite()).collect();
    let body = match &lines.cjk {
        Some(cjk) => Some(cjk.bounds),
        None => ys.iter().copied().reduce(f64::min).zip(ys.iter().copied().reduce(f64::max)).map(|(lo, hi)| [0.0, lo, lines.width, hi]),
    };
    if let Some(cjk) = &lines.cjk {
        let [x0, y0, x1, y1] = cjk.bounds;
        for (count, vertical) in [(cjk.columns, true), (cjk.rows, false)] {
            let Some(n) = count.filter(|n| n.is_finite() && *n >= 1.0 && *n <= MAX_GRID_LINES as f64) else { continue };
            for k in 1..(n as usize) {
                let t = k as f64 / n;
                if vertical {
                    let x = x0 + (x1 - x0) * t;
                    dotted(screen(c, x, y0), screen(c, x, y1));
                } else {
                    let y = y0 + (y1 - y0) * t;
                    dotted(screen(c, x0, y), screen(c, x1, y));
                }
            }
        }
        if let Some([gx0, gy0, gx1, gy1]) = cjk.guide {
            let corners = vec![screen(c, gx0, gy0), screen(c, gx1, gy0), screen(c, gx1, gy1), screen(c, gx0, gy1)];
            if corners.iter().all(|p| usable(*p)) {
                c.painter.add(Shape::closed_line(corners, Stroke::new(1.0, tokens.cjk_grid)));
            }
        }
    }
    if let (Some(values), Some([x0, y0, x1, y1])) = (&lines.design_grid, body) {
        let h = values.first().copied().unwrap_or(0.0);
        let v = values.get(1).copied().unwrap_or(h);
        let readable = |s: f64| s.is_finite() && s > 0.0 && s * scale >= f64::from(tokens.grid_min_spacing);
        if readable(h) {
            for x in steps(x0, x1, h) {
                dotted(screen(c, x, y0), screen(c, x, y1));
            }
        }
        if readable(v) {
            for y in steps(y0, y1, v) {
                dotted(screen(c, x0, y), screen(c, x1, y));
            }
        }
    }
}

/// The multiples of `step` from `lo` to `hi`, at most [`MAX_GRID_LINES`] of them.
fn steps(lo: f64, hi: f64, step: f64) -> impl Iterator<Item = f64> {
    let (lo, hi) = (lo.min(hi), lo.max(hi));
    let first = if step > 0.0 && lo.is_finite() { (lo / step).ceil() } else { f64::INFINITY };
    (0..MAX_GRID_LINES).map(move |k| (first + k as f64) * step).take_while(move |v| v.is_finite() && *v <= hi)
}

/// Over the outline: the guides of `layer` and of its master, with knobs and names, and what
/// measurement guides measure.
pub fn draw_guides(c: &Painting, font: &Font, layer: &Layer, selection: &Selection, extras: &Extras, display: &DisplayOptions) {
    if !display.guides {
        return;
    }
    let tokens = c.tokens;
    let global: &[Guide] = font.master(&layer.master).map(|m| m.guides.as_slice()).unwrap_or_default();
    let local = layer.info.guides.iter().enumerate().map(|(index, g)| (GuideRef { scope: GuideScope::Local, index }, g));
    let global = global.iter().enumerate().map(|(index, g)| (GuideRef { scope: GuideScope::Global, index }, g));
    let color_of = |r: GuideRef, g: &Guide| {
        g.color.as_deref().and_then(parse_color).unwrap_or(match r.scope {
            GuideScope::Local => tokens.guide_local,
            GuideScope::Global => tokens.guide_global,
        })
    };
    let mut colors = BTreeMap::new();
    for (r, g) in local.chain(global) {
        let color = color_of(r, g);
        colors.insert(r, color);
        let Some((left, right)) = guide_line(c, g) else { continue };
        c.painter.line_segment([left, right], Stroke::new(tokens.guide_width, color));
        let knob = screen(c, g.x, g.y);
        if usable(knob) {
            if g.locked {
                c.painter.extend(padlock(knob, tokens.guide_knob_radius, color));
            } else if selection.guides.contains(&r) {
                c.painter.circle_filled(knob, tokens.guide_knob_radius, color);
            } else {
                c.painter.circle_stroke(knob, tokens.guide_knob_radius, Stroke::new(tokens.guide_width, color));
            }
        }
        if let Some(name) = g.name.as_deref().filter(|n| !n.is_empty()) {
            let font = FontId::proportional(tokens.guide_name_size);
            c.painter.text(Pos2::new(left.x + LABEL_OFFSET, left.y - 2.0), Align2::LEFT_BOTTOM, name, font, color);
        }
    }
    for (r, ruler) in &extras.guide_rulers {
        let color = colors.get(r).copied().unwrap_or(tokens.measure_ruler);
        draw_crossings(c, ruler, color);
    }
}

/// Dots at a ruler's crossings and the distance of each gap at its middle.
fn draw_crossings(c: &Painting, ruler: &Ruler, color: Color32) {
    let tokens = c.tokens;
    let points: Vec<Pos2> = ruler.crossings.iter().map(|p| screen(c, p.x, p.y)).collect();
    for p in points.iter().filter(|p| usable(**p)) {
        c.painter.circle_filled(*p, tokens.measure_crossing_radius, color);
    }
    let font = FontId::proportional(tokens.measure_label_size);
    for gap in ruler.gaps.iter().take(MAX_LABELS) {
        let (Some(a), Some(b)) = (points.get(gap.from), points.get(gap.to)) else { continue };
        let mid = Pos2::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
        if usable(mid) {
            c.painter.text(Pos2::new(mid.x, mid.y - LABEL_OFFSET), Align2::CENTER_BOTTOM, num(gap.distance), font.clone(), color);
        }
    }
}

/// Over everything: the Measurement tool's coordinates, deltas and ruler.
pub fn draw_measurement(c: &Painting, extras: &Extras) {
    let tokens = c.tokens;
    let font = FontId::proportional(tokens.measure_label_size);
    if let Some(coords) = &extras.coordinates {
        let mut at: BTreeMap<(usize, usize), Pos2> = BTreeMap::new();
        for p in coords.points.iter().take(MAX_LABELS) {
            let s = screen(c, p.x, p.y);
            at.insert((p.shape, p.node), s);
            if usable(s) {
                let color = if p.on_curve { tokens.measure_on_curve } else { tokens.measure_off_curve };
                c.painter.text(
                    Pos2::new(s.x + LABEL_OFFSET, s.y - LABEL_OFFSET),
                    Align2::LEFT_BOTTOM,
                    format!("{}, {}", num(p.x), num(p.y)),
                    font.clone(),
                    color,
                );
            }
        }
        for a in coords.anchors.iter().take(MAX_LABELS) {
            let s = screen(c, a.x, a.y);
            if usable(s) {
                c.painter.text(
                    Pos2::new(s.x + LABEL_OFFSET, s.y - LABEL_OFFSET),
                    Align2::LEFT_BOTTOM,
                    format!("{}, {}", num(a.x), num(a.y)),
                    font.clone(),
                    tokens.measure_on_curve,
                );
            }
        }
        for d in coords.deltas.iter().take(MAX_LABELS) {
            let (Some(a), Some(b)) = (at.get(&(d.shape, d.from)), at.get(&(d.shape, d.to))) else { continue };
            let mid = Pos2::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
            if usable(mid) {
                c.painter.text(mid, Align2::CENTER_CENTER, format!("{}, {}", num(d.italic_dx), num(d.dy)), font.clone(), tokens.measure_delta);
            }
        }
    }
    if let Some((from, to, ruler)) = &extras.ruler {
        let (a, b) = (pos(c.view.to_screen(*from)), pos(c.view.to_screen(*to)));
        if usable(a) && usable(b) {
            c.painter.line_segment([a, b], Stroke::new(1.0, tokens.measure_ruler));
            if let Some(angle) = ruler.angle {
                c.painter.text(
                    Pos2::new(b.x + LABEL_OFFSET, b.y - LABEL_OFFSET),
                    Align2::LEFT_BOTTOM,
                    format!("{}°", num(angle)),
                    font.clone(),
                    tokens.measure_ruler,
                );
            }
        }
        draw_crossings(c, ruler, tokens.measure_ruler);
    }
}
