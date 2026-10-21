//! The text-mode measurement line: its numbers are those of `measure.sidebearingsAtHeight`, it
//! shows only with the Text tool, Cmd+Ctrl+Alt with a click or a drag moves it; and glyph-target
//! commands take every glyph of the text selection.

use egui::{Pos2, Shape};
use serde_json::{Value, json};

use crate::canvas::view;
use crate::panels::widgets::format_number;
use crate::{FontcraftApp, control, tools_glue};

/// One frame of a headless window; the shapes it drew, nested lists flattened.
fn frame(ctx: &egui::Context, app: &mut FontcraftApp) -> Vec<Shape> {
    let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
    let mut output = ctx.run_ui(input, |ui| app.ui(ui));
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

/// The measurement line's numbers and rules in two settled frames, numbers sorted.
fn drawn(ctx: &egui::Context, app: &mut FontcraftApp) -> (Vec<String>, usize) {
    frame(ctx, app);
    let shapes = frame(ctx, app);
    let t = &app.tokens;
    let mut numbers: Vec<String> = shapes
        .iter()
        .filter_map(|s| match s {
            Shape::Text(text) if text.fallback_color == t.measurement_line_label => Some(text.galley.text().to_string()),
            _ => None,
        })
        .collect();
    numbers.sort();
    let rules = shapes.iter().filter(|s| matches!(s, Shape::LineSegment { stroke, .. } if stroke.color == t.measurement_line)).count();
    (numbers, rules)
}

/// A: a right triangle (100, 0), (500, 0), (100, 600), so its right sidebearing changes with the
/// height; V: a rectangle from 100 to 500. Both 600 wide. A tab in text mode holding "AVA", the
/// text in view at 0.3 points per unit.
fn measure_app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    let nodes = json!([{"x": 100, "y": 0}, {"x": 500, "y": 0}, {"x": 100, "y": 600}]);
    app.run("path.addPath", &json!({"glyph": "A", "nodes": nodes})).unwrap();
    app.run("path.addRectangle", &json!({"glyph": "V", "x": 100, "y": 0, "width": 400, "height": 700})).unwrap();
    app.run("view.newTab", &json!({"glyphs": ["A"]})).unwrap();
    app.run("editView.toggleTextMode", &json!({"mode": "text"})).unwrap();
    app.run("editView.setText", &json!({"text": "AVA"})).unwrap();
    let [x, y] = view::edited_origin(&app.session, &app.layouts, app.state.tab().unwrap());
    app.run("view.setZoom", &json!({"size": 300, "center": [900.0 - x, 300.0 - y]})).unwrap();
    app
}

/// What `measure.sidebearingsAtHeight` gives for the text "AVA" at `y`, as the line prints it.
fn expected(app: &mut FontcraftApp, y: f64) -> Vec<String> {
    let r = app.run("measure.sidebearingsAtHeight", &json!({"glyphs": ["A", "V"], "y": y})).unwrap();
    let side = |glyph: &str, key: &str| {
        let entry = r["glyphs"].as_array().unwrap().iter().find(|g| g["glyph"] == glyph).unwrap();
        format_number(entry[key].as_f64().unwrap())
    };
    let mut out: Vec<String> = ["A", "V", "A"].iter().flat_map(|g| [side(g, "left"), side(g, "right")]).collect();
    out.sort();
    out
}

