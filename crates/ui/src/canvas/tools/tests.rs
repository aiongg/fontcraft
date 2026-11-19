//! The tools behind the canvas: pointer and key input through the toolbox, as the canvas and the control channel
//! hand it in.

use std::sync::Arc;

use craft::behavior::kurbo::Point as KPoint;
use craft::behavior::{CanvasView, PointerButton};
use craft::host::Host;
use craft::keymap::Mods;
use craft::ui::canvas::{CanvasTool, ToolContext, ToolPointer};
use craft::ui::runtime::commands::CommandRef;
use egui::{Pos2, Rect, vec2};
use fontcraft_engine::Session;
use fontcraft_engine::model::{Layer, Shape};
use fontcraft_tools::{Key, KeyEvent, Modifiers, PointerEvent, PointerKind};
use serde_json::{Value, json};

use super::ToolInput;
use super::bridge::ToolBridge;
use crate::app::{APP, FontCraftApp, Msg, ViewMsg};
use crate::canvas::ActiveTool;
use crate::canvas::camera::Camera;

fn run(app: &mut FontCraftApp, id: &str, params: Value) {
    app.sent.push(Msg::run(id, params));
    app.drain();
    assert!(app.status.is_empty(), "{id}: {}", app.status);
}

fn shell(app: &mut FontCraftApp, id: &str, params: Value) {
    app.sent.push(Msg::Shell(CommandRef::with_params(id, params)));
    app.drain();
    assert!(app.status.is_empty(), "{id}: {}", app.status);
}

/// Glyph A with a 200-unit square (shape 0, its top right node at (300, 300) is node 2) and an open line (shape 1),
/// shown at one point per unit.
fn edit_app() -> FontCraftApp {
    let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
    app.sent.push(Msg::View(ViewMsg::SelectGlyph("A".into())));
    app.drain();
    run(&mut app, "path.addRectangle", json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200}));
    run(&mut app, "path.addPath", json!({"glyph": "A", "closed": false, "nodes": [{"x": 600, "y": 100}, {"x": 700, "y": 100}]}));
    app.view.canvas = CanvasView::new(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0)), KPoint::new(400.0, 400.0), 1.0);
    app
}

fn layer(app: &FontCraftApp) -> Layer {
    let font = app.session.font().unwrap();
    font.glyph("A").unwrap().layer(&font.masters[0].id).unwrap().clone()
}

fn node(app: &FontCraftApp, shape: usize, n: usize) -> (f64, f64) {
    match layer(app).shapes.get(shape) {
        Some(Shape::Path(p)) => (p.nodes[n].x, p.nodes[n].y),
        other => panic!("shape {shape} is not a path: {other:?}"),
    }
}

fn undo_steps(app: &FontCraftApp) -> usize {
    app.session.inspect()["documents"][0]["undo"].as_array().map_or(0, Vec::len)
}

fn call(app: &mut FontCraftApp, method: &str, params: Value) -> Value {
    crate::control::dispatch(app, method, &params).unwrap_or_else(|e| panic!("{method}: {e}"))
}

fn pointer(app: &mut FontCraftApp, events: Value) -> Value {
    call(app, "ui.pointer", json!({ "events": events }))
}

fn selection(app: &FontCraftApp) -> Value {
    serde_json::to_value(&app.tools.selection).unwrap()
}

/// A press, a drag and a release on a node move it like `selection.move`, as one undo step; Escape in the middle
/// of a drag leaves the font and the selection as they were.
#[test]
fn a_drag_moves_like_the_command_in_one_undo_step_and_escape_undoes_it() {
    let mut app = edit_app();
    let before = layer(&app);
    let steps = undo_steps(&app);
    pointer(&mut app, json!([{"kind": "move", "x": 300, "y": 300}, {"kind": "down", "x": 300, "y": 300}]));
    assert_eq!(selection(&app), json!({"points": [[0, 2]]}), "the press selects the node");
    pointer(&mut app, json!([{"kind": "drag", "x": 320, "y": 300}]));
    assert_eq!(app.session.interaction(), Some("Move"), "a drag runs one interaction");
    pointer(&mut app, json!([{"kind": "drag", "x": 350, "y": 330}, {"kind": "up", "x": 350, "y": 330}]));
    assert_eq!(app.session.interaction(), None);
    assert_eq!(undo_steps(&app), steps + 1, "the drag is one undo step");
    let mut headless = edit_app();
    run(&mut headless, "selection.move", json!({"glyph": "A", "points": [[0, 2]], "dx": 50, "dy": 30}));
    assert_eq!(layer(&app), layer(&headless), "the drag gives the font the command gives");
    run(&mut app, "edit.undo", json!({}));
    assert_eq!(layer(&app), before);

    shell(&mut app, "selection.set", json!({"points": [[0, 2], [0, 3]]}));
    pointer(&mut app, json!([{"kind": "down", "x": 300, "y": 300}, {"kind": "drag", "x": 340, "y": 340}]));
    assert_ne!(layer(&app), before, "the preview shows the move");
    let state = call(&mut app, "ui.key", json!({"key": "Escape"}));
    assert_eq!(state["handled"], true, "{state}");
    assert_eq!(app.session.interaction(), None);
    assert_eq!(layer(&app), before);
    assert_eq!(selection(&app), json!({"points": [[0, 2], [0, 3]]}));
    pointer(&mut app, json!([{"kind": "up", "x": 340, "y": 340}]));
    assert_eq!(layer(&app), before, "the release after Escape does nothing");
}

