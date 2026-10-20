//! The auto-meter (M13.34): the View › Measure switches, stem labels drawn on an H, a typed width
//! moving the stem, and the Measurements section of the Palette.

use egui::{Event, Key, Modifiers, PointerButton, Pos2, Shape};
use fontcraft_geom::Point;
use serde_json::{Value, json};

use crate::panels::widgets;
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

/// `H`: 80-unit stems at 0 and 520, a 60-unit crossbar from 300 to 360. Edited at actual size.
fn h_app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    let h = [(0, 0), (80, 0), (80, 300), (520, 300), (520, 0), (600, 0), (600, 700), (520, 700), (520, 360), (80, 360), (80, 700), (0, 700)];
    let nodes: Vec<Value> = h.iter().map(|(x, y)| json!({"x": x, "y": y})).collect();
    app.run("path.addPath", &json!({"glyph": "H", "nodes": nodes})).unwrap();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["H"]})).unwrap();
    app.run("view.setZoom", &json!({"size": 1000, "center": [300, 350]})).unwrap();
    app
}

/// The texts drawn in `color`, sorted.
fn texts(shapes: &[Shape], color: egui::Color32) -> Vec<String> {
    let mut out: Vec<String> = shapes
        .iter()
        .filter_map(|s| match s {
            Shape::Text(text) if text.fallback_color == color => Some(text.galley.text().to_string()),
            _ => None,
        })
        .collect();
    out.sort();
    out
}

/// Where the label of the vertical stem whose first edge is at `x` sits on screen.
fn label_at(app: &mut FontcraftApp, x: f64) -> Pos2 {
    let found = app.session.query("autometer.measure", &json!({"glyph": "H"})).unwrap();
    let stem =
        found["stems"].as_array().unwrap().iter().find(|s| s["direction"] == "vertical" && s["first"][0][0].as_f64() == Some(x)).unwrap().clone();
    let mid = |edge: &Value| {
        let p = |v: &Value| Point::new(v[0].as_f64().unwrap(), v[1].as_f64().unwrap());
        p(&edge[0]).midpoint(p(&edge[1]))
    };
    let at = mid(&stem["first"]).midpoint(mid(&stem["second"]));
    let view = tools_glue::current_view(app).unwrap();
    let s = view.to_screen(at);
    Pos2::new(s.x as f32, s.y as f32)
}

#[test]
fn view_measure_switches_toggle_the_auto_meter() {
    let mut app = FontcraftApp::default();
    for id in ["view.showStemWidths", "view.showCounterWidths", "view.showCornerAngles", "view.showQuickMeasurement"] {
        assert_eq!(app.run(id, &json!({})).unwrap()["on"], true, "{id}");
        assert_eq!(app.run(id, &json!({"on": false})).unwrap()["on"], false, "{id}");
    }
    assert_eq!(app.state.panels.autometer, Default::default());
}

#[test]
fn stem_widths_label_the_stems_of_h_and_counters_label_the_space_between() {
    let mut app = h_app();
    let w = Window::new();
    let t = app.tokens.clone();
    w.idle(&mut app);
    assert!(texts(&w.idle(&mut app), t.aids.stem_other).is_empty(), "nothing while the switches are off");

    app.run("view.showStemWidths", &json!({"on": true})).unwrap();
    w.idle(&mut app);
    // No stem values are defined, so no stem is a common one.
    assert_eq!(texts(&w.idle(&mut app), t.aids.stem_other), vec!["60", "80", "80"]);

    app.run("font.stems.add", &json!({"name": "lc", "orientation": "vertical", "values": {"m01": 80}})).unwrap();
    w.idle(&mut app);
    let shapes = w.idle(&mut app);
    assert_eq!(texts(&shapes, t.aids.stem_common), vec!["80", "80"], "the bars match the stem value");
    assert_eq!(texts(&shapes, t.aids.stem_other), vec!["60"]);

    app.run("view.showCounterWidths", &json!({"on": true})).unwrap();
    w.idle(&mut app);
    assert_eq!(texts(&w.idle(&mut app), t.aids.counter).iter().filter(|s| *s == "440").count(), 1);

    // Corner angles wait for a closer zoom.
    app.run("view.showCornerAngles", &json!({"on": true})).unwrap();
    w.idle(&mut app);
    assert!(!texts(&w.idle(&mut app), t.aids.corner_angle).contains(&"90°".to_string()));
    app.run("view.setZoom", &json!({"size": 2000, "center": [80, 300]})).unwrap();
    w.idle(&mut app);
    assert!(texts(&w.idle(&mut app), t.aids.corner_angle).contains(&"90°".to_string()));
}

#[test]
fn typing_a_width_into_a_stem_label_moves_the_stem() {
    let mut app = h_app();
    app.run("view.showStemWidths", &json!({"on": true})).unwrap();
    let w = Window::new();
    w.idle(&mut app);
    w.idle(&mut app);
    let at = label_at(&mut app, 0.0);
    let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    w.frame(&mut app, vec![Event::PointerMoved(at)]);
    w.frame(&mut app, vec![press(true)]);
    w.frame(&mut app, vec![press(false)]);
    // The field opens, takes the focus, then the typing.
    w.idle(&mut app);
    w.idle(&mut app);
    w.frame(&mut app, vec![Event::Text("90".into())]);
    w.frame(&mut app, vec![Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }]);
    w.idle(&mut app);
    let font = app.session.font().unwrap();
    let layer = font.glyph("H").unwrap().layer("m01").unwrap();
    let path = layer.path(0).unwrap();
    let xs: Vec<f64> = path.nodes.iter().map(|n| n.x).collect();
    assert_eq!(xs, vec![-5.0, 85.0, 85.0, 520.0, 520.0, 600.0, 600.0, 520.0, 520.0, 85.0, 85.0, -5.0]);
}

#[test]
fn the_measurements_section_writes_and_recalculates_master_values() {
    let mut app = h_app();
    app.run("font.stems.add", &json!({"name": "lc", "orientation": "vertical"})).unwrap();
    // The other sections folded, so this one is in view.
    for section in ["dimensions", "fitCurve", "layers", "transformations", "relatedGlyphs", "referenceFont", "selections"] {
        app.run("palette.toggleSection", &json!({"section": section, "expanded": false})).unwrap();
    }
    let w = Window::new();
    w.idle(&mut app);
    w.idle(&mut app);
    // The x-height's position field, the third metric of a new font.
    let field = widgets::id(("measurements.position", 2u64));
    assert!(w.ctx.read_response(field).is_some(), "the section shows the x-height");
    w.ctx.memory_mut(|m| m.request_focus(field));
    w.idle(&mut app);
    w.frame(&mut app, vec![Event::Text("480".into())]);
    w.frame(&mut app, vec![Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }]);
    w.idle(&mut app);
    let master = app.session.query("font.master.get", &json!({"master": "m01"})).unwrap();
    assert_eq!(master["metrics"]["x-height"]["position"], 480.0);

    let button = w.ctx.read_response(widgets::id("measurements.recalculate")).expect("the Recalculate button is drawn").rect.center();
    let press = |pressed| Event::PointerButton { pos: button, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    w.frame(&mut app, vec![Event::PointerMoved(button)]);
    w.frame(&mut app, vec![press(true)]);
    w.frame(&mut app, vec![press(false)]);
    w.idle(&mut app);
    let master = app.session.query("font.master.get", &json!({"master": "m01"})).unwrap();
    assert_eq!(master["metrics"]["cap height"]["position"], 700.0, "measured from H");
    assert_eq!(master["stems"]["lc"], 80.0, "H's bars stand in for the missing lowercase");
}
