//! The drawing tools in the shell, driven over the control channel (`ui.pointer`, `ui.key`):
//! each gesture gives the outline the headless commands give.

use fontcraft_model::{NodeKind, Path, Shape};
use serde_json::{Value, json};

use crate::{FontcraftApp, control};

/// The app editing `glyph` (an empty glyph of the new font) at actual size.
fn editing(glyph: &str) -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": [glyph]})).unwrap();
    app.run("view.setZoom", &json!({"size": 1000, "center": [300, 300]})).unwrap();
    app
}

fn call(app: &mut FontcraftApp, method: &str, params: Value) -> Value {
    control::handle(app, &json!({"id": 1, "method": method, "params": params}))
}

/// Pointer events in font units; every action they ask for must succeed.
fn pointer(app: &mut FontcraftApp, events: Value) -> Value {
    let r = call(app, "ui.pointer", json!({ "events": events }));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["result"]["errors"], json!([]), "{r}");
    r["result"].clone()
}

fn key(app: &mut FontcraftApp, params: Value) -> Value {
    let r = call(app, "ui.key", params);
    assert_eq!(r["ok"], true, "{r}");
    r["result"].clone()
}

fn click(app: &mut FontcraftApp, x: f64, y: f64) {
    pointer(app, json!([{"kind": "down", "x": x, "y": y}, {"kind": "up", "x": x, "y": y}]));
}

/// A drag from (x, y) to (tx, ty) with `mods` held throughout.
fn drag(app: &mut FontcraftApp, (x, y): (f64, f64), (tx, ty): (f64, f64), mods: Value) {
    let mid = ((x + tx) / 2.0, (y + ty) / 2.0);
    pointer(
        app,
        json!([
            {"kind": "down", "x": x, "y": y, "mods": mods},
            {"kind": "drag", "x": mid.0, "y": mid.1, "mods": mods},
            {"kind": "drag", "x": tx, "y": ty, "mods": mods},
            {"kind": "up", "x": tx, "y": ty, "mods": mods},
        ]),
    );
}

fn paths(app: &FontcraftApp, glyph: &str) -> Vec<Path> {
    let font = app.session.font().unwrap();
    let layer = font.glyph(glyph).unwrap().layer(&font.masters[0].id).unwrap();
    layer
        .shapes
        .iter()
        .map(|s| match s {
            Shape::Path(p) => p.clone(),
            other => panic!("not a path: {other:?}"),
        })
        .collect()
}

/// A path's nodes as `x,y,kind`: `l` line, `c` curve, `o` off-curve, `*` for smooth.
fn nodes(p: &Path) -> Vec<String> {
    p.nodes
        .iter()
        .map(|n| {
            let kind = match n.kind {
                NodeKind::Line => "l",
                NodeKind::Curve => "c",
                NodeKind::QCurve => "q",
                NodeKind::OffCurve => "o",
            };
            format!("{},{},{}{}", n.x, n.y, kind, if n.smooth { "*" } else { "" })
        })
        .collect()
}

fn undo_steps(app: &FontcraftApp) -> usize {
    app.session.inspect()["documents"][0]["undo"].as_array().map_or(0, Vec::len)
}

fn none() -> Value {
    json!([])
}

