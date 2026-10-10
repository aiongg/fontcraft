//! The pointer tools in the shell, driven like a user would: egui events in headless frames, and
//! the control channel's `ui.pointer` and `ui.key`.

use egui::{Event, Modifiers as EguiMods, PointerButton, Pos2, Shape};
use fontcraft_geom::Point;
use fontcraft_model::Layer;
use serde_json::{Value, json};

use crate::theme::Tokens;
use crate::{FontcraftApp, control, tools_glue};

/// A headless window: one egui context kept across frames, so pointer state carries over.
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

/// Where the font point (x, y) of the edited glyph is on screen now.
fn screen(app: &FontcraftApp, x: f64, y: f64) -> Pos2 {
    let p = tools_glue::current_view(app).unwrap().to_screen(Point::new(x, y));
    Pos2::new(p.x as f32, p.y as f32)
}

fn button(pos: Pos2, pressed: bool, modifiers: EguiMods) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers }
}

fn key(key: egui::Key, modifiers: EguiMods) -> Event {
    Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
}

/// Glyph A with a closed square (shape 0: (100,100) (300,100) (300,300) (100,300)), an open
/// curve (shape 1: (400,100) handles (400,200) (450,300) to (500,300)) and an open line (shape 2:
/// (600,100) to (700,100)), edited at actual size.
fn edit_app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.run("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    let curve = json!([
        {"x": 400, "y": 100}, {"x": 400, "y": 200, "type": "offcurve"}, {"x": 450, "y": 300, "type": "offcurve"}, {"x": 500, "y": 300, "type": "curve"}
    ]);
    app.run("path.addPath", &json!({"glyph": "A", "closed": false, "nodes": curve})).unwrap();
    app.run("path.addPath", &json!({"glyph": "A", "closed": false, "nodes": [{"x": 600, "y": 100}, {"x": 700, "y": 100}]})).unwrap();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
    app.run("view.setZoom", &json!({"size": 1000, "center": [400, 250]})).unwrap();
    app
}

fn layer(app: &FontcraftApp) -> Layer {
    let font = app.session.font().unwrap();
    let master = &font.masters[0].id;
    font.glyph("A").unwrap().layer(master).unwrap().clone()
}

fn path(app: &FontcraftApp, shape: usize) -> fontcraft_model::Path {
    match layer(app).shapes.into_iter().nth(shape) {
        Some(fontcraft_model::Shape::Path(p)) => p,
        other => panic!("shape {shape} is not a path: {other:?}"),
    }
}

fn call(app: &mut FontcraftApp, method: &str, params: Value) -> Value {
    control::handle(app, &json!({"id": 1, "method": method, "params": params}))
}

fn pointer(app: &mut FontcraftApp, events: Value) -> Value {
    let r = call(app, "ui.pointer", json!({ "events": events }));
    assert_eq!(r["ok"], true, "{r}");
    r["result"].clone()
}

fn undo_steps(app: &FontcraftApp) -> usize {
    app.session.inspect()["documents"][0]["undo"].as_array().map_or(0, Vec::len)
}

fn selection(app: &FontcraftApp) -> Value {
    serde_json::to_value(&app.state.tab().unwrap().selection).unwrap()
}

