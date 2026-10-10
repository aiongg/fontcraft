//! The canvas context menu and menu greying, driven like a user would: right-clicks and clicks
//! as egui events in headless frames.

use egui::{Event, Modifiers as EguiMods, PointerButton, Pos2, Shape};
use fontcraft_engine::find_command;
use fontcraft_geom::Point;
use fontcraft_model::Layer;
use fontcraft_tools::Selection;
use serde_json::{Value, json};

use crate::shell::context_menu::{self, ContextMenu};
use crate::shell::menus::MenuItem;
use crate::{FontcraftApp, tools_glue};

/// A headless window: one egui context kept across frames, so pointer and popup state carry over.
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

    /// A right-click at `at`: press in one frame, release in the next.
    fn right_click(&self, app: &mut FontcraftApp, at: Pos2) {
        self.frame(app, vec![Event::PointerMoved(at), button(at, PointerButton::Secondary, true, EguiMods::NONE)]);
        self.frame(app, vec![button(at, PointerButton::Secondary, false, EguiMods::NONE)]);
    }
}

fn button(pos: Pos2, button: PointerButton, pressed: bool, modifiers: EguiMods) -> Event {
    Event::PointerButton { pos, button, pressed, modifiers }
}

/// Where the font point (x, y) of the edited glyph is on screen now.
fn screen(app: &FontcraftApp, x: f64, y: f64) -> Pos2 {
    let p = tools_glue::current_view(app).unwrap().to_screen(Point::new(x, y));
    Pos2::new(p.x as f32, p.y as f32)
}

/// Glyph A with a closed square (shape 0: (100,100) (300,100) (300,300) (100,300)), edited at
/// actual size, nothing selected.
fn edit_app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.run("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
    app.run("view.setZoom", &json!({"size": 1000, "center": [200, 200]})).unwrap();
    app.state.tab_mut().unwrap().selection = Selection::new();
    app
}

fn layer(app: &FontcraftApp) -> Layer {
    let font = app.session.font().unwrap();
    let master = &font.masters[0].id;
    font.glyph("A").unwrap().layer(master).unwrap().clone()
}

fn selection(app: &FontcraftApp) -> Value {
    serde_json::to_value(&app.state.tab().unwrap().selection).unwrap()
}

fn menu(app: &FontcraftApp) -> ContextMenu {
    app.tools.context_menu.clone().expect("a context menu is open")
}

fn contexts(menu: &ContextMenu) -> Vec<&'static str> {
    menu.sections.iter().map(|s| s.context).collect()
}

fn commands(menu: &ContextMenu, section: usize) -> Vec<&'static str> {
    menu.sections[section].items.iter().map(|i| i.command).collect()
}

/// The centre of the drawn text `text`, if it was drawn.
fn text_at(shapes: &[Shape], text: &str) -> Option<Pos2> {
    shapes.iter().find_map(|s| match s {
        Shape::Text(t) if t.galley.text() == text => Some(t.pos + t.galley.rect.center().to_vec2()),
        _ => None,
    })
}

#[test]
fn a_right_click_on_a_node_lists_the_node_commands_and_runs_them_like_the_command() {
    let mut app = edit_app();
    let w = Window::new();
    w.idle(&mut app);
    let node_at = screen(&app, 300.0, 300.0);
    w.right_click(&mut app, node_at);
    assert_eq!(selection(&app), json!({"points": [[0, 2]]}), "the clicked node is selected first");

    let open = menu(&app);
    assert_eq!(contexts(&open), ["canvas.node", "canvas"]);
    let node = commands(&open, 0);
    for id in ["path.toggleNodeType", "path.openCorner", "path.reconnectNodes", "path.makeNodeFirst", "path.lockPath", "guide.add"] {
        assert!(node.contains(&id), "{id} is in the node section: {node:?}");
    }
    assert!(!node.contains(&"path.convertSegmentToCurve"), "segment commands stay out: {node:?}");
    let position = |id: &str| node.iter().position(|c| *c == id).unwrap();
    assert!(position("path.toggleNodeType") < position("path.openCorner"), "rows' order: {node:?}");
    assert!(position("path.openCorner") < position("path.makeNodeFirst"), "rows' order: {node:?}");
    assert!(commands(&open, 1).contains(&"glyph.toggleLocked"), "the glyph section follows");
    let greyed: Vec<&str> = open.items().filter(|i| !i.enabled).map(|i| i.command).collect();
    assert_eq!(greyed, ["path.connectNodes", "path.closeOpenPaths", "path.clearFocusOnPaths"], "one node of a closed path: {:?}", open.sections);
    assert!(node.contains(&"path.lockPath") && !node.contains(&"path.unlockPath"), "an unlocked path offers Lock Path only");
    assert_eq!(open.item("glyph.toggleLocked").unwrap().checked, Some(false), "Locked is a check item");

    // The first frame of a popup only measures it; the second draws it.
    w.idle(&mut app);
    let shapes = w.idle(&mut app);
    let item = text_at(&shapes, "Open Corner").expect("the menu shows Open Corner");
    w.frame(&mut app, vec![Event::PointerMoved(item), button(item, PointerButton::Primary, true, EguiMods::NONE)]);
    w.frame(&mut app, vec![button(item, PointerButton::Primary, false, EguiMods::NONE)]);
    assert!(app.tools.context_menu.is_none(), "picking an item closes the menu");

    let mut direct = edit_app();
    direct.run("path.openCorner", &json!({"glyph": "A", "points": [[0, 2]]})).unwrap();
    assert_eq!(layer(&app), layer(&direct), "the item gives the same font as the command");
    assert_ne!(layer(&app), layer(&edit_app()), "the corner was opened");
}

