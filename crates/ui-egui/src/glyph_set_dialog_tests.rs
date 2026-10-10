//! The glyph set dialogs and Font View's clipboard, driven as a person and an agent would: each
//! dialog gives the font its command gives with the same answers, Cancel leaves the very font the
//! dialog opened on, and glyphs copied in one document paste into another.

use std::sync::Arc;

use egui::{Event, Key, Modifiers, OutputCommand, Pos2, ViewportCommand, ViewportId};
use fontcraft_model::{Component, Font, Shape};
use serde_json::{Value, json};

use crate::{FontcraftApp, control};

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

fn inspect(app: &mut FontcraftApp) -> Value {
    call(app, "ui.dialog.inspect", json!({}))
}

fn open(app: &mut FontcraftApp, dialog: &str, params: Value) -> Value {
    call(app, "engine.execute", json!({"command": "dialog.open", "params": {"dialog": dialog, "params": params}}))
}

fn font(app: &FontcraftApp) -> Arc<Font> {
    app.session.doc().unwrap().font.clone()
}

fn shapes(app: &FontcraftApp, glyph: &str) -> usize {
    font(app).glyph(glyph).unwrap().layers[0].shapes.len()
}

/// One frame at 1280 × 800 with `events`, the keyboard's modifiers held.
fn frame(ctx: &egui::Context, app: &mut FontcraftApp, events: Vec<Event>, modifiers: Modifiers) -> egui::FullOutput {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))),
        events,
        modifiers,
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| app.ui(ui));
    output.textures_delta.clear();
    output
}

fn key(key: Key, modifiers: Modifiers) -> Event {
    Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
}

fn copied(output: &egui::FullOutput) -> Vec<String> {
    output
        .platform_output
        .commands
        .iter()
        .filter_map(|c| match c {
            OutputCommand::CopyText(t) => Some(t.clone()),
            _ => None,
        })
        .collect()
}

/// The starting glyph set with a 200-unit square in A, shown in Font View.
fn square_app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.run("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    app
}