#[test]
fn a_canvas_drag_moves_like_the_command_in_one_undo_step() {
    let mut app = edit_app();
    let w = Window::new();
    w.idle(&mut app);
    let before = layer(&app);
    let steps = undo_steps(&app);
    let (from, to) = (screen(&app, 300.0, 300.0), screen(&app, 350.0, 330.0));
    w.frame(&mut app, vec![Event::PointerMoved(from)]);
    w.frame(&mut app, vec![button(from, true, EguiMods::NONE)]);
    assert_eq!(selection(&app), json!({"points": [[0, 2]]}), "the press selects the node");
    w.frame(&mut app, vec![Event::PointerMoved(Pos2::new(from.x + 20.0, from.y))]);
    assert_eq!(app.session.interaction(), Some("Move"), "a drag runs one interaction");
    w.frame(&mut app, vec![Event::PointerMoved(to)]);
    w.frame(&mut app, vec![button(to, false, EguiMods::NONE)]);
    assert_eq!(app.session.interaction(), None);
    assert_eq!(undo_steps(&app), steps + 1, "the drag is one undo step");
    let mut headless = edit_app();
    headless.run("selection.move", &json!({"glyph": "A", "points": [[0, 2]], "dx": 50, "dy": 30})).unwrap();
    assert_eq!(layer(&app), layer(&headless), "the drag gives the font the command gives");
    app.run("edit.undo", &json!({})).unwrap();
    assert_eq!(layer(&app), before);
}

#[test]
fn escape_in_the_middle_of_a_drag_leaves_the_font_and_selection_as_they_were() {
    let mut app = edit_app();
    let w = Window::new();
    w.idle(&mut app);
    app.run("selection.set", &json!({"points": [[0, 2], [0, 3]]})).unwrap();
    let before = layer(&app);
    let steps = undo_steps(&app);
    let from = screen(&app, 300.0, 300.0);
    w.frame(&mut app, vec![Event::PointerMoved(from)]);
    w.frame(&mut app, vec![button(from, true, EguiMods::NONE), Event::PointerMoved(Pos2::new(from.x + 40.0, from.y + 40.0))]);
    assert_ne!(layer(&app), before, "the preview shows the move");
    w.frame(&mut app, vec![key(egui::Key::Escape, EguiMods::NONE)]);
    assert_eq!(app.session.interaction(), None);
    assert_eq!(layer(&app), before);
    assert_eq!(selection(&app), json!({"points": [[0, 2], [0, 3]]}));
    // The release after Escape does nothing.
    w.frame(&mut app, vec![button(Pos2::new(from.x + 40.0, from.y + 40.0), false, EguiMods::NONE)]);
    assert_eq!((layer(&app), undo_steps(&app)), (before, steps));
}

#[test]
fn a_marquee_selects_and_is_drawn_while_it_is_dragged() {
    let mut app = edit_app();
    let w = Window::new();
    w.idle(&mut app);
    let t = Tokens::dark();
    let (a, b) = (screen(&app, 50.0, 50.0), screen(&app, 350.0, 350.0));
    w.frame(&mut app, vec![Event::PointerMoved(a)]);
    w.frame(&mut app, vec![button(a, true, EguiMods::NONE), Event::PointerMoved(b)]);
    let shapes = w.idle(&mut app);
    let marquees = shapes.iter().filter(|s| matches!(s, Shape::Path(p) if p.fill == t.marquee_fill)).count();
    assert_eq!(marquees, 1, "the marquee in progress");
    assert_eq!(selection(&app), json!({"points": [[0, 0], [0, 1], [0, 2], [0, 3]]}), "the selection follows the marquee");
    w.frame(&mut app, vec![button(b, false, EguiMods::NONE)]);
    let shapes = w.idle(&mut app);
    assert!(!shapes.iter().any(|s| matches!(s, Shape::Path(p) if p.fill == t.marquee_fill)));
    let rings = shapes.iter().filter(|s| matches!(s, Shape::Circle(c) if c.stroke.color == t.selection)).count();
    assert_eq!(rings, 4, "a ring around each selected node");
    let segments = shapes.iter().filter(|s| matches!(s, Shape::Path(p) if p.stroke.color == egui::epaint::ColorMode::Solid(t.selection))).count();
    assert_eq!(segments, 4, "the four segments between selected nodes");
    // Alt takes on-curve nodes only; Shift adds to the selection.
    pointer(
        &mut app,
        json!([{"kind": "down", "x": 380, "y": 80, "mods": ["alt"]}, {"kind": "drag", "x": 520, "y": 320, "mods": ["alt"]}, {"kind": "up", "x": 520, "y": 320}]),
    );
    assert_eq!(selection(&app), json!({"points": [[1, 0], [1, 3]]}));
    pointer(
        &mut app,
        json!([{"kind": "down", "x": 580, "y": 80, "mods": ["shift"]}, {"kind": "drag", "x": 720, "y": 120, "mods": ["shift"]}, {"kind": "up", "x": 720, "y": 120, "mods": ["shift"]}]),
    );
    assert_eq!(selection(&app), json!({"points": [[1, 0], [1, 3], [2, 0], [2, 1]]}));
}