#[test]
fn a_right_click_on_a_segment_acts_on_that_segment() {
    let mut app = edit_app();
    let w = Window::new();
    w.idle(&mut app);
    let segment_at = screen(&app, 200.0, 100.0);
    w.right_click(&mut app, segment_at);
    assert_eq!(selection(&app), json!({"points": [[0, 0], [0, 1]]}), "a segment selects its end nodes");

    let open = menu(&app);
    assert_eq!(contexts(&open), ["canvas.segment", "canvas"]);
    assert_eq!(commands(&open, 0), ["path.convertSegmentToCurve", "path.openCorner", "path.insertNode"]);

    context_menu::invoke(&mut app, "path.convertSegmentToCurve").unwrap();
    let mut direct = edit_app();
    direct.run("path.convertSegmentToCurve", &json!({"glyph": "A", "segments": [[0, 1]]})).unwrap();
    assert_eq!(layer(&app), layer(&direct));
}

#[test]
fn a_right_click_on_empty_canvas_keeps_the_selection() {
    let mut app = edit_app();
    let w = Window::new();
    w.idle(&mut app);
    let empty = screen(&app, 200.0, 200.0);
    w.right_click(&mut app, empty);
    let open = menu(&app);
    assert_eq!(contexts(&open), ["canvas"], "nothing selected: the glyph section only");
    assert_eq!(selection(&app), json!({}));
    let clear = open.item("path.clearFocusOnPaths").unwrap();
    assert!(!clear.enabled && clear.disabled_reason.is_some(), "no path has the focus: {clear:?}");
    assert!(open.item("glyph.toggleLocked").unwrap().enabled);

    app.run("selection.set", &json!({"points": [[0, 2]]})).unwrap();
    // Close the first menu (as Escape does) so the second click lands on the canvas.
    app.tools.context_menu = None;
    w.idle(&mut app);
    w.right_click(&mut app, empty);
    assert_eq!(selection(&app), json!({"points": [[0, 2]]}), "empty canvas keeps the selection");
    assert_eq!(contexts(&menu(&app)), ["canvas.node", "canvas"], "the menu follows the selection");
}

#[test]
fn a_mac_ctrl_click_opens_the_menu_without_reaching_the_tool() {
    let mut app = edit_app();
    let w = Window::new();
    w.ctx.set_os(egui::os::OperatingSystem::Mac);
    w.idle(&mut app);
    app.run("selection.set", &json!({"points": [[0, 2], [0, 3]]})).unwrap();
    let at = screen(&app, 300.0, 300.0);
    let ctrl = EguiMods { ctrl: true, ..EguiMods::NONE };
    w.frame(&mut app, vec![Event::PointerMoved(at), button(at, PointerButton::Primary, true, ctrl)]);
    w.frame(&mut app, vec![button(at, PointerButton::Primary, false, ctrl)]);
    assert_eq!(contexts(&menu(&app)), ["canvas.node", "canvas"]);
    assert_eq!(selection(&app), json!({"points": [[0, 2], [0, 3]]}), "a selected node keeps the whole selection");
}

