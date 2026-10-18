//! Reporter module drawings (`view.showReporter`, `plugin.kind.reporter`). Each shown reporter
//! that may run draws the edited layer: its background primitives go behind the outline, its
//! foreground primitives in front. Drawings come from the reporter cache in the session's
//! extension state, keyed by module, parameters, layer revision and scale bucket, so a module
//! runs again only when one of them changes.
//!
//! A run that fails is cached as an empty drawing, so the module is not called again every
//! frame, and its error is written at the canvas's top left until a later run succeeds.
//!
//! Coordinates are font units in the layer's space; stroke widths and text sizes are screen
//! points. A filled path fills each of its contours on its own, as convex shapes; a drawing that
//! needs holes or concave fills strokes them instead.

use std::sync::Arc;

use egui::{Align2, Color32, FontId, Pos2, Rect, Shape, Stroke, Vec2};
use fontcraft_engine::plugins::layer_json::LayerInput;
use fontcraft_engine::plugins::reporter::{CacheKey, Drawing, PathCommand, Primitive, Rgba, StrokeStyle, layer_key};
use fontcraft_engine::plugins::{Error, Kind, Limits, Plugin};
use fontcraft_geom::{PathEl, Point};
use fontcraft_model::Glyph;
use fontcraft_tools::View;
use serde_json::{Map, Value};

use super::{Painting, Scene};
use crate::panels::Pending;

/// Most screen points the shown drawings of one frame flatten into, all paths together.
pub const MAX_DRAWN_POINTS: usize = 1_000_000;
/// Most text items drawn in one frame, all reporters together.
pub const MAX_DRAWN_TEXTS: usize = 2_000;
/// Largest text size drawn, in screen points; larger sizes are drawn at this size.
pub const MAX_TEXT_SIZE: f32 = 256.0;
/// Flattening tolerance in screen points for paths of ordinary size.
const TOLERANCE: f64 = 0.25;
/// Points a path's flattening may spend on its longest extent: a path larger than
/// `TOLERANCE × this` on screen is flattened more coarsely.
const EXTENT_STEPS: f64 = 10_000.0;

/// One shown reporter's drawing for this frame.
struct Shown {
    name: String,
    drawing: Arc<Drawing>,
    /// Where the last failed run's error is kept in egui's memory.
    error: egui::Id,
}

/// Draws the shown reporters' background primitives.
pub fn draw_back(ui: &egui::Ui, c: &Painting, scene: &Scene, pending: &mut Pending) {
    let _ = pending;
    let shown = drawings(ui, c, scene);
    let mut budget = Budget::default();
    for s in &shown {
        paint(c, &s.drawing.background, &mut budget);
    }
}

/// Draws the shown reporters' foreground primitives, and the errors of reporters that failed.
pub fn draw_front(ui: &egui::Ui, c: &Painting, scene: &Scene, pending: &mut Pending) {
    let _ = pending;
    let shown = drawings(ui, c, scene);
    let mut budget = Budget::default();
    for s in &shown {
        paint(c, &s.drawing.foreground, &mut budget);
    }
    let line = c.tokens.font_size * 1.4;
    let mut at = c.rect.left_top() + Vec2::splat(c.tokens.font_size);
    for s in &shown {
        if let Some(error) = ui.data(|d| d.get_temp::<String>(s.error)) {
            let text = format!("{}: {error}", s.name);
            c.painter.text(at, Align2::LEFT_TOP, text, FontId::proportional(c.tokens.font_size), c.tokens.error_text);
            at.y += line;
        }
    }
}