#[test]
fn an_o_with_a_counter_drawn_with_drags() {
    let mut app = editing("o");
    app.run("tool.select", &json!({"tool": "draw"})).unwrap();
    let steps = undo_steps(&app);
    // Outer contour, counter-clockwise: a drag at each extreme, then a click on the first node.
    drag(&mut app, (250.0, 50.0), (360.0, 50.0), none());
    drag(&mut app, (450.0, 250.0), (450.0, 360.0), none());
    drag(&mut app, (250.0, 450.0), (140.0, 450.0), none());
    drag(&mut app, (50.0, 250.0), (50.0, 140.0), none());
    click(&mut app, 250.0, 50.0);
    // The counter, clockwise.
    drag(&mut app, (250.0, 150.0), (195.0, 150.0), none());
    drag(&mut app, (150.0, 250.0), (150.0, 305.0), none());
    drag(&mut app, (250.0, 350.0), (305.0, 350.0), none());
    drag(&mut app, (350.0, 250.0), (350.0, 195.0), none());
    click(&mut app, 250.0, 150.0);

    let drawn = paths(&app, "o");
    assert_eq!(drawn.len(), 2, "{}", app.state.status);
    assert!(drawn.iter().all(|p| p.closed));
    let outer = [
        "250,50,c*",
        "360,50,o",
        "450,140,o",
        "450,250,c*",
        "450,360,o",
        "360,450,o",
        "250,450,c*",
        "140,450,o",
        "50,360,o",
        "50,250,c*",
        "50,140,o",
        "140,50,o",
    ];
    let counter = [
        "250,150,c*",
        "195,150,o",
        "150,195,o",
        "150,250,c*",
        "150,305,o",
        "195,350,o",
        "250,350,c*",
        "305,350,o",
        "350,305,o",
        "350,250,c*",
        "350,195,o",
        "305,150,o",
    ];
    assert_eq!(nodes(&drawn[0]), outer);
    assert_eq!(nodes(&drawn[1]), counter);
    assert_eq!(undo_steps(&app), steps + 10, "one undo step per node and per close");

    let mut headless = editing("o");
    headless.run("path.addEllipse", &json!({"glyph": "o", "x": 50, "y": 50, "width": 400, "height": 400})).unwrap();
    assert_eq!(nodes(&paths(&headless, "o")[0]), outer, "the drags give the ellipse the command gives");
}

#[test]
fn an_n_drawn_with_clicks_and_alt_drags() {
    let mut app = editing("n");
    key(&mut app, json!({"key": "p"}));
    assert_eq!(app.tools.toolbox.active_id(), "draw");
    let steps = undo_steps(&app);
    let alt = || json!(["alt"]);
    click(&mut app, 50.0, 0.0);
    click(&mut app, 130.0, 0.0);
    // Alt from the start: a corner with an outgoing handle only.
    drag(&mut app, (130.0, 380.0), (130.0, 420.0), alt());
    drag(&mut app, (250.0, 450.0), (310.0, 450.0), none());
    // Alt pressed during the drag: the incoming handle stays; back on the node, no outgoing one.
    pointer(
        &mut app,
        json!([
            {"kind": "down", "x": 370, "y": 380},
            {"kind": "drag", "x": 370, "y": 340},
            {"kind": "drag", "x": 370, "y": 380, "mods": ["alt"]},
            {"kind": "up", "x": 370, "y": 380, "mods": ["alt"]},
        ]),
    );
    click(&mut app, 370.0, 0.0);
    click(&mut app, 450.0, 0.0);
    drag(&mut app, (450.0, 400.0), (450.0, 460.0), alt());
    drag(&mut app, (260.0, 510.0), (190.0, 510.0), none());
    pointer(
        &mut app,
        json!([
            {"kind": "down", "x": 130, "y": 450},
            {"kind": "drag", "x": 100, "y": 410},
            {"kind": "drag", "x": 130, "y": 450, "mods": ["alt"]},
            {"kind": "up", "x": 130, "y": 450, "mods": ["alt"]},
        ]),
    );
    click(&mut app, 130.0, 500.0);
    click(&mut app, 50.0, 500.0);
    click(&mut app, 50.0, 0.0);

    let drawn = paths(&app, "n");
    assert_eq!(drawn.len(), 1, "{}", app.state.status);
    assert!(drawn[0].closed);
    let expected = [
        "50,0,l",
        "130,0,l",
        "130,380,l",
        "130,420,o",
        "190,450,o",
        "250,450,c*",
        "310,450,o",
        "370,420,o",
        "370,380,c",
        "370,0,l",
        "450,0,l",
        "450,400,l",
        "450,460,o",
        "330,510,o",
        "260,510,c*",
        "190,510,o",
        "160,490,o",
        "130,450,c",
        "130,500,l",
        "50,500,l",
    ];
    assert_eq!(nodes(&drawn[0]), expected);
    assert_eq!(undo_steps(&app), steps + 13);

    let list: Vec<Value> = expected
        .iter()
        .map(|s| {
            let parts: Vec<&str> = s.split(',').collect();
            let kind = match parts[2].trim_end_matches('*') {
                "l" => "line",
                "c" => "curve",
                _ => "offcurve",
            };
            json!({"x": parts[0].parse::<f64>().unwrap(), "y": parts[1].parse::<f64>().unwrap(), "type": kind, "smooth": parts[2].ends_with('*')})
        })
        .collect();
    let mut headless = editing("n");
    headless.run("path.addPath", &json!({"glyph": "n", "nodes": list})).unwrap();
    assert_eq!(nodes(&paths(&headless, "n")[0]), expected, "the clicks and drags give the path path.addPath gives");
}

