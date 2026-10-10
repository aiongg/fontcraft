//! The active layer: the layer picked in the Layers section, or the master picked with Cmd+1 to
//! Cmd+9, is the one the canvas draws and the tools edit.

use std::sync::Arc;

use egui::{Pos2, Shape};
use fontcraft_model::{Layer, metric};
use serde_json::{Value, json};

use crate::canvas::view;
use crate::theme::Tokens;
use crate::{FontcraftApp, control, tools_glue};

/// Glyph A with a square (100,100)–(300,300) in the first master `m01` and a rectangle
/// (50,50)–(450,350) in a second master `m02` whose x-height is 560.
fn two_masters() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.run("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    app.session
        .edit("setup", |font| {
            let mut m = font.default_master()?.clone();
            m.id = "m02".into();
            m.name = "Bold".into();
            Arc::make_mut(&mut font.masters).push(m);
            font.set_metric_pos("m02", metric::X_HEIGHT, 560.0)?;
            font.glyph_mut("A")?.layers.push(Layer::new("m02", 700.0));
            Ok(())
        })
        .unwrap();
    app.run("path.addRectangle", &json!({"glyph": "A", "layer": "m02", "x": 50, "y": 50, "width": 400, "height": 300})).unwrap();
    app
}

fn edit(mut app: FontcraftApp) -> FontcraftApp {
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
    app.run("view.setZoom", &json!({"size": 1000, "center": [400, 250]})).unwrap();
    app
}

fn layer(app: &FontcraftApp, id: &str) -> Layer {
    app.session.font().unwrap().glyph("A").unwrap().layers.iter().find(|l| l.id == id).unwrap().clone()
}

/// The index in path 0 of `layer` of the node at (x, y).
fn node_at(layer: &Layer, x: f64, y: f64) -> usize {
    let fontcraft_model::Shape::Path(path) = &layer.shapes[0] else { panic!("shape 0 is a path") };
    path.nodes.iter().position(|n| n.x == x && n.y == y).unwrap()
}

