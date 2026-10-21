//! The Select tool's drag modifiers driven over the control channel (`ui.pointer`): each gesture
//! commits one command, and that command run on a fresh copy of the glyph gives the same outline.

use fontcraft_model::{Path, Shape};
use serde_json::{Value, json};

use fontcraft_tools::PointerKind;

use crate::tools_glue::{self, RawPointer, Space};
use crate::{FontcraftApp, control};

/// A corner (0,0), handles (0,100) (100,200), a smooth node (200,200), handles (300,200)
/// (400,100), and a corner (400,0).
fn smooth_nodes() -> Value {
    json!([
        {"x": 0, "y": 0},
        {"x": 0, "y": 100, "type": "offcurve"}, {"x": 100, "y": 200, "type": "offcurve"}, {"x": 200, "y": 200, "type": "curve", "smooth": true},
        {"x": 300, "y": 200, "type": "offcurve"}, {"x": 400, "y": 100, "type": "offcurve"}, {"x": 400, "y": 0, "type": "curve"},
    ])
}

/// One cubic from (0,0) to (300,300): the first handle points up, the second left. Its point at
/// t = 0.5 is (112.5, 187.5).
fn bend_nodes() -> Value {
    json!([
        {"x": 0, "y": 0},
        {"x": 0, "y": 100, "type": "offcurve"}, {"x": 200, "y": 300, "type": "offcurve"}, {"x": 300, "y": 300, "type": "curve"},
    ])
}

fn edit_app(nodes: &Value) -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.run("path.addPath", &json!({"glyph": "A", "closed": false, "nodes": nodes})).unwrap();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
    app.run("view.setZoom", &json!({"size": 1000, "center": [200, 200]})).unwrap();
    app
}

fn path(app: &FontcraftApp) -> Path {
    let font = app.session.font().unwrap();
    let master = &font.masters[0].id;
    match font.glyph("A").unwrap().layer(master).unwrap().shapes.last() {
        Some(Shape::Path(p)) => p.clone(),
        other => panic!("not a path: {other:?}"),
    }
}

fn shape_count(app: &FontcraftApp) -> usize {
    let font = app.session.font().unwrap();
    font.glyph("A").unwrap().layer(&font.masters[0].id).unwrap().shapes.len()
}

fn at(p: &Path, i: usize) -> (f64, f64) {
    (p.nodes[i].x, p.nodes[i].y)
}

fn undo_steps(app: &FontcraftApp) -> usize {
    app.session.inspect()["documents"][0]["undo"].as_array().map_or(0, Vec::len)
}

