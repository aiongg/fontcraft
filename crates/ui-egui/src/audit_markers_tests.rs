//! Live audit and the audit markers in Edit View, in headless frames.

use egui::{Color32, Event, Modifiers, PointerButton, Pos2, Shape};
use serde_json::json;

use crate::canvas::audit_markers;
use crate::theme::Tokens;
use crate::{FontcraftApp, tools_glue};

/// A headless window: one egui context kept across frames, so widget and pointer state carry over.
struct Window {
    ctx: egui::Context,
}

impl Window {
    fn new() -> Self {
        Self { ctx: egui::Context::default() }
    }

    /// One frame with these events; the shapes it drew, nested lists flattened.
    fn frame(&self, app: &mut FontcraftApp, events: Vec<Event>) -> Vec<Shape> {
        let input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
        let mut output = self.ctx.run_ui(input, |ui| app.ui(ui));
        output.textures_delta.clear();
        let mut out: Vec<Shape> = output.shapes.into_iter().map(|c| c.shape).collect();
        let mut flat = Vec::new();
        while let Some(shape) = out.pop() {
            match shape {
                Shape::Vec(list) => out.extend(list),
                other => flat.push(other),
            }
        }
        flat
    }

    fn idle(&self, app: &mut FontcraftApp) -> Vec<Shape> {
        self.frame(app, Vec::new())
    }
}

/// Glyph A with one closed path whose curve reaches its rightmost point, (250, 200), between its
/// nodes: a missing extreme. Edited at actual size.
fn curve_app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    let nodes = json!([
        {"x": 100, "y": 100}, {"x": 300, "y": 100, "type": "offcurve"}, {"x": 300, "y": 300, "type": "offcurve"}, {"x": 100, "y": 300, "type": "curve"}
    ]);
    app.run("path.addPath", &json!({"glyph": "A", "closed": true, "nodes": nodes})).unwrap();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
    app.run("view.setZoom", &json!({"size": 1000, "center": [300, 250]})).unwrap();
    app
}

/// Adds an open line, shape 1 of glyph A.
fn add_open_path(app: &mut FontcraftApp, y: f64) {
    app.run("path.addPath", &json!({"glyph": "A", "closed": false, "nodes": [{"x": 400, "y": y}, {"x": 500, "y": y}]})).unwrap();
}

fn badges(shapes: &[Shape], color: Color32) -> usize {
    shapes.iter().filter(|s| matches!(s, Shape::Circle(c) if c.fill == color)).count()
}

fn all_badges(shapes: &[Shape], t: &Tokens) -> usize {
    badges(shapes, t.audit_fail) + badges(shapes, t.audit_warn) + badges(shapes, t.audit_info)
}

fn marked(app: &FontcraftApp, check: &str) -> bool {
    app.state.audit.shown.markers.iter().any(|m| m.check == check)
}

