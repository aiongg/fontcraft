//! Guides, the background, annotations and the Measurement tool in Edit View: what headless
//! frames draw, and gestures that must give the font the headless commands give.

use egui::{Pos2, Shape};
use fontcraft_model::Layer;
use serde_json::{Value, json};

use crate::theme::Tokens;
use crate::{FontcraftApp, control};

/// One headless frame; the shapes it drew, nested lists flattened.
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

/// Glyph A with a closed square (100,100)–(300,300), edited at actual size.
fn edit_app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.run("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
    app.run("view.setZoom", &json!({"size": 1000, "center": [400, 250]})).unwrap();
    app
}

fn layer(app: &FontcraftApp) -> Layer {
    let font = app.session.font().unwrap();
    let master = &font.masters[0].id;
    font.glyph("A").unwrap().layer(master).unwrap().clone()
}

fn pointer(app: &mut FontcraftApp, events: Value) {
    let r = control::handle(app, &json!({"id": 1, "method": "ui.pointer", "params": {"events": events}}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["result"]["errors"], json!([]), "{r}");
}

fn undo_steps(app: &FontcraftApp) -> usize {
    app.session.inspect()["documents"][0]["undo"].as_array().map_or(0, Vec::len)
}

fn texts(shapes: &[Shape]) -> Vec<String> {
    shapes
        .iter()
        .filter_map(|s| match s {
            Shape::Text(t) => Some(t.galley.text().to_string()),
            _ => None,
        })
        .collect()
}

fn has_path(shapes: &[Shape], color: egui::Color32) -> bool {
    shapes.iter().any(|s| matches!(s, Shape::Path(p) if p.stroke.color == egui::epaint::ColorMode::Solid(color)))
}

fn has_segment(shapes: &[Shape], color: egui::Color32) -> bool {
    shapes.iter().any(|s| matches!(s, Shape::LineSegment { stroke, .. } if stroke.color == color))
}

#[test]
fn guides_and_measurement_guides_are_drawn() {
    let mut app = edit_app();
    let t = Tokens::dark();
    let ctx = egui::Context::default();
    app.run("guide.add", &json!({"glyph": "A", "x": 0, "y": 200})).unwrap();
    app.run("guide.toggleMeasurement", &json!({"glyph": "A", "guide": {"scope": "local", "index": 0}, "on": true})).unwrap();
    app.run("guide.addGlobal", &json!({"x": 150, "y": 0, "angle": 90})).unwrap();
    let shapes = frame(&ctx, &mut app);
    assert!(has_segment(&shapes, t.guide_local) && has_segment(&shapes, t.guide_global), "a local and a global guide");
    assert!(texts(&shapes).iter().any(|s| s == "200"), "the measurement guide crosses the square 200 units apart: {:?}", texts(&shapes));
    app.run("view.showGuides", &json!({"on": false})).unwrap();
    let shapes = frame(&ctx, &mut app);
    assert!(!has_segment(&shapes, t.guide_local) && !has_segment(&shapes, t.guide_global));
    assert!(!texts(&shapes).iter().any(|s| s == "200"));
}

#[test]
fn the_background_and_visible_layers_are_drawn_behind_the_outline() {
    let mut app = edit_app();
    let t = Tokens::dark();
    let ctx = egui::Context::default();
    app.run("path.addRectangle", &json!({"glyph": "A", "side": "background", "x": 400, "y": 400, "width": 100, "height": 100})).unwrap();
    let shapes = frame(&ctx, &mut app);
    assert!(has_path(&shapes, t.background_outline), "the background's outline");
    let corners = |shapes: &[Shape]| shapes.iter().filter(|s| matches!(s, Shape::Rect(r) if r.fill == t.node_corner)).count();
    let before = corners(&shapes);
    app.run("view.showNodesInBackground", &json!({"on": true})).unwrap();
    assert_eq!(corners(&frame(&ctx, &mut app)), before + 3, "the background's corners, its start node an arrow");
    app.run("view.showBackground", &json!({"on": false})).unwrap();
    let shapes = frame(&ctx, &mut app);
    assert!(!has_path(&shapes, t.background_outline));
    assert_eq!(corners(&shapes), before);

    assert!(!has_path(&shapes, t.visible_layer_outline));
    let backup = app.run("layers.addBackupLayer", &json!({"glyphs": ["A"]})).unwrap();
    app.run("layers.toggleVisibility", &json!({"glyphs": ["A"], "layer": backup["layer"], "visible": true})).unwrap();
    assert!(has_path(&frame(&ctx, &mut app), t.visible_layer_outline), "the backup layer, its eye on");
}

#[test]
fn the_measurement_tool_labels_points_deltas_and_its_ruler() {
    let mut app = edit_app();
    let ctx = egui::Context::default();
    frame(&ctx, &mut app);
    app.run("tool.select", &json!({"tool": "measurement"})).unwrap();
    let labels = texts(&frame(&ctx, &mut app));
    assert!(labels.iter().any(|s| s == "100, 100") && labels.iter().any(|s| s == "300, 300"), "coordinates: {labels:?}");
    assert!(labels.iter().any(|s| s == "200, 0"), "the delta of the bottom side: {labels:?}");
    let steps = undo_steps(&app);
    pointer(
        &mut app,
        json!([{"kind": "down", "x": 50, "y": 200}, {"kind": "drag", "x": 200, "y": 200}, {"kind": "drag", "x": 350, "y": 200}, {"kind": "up", "x": 350, "y": 200}]),
    );
    let labels = texts(&frame(&ctx, &mut app));
    assert!(labels.iter().any(|s| s == "200"), "the ruler crosses the square 200 units apart: {labels:?}");
    assert!(labels.iter().any(|s| s == "270°"), "the ruler's angle, 0 pointing up: {labels:?}");
    assert_eq!(undo_steps(&app), steps, "measuring changes nothing");
    app.run("tool.select", &json!({"tool": "select"})).unwrap();
    let labels = texts(&frame(&ctx, &mut app));
    assert!(!labels.iter().any(|s| s == "100, 100" || s == "270°"), "{labels:?}");
}

#[test]
fn annotation_clicks_and_drags_give_the_font_the_commands_give() {
    let mut app = edit_app();
    let ctx = egui::Context::default();
    frame(&ctx, &mut app);
    let r = app.run("annotation.setTool", &json!({"tool": "circle"})).unwrap();
    assert_eq!(r["tool"], "annotationCircle");
    let steps = undo_steps(&app);
    pointer(&mut app, json!([{"kind": "down", "x": 500, "y": 500}, {"kind": "up", "x": 500, "y": 500}]));
    assert_eq!(undo_steps(&app), steps + 1);
    let mut headless = edit_app();
    headless.run("annotation.add", &json!({"glyph": "A", "type": "circle", "position": [500, 500]})).unwrap();
    assert_eq!(layer(&app), layer(&headless), "a click adds what annotation.add adds");
    assert_eq!(serde_json::to_value(&app.state.tab().unwrap().selection).unwrap(), json!({"annotations": [0]}));

    app.run("annotation.setTool", &json!({"tool": "select"})).unwrap();
    pointer(
        &mut app,
        json!([{"kind": "down", "x": 500, "y": 500}, {"kind": "drag", "x": 520, "y": 510}, {"kind": "drag", "x": 530, "y": 520}, {"kind": "up", "x": 530, "y": 520}]),
    );
    assert_eq!(undo_steps(&app), steps + 2, "the drag is one undo step");
    headless.run("annotation.move", &json!({"glyph": "A", "annotations": [0], "dx": 30, "dy": 20})).unwrap();
    assert_eq!(layer(&app), layer(&headless), "a drag moves as annotation.move moves");

    let shapes = frame(&ctx, &mut app);
    let t = Tokens::dark();
    assert!(shapes.iter().any(|s| matches!(s, Shape::Circle(c) if c.stroke.color == t.annotation)), "the circle is drawn");
    app.run("view.showAnnotations", &json!({"on": false})).unwrap();
    let shapes = frame(&ctx, &mut app);
    assert!(!shapes.iter().any(|s| matches!(s, Shape::Circle(c) if c.stroke.color == t.annotation)));
}

#[test]
fn annotation_shell_commands_select_all_and_choose_sub_tools() {
    let mut app = edit_app();
    app.run("annotation.add", &json!({"glyph": "A", "type": "plus", "position": [0, 0]})).unwrap();
    app.run("annotation.add", &json!({"glyph": "A", "type": "text", "position": [0, 600]})).unwrap();
    app.run("selection.set", &json!({"points": [[0, 0]]})).unwrap();
    let r = app.run("annotation.selectAll", &json!({})).unwrap();
    assert_eq!(r["selection"]["annotations"], json!([0, 1]));
    assert_eq!(serde_json::to_value(&app.state.tab().unwrap().selection).unwrap(), json!({"annotations": [0, 1]}), "annotations only");
    assert!(app.run("annotation.selectAll", &json!({"glyph": "B"})).is_err());
    for (name, id) in [("text", "annotationText"), ("minus", "annotationMinus"), ("select", "annotationSelect")] {
        assert_eq!(app.run("annotation.setTool", &json!({"tool": name})).unwrap()["tool"], id);
    }
    assert!(app.run("annotation.setTool", &json!({"tool": "eraser"})).is_err());
    assert!(app.run("annotation.setTool", &json!({})).is_err());
}