#[test]
fn clicks_select_and_double_clicks_edit() {
    let mut app = edit_app();
    let click = |app: &mut FontcraftApp, x: f64, y: f64, mods: Value| {
        pointer(app, json!([{"kind": "down", "x": x, "y": y, "mods": mods}, {"kind": "up", "x": x, "y": y, "mods": mods}]))
    };
    click(&mut app, 300.0, 300.0, json!([]));
    assert_eq!(selection(&app), json!({"points": [[0, 2]]}));
    click(&mut app, 100.0, 300.0, json!(["shift"]));
    assert_eq!(selection(&app), json!({"points": [[0, 2], [0, 3]]}), "Shift adds");
    click(&mut app, 300.0, 300.0, json!(["shift"]));
    assert_eq!(selection(&app), json!({"points": [[0, 3]]}), "Shift removes");
    click(&mut app, 200.0, 100.0, json!([]));
    assert_eq!(selection(&app), json!({"points": [[0, 0], [0, 1]]}), "a segment selects its ends");
    click(&mut app, 2000.0, 2000.0, json!([]));
    assert_eq!(selection(&app), json!({}), "a click on nothing clears the selection");
    pointer(&mut app, json!([{"kind": "doubleclick", "x": 200, "y": 100}]));
    assert_eq!(selection(&app), json!({"points": [[0, 0], [0, 1], [0, 2], [0, 3]]}), "a double-click on a segment selects the path");
    let smooth = |app: &FontcraftApp| path(app, 1).nodes[3].smooth;
    assert!(!smooth(&app));
    pointer(&mut app, json!([{"kind": "doubleclick", "x": 500, "y": 300}]));
    assert!(smooth(&app), "a double-click on a node toggles its type");
    let before = layer(&app).shapes[2].clone();
    click(&mut app, 650.0, 100.0, json!(["alt"]));
    assert_ne!(layer(&app).shapes[2], before, "Alt+click on a line segment makes it a curve");
}

#[test]
fn alt_drag_duplicates_and_an_end_dropped_on_an_end_joins() {
    let mut app = edit_app();
    let steps = undo_steps(&app);
    pointer(&mut app, json!([{"kind": "doubleclick", "x": 200, "y": 100}]));
    pointer(
        &mut app,
        json!([
            {"kind": "down", "x": 100, "y": 100, "mods": ["alt"]},
            {"kind": "drag", "x": 100, "y": 500, "mods": ["alt"]},
            {"kind": "up", "x": 100, "y": 500, "mods": ["alt"]},
        ]),
    );
    let mut headless = edit_app();
    let points = json!([[0, 0], [0, 1], [0, 2], [0, 3]]);
    headless.run("selection.move", &json!({"glyph": "A", "points": points, "dx": 0, "dy": 400, "duplicate": true})).unwrap();
    assert_eq!(layer(&app), layer(&headless), "an Alt-drag is selection.move with duplicate");
    assert_eq!(undo_steps(&app), steps + 1);
    assert_eq!(selection(&app), json!({"points": [[3, 0], [3, 1], [3, 2], [3, 3]]}), "the copy is selected");
    // The line's last node dropped on the curve's first node joins the two paths.
    let paths = layer(&app).shapes.len();
    pointer(
        &mut app,
        json!([{"kind": "down", "x": 700, "y": 100}, {"kind": "drag", "x": 550, "y": 100}, {"kind": "drag", "x": 401, "y": 101}, {"kind": "up", "x": 401, "y": 101}]),
    );
    assert_eq!(layer(&app).shapes.len(), paths - 1, "{}", app.state.status);
    assert_eq!(undo_steps(&app), steps + 2, "moving and joining is one step");
}