/// A marquee drag shows its rectangle as an overlay while it runs and selects what it encloses.
#[test]
fn a_marquee_is_drawn_while_dragged_and_selects() {
    let mut app = edit_app();
    pointer(&mut app, json!([{"kind": "down", "x": 50, "y": 50}, {"kind": "drag", "x": 250, "y": 250}]));
    assert!(!app.tools.view.overlays.is_empty(), "the marquee is drawn");
    pointer(&mut app, json!([{"kind": "drag", "x": 350, "y": 350}, {"kind": "up", "x": 350, "y": 350}]));
    assert_eq!(selection(&app), json!({"points": [[0, 0], [0, 1], [0, 2], [0, 3]]}));
    assert!(app.tools.view.overlays.is_empty());
    assert_eq!(app.tools.view.selection, app.tools.selection, "the selection layer paints the selection");
}

/// Arrows, Tab and the menu shortcuts reach the tools and the commands; an arrow move the engine refuses leaves its
/// error in the status line.
#[test]
fn keys_run_the_edit_keys_and_refusals_reach_the_status_line() {
    let mut app = edit_app();
    shell(&mut app, "selection.set", json!({"points": [[0, 2]]}));
    call(&mut app, "ui.key", json!({"key": "ArrowRight"}));
    call(&mut app, "ui.key", json!({"key": "ArrowUp", "shift": true}));
    assert_eq!(node(&app, 0, 2), (301.0, 310.0), "arrows move by 1, with Shift by 10");
    call(&mut app, "ui.key", json!({"key": "Tab"}));
    assert_eq!(selection(&app), json!({"points": [[0, 3]]}));
    let r = call(&mut app, "ui.key", json!({"key": "a", "cmd": true}));
    assert_eq!(r["command"], "edit.selectAll", "{r}");
    assert_eq!(app.tools.selection.points.len(), 6);

    run(&mut app, "glyph.toggleLocked", json!({"glyphs": ["A"]}));
    shell(&mut app, "selection.set", json!({"points": [[0, 2]]}));
    app.sent.push(Msg::Tool(ToolInput::Key(KeyEvent::press(Key::Right, Modifiers::NONE))));
    app.drain();
    assert!(app.status.starts_with("selection.move"), "{}", app.status);
}

/// Tool letters, Space and the rail choose the toolbox's tool, and the view state follows.
#[test]
fn tool_letters_space_and_the_rail_choose_tools() {
    let mut app = edit_app();
    call(&mut app, "ui.key", json!({"key": "h"}));
    assert_eq!((app.view.tool, app.tools.toolbox.active_id()), ("hand", "hand"));
    call(&mut app, "ui.key", json!({"key": "v", "shift": true}));
    assert_eq!(app.view.tool, "lasso");
    call(&mut app, "ui.key", json!({"key": "Space", "action": "press"}));
    assert_eq!(app.tools.toolbox.active_id(), "hand", "Space holds the Hand tool");
    call(&mut app, "ui.key", json!({"key": "Space", "action": "release"}));
    assert_eq!(app.tools.toolbox.active_id(), "lasso");
    assert_eq!(call(&mut app, "ui.tool.select", json!({"tool": "zoom"})), json!({"tool": "zoom", "previous": "lasso"}));
    assert!(crate::control::dispatch(&mut app, "ui.tool.select", &json!({"tool": "nope"})).is_err());
    app.sent.push(Msg::View(ViewMsg::SelectTool("draw")));
    app.drain();
    assert_eq!(app.tools.toolbox.base_id(), "draw", "a rail pick is the toolbox's tool");
    shell(&mut app, "tool.drawCircle", json!({}));
    assert_eq!(app.view.tool, "ellipse");
}

/// A menu command run without parameters takes its targets from the selection.
#[test]
fn menu_commands_take_their_targets_from_the_selection() {
    let mut app = edit_app();
    shell(&mut app, "selection.set", json!({"points": [[1, 0], [1, 1]]}));
    let master = app.session.font().unwrap().masters[0].id.clone();
    let filled = super::targets::with_targets(&mut app, CommandRef::new("edit.delete"));
    assert_eq!(filled.params, json!({"glyph": "A", "layer": master, "points": [[1, 0], [1, 1]]}));
    let given = CommandRef::with_params("edit.delete", json!({"glyph": "A", "points": [[0, 0]]}));
    assert_eq!(super::targets::with_targets(&mut app, given.clone()), given, "given parameters are kept");
}

/// The bridge posts what the canvas hands it in font units: a click run's second press adds a double click.
#[test]
fn the_bridge_posts_pointer_input_in_font_units() {
    let view = CanvasView::new(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0)), KPoint::new(400.0, 400.0), 2.0);
    let camera = Camera::new(view, 800.0);
    let mut bridge = ToolBridge::default();
    bridge.set_camera(camera);
    let mut cx = ToolContext::new(view.rect, None);
    let at = camera.to_screen(KPoint::new(300.0, 300.0));
    let press = ToolPointer::new(at, PointerButton::Primary, 0.0).mods(Mods::SHIFT).clicks(2);
    bridge.press(&press, &mut cx);
    bridge.release(&press, &mut cx);
    let sent = bridge.take_sent();
    let kinds: Vec<PointerKind> = sent
        .iter()
        .filter_map(|m| match m {
            Msg::Tool(ToolInput::Pointer(PointerEvent { kind, pos, mods })) => {
                assert!((pos.x - 300.0).abs() < 1e-6 && (pos.y - 300.0).abs() < 1e-6, "{pos:?}");
                assert!(mods.shift);
                Some(*kind)
            }
            _ => None,
        })
        .collect();
    assert_eq!(kinds, vec![PointerKind::Down, PointerKind::Up, PointerKind::DoubleClick]);
}
