//! PostScript hints in Edit View, driven like a user would: Show Hints, headless frames for the
//! bars, badges and markers, the control channel's `ui.pointer` and `ui.key` for the Select
//! tool's gestures, the Info box's fields and the context menu.

use egui::{Color32, Event, Key, Modifiers, PointerButton, Pos2, Shape};
use fontcraft_geom::Point;
use fontcraft_tools::hint_drag::End;
use fontcraft_tools::{Item, Selection};
use serde_json::{Value, json};

use crate::canvas::hints_ps;
use crate::panels::info_box_ps_hint;
use crate::shell::context_menu;
use crate::tools_glue::Space;
use crate::{FontcraftApp, control, tools_glue};

/// A headless window: one egui context kept across frames, so focus and field state carry over.
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

    fn settle(&self, app: &mut FontcraftApp) -> Vec<Shape> {
        for _ in 0..3 {
            self.frame(app, Vec::new());
        }
        self.frame(app, Vec::new())
    }

    /// Focuses `id`, types `text` and presses Return.
    fn enter(&self, app: &mut FontcraftApp, id: egui::Id, text: &str) {
        self.ctx.memory_mut(|m| m.request_focus(id));
        self.frame(app, Vec::new());
        assert!(self.ctx.memory(|m| m.has_focus(id)), "the field takes the focus");
        self.frame(app, vec![Event::Text(text.into())]);
        self.frame(app, vec![Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }]);
    }

    fn click(&self, app: &mut FontcraftApp, id: egui::Id) {
        let at = self.ctx.read_response(id).unwrap_or_else(|| panic!("{id:?} was not drawn")).rect.center();
        let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        self.frame(app, vec![Event::PointerMoved(at)]);
        self.frame(app, vec![button(true)]);
        self.frame(app, vec![button(false)]);
    }
}

fn call(app: &mut FontcraftApp, method: &str, params: Value) -> Value {
    let r = control::handle(app, &json!({"id": 1, "method": method, "params": params}));
    assert_eq!(r["ok"], true, "{method}: {r}");
    r["result"].clone()
}

/// Glyph n with one path (100,0) (300,0) (300,700) (100,700), edited at half size, and three
/// PostScript hints: 0, a horizontal stem from 0 to 80 with numeric edges; 1, a vertical stem
/// linked to nodes [0, 0] and [0, 1] (x 100 to 300); 2, a ghost linked to node [0, 2] (y 700).
fn hinted_app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    let nodes = json!([{"x": 100, "y": 0}, {"x": 300, "y": 0}, {"x": 300, "y": 700}, {"x": 100, "y": 700}]);
    app.run("path.addPath", &json!({"glyph": "n", "closed": true, "nodes": nodes})).unwrap();
    app.run("hint.ps.addHorizontalHint", &json!({"glyph": "n", "origin": 0, "size": 80})).unwrap();
    app.run("hint.ps.addVerticalHint", &json!({"glyph": "n", "nodes": [[0, 0], [0, 1]]})).unwrap();
    app.run("hint.ps.addHorizontalHint", &json!({"glyph": "n", "nodes": [[0, 2]]})).unwrap();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["n"]})).unwrap();
    app.run("view.setZoom", &json!({"size": 500, "center": [200, 250]})).unwrap();
    app
}

fn hints(app: &mut FontcraftApp) -> Vec<Value> {
    app.run("hint.ps.listHints", &json!({"glyph": "n"})).unwrap()["hints"].as_array().cloned().unwrap_or_default()
}

fn selected_hints(app: &FontcraftApp) -> Vec<usize> {
    app.state.tab().unwrap().selection.hints.iter().copied().collect()
}

/// Where hint `index`'s badge, or one of its markers, is on screen now.
fn badge(app: &mut FontcraftApp, index: usize) -> Point {
    let marks = hints_ps::marks(&mut app.session, &app.state, &app.tokens);
    let view = tools_glue::current_view(app).unwrap();
    marks.badge(&view, marks.get(index).unwrap()).unwrap().center()
}

fn marker(app: &mut FontcraftApp, index: usize, end: End) -> Point {
    let marks = hints_ps::marks(&mut app.session, &app.state, &app.tokens);
    let view = tools_glue::current_view(app).unwrap();
    marks.marker(&view, marks.get(index).unwrap(), end).unwrap()
}

fn screen(app: &FontcraftApp, x: f64, y: f64) -> Point {
    tools_glue::current_view(app).unwrap().to_screen(Point::new(x, y))
}

fn at(kind: &str, p: Point) -> Value {
    json!({"kind": kind, "x": p.x, "y": p.y, "space": "screen"})
}