#[test]
fn snapping_draws_its_line_and_ctrl_suspends_it() {
    let mut app = edit_app();
    let w = Window::new();
    w.idle(&mut app);
    // Dragging (300,300) to x = 102 snaps to the x of (100,300) and (100,100).
    let r =
        pointer(&mut app, json!([{"kind": "down", "x": 300, "y": 300}, {"kind": "drag", "x": 250, "y": 400}, {"kind": "drag", "x": 102, "y": 400}]));
    let overlays = r["overlays"].as_array().unwrap();
    assert!(overlays.iter().any(|o| o["kind"] == "snapLine"), "{r}");
    let t = Tokens::dark();
    let shapes = w.idle(&mut app);
    assert!(shapes.iter().any(|s| matches!(s, Shape::LineSegment { stroke, .. } if stroke.color == t.snap_line)), "the snapping line is drawn");
    pointer(&mut app, json!([{"kind": "up", "x": 102, "y": 400}]));
    let node = |app: &FontcraftApp| (path(app, 0).nodes[2].x, path(app, 0).nodes[2].y);
    assert_eq!(node(&app), (100.0, 400.0));
    pointer(
        &mut app,
        json!([{"kind": "down", "x": 100, "y": 400}, {"kind": "drag", "x": 150, "y": 400, "mods": ["ctrl"]}, {"kind": "drag", "x": 303, "y": 400, "mods": ["ctrl"]}, {"kind": "up", "x": 303, "y": 400, "mods": ["ctrl"]}]),
    );
    assert_eq!(node(&app), (303.0, 400.0), "Ctrl suspends snapping");
}

#[test]
fn keys_run_the_edit_view_commands() {
    let mut app = edit_app();
    let w = Window::new();
    w.idle(&mut app);
    app.run("selection.set", &json!({"points": [[0, 2]]})).unwrap();
    let node = |app: &FontcraftApp| (path(app, 0).nodes[2].x, path(app, 0).nodes[2].y);
    w.frame(&mut app, vec![key(egui::Key::ArrowRight, EguiMods::NONE)]);
    w.frame(&mut app, vec![key(egui::Key::ArrowUp, EguiMods::SHIFT)]);
    assert_eq!(node(&app), (301.0, 310.0), "arrows move by 1, with Shift by 10");
    let r = call(&mut app, "ui.key", json!({"key": "ArrowLeft", "cmd": true}));
    assert_eq!(r["result"]["handled"], true, "{r}");
    assert_eq!(node(&app), (201.0, 310.0), "with Cmd by 100");
    // Tab walks the path; Delete removes the selection.
    call(&mut app, "ui.key", json!({"key": "Tab"}));
    assert_eq!(selection(&app), json!({"points": [[0, 3]]}));
    call(&mut app, "ui.key", json!({"key": "Tab", "shift": true}));
    assert_eq!(selection(&app), json!({"points": [[0, 2]]}));
    w.frame(&mut app, vec![key(egui::Key::Delete, EguiMods::NONE)]);
    assert_eq!(path(&app, 0).nodes.len(), 3);
    // Return toggles the selected node's type.
    app.run("selection.set", &json!({"points": [[0, 0]]})).unwrap();
    call(&mut app, "ui.key", json!({"key": "Return"}));
    assert!(path(&app, 0).nodes[0].smooth);
    // Keys with a modifier reach the menu shortcuts: Cmd+A selects all, Cmd+Alt+A nothing.
    w.frame(&mut app, vec![key(egui::Key::A, EguiMods::COMMAND)]);
    let points: usize = (0..3).map(|i| path(&app, i).nodes.len()).sum();
    assert_eq!(app.state.tab().unwrap().selection.points.len(), points);
    let r = call(&mut app, "ui.key", json!({"key": "a", "cmd": true, "alt": true}));
    assert_eq!(r["result"]["command"], "edit.deselectAll", "{r}");
    assert_eq!(selection(&app), json!({}));
}

