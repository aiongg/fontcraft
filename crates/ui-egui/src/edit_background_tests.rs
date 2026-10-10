//! Editing the background (`path.editBackground`): the tab's side, what the canvas draws, and
//! gestures and shell commands that act on the background as the headless commands with
//! `side: "background"` do.

use egui::{Pos2, Shape};
use fontcraft_model::{Layer, Shape as ModelShape};
use serde_json::{Value, json};

use crate::theme::Tokens;
use crate::{FontcraftApp, control, tools_glue};

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

/// Glyph A with the square (100,100)–(300,300) in the foreground and (400,400)–(500,500) in the
/// background, edited at actual size.
fn edit_app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.run("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    app.run("path.addRectangle", &json!({"glyph": "A", "side": "background", "x": 400, "y": 400, "width": 100, "height": 100})).unwrap();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
    app.run("view.setZoom", &json!({"size": 1000, "center": [400, 250]})).unwrap();
    app
}

fn layer(app: &FontcraftApp) -> Layer {
    let font = app.session.font().unwrap();
    font.glyph("A").unwrap().layer(&font.masters[0].id).unwrap().clone()
}

/// The `[shape, node]` of the node at (x, y) among `shapes`.
fn node_at(shapes: &[ModelShape], x: f64, y: f64) -> Value {
    for (s, shape) in shapes.iter().enumerate() {
        let ModelShape::Path(p) = shape else { continue };
        if let Some(n) = p.nodes.iter().position(|n| n.x == x && n.y == y) {
            return json!([s, n]);
        }
    }
    panic!("no node at ({x}, {y})");
}

fn drag(app: &mut FontcraftApp, from: (f64, f64), to: (f64, f64)) {
    let mid = ((from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0);
    let events = json!([
        {"kind": "down", "x": from.0, "y": from.1},
        {"kind": "drag", "x": mid.0, "y": mid.1},
        {"kind": "drag", "x": to.0, "y": to.1},
        {"kind": "up", "x": to.0, "y": to.1},
    ]);
    let r = control::handle(app, &json!({"id": 1, "method": "ui.pointer", "params": {"events": events}}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["result"]["errors"], json!([]), "{r}");
}

fn undo_steps(app: &FontcraftApp) -> usize {
    app.session.inspect()["documents"][0]["undo"].as_array().map_or(0, Vec::len)
}

fn tinted(shapes: &[Shape], t: &Tokens) -> bool {
    shapes.iter().any(|s| matches!(s, Shape::Rect(r) if r.fill == t.background_tint && r.rect.width() > 100.0))
}

#[test]
fn edit_background_switches_the_side_and_clears_the_selection() {
    let mut app = edit_app();
    app.run("selection.set", &json!({"points": [[0, 0]]})).unwrap();
    assert_eq!(app.run("path.editBackground", &json!({})).unwrap()["side"], "background");
    let tab = app.state.tab().unwrap();
    assert!(tab.edit_background && tab.selection.is_empty(), "switching sides clears the selection");
    assert_eq!(app.run("path.editBackground", &json!({"on": true})).unwrap()["side"], "background");
    // A menu command run from the shell targets the background.
    app.run("selection.set", &json!({"points": [[0, 0]]})).unwrap();
    let params = tools_glue::with_targets(&app, "edit.delete", Value::Null);
    assert_eq!((params["side"].as_str(), params["points"].clone()), (Some("background"), json!([[0, 0]])));
    let params = tools_glue::with_targets(&app, "path.swapWithBackground", Value::Null);
    assert!(params.get("side").is_none(), "a whole-layer command that takes no side: {params}");
    // The side belongs to the tab: another master keeps it.
    app.run("view.selectMaster", &json!({"master": 0})).unwrap();
    assert!(app.state.tab().unwrap().edit_background);
    assert_eq!(app.run("path.editBackground", &json!({})).unwrap()["side"], "foreground");
    assert!(app.run("path.editBackground", &json!({"on": 1})).is_err());
}

#[test]
fn a_drag_while_editing_the_background_changes_only_the_background() {
    let mut app = edit_app();
    let ctx = egui::Context::default();
    frame(&ctx, &mut app);
    let before = layer(&app);
    app.run("path.editBackground", &json!({})).unwrap();
    let steps = undo_steps(&app);
    drag(&mut app, (400.0, 400.0), (417.0, 423.0));
    assert_eq!(undo_steps(&app), steps + 1, "the drag is one undo step");

    let mut headless = edit_app();
    let point = node_at(&before.info.background.as_ref().unwrap().shapes, 400.0, 400.0);
    headless.run("selection.move", &json!({"glyph": "A", "side": "background", "points": [point], "dx": 17, "dy": 23})).unwrap();
    let after = layer(&app);
    assert_eq!(after, layer(&headless), "the drag moves as selection.move with side background does");
    assert_eq!(after.shapes, before.shapes, "the foreground is untouched");
    let selected = &app.state.tab().unwrap().selection;
    assert_eq!(selected.points.len(), 1, "the dragged background node stays selected");

    // Back in the foreground, the same drag moves the foreground's node.
    app.run("path.editBackground", &json!({"on": false})).unwrap();
    drag(&mut app, (100.0, 100.0), (117.0, 123.0));
    let point = node_at(&before.shapes, 100.0, 100.0);
    headless.run("selection.move", &json!({"glyph": "A", "points": [point], "dx": 17, "dy": 23})).unwrap();
    assert_eq!(layer(&app), layer(&headless));
}

#[test]
fn the_canvas_is_darker_while_the_background_is_edited() {
    let mut app = edit_app();
    let t = Tokens::dark();
    let ctx = egui::Context::default();
    let foreground = frame(&ctx, &mut app);
    assert!(!tinted(&foreground, &t));
    app.run("path.editBackground", &json!({})).unwrap();
    let background = frame(&ctx, &mut app);
    assert!(tinted(&background, &t), "the tint over the canvas");
    // The foreground is the reference outline behind the edited background.
    let outlines = |shapes: &[Shape], color| {
        shapes.iter().filter(|s| matches!(s, Shape::Path(p) if p.stroke.color == egui::epaint::ColorMode::Solid(color))).count()
    };
    assert_eq!(outlines(&background, t.background_outline), outlines(&foreground, t.background_outline));
    app.run("view.showBackground", &json!({"on": false})).unwrap();
    assert_eq!(outlines(&frame(&ctx, &mut app), t.background_outline), 0, "Show Background hides the foreground's outline");
    app.run("path.editBackground", &json!({})).unwrap();
    assert!(!tinted(&frame(&ctx, &mut app), &t));
}