fn filled_rects(shapes: &[Shape], color: Color32) -> usize {
    shapes.iter().filter(|s| matches!(s, Shape::Rect(r) if r.fill == color)).count()
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

fn undo_labels(app: &FontcraftApp) -> Vec<String> {
    app.session.inspect()["documents"][0]["undo"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}

#[test]
fn show_hints_draws_bars_badges_and_markers_by_kind() {
    let mut app = hinted_app();
    let t = app.tokens.ps_hints.clone();
    let w = Window::new();
    let shapes = w.settle(&mut app);
    assert_eq!(filled_rects(&shapes, t.horizontal) + filled_rects(&shapes, t.badge), 0, "hints stay hidden until Show Hints");
    assert_eq!(app.run("view.showHints", &json!({})).unwrap()["on"], true);
    let shapes = w.settle(&mut app);
    // One bar per kind, tinted: horizontal yellow, vertical green, ghost blue.
    assert_eq!((filled_rects(&shapes, t.horizontal), filled_rects(&shapes, t.vertical), filled_rects(&shapes, t.ghost)), (1, 1, 1));
    assert_eq!(filled_rects(&shapes, t.badge), 3, "a gray badge per hint");
    let shown = texts(&shapes);
    assert!(shown.contains(&"0 / 80".to_string()) && shown.contains(&"100 / 200".to_string()), "origin and size: {shown:?}");
    assert!(shown.contains(&"700".to_string()), "a ghost's badge shows its position: {shown:?}");
    // Circles at the origins: filled where linked (hints 1 and 2), hollow where a number.
    let filled = shapes.iter().filter(|s| matches!(s, Shape::Circle(c) if c.fill == t.marker)).count();
    let hollow = shapes.iter().filter(|s| matches!(s, Shape::Circle(c) if c.fill == Color32::TRANSPARENT && c.stroke.color == t.marker)).count();
    assert_eq!((filled, hollow), (2, 1));
    // Triangles at the stems' other edges; hint 1's is linked, so filled.
    let triangles = shapes.iter().filter(|s| matches!(s, Shape::Path(p) if p.points.len() == 3 && p.fill == t.marker)).count();
    assert_eq!(triangles, 1);
    // A selected hint's badge turns orange.
    app.state.tab_mut().unwrap().selection = Selection::from_items([Item::Hint(1)]);
    let shapes = w.settle(&mut app);
    assert_eq!((filled_rects(&shapes, t.badge_selected), filled_rects(&shapes, t.badge)), (1, 2));
    // Hiding the hints deselects them.
    app.run("view.showHints", &json!({"on": false})).unwrap();
    assert!(selected_hints(&app).is_empty());
}

#[test]
fn badge_clicks_select_hints_tab_steps_through_them_and_delete_removes_them() {
    let mut app = hinted_app();
    app.run("view.showHints", &json!({"on": true})).unwrap();
    let b0 = badge(&mut app, 0);
    call(&mut app, "ui.pointer", json!({"events": [at("down", b0), at("up", b0)]}));
    assert_eq!(selected_hints(&app), vec![0]);
    let mut steps = Vec::new();
    for _ in 0..3 {
        call(&mut app, "ui.key", json!({"key": "Tab"}));
        steps.push(selected_hints(&app));
    }
    assert_eq!(steps, vec![vec![1], vec![2], vec![0]], "Tab walks the list and wraps after the last hint");
    call(&mut app, "ui.key", json!({"key": "Tab", "shift": true}));
    assert_eq!(selected_hints(&app), vec![2], "Shift+Tab goes back, before the first to the last");
    // Shift-click adds hint 1; Delete removes both selected hints.
    let b1 = badge(&mut app, 1);
    let shift = |kind: &str| json!({"kind": kind, "x": b1.x, "y": b1.y, "space": "screen", "mods": ["shift"]});
    call(&mut app, "ui.pointer", json!({"events": [shift("down"), shift("up")]}));
    assert_eq!(selected_hints(&app), vec![1, 2]);
    call(&mut app, "ui.key", json!({"key": "Delete"}));
    let left = hints(&mut app);
    assert_eq!(left.len(), 1);
    assert_eq!(left[0]["direction"], "horizontal");
    assert!(selected_hints(&app).is_empty());
    assert_eq!(undo_labels(&app).last().map(String::as_str), Some("Delete Hints"));
}

#[test]
fn dragging_a_marker_equals_set_hint_values_and_a_drop_on_a_node_links_it() {
    let mut app = hinted_app();
    app.run("view.showHints", &json!({"on": true})).unwrap();
    let undo_before = undo_labels(&app).len();
    // The circle of hint 0 up by 20 points: 40 units at half size. Its numeric other edge stays.
    let from = marker(&mut app, 0, End::Origin);
    let to = Point::new(from.x, from.y - 20.0);
    call(&mut app, "ui.pointer", json!({"events": [at("down", from), at("drag", to), at("up", to)]}));
    let h = hints(&mut app);
    assert_eq!((h[0]["origin"].as_f64(), h[0]["size"].as_f64()), (Some(40.0), Some(40.0)));
    assert_eq!(undo_labels(&app).len(), undo_before + 1, "one undo step");
    assert_eq!(undo_labels(&app).last().map(String::as_str), Some("Move Hint"));
    assert_eq!(selected_hints(&app), vec![0], "the dragged hint stays selected");
    // Its triangle dropped on node [0, 3] at (100, 700): the other end follows that node.
    let from = marker(&mut app, 0, End::Target);
    let node = screen(&app, 100.0, 700.0);
    call(&mut app, "ui.pointer", json!({"events": [at("down", from), at("drag", node), at("up", node)]}));
    let h = hints(&mut app);
    assert_eq!(h[0]["nodes"]["target"], json!([0, 3]));
    assert_eq!((h[0]["origin"].as_f64(), h[0]["size"].as_f64()), (Some(40.0), Some(660.0)));
    // Escape during a drag leaves the hint as it was.
    let from = marker(&mut app, 0, End::Origin);
    call(&mut app, "ui.pointer", json!({"events": [at("down", from), at("drag", Point::new(from.x, from.y - 50.0))]}));
    call(&mut app, "ui.key", json!({"key": "Escape"}));
    assert_eq!(hints(&mut app)[0]["origin"].as_f64(), Some(40.0));
}

#[test]
fn info_box_edits_run_set_hint_values_and_set_ghost_direction() {
    let mut app = hinted_app();
    let layer = app.session.font().unwrap().masters[0].id.clone();
    app.run("view.showHints", &json!({"on": true})).unwrap();
    // A frame first: the first one drops a selection made before the tab's glyph was synced.
    let w = Window::new();
    w.settle(&mut app);
    app.state.tab_mut().unwrap().selection = Selection::from_items([Item::Hint(0)]);
    w.settle(&mut app);
    w.enter(&mut app, info_box_ps_hint::field("origin"), "10");
    let (id, params) = app.session.journal().last().cloned().unwrap();
    assert_eq!(id, "hint.ps.setHintValues");
    assert_eq!((params["glyph"].clone(), params["layer"].clone(), params["hint"].clone()), (json!("n"), json!(layer), json!(0)));
    assert_eq!(params["origin"].as_f64(), Some(10.0));
    assert_eq!(hints(&mut app)[0]["size"].as_f64(), Some(80.0), "a typed origin keeps a numeric hint's size");
    w.settle(&mut app);
    w.enter(&mut app, info_box_ps_hint::field("size"), "50");
    assert_eq!(hints(&mut app)[0]["size"].as_f64(), Some(50.0));
    // The ghost: no size field, an up and a down button.
    app.state.tab_mut().unwrap().selection = Selection::from_items([Item::Hint(2)]);
    w.settle(&mut app);
    let kind = hints(&mut app)[2]["kind"].as_str().unwrap().to_string();
    let (button, wanted) = if kind == "ghostTop" { ("bottom", "ghostBottom") } else { ("top", "ghostTop") };
    w.click(&mut app, info_box_ps_hint::field(button));
    let (id, params) = app.session.journal().last().cloned().unwrap();
    assert_eq!((id.as_str(), params["hints"].clone(), params["direction"].clone()), ("hint.ps.setGhostDirection", json!([2]), json!(button)));
    assert_eq!(hints(&mut app)[2]["kind"], wanted);
}

#[test]
fn the_context_menu_offers_hint_commands_while_hints_show() {
    let mut app = hinted_app();
    context_menu::open(&mut app, Point::new(300.0, 700.0), Space::Font).unwrap();
    assert!(app.tools.context_menu.as_ref().unwrap().item("hint.ps.addHorizontalHint").is_none(), "no hint items with hints hidden");
    app.tools.context_menu = None;
    app.run("view.showHints", &json!({"on": true})).unwrap();
    // A right-click on hint 1's badge selects it and opens the hint menu.
    let b1 = badge(&mut app, 1);
    context_menu::open(&mut app, b1, Space::Screen).unwrap();
    let menu = app.tools.context_menu.clone().unwrap();
    assert_eq!(menu.sections.first().map(|s| s.context), Some(context_menu::HINT));
    assert!(menu.item("hint.ps.makeGhostHint").is_some_and(|i| i.enabled));
    assert_eq!(selected_hints(&app), vec![1]);
    context_menu::invoke(&mut app, "hint.ps.deleteHints").unwrap();
    assert_eq!(hints(&mut app).len(), 2);
    // On node [0, 2] at (300, 700), Add Horizontal Hint makes a ghost linked to it, and selects it.
    app.state.tab_mut().unwrap().selection = Selection::new();
    context_menu::open(&mut app, Point::new(300.0, 700.0), Space::Font).unwrap();
    let menu = app.tools.context_menu.clone().unwrap();
    assert!(menu.item("hint.ps.addHorizontalHint").is_some_and(|i| i.enabled));
    assert!(menu.item("hint.ps.deleteHints").is_none(), "hint items stay in the hint menu");
    context_menu::invoke(&mut app, "hint.ps.addHorizontalHint").unwrap();
    let h = hints(&mut app);
    assert_eq!(h.len(), 3);
    assert_eq!((h[2]["nodes"]["origin"].clone(), h[2]["kind"].as_str().is_some_and(|k| k.starts_with("ghost"))), (json!([0, 2]), true));
    assert_eq!(selected_hints(&app), vec![2]);
}