#[test]
fn tool_letters_space_and_the_tool_commands_choose_tools() {
    let mut app = edit_app();
    let w = Window::new();
    w.idle(&mut app);
    w.frame(&mut app, vec![key(egui::Key::H, EguiMods::NONE)]);
    assert_eq!(app.tools.toolbox.active_id(), "hand");
    w.idle(&mut app);
    assert!(app.preview_tool, "the Hand tool fills the edited glyph");
    call(&mut app, "ui.key", json!({"key": "v", "shift": true}));
    assert_eq!(app.tools.toolbox.active_id(), "lasso");
    assert!(!app.preview_tool);
    call(&mut app, "ui.key", json!({"key": "Space", "action": "press"}));
    assert_eq!(app.tools.toolbox.active_id(), "hand", "Space holds the Hand tool");
    call(&mut app, "ui.key", json!({"key": "Space", "action": "release"}));
    assert_eq!(app.tools.toolbox.active_id(), "lasso");
    let r = call(&mut app, "ui.tool.select", json!({"tool": "zoom"}));
    assert_eq!(r["result"], json!({"tool": "zoom", "previous": "lasso"}));
    let list = call(&mut app, "ui.tool.list", json!({}));
    assert_eq!(list["result"]["active"], "zoom");
    assert!(list["result"]["tools"].as_array().unwrap().iter().any(|t| t["id"] == "select"));
    assert_eq!(call(&mut app, "ui.tool.select", json!({"tool": "nope"}))["ok"], false);
    assert_eq!(call(&mut app, "ui.inspect", json!({}))["result"]["tool"], "zoom");
}

#[test]
fn hand_and_zoom_move_the_camera() {
    let mut app = edit_app();
    let w = Window::new();
    w.idle(&mut app);
    app.run("tool.select", &json!({"tool": "hand"})).unwrap();
    let a = screen(&app, 300.0, 300.0);
    let at = |p: Pos2, dx: f32, dy: f32| json!({"x": p.x + dx, "y": p.y + dy, "space": "screen"});
    let mut events = vec![at(a, 0.0, 0.0), at(a, 30.0, 10.0), at(a, 60.0, 20.0)];
    for (e, kind) in events.iter_mut().zip(["down", "drag", "up"]) {
        e["kind"] = json!(kind);
    }
    pointer(&mut app, json!(events));
    let moved = screen(&app, 300.0, 300.0);
    assert!((moved.x - a.x - 60.0).abs() < 0.01 && (moved.y - a.y - 20.0).abs() < 0.01, "the content follows the pointer: {a:?} → {moved:?}");
    // A Zoom click keeps the clicked point where it is; a drag zooms to the rectangle.
    app.run("tool.select", &json!({"tool": "zoom"})).unwrap();
    let scale = app.state.tab().unwrap().view.scale;
    let before = screen(&app, 200.0, 200.0);
    pointer(&mut app, json!([{"kind": "down", "x": 200, "y": 200}, {"kind": "up", "x": 200, "y": 200}]));
    assert!((app.state.tab().unwrap().view.scale / scale - std::f64::consts::SQRT_2).abs() < 1e-9);
    let after = screen(&app, 200.0, 200.0);
    assert!((after.x - before.x).abs() < 0.01 && (after.y - before.y).abs() < 0.01, "{before:?} → {after:?}");
    pointer(&mut app, json!([{"kind": "down", "x": 100, "y": 100}, {"kind": "drag", "x": 300, "y": 300}, {"kind": "up", "x": 300, "y": 300}]));
    let v = app.state.tab().unwrap().view;
    assert_eq!(v.center, [200.0, 200.0], "centred on the rectangle (A is first on the line)");
}

