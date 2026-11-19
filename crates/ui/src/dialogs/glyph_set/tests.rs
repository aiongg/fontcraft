//! The glyph-set dialogs driven over the control channel, each checked against its command run directly.

use std::sync::Arc;

use craft::host::Host;
use fontcraft_engine::Session;
use fontcraft_engine::model::{Component, Font, Shape};
use serde_json::{Value, json};

use crate::app::clipboard::{self, PasteKind};
use crate::app::{APP, FontCraftApp, Msg, ViewMsg};
use crate::control;

fn call(app: &mut FontCraftApp, method: &str, params: Value) -> Value {
    control::dispatch(app, method, &params).unwrap_or_else(|e| panic!("{method}: {e}"))
}

fn inspect(app: &mut FontCraftApp) -> Value {
    call(app, "ui.dialog.inspect", json!({}))
}

fn open(app: &mut FontCraftApp, dialog: &str, params: Value) -> Value {
    crate::dialogs::form::open_command(app, &json!({"dialog": dialog, "params": params})).unwrap()
}

/// Glyphs A, B and C, with a 200-unit square in A, A shown in the glyph panels.
fn square_app() -> FontCraftApp {
    let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
    for name in ["A", "B", "C"] {
        app.session.execute("glyph.newGlyph", &json!({ "name": name })).ok();
    }
    app.session.execute("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    app.sent.push(Msg::View(ViewMsg::SelectGlyph("A".into())));
    app.drain();
    app
}

/// [`square_app`] where B uses A as a component and C's left metrics key is `=A`.
fn referenced_app() -> FontCraftApp {
    let mut app = square_app();
    app.session
        .edit("references", |f| {
            f.glyph_mut("B")?.layers[0].shapes.push(Shape::Component(Component::new("A", 10.0, 0.0)));
            f.glyph_mut("C")?.info.metrics_keys.left = Some("=A".into());
            Ok(())
        })
        .unwrap();
    app
}

fn font(app: &FontCraftApp) -> Arc<Font> {
    app.session.doc().unwrap().font.clone()
}

fn shapes(app: &FontCraftApp, glyph: &str) -> usize {
    font(app).glyph(glyph).map_or(0, |g| g.layers[0].shapes.len())
}

fn direct(start: fn() -> FontCraftApp, command: &str, params: Value) -> Arc<Font> {
    let mut app = start();
    app.session.execute(command, &params).unwrap();
    font(&app)
}

#[test]
fn add_glyphs_generates_and_asks_about_names_the_font_has() {
    let mut app = square_app();
    let opened = open(&mut app, "dialog.addGlyphs", json!({}));
    assert_eq!((opened["captions"]["ok"].clone(), opened["mode"].clone()), (json!("Generate"), json!("runOnOk")), "{opened}");
    call(&mut app, "ui.dialog.set", json!({"field": "text", "value": "zero one"}));
    assert_eq!(inspect(&mut app)["params"], json!({"text": "zero one", "onExisting": "ask"}));
    call(&mut app, "ui.dialog.confirm", json!({}));
    assert_eq!(inspect(&mut app)["open"], false, "no name was in the font: nothing to ask");
    assert!(font(&app).same_as(&direct(square_app, "glyph.addGlyphs", json!({"text": "zero one", "onExisting": "ask"}))));

    // A name in the font: Generate adds nothing and the question follows. Replace.
    let mut app = square_app();
    open(&mut app, "dialog.addGlyphs", json!({"text": "A two"}));
    call(&mut app, "ui.dialog.confirm", json!({}));
    let asked = inspect(&mut app);
    assert_eq!(asked["dialog"], "dialog.addGlyphsExisting", "{asked}");
    assert_eq!(asked["fields"][0]["value"], json!(["A"]), "the names in the font are listed: {asked}");
    assert_eq!(asked["captions"]["replace"], "Replace", "{asked}");
    call(&mut app, "ui.click", json!({"id": "replace"}));
    assert_eq!(shapes(&app, "A"), 0, "Replace made A a new empty glyph");
    assert!(font(&app).same_as(&direct(square_app, "glyph.addGlyphs", json!({"text": "A two", "onExisting": "replace"}))));

    // The question previews Keep Existing; Cancel adds nothing at all.
    let mut app = square_app();
    let before = font(&app);
    open(&mut app, "dialog.addGlyphs", json!({"text": "A two"}));
    call(&mut app, "ui.dialog.confirm", json!({}));
    assert!(!font(&app).same_as(&before), "the question previews Keep Existing");
    call(&mut app, "ui.dialog.cancel", json!({}));
    assert!(font(&app).same_as(&before), "Cancel adds nothing at all");
}

#[test]
fn remove_glyph_confirms_and_asks_about_components_and_metrics_keys() {
    let mut app = referenced_app();
    let before = font(&app);
    let asked = open(&mut app, "dialog.removeGlyph", json!({"glyphs": ["A"]}));
    assert_eq!(asked["captions"]["ok"], "Decompose", "Decompose is the default: {asked}");
    assert_eq!(asked["params"], json!({"glyphs": ["A"], "components": "decompose", "metricsKeys": "dissolve"}));
    assert!(asked["fields"][0]["label"].as_str().unwrap().contains('B'), "{asked}");
    assert!(asked["fields"][1]["label"].as_str().unwrap().contains('C'), "{asked}");
    assert!(Arc::ptr_eq(&font(&app), &before), "asking changed nothing");
    call(&mut app, "ui.dialog.confirm", json!({}));
    let decomposed = direct(referenced_app, "glyph.removeGlyph", json!({"glyphs": ["A"], "components": "decompose", "metricsKeys": "dissolve"}));
    assert!(font(&app).glyph("A").is_none());
    assert!(font(&app).same_as(&decomposed), "OK decomposes, as the command does");

    let mut app = referenced_app();
    open(&mut app, "dialog.removeGlyph", json!({"glyphs": ["A"]}));
    call(&mut app, "ui.click", json!({"id": "remove"}));
    let removed = direct(referenced_app, "glyph.removeGlyph", json!({"glyphs": ["A"], "components": "remove", "metricsKeys": "dissolve"}));
    assert!(font(&app).same_as(&removed), "Remove gives the font the command gives");

    // Nothing refers to C: no question, no Decompose.
    let mut app = referenced_app();
    let opened = open(&mut app, "dialog.removeGlyph", json!({"glyphs": ["C"]}));
    assert_eq!(opened["fields"], json!([]), "{opened}");
    call(&mut app, "ui.dialog.confirm", json!({}));
    assert!(font(&app).glyph("C").is_none());
}

#[test]
fn paste_special_and_find_and_replace_run_their_commands() {
    let text = square_app().session.execute("edit.copy", &json!({"glyphs": ["A"]})).unwrap()["json"].as_str().unwrap().to_string();
    let mut app = square_app();
    app.sent.push(Msg::Shell(craft::ui::runtime::commands::CommandRef::with_params("fontView.setSelection", json!({"glyphs": ["B"]}))));
    app.sent.push(Msg::Shell(craft::ui::runtime::commands::CommandRef::new("fontView.show")));
    app.drain();
    // Paste Special with the clipboard's text opens the dialog on the selected glyphs.
    let msg = clipboard::paste_msg(&mut app, &text, PasteKind::Special).unwrap();
    app.run_now(msg, false).unwrap();
    let asked = inspect(&mut app);
    assert_eq!(asked["dialog"], "dialog.pasteSpecial", "{asked}");
    assert_eq!(asked["params"]["targets"], json!(["B"]), "{asked}");
    assert!(asked["params"].get("allMasters").is_none(), "All Masters goes to As Components only");
    let master = asked["params"]["master"].clone();
    call(&mut app, "ui.dialog.set", json!({"field": "mode", "value": "selected"}));
    call(&mut app, "ui.dialog.confirm", json!({}));
    let params = json!({"text": text, "mode": "selected", "parts": ["all"], "targets": ["B"], "master": master});
    assert_eq!(shapes(&app, "B"), 1, "B took A's square");
    assert!(font(&app).same_as(&direct(square_app, "edit.pasteSpecial", params)), "the dialog and the command give the same font");
    assert!(clipboard::paste_msg(&mut app, "plain words", PasteKind::Special).is_err(), "no copied glyphs: nothing to paste");

    // Find and Replace renames on Replace.
    let mut app = square_app();
    open(&mut app, "dialog.findAndReplaceGlyphNames", json!({"glyphs": ["A", "B"]}));
    call(&mut app, "ui.dialog.set", json!({"field": "find", "value": "B"}));
    call(&mut app, "ui.dialog.set", json!({"field": "replace", "value": "B.alt"}));
    assert!(font(&app).glyph("B.alt").is_none(), "nothing runs before Replace");
    call(&mut app, "ui.dialog.confirm", json!({}));
    assert!(font(&app).glyph("B.alt").is_some() && font(&app).glyph("B").is_none());
}

#[test]
fn a_paste_far_outside_the_glyph_asks_to_move_it() {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1000 1000"><rect x="5000" y="0" width="100" height="100"/></svg>"#;
    let mut app = square_app();
    let result = app.run_now(Msg::run("edit.paste", json!({"glyph": "A", "svg": svg})), false).unwrap();
    let asked = inspect(&mut app);
    assert_eq!(asked["dialog"], "dialog.pastedOutlineBounds", "{asked}");
    let given = super::bounds_follow_up(&Value::Null, &result).unwrap();
    assert_eq!((given["dx"].clone(), given["dy"].clone()), (json!(-5000.0), json!(100.0)), "{result}");
    call(&mut app, "ui.dialog.confirm", json!({}));
    let moved = font(&app);
    let mut expected = square_app();
    expected.session.execute("edit.paste", &json!({"glyph": "A", "svg": svg})).unwrap();
    expected.session.execute("selection.move", &Value::Object(given)).unwrap();
    assert!(moved.same_as(&font(&expected)), "Move gives the font the command gives");
}
