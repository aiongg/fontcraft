//! Dialogs driven like a user and an agent would: the control channel's `ui.dialog.*`,
//! `ui.click`, `ui.key` and `ui.text`, and egui keys in headless frames.

use std::sync::Arc;

use egui::{Event, Pos2};
use fontcraft_model::Font;
use serde_json::{Value, json};

use crate::{FontcraftApp, control, dialogs};

fn call(app: &mut FontcraftApp, method: &str, params: Value) -> Value {
    let r = control::handle(app, &json!({"id": 1, "method": method, "params": params}));
    assert_eq!(r["ok"], true, "{method}: {r}");
    r["result"].clone()
}

fn fails(app: &mut FontcraftApp, method: &str, params: Value) -> String {
    let r = control::handle(app, &json!({"id": 1, "method": method, "params": params}));
    assert_eq!(r["ok"], false, "{method} should fail: {r}");
    r["error"].as_str().unwrap_or_default().to_string()
}

/// Glyph A with a 200-unit square, edited in a tab.
fn square_app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.run("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
    app
}

fn font(app: &FontcraftApp) -> Arc<Font> {
    app.session.doc().unwrap().font.clone()
}

fn undo_steps(app: &FontcraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

fn open_round_corners(app: &mut FontcraftApp) -> Value {
    call(app, "engine.execute", json!({"command": "dialog.open", "params": {"dialog": "dialog.filterRoundCorners"}}))
}

/// One frame of the app at 1280 × 800 with `events`; the number of shapes it drew.
fn frame(ctx: &egui::Context, app: &mut FontcraftApp, events: Vec<Event>) -> usize {
    let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
    let mut output = ctx.run_ui(input, |ui| app.ui(ui));
    output.textures_delta.clear();
    output.shapes.len()
}

fn key(key: egui::Key, modifiers: egui::Modifiers) -> Event {
    Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
}

#[test]
fn the_dialog_filled_over_the_control_channel_gives_the_commands_font() {
    let mut app = square_app();
    let opened = open_round_corners(&mut app);
    assert_eq!(opened["title"], "Round Corners");
    assert_eq!(opened["fields"][0]["id"], "radius");
    assert_eq!(opened["fields"][0]["value"], 20.0, "the filter's default");
    assert_eq!(opened["fields"][1]["id"], "visualCorrections");
    assert_eq!(opened["params"]["glyph"], "A");

    call(&mut app, "ui.dialog.set", json!({"field": "radius", "value": 30}));
    call(&mut app, "ui.click", json!({"id": "visualCorrections"}));
    let shown = call(&mut app, "ui.dialog.inspect", json!({}));
    assert_eq!(shown["params"]["visualCorrections"], true);
    let error = fails(&mut app, "ui.dialog.set", json!({"field": "radius", "value": -5}));
    assert!(error.contains("from 0"), "{error}");
    assert_eq!(call(&mut app, "ui.dialog.inspect", json!({}))["params"]["radius"], 30.0, "a refused value changes nothing");

    let before = undo_steps(&app);
    let done = call(&mut app, "ui.dialog.confirm", json!({}));
    assert_eq!(done["changed"], true);
    assert_eq!(undo_steps(&app), before + 1, "OK is one undo step");
    assert_eq!(call(&mut app, "ui.dialog.inspect", json!({}))["open"], false);

    let mut direct = square_app();
    direct.run("filter.roundCorners", &json!({"glyph": "A", "radius": 30, "visualCorrections": true})).unwrap();
    assert!(font(&app).same_as(&font(&direct)), "the dialog and the command give the same font");
    assert_eq!(app.session.journal().last().map(|(id, _)| id.as_str()), Some("filter.roundCorners"), "Repeat Last Filter sees it");
}

#[test]
fn the_preview_changes_the_canvas_but_not_the_history_and_cancel_puts_the_font_back() {
    let mut app = square_app();
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, Vec::new());
    let square = frame(&ctx, &mut app, Vec::new());
    let original = font(&app);
    let steps = undo_steps(&app);

    open_round_corners(&mut app);
    call(&mut app, "ui.dialog.set", json!({"field": "radius", "value": 50}));
    assert!(!font(&app).same_as(&original), "the preview shows the rounded square");
    assert_eq!(undo_steps(&app), steps, "a preview is not an undo step");
    assert_eq!(call(&mut app, "ui.inspect", json!({}))["interaction"], "Round Corners");
    frame(&ctx, &mut app, Vec::new());
    let rounded = frame(&ctx, &mut app, Vec::new());
    assert!(rounded > square, "the canvas draws the preview's extra nodes and the dialog ({square} → {rounded} shapes)");

    // Escape with no field taking the keyboard is Cancel.
    frame(&ctx, &mut app, vec![key(egui::Key::Escape, egui::Modifiers::NONE)]);
    assert!(app.dialog.is_none());
    assert!(Arc::ptr_eq(&font(&app), &original), "Cancel puts back the very font the dialog opened on");
    assert_eq!(undo_steps(&app), steps);
    assert!(app.session.interaction().is_none());
    frame(&ctx, &mut app, Vec::new());
    assert_eq!(frame(&ctx, &mut app, Vec::new()), square, "the canvas shows the square again");
}