/// [`square_app`] where B uses A as a component and C's left metrics key is `=A`.
fn referenced_app() -> FontcraftApp {
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

/// The copied-glyphs text of [`square_app`]'s A.
fn copied_a() -> String {
    square_app().run("edit.copy", &json!({"glyphs": ["A"]})).unwrap()["json"].as_str().unwrap().to_string()
}

#[test]
fn add_glyphs_generates_and_asks_about_names_the_font_has() {
    let mut app = square_app();
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, vec![key(Key::G, Modifiers::COMMAND | Modifiers::SHIFT)], Modifiers::NONE);
    let opened = inspect(&mut app);
    assert_eq!(opened["dialog"], "dialog.addGlyphs", "Cmd+Shift+G: {opened}");
    assert_eq!((opened["captions"]["ok"].clone(), opened["mode"].clone()), (json!("Generate"), json!("runOnOk")));
    call(&mut app, "ui.dialog.set", json!({"field": "text", "value": "zero one"}));
    assert_eq!(inspect(&mut app)["params"], json!({"text": "zero one", "onExisting": "ask"}));
    call(&mut app, "ui.dialog.confirm", json!({}));
    assert_eq!(inspect(&mut app)["open"], false, "no name was in the font: nothing to ask");
    let mut direct = square_app();
    direct.run("glyph.addGlyphs", &json!({"text": "zero one", "onExisting": "ask"})).unwrap();
    assert!(font(&app).glyph("zero").is_some());
    assert!(font(&app).same_as(&font(&direct)), "the dialog and the command give the same font");

    // A name in the font: Generate adds nothing and the question follows. Replace.
    let mut app = square_app();
    let before = font(&app);
    open(&mut app, "dialog.addGlyphs", json!({"text": "A two"}));
    let r = call(&mut app, "ui.dialog.confirm", json!({}));
    assert_eq!(r["result"]["existing"], json!(["A"]), "{r}");
    assert!(Arc::ptr_eq(&font(&app), &before), "asking changed nothing");
    let asked = inspect(&mut app);
    assert_eq!(asked["dialog"], "dialog.addGlyphsExisting", "{asked}");
    assert_eq!(asked["fields"][0]["value"], "keep");
    assert!(asked["fields"][0]["label"].as_str().unwrap().contains('A'), "{asked}");
    call(&mut app, "ui.dialog.set", json!({"field": "onExisting", "value": "replace"}));
    call(&mut app, "ui.dialog.confirm", json!({}));
    let mut direct = square_app();
    direct.run("glyph.addGlyphs", &json!({"text": "A two", "onExisting": "replace"})).unwrap();
    assert_eq!(shapes(&app, "A"), 0, "Replace made A a new empty glyph");
    assert!(font(&app).same_as(&font(&direct)));

    // Keep Existing, then Cancel, which adds nothing at all.
    let mut app = square_app();
    open(&mut app, "dialog.addGlyphs", json!({"text": "A two"}));
    call(&mut app, "ui.dialog.confirm", json!({}));
    call(&mut app, "ui.dialog.confirm", json!({}));
    let mut direct = square_app();
    direct.run("glyph.addGlyphs", &json!({"text": "A two", "onExisting": "keep"})).unwrap();
    assert_eq!(shapes(&app, "A"), 1, "Keep Existing left A as it was");
    assert!(font(&app).same_as(&font(&direct)));
    let mut app = square_app();
    let before = font(&app);
    open(&mut app, "dialog.addGlyphs", json!({"text": "A two"}));
    call(&mut app, "ui.dialog.confirm", json!({}));
    assert!(!font(&app).same_as(&before), "the question previews Keep Existing");
    call(&mut app, "ui.dialog.cancel", json!({}));
    assert!(Arc::ptr_eq(&font(&app), &before), "Cancel adds nothing at all");

    // The question follows the person's run of the command, never an agent's.
    let mut app = square_app();
    let ask = json!({"text": "A", "onExisting": "ask"});
    call(&mut app, "engine.execute", json!({"command": "glyph.addGlyphs", "params": ask}));
    assert_eq!(inspect(&mut app)["open"], false);
    call(&mut app, "ui.menu.invoke", json!({"command": "glyph.addGlyphs", "params": ask}));
    assert_eq!(inspect(&mut app)["dialog"], "dialog.addGlyphsExisting");
}

#[test]
fn remove_glyph_confirms_and_asks_about_components_and_metrics_keys() {
    let mut app = referenced_app();
    app.state.selected_glyphs = vec!["A".into()];
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, Vec::new(), Modifiers::NONE);
    let before = font(&app);
    frame(&ctx, &mut app, vec![key(Key::Delete, Modifiers::COMMAND)], Modifiers::NONE);
    let asked = inspect(&mut app);
    assert_eq!(asked["dialog"], "dialog.removeGlyph", "Cmd+Delete: {asked}");
    assert_eq!(asked["captions"], json!({"ok": "Remove", "cancel": "Cancel"}));
    assert_eq!(asked["params"], json!({"glyphs": ["A"], "components": "decompose", "metricsKeys": "dissolve"}));
    assert!(asked["fields"][0]["label"].as_str().unwrap().contains('B'), "{asked}");
    assert!(asked["fields"][1]["label"].as_str().unwrap().contains('C'), "{asked}");
    assert!(Arc::ptr_eq(&font(&app), &before), "asking changed nothing");
    call(&mut app, "ui.dialog.set", json!({"field": "components", "value": "remove"}));
    call(&mut app, "ui.dialog.confirm", json!({}));
    let mut direct = referenced_app();
    direct.run("glyph.removeGlyph", &json!({"glyphs": ["A"], "components": "remove", "metricsKeys": "dissolve"})).unwrap();
    assert!(font(&app).glyph("A").is_none());
    assert!(font(&app).same_as(&font(&direct)), "the dialog and the command give the same font");
    assert!(app.state.selected_glyphs.is_empty(), "the removed glyph left the selection");

    let mut app = referenced_app();
    let before = font(&app);
    open(&mut app, "dialog.removeGlyph", json!({"glyphs": ["A"]}));
    call(&mut app, "ui.dialog.cancel", json!({}));
    assert!(Arc::ptr_eq(&font(&app), &before), "Cancel removes nothing");

    // Nothing refers to Z: only Remove and Cancel.
    let mut app = referenced_app();
    let opened = open(&mut app, "dialog.removeGlyph", json!({"glyphs": ["Z"]}));
    assert_eq!(opened["fields"], json!([]));
    call(&mut app, "ui.dialog.confirm", json!({}));
    assert!(font(&app).glyph("Z").is_none());
}

