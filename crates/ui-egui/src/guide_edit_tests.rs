//! Guide editing on the canvas with the Select tool: knob drags, turning drags, the angle lock,
//! locked guides, the properties dialog and the context menu's guide items.

use fontcraft_geom::Point;
use fontcraft_model::{Guide, Layer};
use serde_json::{Value, json};

use crate::shell::context_menu;
use crate::tools_glue::Space;
use crate::{FontcraftApp, control};

/// Glyph A with a square from (100,100) to (300,300) and one local guide, horizontal through
/// (150,600), edited at actual size.
fn guide_app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.run("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    app.run("guide.add", &json!({"glyph": "A", "x": 150, "y": 600})).unwrap();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
    app.run("view.setZoom", &json!({"size": 1000, "center": [400, 400]})).unwrap();
    app
}

fn layer(app: &FontcraftApp) -> Layer {
    let font = app.session.font().unwrap();
    let master = &font.masters[0].id;
    font.glyph("A").unwrap().layer(master).unwrap().clone()
}

fn guide(app: &FontcraftApp) -> Guide {
    layer(app).info.guides[0].clone()
}

fn call(app: &mut FontcraftApp, method: &str, params: Value) -> Value {
    let r = control::handle(app, &json!({"id": 1, "method": method, "params": params}));
    assert_eq!(r["ok"], true, "{method}: {r}");
    r["result"].clone()
}

fn pointer(app: &mut FontcraftApp, events: Value) -> Value {
    call(app, "ui.pointer", json!({ "events": events }))
}

