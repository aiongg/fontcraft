//! Font View's spacing: the inspector's metric and kerning group fields, the switch between the
//! horizontal and vertical metric fields, and the cell badge of out-of-sync metrics keys, driven
//! in headless frames as a user would: focus a field, type, press Return; click a button.

use egui::epaint::PathShape;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, Shape};
use serde_json::{Value, json};

use crate::FontcraftApp;
use crate::shell::font_view::columns::Column;
use crate::shell::font_view::grid::metrics_out_of_sync;
use crate::shell::font_view::inspector::{self, SALT, Shown};
use crate::shell::font_view::metrics::MetricField;
use fontcraft_engine::prelude::*;

struct Window {
    ctx: egui::Context,
}

impl Window {
    fn new() -> Self {
        Self { ctx: egui::Context::default() }
    }

    /// One frame with these events; the shapes it drew, nested lists flattened.
    fn frame(&self, app: &mut FontcraftApp, events: Vec<Event>) -> Vec<Shape> {
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
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

    fn settle(&self, app: &mut FontcraftApp) -> Vec<Shape> {
        self.frame(app, Vec::new());
        self.frame(app, Vec::new());
        self.frame(app, Vec::new())
    }

    fn key(&self, app: &mut FontcraftApp, key: Key) {
        let event = |pressed| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
        self.frame(app, vec![event(true), event(false)]);
        self.frame(app, Vec::new());
    }

    /// Focuses field `id` (which selects its text), types `text` (none: deletes the text) and
    /// presses Return.
    fn commit(&self, app: &mut FontcraftApp, id: egui::Id, text: &str) {
        self.ctx.memory_mut(|m| m.request_focus(id));
        self.frame(app, Vec::new());
        assert!(self.ctx.memory(|m| m.has_focus(id)), "the field takes the focus");
        if text.is_empty() {
            self.key(app, Key::Backspace);
        } else {
            self.frame(app, vec![Event::Text(text.into())]);
        }
        self.key(app, Key::Enter);
    }

    fn click(&self, app: &mut FontcraftApp, id: egui::Id) {
        let at = self.ctx.read_response(id).unwrap_or_else(|| panic!("{id:?} was not drawn")).rect.center();
        let event = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        self.frame(app, vec![Event::PointerMoved(at)]);
        self.frame(app, vec![event(true)]);
        self.frame(app, vec![event(false)]);
    }

    fn enabled(&self, id: egui::Id) -> bool {
        self.ctx.read_response(id).unwrap_or_else(|| panic!("{id:?} was not drawn")).enabled()
    }
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|n| n.to_string()).collect()
}

fn last(app: &FontcraftApp) -> (String, Value) {
    app.session.journal().last().cloned().expect("a command ran")
}

fn metric(app: &FontcraftApp, glyphs: &[&str], field: MetricField) -> Shown {
    inspector::summarize(app, &names(glyphs)).metric(field)
}

/// A font whose A, B and C hold a 200 × 500 rectangle at x 100: LSB 100, RSB 300, width 600.
fn spaced_app(selection: &[&str]) -> FontcraftApp {
    let mut app = FontcraftApp::default();
    for g in ["A", "B", "C"] {
        app.run("path.addRectangle", &json!({"glyph": g, "x": 100, "y": 0, "width": 200, "height": 500})).unwrap();
    }
    app.run("fontView.setSelection", &json!({ "glyphs": selection })).unwrap();
    app
}

/// Commits `text` into field `id` and checks that exactly one command ran; returns it.
fn commit_once(w: &Window, app: &mut FontcraftApp, id: egui::Id, text: &str) -> (String, Value) {
    let steps = app.session.journal().len();
    w.commit(app, id, text);
    w.settle(app);
    assert_eq!(app.session.journal().len(), steps + 1, "one command for `{text}`");
    last(app)
}

