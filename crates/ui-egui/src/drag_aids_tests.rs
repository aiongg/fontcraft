//! Drag aids driven over the control channel (`ui.pointer`): the lever, sliding a node along its
//! curve, Power Nudge and declared servant nodes.

use fontcraft_model::{Node, Path, Shape};
use fontcraft_tools::drag_aids::LEVER_RAMP;
use serde_json::{Value, json};

use crate::{FontcraftApp, control, tools_glue};

/// Glyph A with one open path, edited at actual size: a corner (0,0), handles (40,40) (80,80), a
/// smooth node (120,120) whose handles are diagonal, handles (160,160) (200,200), and the end
/// (240,240).
fn nudge_app() -> FontcraftApp {
    let nodes = json!([
        {"x": 0, "y": 0},
        {"x": 40, "y": 40, "type": "offcurve"}, {"x": 80, "y": 80, "type": "offcurve"}, {"x": 120, "y": 120, "type": "curve", "smooth": true},
        {"x": 160, "y": 160, "type": "offcurve"}, {"x": 200, "y": 200, "type": "offcurve"}, {"x": 240, "y": 240, "type": "curve"},
    ]);
    edit_app(nodes)
}

/// Glyph A with the curve (0,0) (0,200) (200,400) (400,400) cut in two at its middle: a smooth
/// node (125,275) between handles (50,200) and (200,350).
fn split_app() -> FontcraftApp {
    let nodes = json!([
        {"x": 0, "y": 0},
        {"x": 0, "y": 100, "type": "offcurve"}, {"x": 50, "y": 200, "type": "offcurve"}, {"x": 125, "y": 275, "type": "curve", "smooth": true},
        {"x": 200, "y": 350, "type": "offcurve"}, {"x": 300, "y": 400, "type": "offcurve"}, {"x": 400, "y": 400, "type": "curve"},
    ]);
    edit_app(nodes)
}

fn edit_app(nodes: Value) -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.run("path.addPath", &json!({"glyph": "A", "closed": false, "nodes": nodes})).unwrap();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
    app.run("view.setZoom", &json!({"size": 1000, "center": [200, 200]})).unwrap();
    app
}

fn path(app: &FontcraftApp) -> Path {
    let font = app.session.font().unwrap();
    let master = &font.masters[0].id;
    match font.glyph("A").unwrap().layer(master).unwrap().shapes.first() {
        Some(Shape::Path(p)) => p.clone(),
        other => panic!("not a path: {other:?}"),
    }
}

fn at(app: &FontcraftApp, i: usize) -> (f64, f64) {
    let n: Node = path(app).nodes[i].clone();
    (n.x, n.y)
}

fn undo_steps(app: &FontcraftApp) -> usize {
    app.session.inspect()["documents"][0]["undo"].as_array().map_or(0, Vec::len)
}

fn pointer(app: &mut FontcraftApp, events: Value) {
    let r = control::handle(app, &json!({"id": 1, "method": "ui.pointer", "params": {"events": events}}));
    assert_eq!(r["ok"], true, "{r}");
}

/// Press at `from`, drag through the middle to `to` and release, holding `mods` throughout.
fn drag(app: &mut FontcraftApp, from: (f64, f64), to: (f64, f64), mods: &[&str]) {
    let mid = ((from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0);
    pointer(
        app,
        json!([
            {"kind": "down", "x": from.0, "y": from.1, "mods": mods},
            {"kind": "drag", "x": mid.0, "y": mid.1, "mods": mods},
            {"kind": "drag", "x": to.0, "y": to.1, "mods": mods},
            {"kind": "up", "x": to.0, "y": to.1, "mods": mods},
        ]),
    );
}

#[test]
fn the_lever_slows_a_drag_by_the_ratio_setting() {
    let mut app = nudge_app();
    app.run("settings.set", &json!({"id": "editing.leverRatio", "value": 4})).unwrap();
    // A drag shorter than the lever's ramp, in whole units the ratio divides.
    let ramp = tools_glue::current_view(&app).unwrap().units(LEVER_RAMP);
    let d = 4.0 * (ramp / 2.0 / 4.0).floor();
    assert!(d >= 8.0, "the ramp is {ramp} units");
    let steps = undo_steps(&app);
    drag(&mut app, (240.0, 240.0), (240.0 + d, 240.0), &["cmd", "ctrl"]);
    assert_eq!(at(&app, 6), (240.0 + d / 4.0, 240.0));
    assert_eq!(undo_steps(&app), steps + 1);

    // Without the modifier the node follows the pointer.
    drag(&mut app, (240.0 + d / 4.0, 240.0), (240.0 + d / 4.0 + d, 240.0), &["ctrl"]);
    assert_eq!(at(&app, 6), (240.0 + d / 4.0 + d, 240.0));
}

#[test]
fn shift_alt_dragging_a_node_slides_it_along_the_curve() {
    let mut app = split_app();
    let before = path(&app);
    let steps = undo_steps(&app);
    // (253.125, 365.625) is the whole curve's point at t = 0.75.
    drag(&mut app, (125.0, 275.0), (253.0, 366.0), &["shift", "alt"]);
    let (x, y) = at(&app, 3);
    assert!((x - 253.0).abs() <= 1.0 && (y - 366.0).abs() <= 1.0, "the node slid to ({x}, {y})");
    assert_eq!(undo_steps(&app), steps + 1, "one undo step");
    // The first handle now reaches three quarters of the way to the old one's line end:
    // the merged curve runs from t = 0 to 0.75 of the whole one, whose first handle is (0,200).
    let (hx, hy) = at(&app, 1);
    assert!(hx.abs() <= 1.0 && (hy - 150.0).abs() <= 1.0, "first handle at ({hx}, {hy})");
    app.run("edit.undo", &json!({})).unwrap();
    assert_eq!(path(&app), before);
}

#[test]
fn power_nudge_moves_smooth_neighbours_in_proportion() {
    let mut app = nudge_app();
    assert_eq!(app.run("path.togglePowerNudge", &json!({})).unwrap()["on"], json!(true));
    let steps = undo_steps(&app);
    drag(&mut app, (240.0, 240.0), (264.0, 240.0), &["ctrl"]);
    assert_eq!(at(&app, 6), (264.0, 240.0));
    assert_eq!((at(&app, 2), at(&app, 3), at(&app, 4)), ((92.0, 80.0), (132.0, 120.0), (172.0, 160.0)), "halfway to the fixed corner: half the drag");
    assert_eq!(at(&app, 0), (0.0, 0.0), "the corner stays");
    assert_eq!(undo_steps(&app), steps + 1, "one undo step");

    // Switched off, the same drag moves the end alone.
    app.run("edit.undo", &json!({})).unwrap();
    app.run("path.togglePowerNudge", &json!({"on": false})).unwrap();
    drag(&mut app, (240.0, 240.0), (264.0, 240.0), &["ctrl"]);
    assert_eq!((at(&app, 3), at(&app, 6)), ((120.0, 120.0), (264.0, 240.0)));
}

#[test]
fn a_declared_servant_follows_on_its_axis_only() {
    let mut app = nudge_app();
    app.run("path.setServantNode", &json!({"glyph": "A", "points": [[0, 3]], "axis": "y"})).unwrap();
    drag(&mut app, (240.0, 240.0), (264.0, 264.0), &["ctrl"]);
    assert_eq!(at(&app, 6), (264.0, 264.0));
    assert_eq!(at(&app, 3), (120.0, 132.0), "half the drag, on y only");
}