#[test]
fn paste_special_opens_on_copied_glyphs_and_pastes_as_the_command_does() {
    let text = copied_a();
    let ctx = egui::Context::default();
    let mut app = FontcraftApp::default();
    app.state.selected_glyphs = vec!["B".into()];
    frame(&ctx, &mut app, Vec::new(), Modifiers::NONE);
    let before = font(&app);
    // The paste key with Option held.
    frame(&ctx, &mut app, vec![Event::Paste(text.clone())], Modifiers::ALT);
    let asked = inspect(&mut app);
    assert_eq!(asked["dialog"], "dialog.pasteSpecial", "{asked}");
    assert_eq!((asked["params"]["mode"].clone(), asked["params"]["parts"].clone()), (json!("sameName"), json!(["all"])));
    assert_eq!(asked["params"]["targets"], json!(["B"]));
    assert!(asked["params"].get("allMasters").is_none(), "All Masters goes to As Components only");
    let master = asked["params"]["master"].clone();
    assert!(master.is_string(), "{asked}");
    call(&mut app, "ui.dialog.set", json!({"field": "mode", "value": "selected"}));
    call(&mut app, "ui.dialog.confirm", json!({}));
    let mut direct = FontcraftApp::default();
    direct.run("edit.pasteSpecial", &json!({"text": text, "mode": "selected", "parts": ["all"], "targets": ["B"], "master": master})).unwrap();
    assert_eq!(shapes(&app, "B"), 1, "B took A's square");
    assert!(font(&app).same_as(&font(&direct)), "the dialog and the command give the same font");
    assert!(!font(&app).same_as(&before));

    // As Components on every master.
    let mut app = FontcraftApp::default();
    open(&mut app, "dialog.pasteSpecial", json!({"text": text, "targets": ["B"], "master": master}));
    call(&mut app, "ui.dialog.set", json!({"field": "mode", "value": "asComponents"}));
    let asked = call(&mut app, "ui.dialog.set", json!({"field": "allMasters", "value": true}));
    assert!(asked["params"].get("parts").is_none(), "the parts go to the first two modes only: {asked}");
    call(&mut app, "ui.dialog.confirm", json!({}));
    let mut direct = FontcraftApp::default();
    direct.run("edit.pasteSpecial", &json!({"text": text, "mode": "asComponents", "allMasters": true, "targets": ["B"], "master": master})).unwrap();
    assert!(font(&app).same_as(&font(&direct)));

    let mut app = FontcraftApp::default();
    let before = font(&app);
    open(&mut app, "dialog.pasteSpecial", json!({"text": text}));
    assert!(!font(&app).same_as(&before), "the preview pastes A");
    call(&mut app, "ui.dialog.cancel", json!({}));
    assert!(Arc::ptr_eq(&font(&app), &before), "Cancel puts back the very font");

    // Edit › Paste Special (here its shortcut as a plain key) asks the platform for the clipboard
    // and opens the dialog on the text that arrives.
    let mut app = FontcraftApp::default();
    app.state.selected_glyphs = vec!["B".into()];
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, Vec::new(), Modifiers::NONE);
    let output = frame(&ctx, &mut app, vec![key(Key::V, Modifiers::COMMAND | Modifiers::ALT)], Modifiers::NONE);
    let commands = output.viewport_output.get(&ViewportId::ROOT).map(|v| v.commands.clone()).unwrap_or_default();
    assert!(commands.contains(&ViewportCommand::RequestPaste), "{commands:?}");
    assert_eq!(inspect(&mut app)["open"], false);
    frame(&ctx, &mut app, vec![Event::Paste(text.clone())], Modifiers::NONE);
    assert_eq!(inspect(&mut app)["dialog"], "dialog.pasteSpecial");
}