#[test]
fn inspector_metric_fields_edit_one_and_three_glyphs_with_one_command() {
    let mut app = spaced_app(&["A"]);
    let w = Window::new();
    w.settle(&mut app);
    let lsb = inspector::metric_field_id(MetricField::Lsb);
    let rsb = inspector::metric_field_id(MetricField::Rsb);
    let width = inspector::metric_field_id(MetricField::Width);
    for id in [lsb, rsb, width] {
        assert!(w.enabled(id));
    }
    assert_eq!(metric(&app, &["A"], MetricField::Lsb), Shown::Same("100".into()));
    assert_eq!(metric(&app, &["A"], MetricField::Rsb), Shown::Same("300".into()));

    let (id, params) = commit_once(&w, &mut app, lsb, "50");
    assert_eq!((id.as_str(), params), ("spacing.setLeftSidebearing", json!({"glyphs": ["A"], "layer": "m01", "value": "50"})));
    assert_eq!(metric(&app, &["A"], MetricField::Lsb), Shown::Same("50".into()));
    assert_eq!(metric(&app, &["A"], MetricField::Width), Shown::Same("550".into()), "the width follows the left sidebearing");

    app.run("fontView.setSelection", &json!({"glyphs": ["A", "B", "C"]})).unwrap();
    w.settle(&mut app);
    assert_eq!(metric(&app, &["A", "B", "C"], MetricField::Lsb), Shown::Mixed);
    let (id, params) = commit_once(&w, &mut app, lsb, "80");
    assert_eq!((id.as_str(), &params["glyphs"]), ("spacing.setLeftSidebearing", &json!(["A", "B", "C"])));
    assert_eq!(metric(&app, &["A", "B", "C"], MetricField::Lsb), Shown::Same("80".into()));

    let (id, params) = commit_once(&w, &mut app, width, "700");
    assert_eq!((id.as_str(), &params["value"]), ("spacing.setWidth", &json!("700")));
    assert_eq!(metric(&app, &["A", "B", "C"], MetricField::Width), Shown::Same("700".into()));

    app.run("fontView.setSelection", &json!({"glyphs": ["B", "C"]})).unwrap();
    w.settle(&mut app);
    let (id, params) = commit_once(&w, &mut app, rsb, "=A");
    assert_eq!((id.as_str(), &params["glyphs"], &params["value"]), ("spacing.setRightSidebearing", &json!(["B", "C"]), &json!("=A")));
    assert_eq!(metric(&app, &["B", "C"], MetricField::Rsb), Shown::Same("=A".into()), "a field shows the key that governs it");
}

#[test]
fn inspector_kerning_group_fields_set_and_clear_groups_with_one_command() {
    let mut app = spaced_app(&["A"]);
    let w = Window::new();
    w.settle(&mut app);
    let left = inspector::field_id(SALT, Column::LeftGroup);
    let right = inspector::field_id(SALT, Column::RightGroup);
    assert!(w.enabled(left) && w.enabled(right));

    let ran = commit_once(&w, &mut app, left, "H");
    assert_eq!(ran, ("kerning.setLeftGroup".into(), json!({"glyphs": ["A"], "group": "H"})));

    app.run("fontView.setSelection", &json!({"glyphs": ["A", "B", "C"]})).unwrap();
    w.settle(&mut app);
    assert_eq!(inspector::summarize(&app, &names(&["A", "B", "C"])).field(Column::LeftGroup), Shown::Mixed);
    let ran = commit_once(&w, &mut app, right, "O");
    assert_eq!(ran, ("kerning.setRightGroup".into(), json!({"glyphs": ["A", "B", "C"], "group": "O"})));
    assert_eq!(inspector::summarize(&app, &names(&["A", "B", "C"])).field(Column::RightGroup), Shown::Same("O".into()));

    let ran = commit_once(&w, &mut app, right, "");
    assert_eq!(ran, ("kerning.setRightGroup".into(), json!({"glyphs": ["A", "B", "C"], "group": null})), "an emptied field clears the group");
    assert_eq!(inspector::summarize(&app, &names(&["A", "B", "C"])).field(Column::RightGroup), Shown::Same(String::new()));
}