#[test]
fn live_audit_marks_an_open_path_and_a_missing_extreme_once_per_revision() {
    let mut app = curve_app();
    let w = Window::new();
    let t = Tokens::dark();
    let shapes = w.idle(&mut app);
    assert_eq!(app.state.audit.runs, 0, "live audit is off until turned on");
    assert_eq!(all_badges(&shapes, &t), 0, "nothing has been audited");

    app.run("audit.toggleLiveAudit", &json!({"on": true})).unwrap();
    add_open_path(&mut app, 100.0);
    let shapes = w.idle(&mut app);
    assert_eq!(app.state.audit.runs, 1);
    assert!(marked(&app, "check.outlines.openPath"), "{:?}", app.state.audit.shown.markers);
    assert!(marked(&app, "check.outlines.missingExtremes"), "{:?}", app.state.audit.shown.markers);
    let extreme = app.state.audit.shown.markers.iter().find(|m| m.check == "check.outlines.missingExtremes").unwrap();
    let [x, y] = extreme.point.unwrap();
    assert!((x - 250.0).abs() < 1e-6 && (y - 200.0).abs() < 1e-6, "the extreme is at ({x}, {y})");
    assert!(badges(&shapes, t.audit_fail) >= 1, "the open path is a failure");
    assert!(badges(&shapes, t.audit_warn) >= 1, "the missing extreme is a warning");

    for _ in 0..5 {
        w.frame(&mut app, vec![Event::PointerMoved(Pos2::new(600.0, 400.0))]);
    }
    assert_eq!(app.state.audit.runs, 1, "frames without an edit do not re-run the audit");

    add_open_path(&mut app, 50.0);
    for _ in 0..3 {
        w.idle(&mut app);
    }
    assert_eq!(app.state.audit.runs, 2, "one run per revision");
    let open = app.state.audit.shown.markers.iter().filter(|m| m.check == "check.outlines.openPath").count();
    assert_eq!(open, 2, "both open paths are marked");

    app.run("audit.toggleLiveAudit", &json!({"on": false})).unwrap();
    add_open_path(&mut app, 0.0);
    w.idle(&mut app);
    assert_eq!(app.state.audit.runs, 2, "no run with live audit off");
    let shapes = w.idle(&mut app);
    assert_eq!(all_badges(&shapes, &t), 0, "the kept results are stale after the edit");
}

#[test]
fn toggling_markers_off_hides_them_without_re_running() {
    let mut app = curve_app();
    let w = Window::new();
    let t = Tokens::dark();
    app.run("audit.toggleLiveAudit", &json!({})).unwrap();
    add_open_path(&mut app, 100.0);
    let shapes = w.idle(&mut app);
    let shown = all_badges(&shapes, &t);
    assert!(shown >= 2);

    assert_eq!(app.run("audit.toggleMarkers", &json!({})).unwrap(), json!({"on": false}));
    let shapes = w.idle(&mut app);
    assert_eq!(all_badges(&shapes, &t), 0, "markers off");
    assert_eq!(app.state.audit.runs, 1, "hiding the markers does not re-run the audit");

    app.run("audit.toggleMarkers", &json!({"on": true})).unwrap();
    let shapes = w.idle(&mut app);
    assert_eq!(all_badges(&shapes, &t), shown, "the same markers come back");
    assert_eq!(app.state.audit.runs, 1);
    assert!(app.run("audit.toggleMarkers", &json!({"on": "yes"})).is_err());
}

#[test]
fn markers_show_an_explicit_audit_run_and_a_click_selects_what_it_reports() {
    let mut app = curve_app();
    add_open_path(&mut app, 100.0);
    let w = Window::new();
    w.idle(&mut app);
    // Live audit is off: the markers show the edited layer's results of the last run.
    app.run("audit.run", &json!({"profile": "outlines", "glyphs": ["A"]})).unwrap();
    app.run("audit.toggleMarkers", &json!({"on": true})).unwrap();
    w.idle(&mut app);
    assert_eq!(app.state.audit.runs, 0);
    assert!(marked(&app, "check.outlines.openPath"));

    let markers = app.state.audit.shown.markers.clone();
    let index = markers.iter().position(|m| m.check == "check.outlines.openPath").unwrap();
    let font = app.session.font().unwrap();
    let layer = font.glyph("A").unwrap().layer(&font.masters[0].id).unwrap().clone();
    let view = tools_glue::current_view(&app).unwrap();
    let (_, badge) = audit_markers::badges(&layer, &markers, view, &app.tokens)[index].unwrap();

    let revision = app.session.active().unwrap().revision;
    let press = |pressed| Event::PointerButton { pos: badge, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    w.frame(&mut app, vec![Event::PointerMoved(badge)]);
    w.frame(&mut app, vec![press(true)]);
    w.frame(&mut app, vec![press(false)]);
    w.idle(&mut app);
    let selection = serde_json::to_value(&app.state.tab().unwrap().selection).unwrap();
    assert_eq!(selection, json!({"points": [[1, 0], [1, 1]]}), "the open path's two ends");
    assert_eq!(app.session.active().unwrap().revision, revision, "the click edits nothing");
}
