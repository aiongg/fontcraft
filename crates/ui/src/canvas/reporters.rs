//! Reporter module drawings (`view.showReporter`). Each shown reporter that may run draws the edited layer: its
//! background primitives behind the outline (`reportersBack`), its foreground primitives in front of it
//! (`reportersFront`).
//!
//! [`refresh`] runs after `drain` and reads the drawings into `OverlayData::reporters`. They come from the reporter
//! cache in the session's extension state, keyed by module, parameters, layer revision and scale bucket, so a module
//! runs again only when one of them changes. A run that fails is cached as an empty drawing, so the module is not
//! called again every frame, and its error is written at the canvas's top left until a later run succeeds.
//!
//! Coordinates are font units in the layer's space; stroke widths and text sizes are screen points, and colours are
//! the module's own. A filled path fills each of its contours on its own, as convex shapes; a drawing that needs
//! holes or concave fills strokes them instead.

use std::borrow::Cow;
use std::sync::Arc;

use egui::{Align2, Color32, FontId, Pos2, Rect, Shape, Stroke, Vec2};
use fontcraft_engine::model::{Font, Glyph, Layer, LayerSide};
use fontcraft_engine::plugins::layer_json::LayerInput;
use fontcraft_engine::plugins::reporter::{CacheKey, Drawing, PathCommand, Primitive, Rgba, StrokeStyle, layer_key};
use fontcraft_engine::plugins::{Error, Kind, Limits, Plugin};
use fontcraft_geom::{PathEl, Point};
use serde_json::{Map, Value};

use super::layers::PaintCx;
use super::overlays::tokens::OverlayTokens;
use crate::app::FontCraftApp;

/// Most screen points the shown drawings of one frame flatten into, all paths together.
pub const MAX_DRAWN_POINTS: usize = 1_000_000;
/// Most text items drawn in one frame, all reporters together.
pub const MAX_DRAWN_TEXTS: usize = 2_000;
/// Largest text size drawn, in screen points; larger sizes are drawn at this size.
pub const MAX_TEXT_SIZE: f32 = 256.0;
/// Longest error text kept per reporter, in characters.
const MAX_ERROR_CHARS: usize = 500;
/// Flattening tolerance in screen points for paths of ordinary size.
const TOLERANCE: f64 = 0.25;
/// Points a path's flattening may spend on its longest extent: a path larger than `TOLERANCE × this` on screen is
/// flattened more coarsely.
const EXTENT_STEPS: f64 = 10_000.0;
/// The error lines' spacing, in text sizes.
const ERROR_LINE: f32 = 1.4;

/// One shown reporter's drawing of the edited layer.
#[derive(Clone, Debug, PartialEq)]
pub struct ReporterDrawing {
    pub name: String,
    pub drawing: Arc<Drawing>,
    /// The last failed run's error, until a run succeeds.
    pub error: Option<String>,
}

/// Brings `OverlayData::reporters` up to date with the shown reporters and the edited layer. Runs after `drain`.
pub fn refresh(app: &mut FontCraftApp) {
    let drawn = drawings(app);
    if app.overlays.data.reporters != drawn {
        app.overlays.data.reporters = drawn;
    }
}