#[test]
fn toggle_metrics_orientation_switches_between_the_horizontal_and_vertical_fields() {
    let mut app = spaced_app(&["A", "B"]);
    assert_eq!(app.run("fontView.toggleMetricsOrientation", &json!({})).unwrap(), json!({"orientation": "vertical"}));
    let w = Window::new();
    w.settle(&mut app);
    let tsb = inspector::metric_field_id(MetricField::Tsb);
    assert!(w.enabled(tsb) && w.enabled(inspector::metric_field_id(MetricField::Bsb)));
    for f in [MetricField::VertOrigin, MetricField::VertWidth] {
        assert!(!w.enabled(inspector::metric_field_id(f)), "{f:?} is read-only: no command sets it");
    }
    assert_eq!(metric(&app, &["A", "B"], MetricField::VertOrigin), Shown::Same("0".into()));

    let (id, params) = commit_once(&w, &mut app, tsb, "40");
    assert_eq!((id.as_str(), params), ("spacing.setTopSidebearing", json!({"glyphs": ["A", "B"], "layer": "m01", "value": "40"})));
    assert_eq!(metric(&app, &["A", "B"], MetricField::Tsb), Shown::Same("40".into()));

    w.click(&mut app, inspector::orientation_button_id());
    assert!(!app.state.panels.inspector_vertical, "the icon between the fields switches back");
    w.settle(&mut app);
    assert!(w.enabled(inspector::metric_field_id(MetricField::Lsb)));
    assert_eq!(app.run("fontView.toggleMetricsOrientation", &json!({"orientation": "horizontal"})).unwrap(), json!({"orientation": "horizontal"}));
    assert!(app.run("fontView.toggleMetricsOrientation", &json!({"orientation": "sideways"})).is_err());
}

/// The out-of-sync triangles drawn inside `cell`.
fn triangles_in(app: &FontcraftApp, shapes: &[Shape], cell: Rect) -> usize {
    let fill = app.tokens.font_view_badge_out_of_sync;
    shapes
        .iter()
        .filter(|s| matches!(s, Shape::Path(PathShape { points, fill: f, .. }) if *f == fill && points.len() == 3 && points.iter().all(|p| cell.contains(*p))))
        .count()
}

fn cell(app: &FontcraftApp, glyph: &str) -> Rect {
    app.font_view.laid_out.iter().find(|(n, _)| n == glyph).unwrap_or_else(|| panic!("{glyph} was not laid out")).1
}

#[test]
fn badge_metrics_out_of_sync_follows_an_edit_to_the_referenced_glyph() {
    let mut app = spaced_app(&[]);
    app.run("spacing.setWidth", &json!({"glyphs": ["B"], "value": "=A"})).unwrap();
    let w = Window::new();
    let shapes = w.settle(&mut app);
    assert_eq!(triangles_in(&app, &shapes, cell(&app, "B")), 0, "a key just applied is in sync");

    app.run("spacing.setWidth", &json!({"glyphs": ["A"], "value": 700})).unwrap();
    let shapes = w.frame(&mut app, Vec::new());
    assert_eq!(triangles_in(&app, &shapes, cell(&app, "B")), 1, "B's width key reads A, whose width changed");
    assert_eq!(triangles_in(&app, &shapes, cell(&app, "A")), 0);
    assert!(metrics_out_of_sync(app.session.metrics_sync().unwrap(), "B"));

    app.run("spacing.updateMetricsKey", &json!({"glyph": "B", "field": "width"})).unwrap();
    let shapes = w.frame(&mut app, Vec::new());
    assert_eq!(triangles_in(&app, &shapes, cell(&app, "B")), 0, "updating the key removes the triangle");
}