#[test]
fn the_selection_commands_follow_their_rows() {
    let mut app = edit_app();
    let all = app.run("edit.selectAll", &json!({})).unwrap();
    assert_eq!(all["selection"]["points"].as_array().unwrap().len(), 10);
    app.run("selection.set", &json!({"points": [[0, 0], [0, 1]]})).unwrap();
    let inv = app.run("edit.invertSelection", &json!({})).unwrap();
    assert_eq!(inv["selection"]["points"].as_array().unwrap().len(), 8);
    app.run("selection.set", &json!({"mode": "remove", "points": [[1, 1]]})).unwrap();
    assert_eq!(app.state.tab().unwrap().selection.points.len(), 7);
    assert!(app.run("selection.set", &json!({"points": [[9, 9]]})).is_err(), "an address that does not resolve");
    assert!(app.run("selection.set", &json!({"glyph": "B", "points": [[0, 0]]})).is_err(), "another glyph");
    assert!(app.run("selection.selectNext", &json!({})).is_err(), "more than one item selected");
    let r = app.run("selection.selectInRegion", &json!({"glyph": "A", "rect": [350, 50, 200, 300], "nodesOnly": true})).unwrap();
    assert_eq!(r["selection"]["points"], json!([[1, 0], [1, 3]]));
    let r = app.run("selection.selectInRegion", &json!({"polygon": [[380, 80], [420, 80], [470, 320], [380, 320]]})).unwrap();
    assert_eq!(r["selection"]["points"], json!([[1, 0], [1, 1], [1, 2]]), "a lasso polygon takes handles too");
    assert!(app.run("selection.selectInRegion", &json!({"rect": [0, 0, 1, 1], "polygon": [[0, 0]]})).is_err());
    // Focus: only the focused path's nodes are drawn and can be selected.
    let r = app.run("path.focusOnSelectedPath", &json!({})).unwrap();
    assert_eq!(r["shapes"], json!([1]));
    let all = app.run("edit.selectAll", &json!({})).unwrap();
    assert_eq!(all["selection"]["points"], json!([[1, 0], [1, 1], [1, 2], [1, 3]]));
    let t = Tokens::dark();
    let w = Window::new();
    let corners = |shapes: &[Shape]| shapes.iter().filter(|s| matches!(s, Shape::Rect(r) if r.fill == t.node_corner)).count();
    let focused = corners(&w.idle(&mut app));
    app.run("path.clearFocusOnPaths", &json!({})).unwrap();
    assert!(corners(&w.idle(&mut app)) > focused, "clearing the focus shows every path's nodes");
    // The selection belongs to the edited glyph.
    app.run("view.newTab", &json!({"glyphs": ["B"]})).unwrap();
    w.idle(&mut app);
    assert_eq!(selection(&app), json!({}));
    // In Font View, Select All selects every glyph.
    app.run("view.showTab", &json!({"index": 1})).unwrap();
    let glyphs = app.run("edit.selectAll", &json!({})).unwrap();
    assert_eq!(glyphs["glyphs"].as_array().unwrap().len(), app.session.font().unwrap().glyphs.len());
}

#[test]
fn menu_commands_take_their_targets_from_the_selection() {
    let mut app = edit_app();
    let w = Window::new();
    w.idle(&mut app);
    app.run("selection.set", &json!({"points": [[2, 0], [2, 1]]})).unwrap();
    let params = tools_glue::with_targets(&app, "edit.delete", json!({}));
    assert_eq!(params, json!({"glyph": "A", "layer": app.session.font().unwrap().masters[0].id, "points": [[2, 0], [2, 1]]}));
    let given = json!({"glyph": "A", "points": [[0, 0]]});
    assert_eq!(tools_glue::with_targets(&app, "edit.delete", given.clone()), given, "given parameters are kept");
}