/// The drawings of the shown reporters that may run, in install order: from the cache, or run now. Records each run's
/// error in the extensions' view state.
fn drawings(app: &mut FontCraftApp) -> Vec<ReporterDrawing> {
    if app.view.extensions.reporters.is_empty() {
        return Vec::new();
    }
    let Some(target) = super::tools::target(&app.session, &app.view) else { return Vec::new() };
    let Some(doc) = app.session.active() else { return Vec::new() };
    let font: &Font = &doc.font;
    let Some(glyph) = font.glyph(&target.glyph) else { return Vec::new() };
    let Some(layer) = glyph.layer(&target.layer) else { return Vec::new() };
    // The background, edited as a layer, has the foreground's id: tell the two apart.
    let (layer, side) = match target.side {
        LayerSide::Foreground => (Cow::Borrowed(layer), ""),
        LayerSide::Background => {
            let mut back = layer.clone();
            back.shapes = layer.info.background.as_ref().map(|b| b.shapes.clone()).unwrap_or_default();
            (Cow::Owned(back), "/background")
        }
    };
    let scale = app.view.canvas.zoom;
    let key_layer = layer_key(doc.id, &glyph.name, &format!("{}{side}", target.layer), doc.revision);
    let shown = &app.view.extensions.reporters;
    let errors = &mut app.view.extensions.reporter_errors;
    let ext = app.session.extensions();
    let mut out = Vec::new();
    for m in ext.registry.list() {
        let id = m.plugin.id();
        if !shown.iter().any(|s| s == id) {
            continue;
        }
        let Ok(plugin) = ext.runnable(id, Kind::Reporter) else { continue };
        let params = plugin.manifest().defaults();
        let key = CacheKey::new(plugin.serial(), &params, key_layer, scale);
        let mut ran = None;
        let drawing = ext.reporter_cache.get_or_run(key, || match run(font, glyph, &layer, &plugin, &params, scale) {
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
                errors.insert(id.to_string(), e.chars().take(MAX_ERROR_CHARS).collect());
            }
            Some(None) => {
                errors.remove(id);
            }
            None => {}
        }
        if let Ok(drawing) = drawing {
            out.push(ReporterDrawing { name: plugin.manifest().name.clone(), drawing, error: errors.get(id).cloned() });
        }
    }
    // Errors of reporters no longer shown are dropped.
    errors.retain(|id, _| shown.contains(id));
    out
}

/// Runs the reporter on the edited layer at `scale`, under the reporters' limits.
fn run(font: &Font, glyph: &Glyph, layer: &Layer, plugin: &Plugin, params: &Map<String, Value>, scale: f64) -> Result<Drawing, Error> {
    let input = LayerInput::from_layer(font, glyph, layer)?.with_view(scale, true).to_json()?;
    let params = serde_json::to_vec(params).map_err(|e| Error::Output(e.to_string()))?;
    let out = plugin.call_limited(&input, &params, &Limits::for_kind(Kind::Reporter))?;
    Drawing::parse(&out)
}

/// Paint layer `reportersBack`: the shown reporters' background primitives.
pub fn back(cx: &PaintCx<'_>) {
    let mut budget = Budget::default();
    for shown in &cx.overlays.data.reporters {
        paint(cx, &shown.drawing.background, &mut budget);
    }
}

/// Paint layer `reportersFront`: the shown reporters' foreground primitives, and the errors of reporters that failed.
pub fn front(cx: &PaintCx<'_>) {
    let reporters = &cx.overlays.data.reporters;
    let mut budget = Budget::default();
    for shown in reporters {
        paint(cx, &shown.drawing.foreground, &mut budget);
    }
    let t = OverlayTokens::standard();
    let size = cx.tokens.label_size;
    let mut at = cx.rect.left_top() + Vec2::splat(size);
    for shown in reporters {
        if let Some(error) = &shown.error {
            cx.painter.text(at, Align2::LEFT_TOP, format!("{}: {error}", shown.name), FontId::proportional(size), t.audit_fail);
            at.y += size * ERROR_LINE;
        }
    }
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
    let width = if s.width.is_finite() { s.width.max(0.0) as f32 } else { 0.0 };
    Stroke::new(width, color(s.color))
}

/// A point in font units on screen, as a geometry point for flattening.
fn screen(cx: &PaintCx<'_>, p: [f64; 2]) -> Point {
    let [x, y] = p;
    let at = cx.camera.to_screen(Point::new(x, y));
    Point::new(f64::from(at.x), f64::from(at.y))
}

fn pos(p: Point) -> Option<Pos2> {
    let at = Pos2::new(p.x as f32, p.y as f32);
    (at.x.is_finite() && at.y.is_finite()).then_some(at)
}