/// The drawings of the shown reporters that may run, in install order: from the cache, or run now.
fn drawings(ui: &egui::Ui, c: &Painting, scene: &Scene) -> Vec<Shown> {
    let shown = &scene.state.panels.extensions.reporters;
    if shown.is_empty() {
        return Vec::new();
    }
    let ext = scene.session.extensions();
    let Some(glyph) = scene.tab.edited_glyph().and_then(|g| scene.font.glyph(g)) else { return Vec::new() };
    let scale = c.view.scale();
    // The background, edited as a layer, has the foreground's id: tell the two apart.
    let side = if scene.tab.edit_background { "/background" } else { "" };
    let layer = layer_key(scene.doc.0, &glyph.name, &format!("{}{side}", scene.layer.id), scene.doc.1);
    let mut out = Vec::new();
    for m in ext.registry.list() {
        let id = m.plugin.id();
        if !shown.iter().any(|s| s == id) {
            continue;
        }
        let Ok(plugin) = ext.runnable(id, Kind::Reporter) else { continue };
        let params = plugin.manifest().defaults();
        let key = CacheKey::new(plugin.serial(), &params, layer, scale);
        let error = egui::Id::new(("fontcraft.reporterError", plugin.serial()));
        let mut ran = None;
        let drawing = ext.reporter_cache.get_or_run(key, || match run(scene, glyph, &plugin, &params, scale) {
            Ok(d) => {
                ran = Some(None);
                Ok(d)
            }
            Err(e) => {
                ran = Some(Some(e.to_string()));
                Ok(Drawing::default())
            }
        });
        match ran {
            Some(Some(e)) => {
                ui.data_mut(|d| d.insert_temp(error, e));
            }
            Some(None) => {
                ui.data_mut(|d| d.remove_temp::<String>(error));
            }
            None => {}
        }
        if let Ok(drawing) = drawing {
            out.push(Shown { name: plugin.manifest().name.clone(), drawing, error });
        }
    }
    out
}

/// Runs the reporter on the edited layer at `scale`, under the reporters' limits.
fn run(scene: &Scene, glyph: &Glyph, plugin: &Plugin, params: &Map<String, Value>, scale: f64) -> Result<Drawing, Error> {
    let input = LayerInput::from_layer(scene.font, glyph, scene.layer)?.with_view(scale, true).to_json()?;
    let params = serde_json::to_vec(params).map_err(|e| Error::Output(e.to_string()))?;
    let out = plugin.call_limited(&input, &params, &Limits::for_kind(Kind::Reporter))?;
    Drawing::parse(&out)
}

/// What one frame may still draw.
struct Budget {
    points: usize,
    texts: usize,
}

impl Default for Budget {
    fn default() -> Self {
        Budget { points: MAX_DRAWN_POINTS, texts: MAX_DRAWN_TEXTS }
    }
}