#[test]
fn view_show_measurement_line_prints_the_numbers_of_measure_sidebearings_at_height() {
    let mut app = measure_app();
    let ctx = egui::Context::default();
    assert_eq!(drawn(&ctx, &mut app), (Vec::new(), 0), "off by default");
    assert_eq!(app.run("view.showMeasurementLine", &json!({})).unwrap(), json!({"on": true}));
    // The default height is half the x-height.
    let (numbers, rules) = drawn(&ctx, &mut app);
    let x_height = app.state.panels.measurement.height(app.session.font().unwrap(), app.state.tab().unwrap());
    assert_eq!(numbers, expected(&mut app, x_height));
    assert_eq!(rules, 1, "one line for the one line of text");
    for y in [150.0, 450.0] {
        app.run("view.setMeasurementLineHeight", &json!({"y": y})).unwrap();
        assert_eq!(drawn(&ctx, &mut app).0, expected(&mut app, y), "at {y}");
    }
    // A at 450: left 100, right 600 − 200 = 400; V: 100 and 100.
    assert_eq!(drawn(&ctx, &mut app).0, ["100", "100", "100", "100", "400", "400"]);
    // Above the outlines there is nothing to measure; the line is still drawn.
    app.run("view.setMeasurementLineHeight", &json!({"y": 800})).unwrap();
    assert_eq!(drawn(&ctx, &mut app), (Vec::new(), 1));
    for bad in [json!({}), json!({"y": "high"}), json!({"y": 1e9}), json!({"tab": 999, "y": 0})] {
        assert!(app.run("view.setMeasurementLineHeight", &bad).is_err(), "{bad}");
    }
}

#[test]
fn measurement_line_text_tool_only() {
    let mut app = measure_app();
    let ctx = egui::Context::default();
    app.run("view.showMeasurementLine", &json!({"on": true})).unwrap();
    assert_ne!(drawn(&ctx, &mut app).1, 0);
    app.run("tool.select", &json!({"tool": "measurement"})).unwrap();
    assert_eq!(drawn(&ctx, &mut app), (Vec::new(), 0), "not with another tool");
    app.run("tool.select", &json!({"tool": "text"})).unwrap();
    app.run("editView.toggleTextMode", &json!({"mode": "text"})).unwrap();
    assert_ne!(drawn(&ctx, &mut app).1, 0);
    app.run("editView.setWritingDirection", &json!({"direction": "ttb"})).unwrap();
    assert_eq!(drawn(&ctx, &mut app), (Vec::new(), 0), "not in vertical text");
}