#[test]
fn find_and_replace_renames_the_selected_glyphs_on_replace() {
    let duplicated = || {
        let mut app = FontcraftApp::default();
        app.run("glyph.duplicateGlyph", &json!({"glyphs": ["A", "B"]})).unwrap();
        app
    };
    let mut app = duplicated();
    app.state.selected_glyphs = vec!["A.001".into(), "B.001".into()];
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, Vec::new(), Modifiers::NONE);
    frame(&ctx, &mut app, vec![key(Key::F, Modifiers::COMMAND | Modifiers::SHIFT)], Modifiers::NONE);
    let opened = inspect(&mut app);
    assert_eq!(opened["dialog"], "dialog.findAndReplaceGlyphNames", "Cmd+Shift+F: {opened}");
    assert_eq!(opened["captions"]["ok"], "Replace");
    assert_eq!(opened["params"], json!({"glyphs": ["A.001", "B.001"], "find": "", "replace": "", "matchCase": true, "regex": false}));
    let before = font(&app);
    call(&mut app, "ui.dialog.set", json!({"field": "find", "value": "\\.00(1)"}));
    call(&mut app, "ui.dialog.set", json!({"field": "replace", "value": ".ss0\\1"}));
    call(&mut app, "ui.dialog.set", json!({"field": "regex", "value": true}));
    assert!(Arc::ptr_eq(&font(&app), &before), "nothing runs before Replace");
    call(&mut app, "ui.dialog.confirm", json!({}));
    let mut direct = duplicated();
    direct
        .run(
            "edit.findAndReplace",
            &json!({"glyphs": ["A.001", "B.001"], "find": "\\.00(1)", "replace": ".ss0\\1", "matchCase": true, "regex": true}),
        )
        .unwrap();
    assert!(font(&app).glyph("A.ss01").is_some() && font(&app).glyph("B.ss01").is_some());
    assert!(font(&app).same_as(&font(&direct)), "the dialog and the command give the same font");

    // A name that is taken renames nothing and keeps the dialog open with the problem.
    let mut app = duplicated();
    let before = font(&app);
    open(&mut app, "dialog.findAndReplaceGlyphNames", json!({"glyphs": ["A.001"], "find": "A", "replace": "B"}));
    let e = fails(&mut app, "ui.dialog.confirm", json!({}));
    assert!(e.contains("taken"), "{e}");
    assert_eq!(inspect(&mut app)["open"], true);
    call(&mut app, "ui.dialog.cancel", json!({}));
    assert!(Arc::ptr_eq(&font(&app), &before), "Cancel renames nothing");
}

#[test]
fn glyphs_copied_in_font_view_paste_into_another_document() {
    let mut app = square_app();
    app.state.selected_glyphs = vec!["A".into()];
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, Vec::new(), Modifiers::NONE);
    let output = frame(&ctx, &mut app, vec![Event::Copy], Modifiers::NONE);
    let texts = copied(&output);
    assert_eq!(texts.len(), 1, "Copy puts the copied glyphs on the clipboard");
    let text = texts[0].clone();
    assert!(text.contains("fontcraft-glyphs"), "{text}");
    assert_eq!(text, copied_a(), "edit.copy's text for the selected glyphs");

    app.run("file.new", &json!({})).unwrap();
    assert!(app.state.tab().is_none());
    frame(&ctx, &mut app, vec![Event::Paste(text.clone())], Modifiers::NONE);
    assert_eq!(shapes(&app, "A.001"), 1, "A is taken, so the pasted A is A.001 with its square");
    let mut direct = square_app();
    direct.run("file.new", &json!({})).unwrap();
    direct.run("edit.paste", &json!({"text": text})).unwrap();
    assert!(font(&app).same_as(&font(&direct)), "the same font as edit.paste with the text");

    // Text without copied glyphs pastes nothing in Font View.
    let before = font(&app);
    frame(&ctx, &mut app, vec![Event::Paste("hello".into())], Modifiers::NONE);
    assert!(Arc::ptr_eq(&font(&app), &before));
}