#[test]
fn hostile_pointer_and_key_requests_are_errors() {
    let mut app = edit_app();
    for bad in [
        json!({}),
        json!({"events": 5}),
        json!({"events": [{"kind": "fly", "x": 0, "y": 0}]}),
        json!({"events": [{"kind": "down", "x": "a", "y": 0}]}),
        json!({"events": [{"kind": "down", "x": null, "y": 0}]}),
        json!({"events": [{"kind": "down", "x": 0, "y": 0, "space": "moon"}]}),
        json!({"events": [{"kind": "down", "x": 0, "y": 0, "mods": ["hyper"]}]}),
        json!({"events": [{"kind": "down", "x": 0, "y": 0, "mods": 3}]}),
    ] {
        assert_eq!(call(&mut app, "ui.pointer", bad.clone())["ok"], false, "{bad}");
    }
    let many: Vec<Value> = (0..tools_glue::MAX_POINTER_EVENTS + 1).map(|_| json!({"kind": "move", "x": 0, "y": 0})).collect();
    assert_eq!(call(&mut app, "ui.pointer", json!({ "events": many }))["ok"], false);
    for bad in [json!({}), json!({"key": 5}), json!({"key": "a", "shift": "yes"}), json!({"key": "a", "action": "hold"})] {
        assert_eq!(call(&mut app, "ui.key", bad.clone())["ok"], false, "{bad}");
    }
    assert_eq!(call(&mut app, "ui.key", json!({"key": "NoSuchKey"}))["result"]["handled"], false);
    // Extreme coordinates and drags far outside the font change nothing they should not.
    pointer(
        &mut app,
        json!([{"kind": "down", "x": 300, "y": 300}, {"kind": "drag", "x": 1e300, "y": -1e300}, {"kind": "up", "x": 1e300, "y": -1e300}]),
    );
    assert!(app.session.interaction().is_none());
    assert!(app.session.font().unwrap().check().is_ok());
    // Font View has no canvas.
    app.run("view.showTab", &json!({"index": 1})).unwrap();
    assert_eq!(call(&mut app, "ui.pointer", json!({"events": [{"kind": "down", "x": 0, "y": 0}]}))["ok"], false);
    let r = call(&mut app, "ui.text", json!({"text": "hv"}));
    assert_eq!(r["result"], json!({"keys": 2, "handled": 0}));
}

#[test]
fn alt_pressed_during_a_canvas_drag_nudges_and_the_lasso_selects() {
    let mut app = edit_app();
    let w = Window::new();
    w.idle(&mut app);
    let (from, to) = (screen(&app, 500.0, 300.0), screen(&app, 540.0, 260.0));
    w.frame(&mut app, vec![Event::PointerMoved(from)]);
    w.frame(&mut app, vec![button(from, true, EguiMods::NONE), Event::PointerMoved(to)]);
    w.frame(&mut app, vec![Event::ModifiersChanged(EguiMods::ALT)]);
    w.frame(&mut app, vec![button(to, false, EguiMods::ALT)]);
    let mut headless = edit_app();
    headless.run("selection.move", &json!({"glyph": "A", "points": [[1, 3]], "dx": 40, "dy": -40, "mode": "nudge"})).unwrap();
    assert_eq!(layer(&app), layer(&headless), "Alt pressed after the drag started nudges");
    app.run("tool.select", &json!({"tool": "lasso"})).unwrap();
    let r = pointer(
        &mut app,
        json!([
            {"kind": "down", "x": 80, "y": 80}, {"kind": "drag", "x": 340, "y": 80}, {"kind": "drag", "x": 80, "y": 340}, {"kind": "up", "x": 80, "y": 340}
        ]),
    );
    assert_eq!(r["selection"], json!({"points": [[0, 0], [0, 1], [0, 3]]}), "the nodes inside the drawn triangle");
}