fn menu_item(app: &mut FontcraftApp, id: &str) -> MenuItem {
    let tree = app.menu_cache.tree(&app.session, &app.state);
    tree.iter().flat_map(|m| m.items.iter()).find(|i| i.command == id).cloned().unwrap()
}

#[test]
fn menu_items_grey_out_until_the_selection_supplies_their_targets() {
    let mut app = edit_app();
    let id = "path.selectionToBackground";
    let item = menu_item(&mut app, id);
    assert!(!item.enabled, "nothing selected");
    assert!(item.disabled_reason.unwrap().contains("select"));
    assert!(menu_item(&mut app, "path.reverseContours").enabled, "a command that takes whole glyphs needs no selection");
    assert!(menu_item(&mut app, "edit.copy").enabled, "copy without a selection copies the layer");

    app.run("selection.set", &json!({"points": [[0, 2]]})).unwrap();
    assert!(menu_item(&mut app, id).enabled, "the cached tree follows the selection");

    let corner = find_command("path.openCorner").unwrap();
    assert!(tools_glue::selection_supplies(&app.session, &app.state, corner).is_ok());
    app.state.tab_mut().unwrap().selection = Selection::new();
    assert!(tools_glue::selection_supplies(&app.session, &app.state, corner).is_err(), "no node selected: Open Corner is greyed out");
    assert!(!menu_item(&mut app, id).enabled);
}

#[test]
fn items_follow_the_state_of_what_is_selected() {
    let mut app = edit_app();
    app.run("path.addPath", &json!({"glyph": "A", "closed": false, "nodes": [{"x": 400, "y": 100}, {"x": 500, "y": 100}, {"x": 500, "y": 200}]}))
        .unwrap();
    let open_menu = |app: &mut FontcraftApp, points: Value| {
        app.run("selection.set", &json!({ "points": points })).unwrap();
        context_menu::open(app, Point::new(900.0, 900.0), tools_glue::Space::Font).unwrap();
        menu(app)
    };

    let ends = open_menu(&mut app, json!([[1, 0], [1, 2]]));
    assert!(ends.item("path.connectNodes").unwrap().enabled, "two open ends can be connected");
    assert!(ends.item("path.closeOpenPaths").unwrap().enabled, "the path is open");

    app.run("path.lockPath", &json!({"glyph": "A", "shapes": [1]})).unwrap();
    let locked = open_menu(&mut app, json!([[1, 0]]));
    assert!(locked.item("path.unlockPath").is_some() && locked.item("path.lockPath").is_none(), "{:?}", locked.sections);
    assert!(!locked.item("path.connectNodes").unwrap().enabled, "one node is not two ends");

    app.run("glyph.toggleLocked", &json!({"glyph": "A"})).unwrap();
    let glyph_locked = open_menu(&mut app, json!([]));
    assert_eq!(glyph_locked.item("glyph.toggleLocked").unwrap().checked, Some(true));
}

#[test]
fn a_layers_row_has_a_context_menu_that_acts_on_that_row() {
    let mut app = edit_app();
    app.run("layers.addBackupLayer", &json!({"glyphs": ["A"], "name": "Sketch"})).unwrap();
    let w = Window::new();
    let layers = |app: &FontcraftApp| app.session.font().unwrap().glyph("A").unwrap().layers.len();
    let before = layers(&app);
    let shapes = w.idle(&mut app);
    let row = text_at(&shapes, "Sketch").expect("the backup layer's row is drawn");

    let pick = |app: &mut FontcraftApp, row: Pos2, item: &str| {
        w.right_click(app, row);
        w.idle(app);
        let shapes = w.idle(app);
        let at = text_at(&shapes, item).unwrap_or_else(|| panic!("the row menu shows {item}"));
        w.frame(app, vec![Event::PointerMoved(at), button(at, PointerButton::Primary, true, EguiMods::NONE)]);
        w.frame(app, vec![button(at, PointerButton::Primary, false, EguiMods::NONE)]);
        w.idle(app);
    };

    // The master row refuses Delete Layer: picking it changes nothing.
    let master = text_at(&shapes, "Regular").expect("the master row is drawn");
    pick(&mut app, master, "Delete Layer");
    assert_eq!(layers(&app), before, "a master layer is not deleted");

    pick(&mut app, row, "Delete Layer");
    assert_eq!(layers(&app), before - 1, "the backup row's Delete Layer deletes that layer");
    assert_eq!(app.session.journal().last().map(|(id, _)| id.as_str()), Some("layers.delete"));
}