fn drag(app: &mut FontcraftApp, from: (f64, f64), to: (f64, f64)) {
    let mid = ((from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0);
    pointer(
        app,
        json!([
            {"kind": "down", "x": from.0, "y": from.1},
            {"kind": "drag", "x": mid.0, "y": mid.1},
            {"kind": "drag", "x": to.0, "y": to.1},
            {"kind": "up", "x": to.0, "y": to.1},
        ]),
    );
}

fn undo_steps(app: &FontcraftApp) -> usize {
    app.session.inspect()["documents"][0]["undo"].as_array().map_or(0, Vec::len)
}

fn local_0() -> Value {
    json!({"scope": "local", "index": 0})
}

#[test]
fn dragging_the_knob_is_set_properties_with_the_same_coordinates_in_one_undo_step() {
    let mut app = guide_app();
    let steps = undo_steps(&app);
    pointer(&mut app, json!([{"kind": "down", "x": 150, "y": 600}, {"kind": "drag", "x": 180, "y": 620}]));
    assert_eq!(app.session.interaction(), Some("Move Guide"));
    pointer(&mut app, json!([{"kind": "drag", "x": 220, "y": 640}, {"kind": "up", "x": 220, "y": 640}]));
    assert_eq!(app.session.interaction(), None);
    assert_eq!(undo_steps(&app), steps + 1, "the drag is one undo step");
    assert_eq!(serde_json::to_value(&app.state.tab().unwrap().selection).unwrap(), json!({"guides": [local_0()]}));

    let mut headless = guide_app();
    headless.run("guide.setProperties", &json!({"glyph": "A", "guide": local_0(), "x": 220, "y": 640})).unwrap();
    assert_eq!(layer(&app), layer(&headless), "the drag gives the font the command gives");
    assert_eq!((guide(&app).x, guide(&app).y, guide(&app).angle), (220.0, 640.0, 0.0));
}

#[test]
fn dragging_the_line_turns_the_guide_about_its_knob_unless_the_angle_is_locked() {
    let mut app = guide_app();
    let steps = undo_steps(&app);
    // Pressed 300 units right of the knob, dragged 300 units up: a 45° turn.
    drag(&mut app, (450.0, 600.0), (450.0, 900.0));
    let g = guide(&app);
    assert!((g.angle - 45.0).abs() < 1e-9, "{}", g.angle);
    assert_eq!((g.x, g.y), (150.0, 600.0), "the knob stays");
    assert_eq!(undo_steps(&app), steps + 1);
    app.run("edit.undo", &json!({})).unwrap();

    app.run("guide.setProperties", &json!({"glyph": "A", "guide": local_0(), "angleLocked": true})).unwrap();
    drag(&mut app, (450.0, 600.0), (450.0, 900.0));
    let g = guide(&app);
    assert_eq!(g.angle, 0.0, "an angle-locked guide does not turn");
    assert_eq!((g.x, g.y), (150.0, 900.0), "it moves with the pointer instead");

    // A double-click on the knob turns a guide by 90°, but not an angle-locked one.
    pointer(&mut app, json!([{"kind": "doubleclick", "x": 150, "y": 900}]));
    assert_eq!(guide(&app).angle, 0.0);
    app.run("guide.setProperties", &json!({"glyph": "A", "guide": local_0(), "angleLocked": false})).unwrap();
    pointer(&mut app, json!([{"kind": "doubleclick", "x": 150, "y": 900}]));
    assert_eq!(guide(&app).angle, 90.0);
}

#[test]
fn a_locked_guide_does_not_move_turn_or_open_its_dialog() {
    let mut app = guide_app();
    app.run("guide.lock", &json!({"glyph": "A", "guides": [local_0()]})).unwrap();
    let (before, steps) = (layer(&app), undo_steps(&app));
    drag(&mut app, (150.0, 600.0), (220.0, 640.0));
    drag(&mut app, (450.0, 600.0), (450.0, 900.0));
    pointer(&mut app, json!([{"kind": "doubleclick", "x": 150, "y": 600}, {"kind": "doubleclick", "x": 450, "y": 600}]));
    assert_eq!(layer(&app), before);
    assert_eq!(undo_steps(&app), steps);
    assert!(app.dialog.is_none());
}

#[test]
fn a_double_click_on_the_line_opens_the_properties_dialog_and_ok_sets_the_name() {
    let mut app = guide_app();
    let steps = undo_steps(&app);
    pointer(
        &mut app,
        json!([
            {"kind": "down", "x": 450, "y": 600}, {"kind": "up", "x": 450, "y": 600},
            {"kind": "down", "x": 450, "y": 600}, {"kind": "up", "x": 450, "y": 600},
            {"kind": "doubleclick", "x": 450, "y": 600},
        ]),
    );
    let shown = call(&mut app, "ui.dialog.inspect", json!({}));
    assert_eq!(shown["dialog"], "dialog.guideProperties", "{shown}");
    assert_eq!(shown["params"]["guide"], local_0());
    assert_eq!((shown["params"]["x"].as_f64(), shown["params"]["y"].as_f64()), (Some(150.0), Some(600.0)));
    assert_eq!(shown["params"]["name"], "");

    call(&mut app, "ui.dialog.set", json!({"field": "name", "value": "Stem"}));
    assert_eq!(guide(&app).name.as_deref(), Some("Stem"), "the canvas previews the name");
    call(&mut app, "ui.dialog.confirm", json!({}));
    assert!(app.dialog.is_none());
    assert_eq!(undo_steps(&app), steps + 1);

    let mut headless = guide_app();
    headless.run("guide.setProperties", &json!({"glyph": "A", "guide": local_0(), "name": "Stem"})).unwrap();
    assert_eq!(layer(&app), layer(&headless), "the dialog gives the font the command gives");
}

#[test]
fn the_context_menu_adds_guides_and_lists_the_commands_of_a_guide() {
    let mut app = guide_app();
    let labels = |app: &mut FontcraftApp, at: Point| -> Vec<&'static str> {
        context_menu::open(app, at, Space::Font).unwrap();
        app.tools.context_menu.as_ref().unwrap().items().map(|i| i.label).collect()
    };

    let on_guide = labels(&mut app, Point::new(400.0, 600.0));
    for label in ["Make Global Guide", "Lock Guides", "Remove", "Measurement Guide", "Guide Properties…"] {
        assert!(on_guide.contains(&label), "{label} missing from {on_guide:?}");
    }
    assert!(!on_guide.iter().any(|l| *l == "Unlock Guide" || *l == "Make Local Guide"), "{on_guide:?}");
    assert!(!on_guide.iter().any(|l| *l == "Add Guide" || *l == "Add Global Guide"), "adding is for clicks off guides: {on_guide:?}");
    let opened = context_menu::invoke(&mut app, "dialog.guideProperties").unwrap();
    assert!(app.dialog.is_some(), "Guide Properties… opens the dialog: {opened}");
    crate::dialogs::cancel(&mut app).unwrap();

    app.run("guide.lock", &json!({"glyph": "A", "guides": [local_0()]})).unwrap();
    let locked = labels(&mut app, Point::new(400.0, 600.0));
    assert!(locked.contains(&"Unlock Guide"), "{locked:?}");
    assert!(!locked.iter().any(|l| *l == "Lock Guides" || *l == "Guide Properties…" || *l == "Remove"), "{locked:?}");
    app.run("guide.unlock", &json!({"glyph": "A", "guides": [local_0()]})).unwrap();

    app.state.tab_mut().unwrap().selection = fontcraft_tools::Selection::new();
    let add = labels(&mut app, Point::new(200.0, 400.0));
    assert!(add.contains(&"Add Guide") && add.contains(&"Add Global Guide"), "{add:?}");
    let menu = app.tools.context_menu.clone().unwrap();
    assert!(menu.item("guide.addGlobal").unwrap().alternate, "Add Global Guide is the Alt alternate");
    context_menu::invoke(&mut app, "guide.add").unwrap();
    let added = &layer(&app).info.guides[1];
    assert_eq!((added.x, added.y, added.angle), (200.0, 400.0, 0.0), "horizontal through the click");
    let selected = serde_json::to_value(&app.state.tab().unwrap().selection).unwrap();
    assert_eq!(selected, json!({"guides": [{"scope": "local", "index": 1}]}), "the new guide is selected");

    app.run("selection.set", &json!({"points": [[0, 0], [0, 2]]})).unwrap();
    context_menu::open(&mut app, Point::new(500.0, 50.0), Space::Font).unwrap();
    let menu = app.tools.context_menu.clone().unwrap();
    let steps = context_menu::steps_for(&app, &menu, "guide.add");
    assert_eq!(steps[0].1["throughPoints"], json!([[0, 0], [0, 2]]), "two selected nodes: through them");
}