/// Draws `primitives` until the frame's budget runs out.
fn paint(cx: &PaintCx<'_>, primitives: &[Primitive], budget: &mut Budget) {
    let scale = cx.camera.scale();
    for p in primitives {
        if budget.points == 0 {
            return;
        }
        match p {
            Primitive::Path { commands, fill, stroke: line } => {
                for contour in contours(cx, commands, budget) {
                    match (fill, line) {
                        (Some(f), l) if contour.closed && contour.points.len() > 2 => {
                            let s = l.as_ref().map_or(Stroke::NONE, stroke);
                            cx.painter.add(Shape::convex_polygon(contour.points, color(*f), s));
                        }
                        (_, Some(l)) => {
                            let points = if contour.closed { closed(contour.points) } else { contour.points };
                            cx.painter.add(Shape::line(points, stroke(l)));
                        }
                        _ => {}
                    }
                }
            }
            Primitive::Line { from, to, stroke: line } => {
                if let (Some(a), Some(b)) = (pos(screen(cx, *from)), pos(screen(cx, *to))) {
                    cx.painter.line_segment([a, b], stroke(line));
                }
            }
            Primitive::Circle { center, radius, fill, stroke: line } => {
                let Some(at) = pos(screen(cx, *center)) else { continue };
                let r = (radius * scale) as f32;
                if !r.is_finite() || r <= 0.0 || !cx.rect.intersects(Rect::from_center_size(at, Vec2::splat(r * 2.0))) {
                    continue;
                }
                if let Some(f) = fill {
                    cx.painter.circle_filled(at, r, color(*f));
                }
                if let Some(l) = line {
                    cx.painter.circle_stroke(at, r, stroke(l));
                }
            }
            Primitive::Text { at, text, color: fill, size } => {
                if budget.texts == 0 || size.is_nan() || *size < 1.0 {
                    continue;
                }
                budget.texts = budget.texts.saturating_sub(1);
                let Some(at) = pos(screen(cx, *at)) else { continue };
                if !cx.rect.contains(at) {
                    continue;
                }
                let size = (*size as f32).min(MAX_TEXT_SIZE);
                cx.painter.text(at, Align2::LEFT_BOTTOM, text, FontId::proportional(size), color(*fill));
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

/// The contours of a primitive path on screen, flattened. A path that lies wholly off the canvas gives none; a very
/// large one is flattened more coarsely, so its point count stays bounded.
fn contours(cx: &PaintCx<'_>, commands: &[PathCommand], budget: &mut Budget) -> Vec<Contour> {
    let mut els = Vec::with_capacity(commands.len());
    let (mut min, mut max) = (Point::new(f64::INFINITY, f64::INFINITY), Point::new(f64::NEG_INFINITY, f64::NEG_INFINITY));
    let mut grow = |p: Point| {
        min = Point::new(min.x.min(p.x), min.y.min(p.y));
        max = Point::new(max.x.max(p.x), max.y.max(p.y));
        p
    };
    for command in commands {
        els.push(match command {
            PathCommand::MoveTo { to } => PathEl::MoveTo(grow(screen(cx, *to))),
            PathCommand::LineTo { to } => PathEl::LineTo(grow(screen(cx, *to))),
            PathCommand::QuadTo { c, to } => PathEl::QuadTo(grow(screen(cx, *c)), grow(screen(cx, *to))),
            PathCommand::CurveTo { c1, c2, to } => PathEl::CurveTo(grow(screen(cx, *c1)), grow(screen(cx, *c2)), grow(screen(cx, *to))),
            PathCommand::Close => PathEl::ClosePath,
        });
    }
    let (Some(lo), Some(hi)) = (pos(min), pos(max)) else { return Vec::new() };
    if !cx.rect.intersects(Rect::from_two_pos(lo, hi)) {
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
                    budget.points = budget.points.saturating_sub(1);
                    out.push(Contour { points: vec![p], closed: false });
                }
            }
            PathEl::LineTo(p) => {
                if let (Some(p), Some(last)) = (pos(p), out.last_mut()) {
                    budget.points = budget.points.saturating_sub(1);
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use std::sync::Arc;

    use craft::host::Host;
    use craft::ui::runtime::commands::CommandRef;
    use fontcraft_engine::Session;
    use fontcraft_engine::plugins::fixtures;
    use fontcraft_engine::settings::{Reserved, UserSettings};
    use serde_json::json;

    use super::*;
    use crate::app::Msg;

    const BOX: &str = "box";

    fn install_reporter(app: &mut FontCraftApp, output: &Value) {
        let manifest = format!(r#"{{"id":"{BOX}","name":"Box","version":"1","kind":"reporter"}}"#);
        app.session.extensions_mut().registry.install_bytes(&fixtures::returning(&manifest, &output.to_string()), None).unwrap();
    }

    fn editing_a() -> FontCraftApp {
        let mut app = FontCraftApp::with_edit_tab(Session::new(), Arc::new(Host::builder(crate::app::APP).build()));
        app.run_now(Msg::run("file.new", json!({})), false).unwrap();
        app.run_now(Msg::run("glyph.newGlyph", json!({"name": "A"})), false).ok();
        app.run_now(Msg::View(crate::app::ViewMsg::SelectGlyph("A".into())), false).unwrap();
        app
    }

    fn show(app: &mut FontCraftApp, on: bool) {
        app.run_now(Msg::Shell(CommandRef::with_params("view.showReporter", json!({"id": BOX, "on": on}))), false).unwrap();
        app.project();
    }

    /// A shown reporter's primitives reach the canvas's overlay data, run once per layer and scale; hidden, it draws
    /// nothing and does not run.
    #[test]
    fn a_shown_reporter_is_drawn_from_its_cached_run() {
        let mut app = editing_a();
        let line = json!({"type": "line", "from": [0, 0], "to": [100, 100], "stroke": {"color": [255, 0, 0, 255], "width": 1}});
        install_reporter(&mut app, &json!({"background": [], "foreground": [line]}));
        app.project();
        assert!(app.overlays.data.reporters.is_empty());
        assert!(app.session.extensions().reporter_cache.is_empty(), "a hidden reporter does not run");

        show(&mut app, true);
        let drawn = &app.overlays.data.reporters;
        assert_eq!(drawn.len(), 1, "the shown reporter is drawn");
        assert_eq!(drawn[0].drawing.foreground.len(), 1);
        assert_eq!(drawn[0].error, None);
        app.project();
        assert_eq!(app.session.extensions().reporter_cache.len(), 1, "one run, cached for the layer and scale");

        show(&mut app, false);
        assert!(app.overlays.data.reporters.is_empty());
    }

    /// A failing reporter's error is kept for the canvas and the failure is cached, not retried every frame.
    #[test]
    fn a_failing_reporter_shows_its_error() {
        let mut app = editing_a();
        install_reporter(&mut app, &json!({"error": "no stems found"}));
        show(&mut app, true);
        app.project();
        let drawn = &app.overlays.data.reporters;
        assert_eq!(drawn.len(), 1);
        assert!(drawn[0].error.as_deref().is_some_and(|e| e.contains("no stems found")), "{:?}", drawn[0].error);
        assert_eq!(app.session.extensions().reporter_cache.len(), 1, "the failure is cached");
    }

    /// The settings a window loads at open, as the desktop app's store reads them from the settings folder.
    struct Saved(UserSettings);

    impl crate::app::user_store::UserStore for Saved {
        fn load(&mut self, session: &mut Session) -> Vec<String> {
            *session.settings_mut() = self.0.clone();
            Vec::new()
        }

        fn save_if_changed(&mut self, _session: &Session) -> Option<String> {
            None
        }
    }

    /// The switches are kept in the user's settings, and the next window that loads them draws the same reporters.
    #[test]
    fn reporter_switches_are_kept_between_runs() {
        let mut app = editing_a();
        install_reporter(&mut app, &json!({"background": [], "foreground": []}));
        show(&mut app, true);
        let saved = app.session.settings().clone();
        assert_eq!(saved.reserved::<Vec<String>>(Reserved::ShownReporters).unwrap(), Some(vec![BOX.to_string()]));

        let mut next = editing_a();
        install_reporter(&mut next, &json!({"background": [], "foreground": []}));
        next.user_store = Some(Box::new(Saved(saved)));
        crate::app::user_store::load(&mut next);
        assert_eq!(next.view.extensions.reporters, vec![BOX.to_string()]);
        assert_eq!(next.overlays.data.reporters.len(), 1, "the kept reporter is drawn");

        show(&mut next, false);
        assert_eq!(next.session.settings().reserved::<Vec<String>>(Reserved::ShownReporters).unwrap(), Some(Vec::new()));
    }
}
