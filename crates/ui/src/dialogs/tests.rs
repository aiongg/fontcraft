//! Spec dialogs driven as an agent drives them (the control channel's `ui.dialog.*`) and as the person does (the
//! window's dialog in a Fixture).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use craft::host::Host;
use craft::ui::dialog::DialogHost;
use craft::ui::testing::Fixture;
use egui::{Key, Modifiers};
use fontcraft_engine::Session;
use fontcraft_engine::model::Font;
use serde_json::{Value, json};

use crate::app::{APP, FontCraftApp, Msg, ViewMsg};
use crate::control;

fn call(app: &mut FontCraftApp, method: &str, params: Value) -> Value {
    control::dispatch(app, method, &params).unwrap_or_else(|e| panic!("{method}: {e}"))
}

fn fails(app: &mut FontCraftApp, method: &str, params: Value) -> String {
    control::dispatch(app, method, &params).expect_err(&format!("{method} should fail"))
}

/// Glyph A with a 200-unit square, shown in the glyph panels.
fn square_app() -> FontCraftApp {
    let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
    app.session.execute("glyph.newGlyph", &json!({"name": "A"})).ok();
    app.session.execute("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    app.sent.push(Msg::View(ViewMsg::SelectGlyph("A".into())));
    app.drain();
    app
}

fn font(app: &FontCraftApp) -> Arc<Font> {
    app.session.doc().unwrap().font.clone()
}

fn undo_steps(app: &FontCraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

fn open_round_corners(app: &mut FontCraftApp) -> Value {
    super::form::open_command(app, &json!({"dialog": "dialog.filterRoundCorners"})).unwrap()
}

/// The same font as the command run directly on a fresh square.
fn direct(params: Value) -> Arc<Font> {
    let mut app = square_app();
    app.session.execute("filter.roundCorners", &params).unwrap();
    font(&app)
}

#[test]
fn the_dialog_filled_over_the_control_channel_gives_the_commands_font() {
    let mut app = square_app();
    let opened = open_round_corners(&mut app);
    assert_eq!(opened["title"], "Round Corners");
    assert_eq!(opened["fields"][0]["id"], "radius");
    assert_eq!(opened["fields"][0]["value"], 20.0, "the filter's default");
    assert_eq!(opened["fields"][1]["id"], "visualCorrections");
    let targets = &opened["params"];
    assert!(targets["glyph"] == "A" || targets["glyphs"] == json!(["A"]), "the shown glyph is the target: {targets}");

    call(&mut app, "ui.dialog.set", json!({"field": "radius", "value": 30}));
    call(&mut app, "ui.dialog.click", json!({"id": "visualCorrections"}));
    let shown = call(&mut app, "ui.dialog.inspect", json!({}));
    assert_eq!(shown["params"]["visualCorrections"], true);
    let error = fails(&mut app, "ui.dialog.set", json!({"field": "radius", "value": -5}));
    assert!(error.contains("from 0"), "{error}");
    let shown = call(&mut app, "ui.dialog.inspect", json!({}));
    assert_eq!(shown["params"]["radius"], 30.0, "a refused value changes nothing");

    let before = undo_steps(&app);
    let mut params = shown["params"].clone();
    let done = call(&mut app, "ui.dialog.confirm", json!({}));
    assert_eq!(done["changed"], true);
    assert_eq!(undo_steps(&app), before + 1, "OK is one undo step");
    assert_eq!(call(&mut app, "ui.dialog.inspect", json!({}))["open"], false);
    assert!(font(&app).same_as(&direct(params.take())), "the dialog and the command give the same font");
}

#[test]
fn the_preview_changes_the_font_but_not_the_history_and_cancel_puts_it_back() {
    let mut app = square_app();
    let original = font(&app);
    let steps = undo_steps(&app);
    open_round_corners(&mut app);
    call(&mut app, "ui.dialog.set", json!({"field": "radius", "value": 50}));
    assert!(!font(&app).same_as(&original), "the preview shows the rounded square");
    assert_eq!(undo_steps(&app), steps, "a preview is not an undo step");
    assert_eq!(app.session.interaction(), Some("Round Corners"));

    // Escape with no field taking the keyboard is Cancel.
    assert_eq!(call(&mut app, "ui.dialog.key", json!({"key": "Escape"}))["open"], false);
    assert!(Arc::ptr_eq(&font(&app), &original), "Cancel puts back the very font the dialog opened on");
    assert_eq!(undo_steps(&app), steps);
    assert!(app.session.interaction().is_none());
}

#[test]
fn number_fields_follow_the_field_rules() {
    let mut app = square_app();
    open_round_corners(&mut app);
    let radius = |app: &mut FontCraftApp| call(app, "ui.dialog.inspect", json!({}))["params"]["radius"].as_f64().unwrap();

    // A click gives the field the keyboard with its text selected; Return takes what was typed and leaves the dialog
    // open.
    call(&mut app, "ui.dialog.click", json!({"id": "radius"}));
    assert_eq!(call(&mut app, "ui.dialog.text", json!({"text": "45"}))["field"], true);
    let r = call(&mut app, "ui.dialog.key", json!({"key": "Return"}));
    assert_eq!((r["handled"].clone(), r["open"].clone()), (json!(true), json!(true)));
    assert_eq!(radius(&mut app), 45.0);

    // Up and Down step by 1, by 10 with Shift.
    call(&mut app, "ui.dialog.click", json!({"id": "radius"}));
    call(&mut app, "ui.dialog.key", json!({"key": "ArrowUp", "shift": true}));
    call(&mut app, "ui.dialog.key", json!({"key": "ArrowDown"}));
    assert_eq!(radius(&mut app), 54.0);

    // Escape in a field drops what was typed; Tab takes it.
    call(&mut app, "ui.dialog.text", json!({"text": "99"}));
    call(&mut app, "ui.dialog.key", json!({"key": "Escape"}));
    assert_eq!(radius(&mut app), 54.0);
    assert_eq!(call(&mut app, "ui.dialog.inspect", json!({}))["open"], true, "the first Escape only leaves the field");
    call(&mut app, "ui.dialog.key", json!({"key": "Tab"}));
    call(&mut app, "ui.dialog.text", json!({"text": "12"}));
    call(&mut app, "ui.dialog.key", json!({"key": "Tab"}));
    assert_eq!(radius(&mut app), 12.0);

    // Text that is not a number is refused and the field keeps its value.
    call(&mut app, "ui.dialog.text", json!({"text": "abc"}));
    let r = call(&mut app, "ui.dialog.key", json!({"key": "Return"}));
    assert!(r["error"].as_str().is_some_and(|e| e.contains("number")), "{r}");
    assert_eq!(radius(&mut app), 12.0);
    assert_eq!(call(&mut app, "ui.dialog.key", json!({"key": "v"}))["handled"], false);

    // Return with no field taking the keyboard is OK.
    let r = call(&mut app, "ui.dialog.key", json!({"key": "Return"}));
    assert_eq!(r["open"], false);
    let params = json!({"glyph": "A", "radius": 12});
    let mut expected = square_app();
    let targets = super::form::open_command(&mut expected, &json!({"dialog": "dialog.filterRoundCorners"})).unwrap()["params"].clone();
    super::form::cancel(&mut expected).unwrap();
    let mut params = if targets["glyph"] == "A" { params } else { json!({"glyphs": ["A"], "radius": 12}) };
    assert!(font(&app).same_as(&direct(params.take())));
}

#[test]
fn every_filter_with_parameters_has_a_dialog_and_bad_requests_are_errors() {
    let mut scratch = Session::new();
    let list = scratch.execute("filter.list", &Value::Null).unwrap();
    for entry in list["filters"].as_array().unwrap() {
        let command = entry["command"].as_str().unwrap();
        let has_params = entry["params"].as_array().is_some_and(|p| !p.is_empty());
        if has_params && entry["menu"] == json!(["Filter"]) {
            assert!(super::for_command(command).is_some(), "{command} has no dialog");
        }
    }
    assert_eq!(super::find("dialog.filterRoundCorners").map(|d| d.command), Some("filter.roundCorners"));

    let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
    for (method, params) in [
        ("ui.dialog.set", json!({"field": "radius", "value": 1})),
        ("ui.dialog.confirm", json!({})),
        ("ui.dialog.cancel", json!({})),
        ("ui.dialog.click", json!({"id": "ok"})),
    ] {
        fails(&mut app, method, params);
    }
    let none = super::form::open_command(&mut app, &json!({"dialog": "dialog.filterRoundCorners"})).unwrap_err();
    assert!(none.contains("select a glyph"), "{none}");
    assert!(super::form::open_command(&mut app, &json!({"dialog": "dialog.nothing"})).is_err());

    let mut app = square_app();
    open_round_corners(&mut app);
    fails(&mut app, "ui.dialog.set", json!({"field": "nothing", "value": 1}));
    fails(&mut app, "ui.dialog.set", json!({"field": "visualCorrections", "value": "yes"}));
    fails(&mut app, "ui.dialog.click", json!({"id": "nothing"}));
    let twice = super::form::open_command(&mut app, &json!({"dialog": "dialog.filterRoundCorners"})).unwrap_err();
    assert!(twice.contains("confirm or cancel"), "{twice}");
    call(&mut app, "ui.dialog.click", json!({"id": "cancel"}));
}

/// The menu item of a filter with parameters opens its dialog; one given parameters runs.
#[test]
fn a_fired_filter_opens_its_dialog() {
    let mut app = square_app();
    assert!(super::after::fired(&mut app, "filter.roundCorners", &Value::Null, &serde_json::Map::new()));
    assert_eq!(super::form::inspect(&app)["dialog"], "dialog.filterRoundCorners");
    super::form::cancel(&mut app).unwrap();
    assert!(!super::after::fired(&mut app, "filter.roundCorners", &json!({"glyph": "A", "radius": 5}), &serde_json::Map::new()));
    assert!(!super::after::fired(&mut app, "spacing.setWidth", &Value::Null, &serde_json::Map::new()), "a command without a dialog runs");
}

/// The person's dialog: the window shows the app's dialog, a value typed into a row previews, and OK keeps it as one
/// undo step and closes the window's dialog.
#[test]
fn the_window_dialog_types_previews_and_confirms() {
    let app = Rc::new(RefCell::new(square_app()));
    let host = Rc::new(RefCell::new(DialogHost::new()));
    open_round_corners(&mut app.borrow_mut());
    let steps = undo_steps(&app.borrow());
    let (shown_app, shown_host) = (Rc::clone(&app), Rc::clone(&host));
    let mut fixture = Fixture::new(move |ui| {
        let (mut app, mut host) = (shown_app.borrow_mut(), shown_host.borrow_mut());
        super::run(&mut host, &mut app);
        host.run(ui);
    });
    fixture.settle();
    let radius = fixture.by_label("Radius").expect("the Radius row");
    fixture.click(&radius).key(Key::A, Modifiers::COMMAND).type_text("35").key(Key::Tab, Modifiers::NONE).settle();
    assert_eq!(super::form::inspect(&app.borrow())["params"]["radius"], 35.0, "the typed value reaches the app");
    assert_eq!(app.borrow().session.interaction(), Some("Round Corners"), "it previews");

    let ok = fixture.by_label("OK").expect("the OK button");
    fixture.click(&ok).settle();
    assert!(app.borrow().dialogs.open.is_none(), "{}", app.borrow().status);
    assert_eq!(undo_steps(&app.borrow()), steps + 1);
    assert!(!host.borrow().is_open::<super::host::SpecDialog>(), "the window's dialog closed with the app's");
}