#[test]
fn cmd_keeps_the_angle_space_moves_the_node_and_escape_ends_the_path() {
    let mut app = editing("c");
    app.run("tool.select", &json!({"tool": "draw"})).unwrap();
    drag(&mut app, (100.0, 100.0), (200.0, 100.0), none());
    // Waiting for the next node: the rubber band and the pending handle are drawn.
    let r = pointer(&mut app, json!([{"kind": "move", "x": 300, "y": 300}]));
    let kinds: Vec<&str> = r["overlays"].as_array().unwrap().iter().filter_map(|o| o["kind"].as_str()).collect();
    assert!(kinds.contains(&"drawPreview") && kinds.contains(&"drawHandle"), "{r}");
    let r = pointer(&mut app, json!([{"kind": "down", "x": 300, "y": 300}, {"kind": "drag", "x": 400, "y": 300}]));
    assert_eq!(r["interaction"], "Draw");
    assert!(r["overlays"].as_array().unwrap().iter().any(|o| o["kind"] == "drawHandle"), "{r}");
    pointer(&mut app, json!([{"kind": "drag", "x": 500, "y": 350, "mods": ["cmd"]}]));
    key(&mut app, json!({"key": "Space", "action": "press"}));
    assert_eq!(app.tools.toolbox.active_id(), "draw", "Space during a drag stays with the Draw tool");
    pointer(&mut app, json!([{"kind": "drag", "x": 520, "y": 370}, {"kind": "up", "x": 520, "y": 370}]));
    key(&mut app, json!({"key": "Space", "action": "release"}));
    let drawn = paths(&app, "c");
    assert_eq!(nodes(&drawn[0]), ["100,100,l*", "200,100,o", "220,320,o", "320,320,c*"]);
    // Escape ends the path: the next click starts another one.
    key(&mut app, json!({"key": "Escape"}));
    click(&mut app, 600.0, 100.0);
    let drawn = paths(&app, "c");
    assert_eq!(drawn.len(), 2);
    assert_eq!(nodes(&drawn[1]), ["600,100,l"]);
    assert_eq!(nodes(&drawn[0]).len(), 4);
}

