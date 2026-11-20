//! The preview features, text mode's Edit menu, the metrics labels and `ui.text`: a text with smcp on shows the small
//! cap, feature code edits show after a compile, and the Edit commands act on the text in text mode.

use std::sync::Arc;

use craft::host::Host;
use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::Session;
use serde_json::{Value, json};

use crate::app::{APP, FontCraftApp, Msg, ViewMsg};

fn post(app: &mut FontCraftApp, msg: Msg) {
    app.status.clear();
    app.sent.push(msg);
    app.drain();
    assert!(app.status.is_empty(), "{}", app.status);
}

fn shell(app: &mut FontCraftApp, id: &str, params: Value) {
    post(app, Msg::Shell(CommandRef::with_params(id, params)));
}

fn run(app: &mut FontCraftApp, id: &str, params: Value) -> Value {
    app.drain();
    app.session.execute(id, &params).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn shown(app: &FontCraftApp) -> Vec<Option<String>> {
    let layout = app.models.text.layout.clone().expect("the text is laid out");
    layout.placed.iter().map(|p| p.glyph.as_deref().map(str::to_string)).collect()
}

fn items(app: &FontCraftApp) -> Value {
    super::reply(app)["items"].clone()
}

/// Glyphs `zz`, `yy`, `zz.sc` and `zz_yy` with smcp and liga features, compiled, and the text `zz zz yy` typed by name.
fn compiled_app() -> FontCraftApp {
    let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
    for name in ["zz", "yy", "zz.sc", "zz_yy"] {
        run(&mut app, "glyph.newGlyph", json!({"name": name, "width": 450}));
    }
    run(&mut app, "font.features.addFeature", json!({"tag": "smcp", "code": "sub zz by zz.sc;"}));
    run(&mut app, "font.features.addFeature", json!({"tag": "liga", "code": "sub zz yy by zz_yy;"}));
    let compiled = run(&mut app, "font.features.compile", json!({}));
    assert_eq!(compiled["preview"], json!(true), "{compiled}");
    post(&mut app, Msg::View(ViewMsg::SelectGlyph("zz".into())));
    shell(&mut app, "editView.setText", json!({"glyphs": ["zz", "zz", "yy"]}));
    app
}

#[test]
fn features_shape_the_text_through_the_compiled_preview_font() {
    let mut app = compiled_app();
    let some = |n: &str| Some(n.to_string());
    assert_eq!(shown(&app), [some("zz"), some("zz"), some("yy")], "no feature is on by default");
    let menu = &super::reply(&app)["features"];
    assert_eq!(menu["compiled"], json!(true), "{menu}");
    assert_eq!(menu["features"].as_array().map(Vec::len), Some(2), "{menu}");

    let revision = app.session.doc().unwrap().revision;
    shell(&mut app, "editView.toggleFeature", json!({"tag": "smcp"}));
    assert_eq!(shown(&app), [some("zz.sc"), some("zz.sc"), some("yy")]);
    assert_eq!(app.models.features.active, ["smcp"]);
    assert_eq!(app.session.doc().unwrap().revision, revision, "a preview feature changes no document");

    // A feature code edit shows after the next compile.
    run(&mut app, "font.features.setCode", json!({"kind": "feature", "name": "smcp", "code": "sub yy by zz.sc;"}));
    app.drain();
    assert_eq!(shown(&app)[2], some("yy"), "the old compile still shapes");
    run(&mut app, "font.features.compile", json!({}));
    app.drain();
    assert_eq!(shown(&app), [some("zz"), some("zz"), some("zz.sc")]);

    shell(&mut app, "editView.clearFeatures", json!({}));
    assert_eq!(shown(&app), [some("zz"), some("zz"), some("yy")]);
    assert!(app.models.features.active.is_empty());
}

#[test]
fn a_feature_on_before_any_compile_says_why_the_text_is_not_shaped() {
    let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
    post(&mut app, Msg::View(ViewMsg::SelectGlyph("A".into())));
    shell(&mut app, "editView.toggleFeature", json!({"tag": "ss01"}));
    assert_eq!(super::reply(&app)["features"]["active"], json!(["ss01"]));
    assert!(app.models.features.shaping_error.as_deref().is_some_and(|e| e.contains("compiled")), "{:?}", app.models.features.shaping_error);
    app.status.clear();
    app.sent.push(Msg::Shell(CommandRef::with_params("editView.toggleFeature", json!({"tag": "toolong"}))));
    app.drain();
    assert!(!app.status.is_empty(), "a tag longer than four characters is refused");
}

#[test]
fn the_edit_menu_acts_on_the_text_in_text_mode() {
    let mut app = compiled_app();
    shell(&mut app, "editView.toggleTextMode", json!({"mode": "text"}));
    shell(&mut app, "editView.setCursor", json!({"index": 1, "selectionLength": 2}));
    let fire = |app: &mut FontCraftApp, id: &str, params: Value| super::edit::fired(app, &CommandRef::with_params(id, params));

    let copied = fire(&mut app, "edit.copy", json!({})).unwrap().unwrap();
    assert!(copied.is_empty());
    let out = app.view.text.clipboard.out.clone().unwrap_or_default();
    assert!(out.starts_with("/zz") && out.contains("/yy") && !out.contains("zz.sc"), "glyphs without a code point copy as /name: {out}");

    for msg in fire(&mut app, "edit.cut", json!({})).unwrap().unwrap() {
        post(&mut app, msg);
    }
    assert_eq!(items(&app), json!(["zz"]));
    for msg in fire(&mut app, "edit.paste", json!({"text": "/yy/zz.sc"})).unwrap().unwrap() {
        post(&mut app, msg);
    }
    assert_eq!(items(&app), json!(["zz", "yy", "zz.sc"]));
    assert!(fire(&mut app, "edit.pasteSpecial", json!({})).unwrap().is_err(), "Paste Special is refused in text mode");
    assert!(fire(&mut app, "edit.paste", json!({})).unwrap().unwrap().is_empty());
    assert_eq!(app.view.text.clipboard.paste, Some(None), "Paste without text asks for the clipboard's text");

    // ui.text types at the cursor.
    let reply = crate::control::dispatch(&mut app, "ui.text", &json!({"text": "/zz"})).unwrap();
    assert_eq!(reply["typed"], json!(true), "{reply}");
    assert_eq!(items(&app).as_array().map(Vec::len), Some(6), "the three characters went in");

    shell(&mut app, "editView.toggleTextMode", json!({"mode": "edit"}));
    assert!(fire(&mut app, "edit.copy", json!({})).is_none(), "outside text mode Copy copies outlines");
}

#[test]
fn show_metrics_labels_every_glyph_in_text_mode() {
    let mut app = compiled_app();
    app.view.display.metrics = true;
    app.view_changed();
    app.drain();
    assert!(app.models.text.labels.is_empty(), "edit mode has no labels");
    shell(&mut app, "editView.toggleTextMode", json!({"mode": "text"}));
    let labels = &app.models.text.labels;
    assert_eq!(labels.len(), 3, "one per glyph");
    assert!(labels.iter().all(|l| l.advance == 450.0 && !l.vertical));
}