/// Press at `from`, drag through the middle to `to` and release, holding `mods` throughout.
/// Returns the command the gesture committed, after checking it made one undo step.
fn gesture(app: &mut FontcraftApp, from: (f64, f64), to: (f64, f64), mods: &[&str]) -> (String, Value) {
    let steps = undo_steps(app);
    let mid = ((from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0);
    let events = json!([
        {"kind": "down", "x": from.0, "y": from.1, "mods": mods},
        {"kind": "drag", "x": mid.0, "y": mid.1, "mods": mods},
        {"kind": "drag", "x": to.0, "y": to.1, "mods": mods},
        {"kind": "up", "x": to.0, "y": to.1, "mods": mods},
    ]);
    let r = control::handle(app, &json!({"id": 1, "method": "ui.pointer", "params": {"events": events}}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(undo_steps(app), steps + 1, "one undo step");
    app.session.journal().last().cloned().expect("the gesture committed a command")
}

/// The egui modifiers egui-winit reports while `keys` are down. `cmd` is the Command key on the
/// Mac; elsewhere it stands for the Windows or Super key, which egui-winit leaves out.
fn native(mac: bool, keys: &[&str]) -> egui::Modifiers {
    let has = |k: &str| keys.contains(&k);
    let ctrl = has("ctrl");
    let command_key = has("cmd");
    egui::Modifiers { alt: has("alt"), ctrl, shift: has("shift"), mac_cmd: mac && command_key, command: if mac { command_key } else { ctrl } }
}

/// [`gesture`] as real input gives it: the keys of `keys` on a Mac (`mac`) or another system,
/// through the same modifier mapping the canvas uses. Returns the undo steps it added.
fn native_gesture(app: &mut FontcraftApp, from: (f64, f64), to: (f64, f64), mac: bool, keys: &[&str]) -> usize {
    app.tools.toolbox.set_keyboard(tools_glue::keyboard(mac));
    let steps = undo_steps(app);
    let mods = tools_glue::pointer_mods(native(mac, keys));
    let mid = ((from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0);
    let events = [(PointerKind::Down, from), (PointerKind::Drag, mid), (PointerKind::Drag, to), (PointerKind::Up, to)];
    for (kind, (x, y)) in events {
        let errors = tools_glue::pointer(app, RawPointer { kind, at: fontcraft_geom::Point::new(x, y), space: Space::Font, mods });
        assert!(errors.is_empty(), "{errors:?}");
    }
    undo_steps(app) - steps
}

/// The outline `command` gives when run headless on a fresh copy of `nodes`.
fn replay(nodes: &Value, (id, params): &(String, Value)) -> Path {
    let mut fresh = edit_app(nodes);
    fresh.run(id, params).unwrap_or_else(|e| panic!("{id} {params}: {e}"));
    path(&fresh)
}

#[test]
fn alt_dragging_a_handle_keeps_its_angle() {
    let nodes = smooth_nodes();
    let mut app = edit_app(&nodes);
    let command = gesture(&mut app, (300.0, 200.0), (340.0, 263.0), &["alt"]);
    assert_eq!(command.0, "selection.move");
    assert_eq!(command.1["mode"], json!("pointsOnly"));
    let p = path(&app);
    let (x, y) = at(&p, 4);
    assert!(y == 200.0 && x > 300.0, "the level handle stays level and grows: ({x}, {y})");
    assert_eq!(p, replay(&nodes, &command));
}

#[test]
fn ctrl_alt_dragging_a_smooth_nodes_handle_mirrors_the_other() {
    let nodes = smooth_nodes();
    let mut app = edit_app(&nodes);
    let command = gesture(&mut app, (300.0, 200.0), (340.0, 263.0), &["ctrl", "alt"]);
    assert_eq!(command.0, "selection.move");
    assert_eq!(command.1["mode"], json!("mirror"));
    let p = path(&app);
    assert_eq!((at(&p, 4), at(&p, 2)), ((340.0, 263.0), (60.0, 137.0)), "the opposite handle mirrors through (200,200)");
    assert_eq!(p, replay(&nodes, &command));
    // The same keys from a real keyboard: off the Mac Control also reports `command` (which
    // slows the drag with the lever there), and the drag must still mirror rather than hold the
    // Measurement tool.
    for mac in [true, false] {
        let mut app = edit_app(&nodes);
        assert_eq!(native_gesture(&mut app, (300.0, 200.0), (340.0, 263.0), mac, &["ctrl", "alt"]), 1, "mac {mac}: one undo step");
        let command = app.session.journal().last().cloned().expect("the drag committed a command");
        assert_eq!((command.0.as_str(), &command.1["mode"]), ("selection.move", &json!("mirror")), "mac {mac}");
        let q = path(&app);
        let (moved, opposite) = (at(&q, 4), at(&q, 2));
        assert_ne!(moved, (300.0, 200.0), "mac {mac}: the handle moved");
        assert!((moved.0 + opposite.0 - 400.0).abs() <= 1.0 && (moved.1 + opposite.1 - 400.0).abs() <= 1.0, "mac {mac}: {moved:?} {opposite:?}");
        assert_eq!(q, replay(&nodes, &command), "mac {mac}");
    }
}

#[test]
fn the_measurement_keys_of_each_system_hold_the_measurement_tool_for_a_drag() {
    let nodes = smooth_nodes();
    for (mac, keys) in [(true, &["cmd", "alt", "ctrl"][..]), (false, &["ctrl", "alt", "shift"][..])] {
        let mut app = edit_app(&nodes);
        app.tools.toolbox.set_keyboard(tools_glue::keyboard(mac));
        let mods = tools_glue::pointer_mods(native(mac, keys));
        let down = RawPointer { kind: PointerKind::Down, at: fontcraft_geom::Point::new(300.0, 200.0), space: Space::Font, mods };
        tools_glue::pointer(&mut app, down);
        assert_eq!(app.tools.toolbox.active_id(), "measurement", "{keys:?}");
        let up = RawPointer { kind: PointerKind::Up, mods: fontcraft_tools::Modifiers::NONE, ..down };
        tools_glue::pointer(&mut app, up);
        assert_eq!(app.tools.toolbox.active_id(), "select", "{keys:?}: the release gives way");
        assert_eq!(native_gesture(&mut app, (300.0, 200.0), (340.0, 263.0), mac, keys), 0, "{keys:?}: measuring changes nothing");
        assert_eq!(path(&app), path(&edit_app(&nodes)), "{keys:?}: the outline stays");
    }
}

#[test]
fn alt_dragging_a_curve_segment_changes_only_its_handle_lengths() {
    let nodes = bend_nodes();
    let mut app = edit_app(&nodes);
    let command = gesture(&mut app, (112.5, 187.5), (100.0, 200.0), &["alt"]);
    assert_eq!(command.0, "path.reshapeSegment");
    let p = path(&app);
    assert_eq!((at(&p, 0), at(&p, 3)), ((0.0, 0.0), (300.0, 300.0)), "the nodes stay");
    assert_eq!((p.nodes[1].x, p.nodes[2].y), (0.0, 300.0), "each handle keeps its direction");
    let (x, y) = (0.125 * 0.0 + 0.375 * p.nodes[1].x + 0.375 * p.nodes[2].x + 0.125 * 300.0, 0.375 * p.nodes[1].y + 0.375 * 300.0 + 0.125 * 300.0);
    assert!((x - 100.0).abs() <= 1.5 && (y - 200.0).abs() <= 1.5, "the dragged point follows the pointer: ({x}, {y})");
    assert_eq!(p, replay(&nodes, &command));
}

#[test]
fn alt_dragging_a_lone_node_moves_only_that_node() {
    let nodes = smooth_nodes();
    let mut app = edit_app(&nodes);
    let command = gesture(&mut app, (400.0, 0.0), (437.0, 61.0), &["alt"]);
    assert_eq!(command.0, "selection.move");
    assert_eq!(command.1["mode"], json!("pointsOnly"));
    assert!(command.1.get("duplicate").is_none(), "nothing to copy");
    assert_eq!(shape_count(&app), 1, "no copy");
    let p = path(&app);
    assert_ne!(at(&p, 6), (400.0, 0.0), "the node moved");
    assert_eq!(at(&p, 5), (400.0, 100.0), "its handle stayed");
    assert_eq!(p, replay(&nodes, &command));
}
