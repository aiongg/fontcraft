//! The transformation settings commands, the bounding box and origin drawing, and the Rotate,
//! Scale and knob drags in the shell: each drag gives the font the headless command gives.

use egui::{Pos2, Shape};
use fontcraft_model::Layer;
use serde_json::{Value, json};

use crate::theme::Tokens;
use crate::{FontcraftApp, control};

/// Glyph A with the closed square (100,100) (300,100) (300,300) (100,300) as shape 0, edited at
/// actual size (one screen point per unit).
fn edit_app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.run("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
    app.run("view.setZoom", &json!({"size": 1000, "center": [200, 200]})).unwrap();
    app
}

fn square_selected() -> FontcraftApp {
    let mut app = edit_app();
    app.run("selection.set", &json!({"points": [[0, 0], [0, 1], [0, 2], [0, 3]]})).unwrap();
    app
}

fn layer(app: &FontcraftApp) -> Layer {
    let font = app.session.font().unwrap();
    let master = &font.masters[0].id;
    font.glyph("A").unwrap().layer(master).unwrap().clone()
}

fn undo_steps(app: &FontcraftApp) -> usize {
    app.session.inspect()["documents"][0]["undo"].as_array().map_or(0, Vec::len)
}

fn pointer(app: &mut FontcraftApp, events: Value) -> Value {
    let r = control::handle(app, &json!({"id": 1, "method": "ui.pointer", "params": {"events": events}}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["result"]["errors"], json!([]), "{r}");
    r["result"].clone()
}

fn drag(app: &mut FontcraftApp, from: [f64; 2], to: [f64; 2], mods: Value) -> Value {
    pointer(
        app,
        json!([
            {"kind": "down", "x": from[0], "y": from[1], "mods": mods},
            {"kind": "drag", "x": (from[0] + to[0]) / 2.0, "y": (from[1] + to[1]) / 2.0, "mods": mods},
            {"kind": "drag", "x": to[0], "y": to[1], "mods": mods},
            {"kind": "up", "x": to[0], "y": to[1], "mods": mods},
        ]),
    )
}

/// The layer after running `command` on a fresh app with the square selected.
fn headless(command: &str, params: Value) -> Layer {
    let mut app = square_selected();
    app.run(command, &params).unwrap();
    layer(&app)
}

/// The shapes one headless frame draws, nested lists flattened.
fn frame(app: &mut FontcraftApp) -> Vec<Shape> {
    let ctx = egui::Context::default();
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

#[test]
fn the_origin_and_lock_commands_keep_the_settings_and_refuse_bad_values() {
    let mut app = edit_app();
    let defaults = app.run("transform.getSettings", &json!({})).unwrap();
    assert_eq!(defaults["origin"], json!({"mode": "box", "boxPoint": "center", "metric": "baseline", "point": null}));
    assert_eq!(defaults["scale"]["locked"], true);

    let r = app.run("transform.setOrigin", &json!({"mode": "box", "boxPoint": "topRight"})).unwrap();
    assert_eq!(r["origin"]["boxPoint"], "topRight");
    assert_eq!(app.state.transform.origin_param(), json!({"mode": "box", "boxPoint": "topRight"}));

    // A placed point keeps the mode; choosing the reference mode then uses it.
    app.run("transform.setOrigin", &json!({"point": [10, -20]})).unwrap();
    assert_eq!(app.state.transform.origin_param(), json!({"mode": "box", "boxPoint": "topRight"}));
    app.run("transform.setOrigin", &json!({"mode": "reference"})).unwrap();
    assert_eq!(app.state.transform.origin_param(), json!({"mode": "reference", "point": [10.0, -20.0]}));
    app.run("transform.setOrigin", &json!({"mode": "metrics", "metric": "halfXHeight"})).unwrap();
    assert_eq!(app.state.transform.origin_param(), json!({"mode": "metrics", "metric": "halfXHeight"}));

    let before = app.state.transform.clone();
    for bad in [
        json!({}),
        json!({"mode": "corner"}),
        json!({"mode": "box", "boxPoint": "middle"}),
        json!({"metric": "ascender"}),
        json!({"point": [1e9, 0]}),
        json!({"point": [1, "a"]}),
        json!({"mode": "layer", "point": [0, 0, 0]}),
    ] {
        assert!(app.run("transform.setOrigin", &bad).is_err(), "{bad}");
        assert_eq!(app.state.transform, before, "a refused {bad} changes nothing");
    }
    let mut fresh = edit_app();
    assert!(fresh.run("transform.setOrigin", &json!({"mode": "reference"})).is_err(), "no point placed yet");

    assert_eq!(app.run("transform.setScaleLock", &json!({})).unwrap(), json!({"locked": false}));
    assert_eq!(app.run("transform.setScaleLock", &json!({"locked": true})).unwrap(), json!({"locked": true}));
    assert!(app.run("transform.setScaleLock", &json!({"locked": "yes"})).is_err());
    assert_eq!(app.run("transform.getSettings", &json!({})).unwrap()["scale"]["locked"], true);
}

#[test]
fn the_rotate_tool_drag_is_the_rotate_command_in_one_undo_step() {
    let mut app = square_selected();
    app.run("tool.select", &json!({"tool": "rotate"})).unwrap();
    let steps = undo_steps(&app);
    let r = drag(&mut app, [300.0, 200.0], [200.0, 300.0], json!([]));
    assert_eq!(r["busy"], false);
    assert_eq!(undo_steps(&app), steps + 1, "the drag is one undo step");
    let expected = headless(
        "transform.rotateCounterclockwise",
        json!({"glyph": "A", "points": [[0, 0], [0, 1], [0, 2], [0, 3]], "angle": 90, "origin": [200, 200]}),
    );
    assert_eq!(layer(&app), expected, "about the centre of the selection");

    // A click places the reference point; the next drag turns about it.
    let mut app = square_selected();
    app.run("tool.select", &json!({"tool": "rotate"})).unwrap();
    pointer(&mut app, json!([{"kind": "down", "x": 100, "y": 100}, {"kind": "up", "x": 100, "y": 100}]));
    assert_eq!(app.state.transform.point, Some([100.0, 100.0]));
    drag(&mut app, [200.0, 100.0], [100.0, 200.0], json!([]));
    let expected = headless(
        "transform.rotateCounterclockwise",
        json!({"glyph": "A", "points": [[0, 0], [0, 1], [0, 2], [0, 3]], "angle": 90, "origin": [100, 100]}),
    );
    assert_eq!(layer(&app), expected);
}

#[test]
fn the_scale_tool_drag_is_the_scale_command() {
    let mut app = square_selected();
    app.run("tool.select", &json!({"tool": "scale"})).unwrap();
    drag(&mut app, [300.0, 300.0], [400.0, 350.0], json!([]));
    // About the centre (200, 200): 200/100 across, 150/100 up.
    let expected =
        headless("transform.scale", json!({"glyph": "A", "points": [[0, 0], [0, 1], [0, 2], [0, 3]], "x": 200, "y": 150, "origin": [200, 200]}));
    assert_eq!(layer(&app), expected);
}

#[test]
fn a_knob_drag_is_the_scale_command_about_the_opposite_knob() {
    let mut app = square_selected();
    app.run("view.showBoundingBox", &json!({"on": true})).unwrap();
    let steps = undo_steps(&app);
    // The top-right knob is drawn 6 points outside the corner (300, 300).
    let r = drag(&mut app, [306.0, 306.0], [406.0, 306.0], json!([]));
    assert_eq!(r["busy"], false);
    assert_eq!(undo_steps(&app), steps + 1);
    let expected =
        headless("transform.scale", json!({"glyph": "A", "points": [[0, 0], [0, 1], [0, 2], [0, 3]], "x": 150, "y": 100, "origin": [100, 100]}));
    assert_eq!(layer(&app), expected);
    assert_eq!(app.state.tab().unwrap().selection.points.len(), 4, "the selection stays");
}

#[test]
fn the_box_knobs_and_origin_are_drawn_when_they_apply() {
    let t = Tokens::dark();
    let boxes = |shapes: &[Shape]| {
        shapes.iter().filter(|s| matches!(s, Shape::Path(p) if p.stroke.color == egui::epaint::ColorMode::Solid(t.bounding_box))).count()
    };
    let knobs = |shapes: &[Shape]| shapes.iter().filter(|s| matches!(s, Shape::Rect(r) if r.fill == t.bounding_box)).count();
    let rings = |shapes: &[Shape]| shapes.iter().filter(|s| matches!(s, Shape::Circle(c) if c.stroke.color == t.transform_origin)).count();

    let mut app = square_selected();
    let shapes = frame(&mut app);
    assert_eq!((boxes(&shapes), knobs(&shapes)), (0, 0), "Show Bounding Box is off");
    app.run("view.showBoundingBox", &json!({"on": true})).unwrap();
    let shapes = frame(&mut app);
    assert_eq!((boxes(&shapes), knobs(&shapes)), (1, 8), "one box with eight knobs");
    app.run("selection.set", &json!({"points": [[0, 0]]})).unwrap();
    let shapes = frame(&mut app);
    assert_eq!(knobs(&shapes), 0, "one node has no box");

    app.run("tool.select", &json!({"tool": "rotate"})).unwrap();
    let shapes = frame(&mut app);
    assert_eq!(rings(&shapes), 1, "the Rotate tool shows its origin");
    assert_eq!(knobs(&shapes), 0, "the knobs belong to the Select tool");
    app.run("tool.select", &json!({"tool": "select"})).unwrap();
    assert_eq!(rings(&frame(&mut app)), 0);
}