fn drag(app: &mut FontcraftApp, from: [f64; 2], to: [f64; 2]) {
    let events = json!([
        {"kind": "down", "x": from[0], "y": from[1]},
        {"kind": "drag", "x": (from[0] + to[0]) / 2.0, "y": (from[1] + to[1]) / 2.0},
        {"kind": "drag", "x": to[0], "y": to[1]},
        {"kind": "up", "x": to[0], "y": to[1]},
    ]);
    let r = control::handle(app, &json!({"id": 1, "method": "ui.pointer", "params": {"events": events}}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["result"]["errors"], json!([]), "{r}");
}

/// A drag in the active layer gives that layer what the headless command with `layer` gives it in
/// `headless` (the same font), leaves the other layers alone, and undoes in one step.
fn drag_edits_only(mut app: FontcraftApp, mut headless: FontcraftApp, active: &str, from: [f64; 2]) {
    let before = app.session.font().unwrap().glyph("A").unwrap().clone();
    let node = node_at(&layer(&app, active), from[0], from[1]);
    drag(&mut app, from, [from[0] + 30.0, from[1] + 10.0]);
    headless.run("selection.move", &json!({"glyph": "A", "layer": active, "points": [[0, node]], "dx": 30, "dy": 10})).unwrap();
    let headless = headless.session;
    let after = app.session.font().unwrap().glyph("A").unwrap().clone();
    for l in &after.layers {
        let old = before.layers.iter().find(|o| o.id == l.id).unwrap();
        if l.id == active {
            assert_ne!(l, old, "the drag edits the active layer `{active}`");
            assert_eq!(
                l,
                headless.font().unwrap().glyph("A").unwrap().layers.iter().find(|h| h.id == l.id).unwrap(),
                "as selection.move with `layer`"
            );
        } else {
            assert_eq!(l, old, "layer `{}` is left alone", l.id);
        }
    }
    app.run("edit.undo", &json!({})).unwrap();
    assert_eq!(app.session.font().unwrap().glyph("A").unwrap(), &before, "one undo step");
}

#[test]
fn a_drag_edits_the_second_master_after_select_master() {
    let mut app = edit(two_masters());
    let r = app.run("view.selectMaster", &json!({"master": 1})).unwrap();
    assert_eq!((r["master"].as_str(), r["layer"].as_str()), (Some("m02"), Some("m02")));
    drag_edits_only(app, two_masters(), "m02", [50.0, 50.0]);
}

#[test]
fn a_drag_edits_a_backup_layer_picked_in_the_layers_section() {
    let with_backup = || {
        let mut app = two_masters();
        let r = app.run("layers.addBackupLayer", &json!({"glyphs": ["A"], "master": "m01"})).unwrap();
        (app, r["layer"].as_str().unwrap().to_string())
    };
    let (app, backup) = with_backup();
    let (headless, same) = with_backup();
    assert_eq!(backup, same, "backup ids are reproducible");
    let mut app = edit(app);
    let r = app.run("layers.select", &json!({"layer": backup})).unwrap();
    assert_eq!((r["layer"].as_str(), r["master"].as_str()), (Some(backup.as_str()), Some("m01")));
    drag_edits_only(app, headless, &backup, [100.0, 100.0]);
}

#[test]
fn the_canvas_and_the_metric_lines_follow_the_active_master() {
    let mut app = edit(two_masters());
    let ctx = egui::Context::default();
    let t = Tokens::dark();
    // The width of the outline drawn in the outline colour, in screen points (zoom 1000: one point
    // per unit).
    let outline_width = |app: &mut FontcraftApp| {
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| app.ui(ui));
        out.textures_delta.clear();
        let mut shapes: Vec<Shape> = out.shapes.into_iter().map(|c| c.shape).collect();
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        while let Some(shape) = shapes.pop() {
            match shape {
                Shape::Vec(list) => shapes.extend(list),
                Shape::Path(p) if p.stroke.color == egui::epaint::ColorMode::Solid(t.outline) => {
                    for q in &p.points {
                        lo = lo.min(q.x);
                        hi = hi.max(q.x);
                    }
                }
                _ => {}
            }
        }
        hi - lo
    };
    assert!((outline_width(&mut app) - 200.0).abs() < 1.0, "the first master's square");
    app.run("view.selectMaster", &json!({"master": "Bold"})).unwrap();
    assert!((outline_width(&mut app) - 400.0).abs() < 1.0, "the second master's rectangle");
    let font = app.session.font().unwrap();
    let tab = app.state.tab().unwrap();
    assert_eq!(view::active_master(font, tab).unwrap().id, "m02");
    assert!(tools_glue::scene(font, tab).unwrap().metric_lines.contains(&560.0), "snapping uses the second master's x-height");
    let (lines, _) = crate::canvas::metrics::of(font, view::active_master(font, tab).unwrap());
    assert!(lines.iter().any(|(kind, pos)| kind == metric::X_HEIGHT && *pos == 560.0), "the metric lines are the second master's: {lines:?}");
    // `ui.render` draws the tab's master, unless it names one.
    let render =
        |app: &mut FontcraftApp, params: Value| control::handle(app, &json!({"id": 1, "method": "ui.render", "params": params}))["result"].clone();
    let bold = render(&mut app, json!({"scale": 0.1}));
    let regular = render(&mut app, json!({"scale": 0.1, "master": "m01"}));
    assert_eq!((bold["master"].as_str(), regular["master"].as_str()), (Some("m02"), Some("m01")));
    assert_ne!(bold["png"], regular["png"]);

    // Picking the first master's layer in the Layers section switches back for this glyph only.
    app.run("layers.select", &json!({"layer": "m01"})).unwrap();
    assert!((outline_width(&mut app) - 200.0).abs() < 1.0);
}

#[test]
fn shift_and_cmd_clicks_extend_the_row_selection_without_moving_the_active_layer() {
    let mut app = edit(two_masters());
    let backup = app.run("layers.addBackupLayer", &json!({"glyphs": ["A"], "master": "m01"})).unwrap()["layer"].as_str().unwrap().to_string();
    // Palette order: m01, its backup, m02.
    let r = app.run("layers.select", &json!({"layer": "m02", "extend": "range"})).unwrap();
    assert_eq!(r["selection"], json!(["m01", backup, "m02"]));
    assert_eq!(r["layer"], "m01");
    let r = app.run("layers.select", &json!({"layer": backup, "extend": "toggle"})).unwrap();
    assert_eq!(r["selection"], json!(["m01", "m02"]));
    assert!(app.run("layers.select", &json!({"layer": "nope"})).is_err());
    assert!(app.run("view.selectMaster", &json!({"master": 2})).is_err(), "two masters");
    let tabs = app.state.tabs.clone();
    assert!(app.run("layers.select", &json!({"layer": "m02", "glyphs": ["B"]})).is_err(), "B is not in the tab");
    assert_eq!(app.state.tabs, tabs, "a command that fails changes nothing");
}

#[test]
fn cmd_digits_select_masters() {
    let (id, params) = crate::shell::keymap::command_for(egui::Key::Num2, egui::Modifiers::COMMAND).unwrap();
    assert_eq!((id, params), ("view.selectMaster", json!({"master": 1})));
    let (id, _): (&str, Value) = crate::shell::keymap::command_for(egui::Key::Num1, egui::Modifiers::COMMAND | egui::Modifiers::ALT).unwrap();
    assert_ne!(id, "view.selectMaster", "Cmd+Alt+1 is Font View's");
}