#[test]
fn number_fields_follow_the_field_rules() {
    let mut app = square_app();
    open_round_corners(&mut app);
    let radius = |app: &mut FontcraftApp| call(app, "ui.dialog.inspect", json!({}))["params"]["radius"].as_f64().unwrap();

    // A click gives the field the keyboard with its text selected; Return takes what was typed
    // and leaves the dialog open.
    call(&mut app, "ui.click", json!({"id": "radius"}));
    assert_eq!(call(&mut app, "ui.text", json!({"text": "45"}))["field"], true);
    let r = call(&mut app, "ui.key", json!({"key": "Return"}));
    assert_eq!((r["handled"].clone(), r["open"].clone()), (json!(true), json!(true)));
    assert_eq!(radius(&mut app), 45.0);

    // Up and Down step by 1, by 10 with Shift.
    call(&mut app, "ui.click", json!({"id": "radius"}));
    call(&mut app, "ui.key", json!({"key": "ArrowUp", "shift": true}));
    call(&mut app, "ui.key", json!({"key": "ArrowDown"}));
    assert_eq!(radius(&mut app), 54.0);

    // Escape in a field drops what was typed; Tab takes it.
    call(&mut app, "ui.text", json!({"text": "99"}));
    call(&mut app, "ui.key", json!({"key": "Escape"}));
    assert_eq!(radius(&mut app), 54.0);
    assert_eq!(call(&mut app, "ui.dialog.inspect", json!({}))["open"], true, "the first Escape only leaves the field");
    call(&mut app, "ui.key", json!({"key": "Tab"}));
    call(&mut app, "ui.text", json!({"text": "12"}));
    call(&mut app, "ui.key", json!({"key": "Tab"}));
    assert_eq!(radius(&mut app), 12.0);

    // Text that is not a number is refused and the field keeps its value.
    call(&mut app, "ui.text", json!({"text": "abc"}));
    let r = call(&mut app, "ui.key", json!({"key": "Return"}));
    assert!(r["error"].as_str().is_some_and(|e| e.contains("number")), "{r}");
    assert_eq!(radius(&mut app), 12.0);

    // While the dialog is open nothing else takes a key or an edit.
    assert_eq!(call(&mut app, "ui.key", json!({"key": "v"}))["handled"], false);
    let blocked = fails(
        &mut app,
        "engine.execute",
        json!({"command": "path.addRectangle", "params": {"glyph": "A", "x": 0, "y": 0, "width": 10, "height": 10}}),
    );
    assert!(blocked.contains("interaction"), "{blocked}");

    // Return with no field taking the keyboard is OK.
    let r = call(&mut app, "ui.key", json!({"key": "Return"}));
    assert_eq!(r["open"], false);
    let mut direct = square_app();
    direct.run("filter.roundCorners", &json!({"glyph": "A", "radius": 12})).unwrap();
    assert!(font(&app).same_as(&font(&direct)));
}