fn color(c: Rgba) -> Color32 {
    let [r, g, b, a] = c;
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

fn stroke(s: &StrokeStyle) -> Stroke {
    Stroke::new(s.width as f32, color(s.color))
}

fn screen(view: View, p: [f64; 2]) -> Point {
    let [x, y] = p;
    view.to_screen(Point::new(x, y))
}

fn pos(p: Point) -> Option<Pos2> {
    let at = Pos2::new(p.x as f32, p.y as f32);
    (at.x.is_finite() && at.y.is_finite()).then_some(at)
}

/// Draws `primitives` until the frame's budget runs out.
fn paint(c: &Painting, primitives: &[Primitive], budget: &mut Budget) {
    let scale = c.view.scale();
    for p in primitives {
        if budget.points == 0 {
            return;
        }
        match p {
            Primitive::Path { commands, fill, stroke: line } => {
                for contour in contours(c, commands, budget) {
                    match (fill, line) {
                        (Some(f), l) if contour.closed && contour.points.len() > 2 => {
                            let s = l.as_ref().map_or(Stroke::NONE, stroke);
                            c.painter.add(Shape::convex_polygon(contour.points, color(*f), s));
                        }
                        (_, Some(l)) => {
                            let points = if contour.closed { closed(contour.points) } else { contour.points };
                            c.painter.add(Shape::line(points, stroke(l)));
                        }
                        _ => {}
                    }
                }
            }
            Primitive::Line { from, to, stroke: line } => {
                if let (Some(a), Some(b)) = (pos(screen(c.view, *from)), pos(screen(c.view, *to))) {
                    c.painter.line_segment([a, b], stroke(line));
                }
            }
            Primitive::Circle { center, radius, fill, stroke: line } => {
                let Some(at) = pos(screen(c.view, *center)) else { continue };
                let r = (radius * scale) as f32;
                if !r.is_finite() || !c.rect.intersects(Rect::from_center_size(at, Vec2::splat(r * 2.0))) {
                    continue;
                }
                if let Some(f) = fill {
                    c.painter.circle_filled(at, r, color(*f));
                }
                if let Some(l) = line {
                    c.painter.circle_stroke(at, r, stroke(l));
                }
            }
            Primitive::Text { at, text, color: fill, size } => {
                if budget.texts == 0 || *size < 1.0 {
                    continue;
                }
                budget.texts -= 1;
                let Some(at) = pos(screen(c.view, *at)) else { continue };
                if !c.rect.contains(at) {
                    continue;
                }
                let size = (*size as f32).min(MAX_TEXT_SIZE);
                c.painter.text(at, Align2::LEFT_BOTTOM, text, FontId::proportional(size), color(*fill));
            }
        }
    }
}

/// `points` with the first point repeated at the end.
fn closed(mut points: Vec<Pos2>) -> Vec<Pos2> {
    if let Some(first) = points.first().copied() {
        points.push(first);
    }
    points
}

/// One flattened contour of a primitive path, in screen points.
struct Contour {
    points: Vec<Pos2>,
    closed: bool,
}

/// The contours of a primitive path on screen, flattened. A path that lies wholly off the canvas
/// gives none; a very large one is flattened more coarsely, so its point count stays bounded.
fn contours(c: &Painting, commands: &[PathCommand], budget: &mut Budget) -> Vec<Contour> {
    let mut els = Vec::with_capacity(commands.len());
    let (mut min, mut max) = (Point::new(f64::INFINITY, f64::INFINITY), Point::new(f64::NEG_INFINITY, f64::NEG_INFINITY));
    let mut grow = |p: Point| {
        min = Point::new(min.x.min(p.x), min.y.min(p.y));
        max = Point::new(max.x.max(p.x), max.y.max(p.y));
        p
    };
    for command in commands {
        els.push(match command {
            PathCommand::MoveTo { to } => PathEl::MoveTo(grow(screen(c.view, *to))),
            PathCommand::LineTo { to } => PathEl::LineTo(grow(screen(c.view, *to))),
            PathCommand::QuadTo { c: c1, to } => PathEl::QuadTo(grow(screen(c.view, *c1)), grow(screen(c.view, *to))),
            PathCommand::CurveTo { c1, c2, to } => PathEl::CurveTo(grow(screen(c.view, *c1)), grow(screen(c.view, *c2)), grow(screen(c.view, *to))),
            PathCommand::Close => PathEl::ClosePath,
        });
    }
    let (Some(lo), Some(hi)) = (pos(min), pos(max)) else { return Vec::new() };
    if !c.rect.intersects(Rect::from_two_pos(lo, hi)) {
        return Vec::new();
    }
    let extent = (max.x - min.x).max(max.y - min.y);
    let tolerance = TOLERANCE.max(extent / EXTENT_STEPS);
    let mut out: Vec<Contour> = Vec::new();
    fontcraft_geom::flatten(els, tolerance, |el| {
        if budget.points == 0 {
            return;
        }
        match el {
            PathEl::MoveTo(p) => {
                if let Some(p) = pos(p) {
                    budget.points -= 1;
                    out.push(Contour { points: vec![p], closed: false });
                }
            }
            PathEl::LineTo(p) => {
                if let (Some(p), Some(last)) = (pos(p), out.last_mut()) {
                    budget.points -= 1;
                    last.points.push(p);
                }
            }
            PathEl::ClosePath => {
                if let Some(last) = out.last_mut() {
                    last.closed = true;
                }
            }
            _ => {}
        }
    });
    out.retain(|c| c.points.len() > 1);
    out
}