fn pointer(app: &mut FontcraftApp, events: Value) {
    let r = control::handle(app, &json!({"id": 1, "method": "ui.pointer", "params": {"events": events}}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["result"]["errors"], json!([]), "{r}");
}

#[test]
fn view_set_measurement_line_height_by_cmd_ctrl_alt_click_and_drag() {
    let mut app = measure_app();
    let tab = app.state.tab().unwrap().id;
    let mods = json!(["cmd", "ctrl", "alt"]);
    app.run("editView.setCursor", &json!({"index": 1})).unwrap();
    // While the line is hidden the click is an ordinary one.
    pointer(&mut app, json!([{"kind": "down", "x": 100, "y": 420, "mods": mods}, {"kind": "up", "x": 100, "y": 420, "mods": mods}]));
    assert_eq!(app.state.panels.measurement.heights.get(&tab), None);
    assert_eq!(app.run("editView.getText", &json!({})).unwrap()["cursor"], 0);
    app.run("view.showMeasurementLine", &json!({"on": true})).unwrap();
    pointer(&mut app, json!([{"kind": "down", "x": 800, "y": 420.4, "mods": mods}, {"kind": "up", "x": 800, "y": 420.4, "mods": mods}]));
    assert_eq!(app.state.panels.measurement.heights.get(&tab), Some(&420.0), "a click puts the line at the pointer, in whole units");
    assert_eq!(app.run("editView.getText", &json!({})).unwrap()["cursor"], 0, "and leaves the cursor alone");
    pointer(
        &mut app,
        json!([{"kind": "down", "x": 800, "y": 300, "mods": mods}, {"kind": "drag", "x": 900, "y": 250, "mods": mods}, {"kind": "up", "x": 900, "y": 250, "mods": mods}]),
    );
    assert_eq!(app.state.panels.measurement.heights.get(&tab), Some(&250.0), "a drag follows the pointer");
}

#[test]
fn text_mode_glyph_commands_take_the_glyphs_of_the_text_selection() {
    let mut app = measure_app();
    let master = app.session.font().unwrap().masters[0].id.clone();
    // Nothing selected: the current glyph, as in edit mode.
    app.run("editView.setCursor", &json!({"index": 1})).unwrap();
    assert_eq!(tools_glue::with_targets(&app, "glyph.toggleLocked", json!({})), json!({"glyphs": ["V"]}));
    app.run("editView.setCursor", &json!({"index": 0, "selectionLength": 3})).unwrap();
    assert_eq!(tools_glue::with_targets(&app, "glyph.updateMetrics", json!({})), json!({"glyphs": ["A", "V"], "layer": master}));
    assert_eq!(tools_glue::with_targets(&app, "glyph.transformMetrics", json!({})), json!({"glyphs": ["A", "V"], "layer": master}));
    let r = app.run_menu_command("glyph.toggleLocked", Value::Null).unwrap();
    assert_eq!(r["glyphs"], json!(["A", "V"]), "the menu runs it on both glyphs");
    // Edit mode keeps the edited glyph.
    app.run("editView.toggleTextMode", &json!({"mode": "edit"})).unwrap();
    let edited = app.state.tab().unwrap().edited_glyph().unwrap().to_string();
    assert_eq!(tools_glue::with_targets(&app, "glyph.toggleLocked", json!({})), json!({"glyphs": [edited]}));
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

/// The Measurement tool in an Edit View tab on "AB", A edited: A is a square 0,0–100,100 and B
/// two squares that overlap in 50,0–100,100. The tool labels B too, draws B's two overlapped
/// edges light grey, and a ruler crosses both glyphs.
#[test]
fn the_measurement_tool_covers_every_glyph_of_the_line() {
    let mut app = FontcraftApp::default();
    let square = |x: f64| json!({"x": x, "y": 0, "width": 100, "height": 100});
    for (glyph, x) in [("A", 0.0), ("B", 0.0), ("B", 50.0)] {
        let mut p = square(x);
        p["glyph"] = json!(glyph);
        app.run("path.addRectangle", &p).unwrap();
    }
    app.run("view.newTab", &json!({"glyphs": ["A", "B"]})).unwrap();
    app.run("tool.select", &json!({"tool": "measurement"})).unwrap();
    let ctx = egui::Context::default();
    frame(&ctx, &mut app);
    let tab = app.state.tab().unwrap();
    assert_eq!(tab.edited_glyph(), Some("A"));
    let layout = view::text_layout(&app.session, &app.layouts, tab).unwrap();
    let b = layout.placed[1].x - layout.placed[0].x;
    assert!(b >= 150.0, "B sits after A's advance: {b}");
    app.run("view.setZoom", &json!({"size": 300, "center": [b / 2.0, 50.0]})).unwrap();
    frame(&ctx, &mut app);
    let shapes = frame(&ctx, &mut app);
    let t = app.tokens.clone();
    let on_curve = texts(&shapes, t.measure_on_curve);
    assert!(on_curve.contains(&"150, 0".to_string()), "B's own coordinates: {on_curve:?}");
    assert_eq!(on_curve.iter().filter(|l| *l == "0, 0").count(), 2, "both glyphs' first corner: {on_curve:?}");
    let grey = shapes.iter().filter(|s| matches!(s, Shape::LineSegment { stroke, .. } if stroke.color == t.measure_overlap)).count();
    assert_eq!(grey, 2, "B's two edges inside the overlap");

    // Ctrl keeps the ends where they are put.
    let mods = json!(["ctrl"]);
    let end = b + 250.0;
    pointer(
        &mut app,
        json!([
            {"kind": "down", "x": -10, "y": 50, "mods": mods},
            {"kind": "drag", "x": end / 2.0, "y": 50, "mods": mods},
            {"kind": "drag", "x": end, "y": 50, "mods": mods},
            {"kind": "up", "x": end, "y": 50, "mods": mods},
        ]),
    );
    frame(&ctx, &mut app);
    let shapes = frame(&ctx, &mut app);
    let ruler = texts(&shapes, t.measure_ruler);
    let mut want = vec!["100".to_string(), format_number(b - 100.0), "150".to_string(), "270°".to_string()];
    want.sort();
    assert_eq!(ruler, want, "A, the space to B, B as one shape, and the angle (0 is up)");
}