#[test]
fn a_click_opens_a_path_continues_an_end_and_shift_click_adds_an_extremum() {
    let mut app = editing("d");
    app.run("path.addRectangle", &json!({"glyph": "d", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    app.run("tool.select", &json!({"tool": "draw"})).unwrap();
    let steps = undo_steps(&app);
    click(&mut app, 300.0, 300.0);
    let p = &paths(&app, "d")[0];
    assert!(!p.closed);
    assert_eq!(nodes(p), ["300,300,l", "100,300,l", "100,100,l", "300,100,l", "300,300,l"], "two ends where the node was");
    assert_eq!(undo_steps(&app), steps + 1);
    // A click on an open end continues the path from there.
    click(&mut app, 300.0, 300.0);
    click(&mut app, 400.0, 400.0);
    let p = &paths(&app, "d")[0];
    assert_eq!(nodes(p), ["400,400,l", "300,300,l", "100,300,l", "100,100,l", "300,100,l", "300,300,l"]);
    assert_eq!(serde_json::to_value(&app.state.tab().unwrap().selection).unwrap(), json!({"points": [[0, 0]]}));
    key(&mut app, json!({"key": "Escape"}));

    let arch = json!([{"x": 0, "y": 600}, {"x": 0, "y": 700, "type": "offcurve"}, {"x": 200, "y": 700, "type": "offcurve"}, {"x": 200, "y": 600, "type": "curve"}]);
    app.run("path.addPath", &json!({"glyph": "d", "closed": false, "nodes": arch})).unwrap();
    pointer(&mut app, json!([{"kind": "down", "x": 90, "y": 672, "mods": ["shift"]}, {"kind": "up", "x": 90, "y": 672, "mods": ["shift"]}]));
    let p = &paths(&app, "d")[1];
    assert_eq!(p.nodes.len(), 7);
    assert_eq!((p.nodes[3].x, p.nodes[3].y), (100.0, 675.0), "the node lands on the top extremum, not at the click");
}

#[test]
fn primitives_drag_with_shift_and_alt_and_a_click_asks_for_the_dialog() {
    let mut app = editing("r");
    key(&mut app, json!({"key": "f"}));
    assert_eq!(app.tools.toolbox.active_id(), "rectangle");
    drag(&mut app, (100.0, 100.0), (300.0, 250.0), none());
    key(&mut app, json!({"key": "f", "shift": true}));
    assert_eq!(app.tools.toolbox.active_id(), "ellipse", "Shift+F switches the mode");
    drag(&mut app, (500.0, 500.0), (550.0, 530.0), json!(["alt"]));
    drag(&mut app, (100.0, 400.0), (160.0, 380.0), json!(["shift"]));
    let mut headless = editing("r");
    for (id, p) in [
        ("path.addRectangle", json!({"x": 100, "y": 100, "width": 200, "height": 150})),
        ("path.addEllipse", json!({"x": 500, "y": 500, "width": 100, "height": 60, "fromCenter": true})),
        ("path.addEllipse", json!({"x": 100, "y": 400, "width": 60, "height": -60})),
    ] {
        let mut p = p;
        p["glyph"] = json!("r");
        headless.run(id, &p).unwrap();
    }
    let (a, b) = (paths(&app, "r"), paths(&headless, "r"));
    assert_eq!(a.iter().map(nodes).collect::<Vec<_>>(), b.iter().map(nodes).collect::<Vec<_>>());
    // A click opens the measurements dialog of the tool's mode at the click.
    let r = call(&mut app, "ui.pointer", json!({"events": [{"kind": "down", "x": 700, "y": 100}, {"kind": "up", "x": 700, "y": 100}]}));
    assert_eq!(r["result"]["errors"], json!([]), "{r}");
    let dialog = call(&mut app, "ui.dialog.inspect", json!({}))["result"].clone();
    assert_eq!((dialog["dialog"].clone(), dialog["command"].clone()), (json!("dialog.primitiveMeasurements"), json!("path.addEllipse")), "{dialog}");
    assert_eq!(dialog["params"]["glyph"], "r");
    assert_eq!(paths(&app, "r").len(), 4, "the preview shows the ellipse");
    call(&mut app, "ui.dialog.cancel", json!({}));
    assert_eq!(paths(&app, "r").len(), 3);
    app.run("tool.drawRectangle", &json!({})).unwrap();
    assert_eq!(app.tools.toolbox.active_id(), "rectangle");
    app.run("tool.drawCircle", &json!({})).unwrap();
    assert_eq!(app.tools.toolbox.active_id(), "ellipse");
}

#[test]
fn the_pencil_turns_a_stroke_into_one_path() {
    let mut app = editing("s");
    key(&mut app, json!({"key": "b"}));
    assert_eq!(app.tools.toolbox.active_id(), "pencil");
    let steps = undo_steps(&app);
    let arc = |from: f64, to: f64, n: usize| -> Vec<(f64, f64)> {
        (0..=n).map(|i| from + (to - from) * i as f64 / n as f64).map(|a| (300.0 - 150.0 * a.cos(), 300.0 + 150.0 * a.sin())).collect()
    };
    let stroke = |points: &[(f64, f64)]| -> Value {
        let last = points.len() - 1;
        Value::Array(
            points
                .iter()
                .enumerate()
                .map(|(i, (x, y))| json!({"kind": if i == 0 { "down" } else if i == last { "up" } else { "drag" }, "x": x, "y": y}))
                .collect(),
        )
    };
    let half = arc(0.0, std::f64::consts::PI, 120);
    pointer(&mut app, stroke(&half));
    let p = &paths(&app, "s")[0];
    assert!(!p.closed);
    assert_eq!((p.nodes[0].x, p.nodes[0].y), (150.0, 300.0));
    let last = p.nodes.last().unwrap();
    assert_eq!((last.x, last.y), (450.0, 300.0));
    for n in p.nodes.iter().filter(|n| n.on_curve()) {
        assert!(((n.x - 300.0).hypot(n.y - 300.0) - 150.0).abs() < 2.0, "({}, {}) is off the stroke", n.x, n.y);
    }
    assert_eq!(undo_steps(&app), steps + 1);
    let full = arc(0.0, 2.0 * std::f64::consts::PI, 240);
    pointer(&mut app, stroke(&full));
    let p = &paths(&app, "s")[1];
    assert!(p.closed, "a stroke that ends where it began closes");
}