#[test]
fn every_filter_with_parameters_has_a_dialog_and_bad_requests_are_errors() {
    let mut scratch = fontcraft_engine::Session::new();
    let list = scratch.execute("filter.list", &Value::Null).unwrap();
    for entry in list["filters"].as_array().unwrap() {
        let command = entry["command"].as_str().unwrap();
        let has_params = entry["params"].as_array().is_some_and(|p| !p.is_empty());
        if has_params && entry["menu"] == json!(["Filter"]) {
            assert!(dialogs::for_command(command).is_some(), "{command} has no dialog");
        }
    }
    assert_eq!(dialogs::find("dialog.filterRoundCorners").map(|d| d.command), Some("filter.roundCorners"));

    let mut app = FontcraftApp::default();
    for (method, params) in [
        ("ui.dialog.set", json!({"field": "radius", "value": 1})),
        ("ui.dialog.confirm", json!({})),
        ("ui.dialog.cancel", json!({})),
        ("ui.click", json!({"id": "ok"})),
    ] {
        fails(&mut app, method, params);
    }
    let none = fails(&mut app, "engine.execute", json!({"command": "dialog.open", "params": {"dialog": "dialog.filterRoundCorners"}}));
    assert!(none.contains("select a glyph"), "{none}");
    fails(&mut app, "engine.execute", json!({"command": "dialog.open", "params": {"dialog": "dialog.nothing"}}));

    let mut app = square_app();
    open_round_corners(&mut app);
    fails(&mut app, "ui.dialog.set", json!({"field": "nothing", "value": 1}));
    fails(&mut app, "ui.dialog.set", json!({"field": "visualCorrections", "value": "yes"}));
    fails(&mut app, "ui.click", json!({"id": "nothing"}));
    let twice = fails(&mut app, "engine.execute", json!({"command": "dialog.open", "params": {"dialog": "dialog.filterRoundCorners"}}));
    assert!(twice.contains("confirm or cancel"), "{twice}");
    call(&mut app, "ui.click", json!({"id": "cancel"}));
}

#[test]
fn the_window_takes_clicks_typing_return_and_escape() {
    let mut app = square_app();
    let ctx = egui::Context::default();
    open_round_corners(&mut app);
    // The window sizes itself over the first frames.
    for _ in 0..3 {
        frame(&ctx, &mut app, Vec::new());
    }
    let field = ctx.read_response(dialogs::form::widget_id("radius")).expect("the radius field is drawn").rect.right_center() - egui::vec2(2.0, 0.0);
    let button = |pressed| Event::PointerButton { pos: field, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    frame(&ctx, &mut app, vec![Event::PointerMoved(field), button(true)]);
    frame(&ctx, &mut app, vec![button(false)]);
    frame(&ctx, &mut app, vec![Event::Text("5".into())]);
    assert_eq!(app.dialog.as_ref().and_then(|d| d.editing.as_ref()).map(|e| e.text.as_str()), Some("205"), "typing goes into the field");

    // Return takes the text and leaves the field; the dialog stays open.
    frame(&ctx, &mut app, vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
    frame(&ctx, &mut app, Vec::new());
    let d = app.dialog.as_ref().expect("still open");
    assert_eq!((d.values["radius"].as_f64(), d.editing.is_none()), (Some(205.0), true));
    assert!(!ctx.memory(|m| m.has_focus(dialogs::form::widget_id("radius"))), "the field gave up the keyboard");

    // Escape in the field drops the typing; a second Escape is Cancel.
    frame(&ctx, &mut app, vec![Event::PointerMoved(field), button(true)]);
    frame(&ctx, &mut app, vec![button(false), Event::Text("9".into())]);
    frame(&ctx, &mut app, vec![key(egui::Key::Escape, egui::Modifiers::NONE)]);
    frame(&ctx, &mut app, Vec::new());
    let d = app.dialog.as_ref().expect("the first Escape only leaves the field");
    assert_eq!(d.values["radius"].as_f64(), Some(205.0));
    frame(&ctx, &mut app, vec![key(egui::Key::Escape, egui::Modifiers::NONE)]);
    assert!(app.dialog.is_none());
}
