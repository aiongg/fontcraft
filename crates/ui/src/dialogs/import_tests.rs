//! The import dialogs give the font their command gives, and Cancel changes nothing.

use std::path::PathBuf;
use std::sync::Arc;

use craft::host::Host;
use fontcraft_engine::Session;
use fontcraft_engine::model::{Font, Layer};
use serde_json::{Value, json};

use crate::app::{APP, FontCraftApp, Msg};
use crate::control;

fn call(app: &mut FontCraftApp, method: &str, params: Value) -> Value {
    control::dispatch(app, method, &params).unwrap_or_else(|e| panic!("{method}: {e}"))
}

fn font(app: &FontCraftApp) -> Arc<Font> {
    app.session.doc().unwrap().font.clone()
}

fn undo_steps(app: &FontCraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-ui-import-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn plain() -> FontCraftApp {
    FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()))
}

/// Masters Regular and m02 Bold; A drawn on the first; A–V −40 on the first and −70 on m02.
fn two_masters() -> FontCraftApp {
    let mut app = plain();
    app.session
        .edit("setup", |font| {
            let mut m = font.default_master()?.clone();
            m.id = "m02".into();
            m.name = "Bold".into();
            Arc::make_mut(&mut font.masters).push(m);
            Ok(())
        })
        .unwrap();
    app.session.execute("glyph.newGlyph", &json!({"name": "A"})).ok();
    app.session.execute("glyph.newGlyph", &json!({"name": "V"})).ok();
    app.session
        .edit("layers", |font| {
            for i in 0..font.glyphs.len() {
                if let Some(mut g) = font.glyphs.get_mut_at(i)
                    && !g.layers.iter().any(|l| l.master == "m02")
                {
                    let width = g.layers.first().map_or(500.0, |l| l.width);
                    g.layers.push(Layer::new("m02", width));
                }
            }
            Ok(())
        })
        .unwrap();
    app.session.execute("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 0, "width": 400, "height": 700})).unwrap();
    app.session.execute("kerning.setPair", &json!({"left": "A", "right": "V", "value": -40})).unwrap();
    app.session.execute("kerning.setPair", &json!({"left": "A", "right": "V", "value": -70, "master": "m02"})).unwrap();
    app
}

#[test]
fn the_metrics_file_tab_opens_on_the_current_master_and_equals_the_command() {
    let dir = temp("metrics");
    let file = dir.join("Regular.tsv");
    let text = two_masters().session.execute("file.exportMetrics", &json!({"format": "metrics"})).unwrap()["text"].as_str().unwrap().to_string();
    std::fs::write(&file, text).unwrap();

    let mut app = two_masters();
    app.view.layers.master = Some("m02".into());
    let before = font(&app);
    let opened = form_open(&mut app, json!({"command": "file.importMetrics"}));
    assert_eq!(opened["dialog"], "dialog.importMetricsFile", "the menu item opens the file tab");
    assert_eq!(opened["tabs"][1]["id"], "dialog.importMetricsFont");
    assert_eq!(opened["captions"]["ok"], "Import");
    assert_eq!(opened["fields"][0]["kind"], "openFile");
    assert_eq!(opened["params"]["master"], "m02", "the current master");
    assert_eq!(opened["params"]["glyphs"], json!([]), "Selected Glyphs off: every glyph");
    call(&mut app, "ui.dialog.set", json!({"field": "file", "value": file}));
    call(&mut app, "ui.click", json!({"id": "cancel"}));
    assert!(Arc::ptr_eq(&font(&app), &before), "Cancel changes nothing");

    form_open(&mut app, json!({"dialog": "dialog.importMetricsFile"}));
    call(&mut app, "ui.dialog.set", json!({"field": "file", "value": file}));
    call(&mut app, "ui.dialog.set", json!({"field": "importMetrics", "value": true}));
    let shown = call(&mut app, "ui.dialog.set", json!({"field": "mode", "value": "rsbAndWidth"}));
    assert_eq!(shown["params"]["importMetrics"], "rsbAndWidth");
    let steps = undo_steps(&app);
    call(&mut app, "ui.dialog.confirm", json!({}));
    assert_eq!(undo_steps(&app), steps + 1, "one undo step");

    let mut direct = two_masters();
    let params = json!({"file": file, "master": "m02", "importKerning": true, "importKerningGroups": true, "importMetrics": "rsbAndWidth", "keepAlignedComponents": false, "glyphs": []});
    direct.session.execute("file.importMetrics", &params).unwrap();
    assert!(font(&app).same_as(&font(&direct)), "the dialog gives the font the command gives");
    let _ = std::fs::remove_dir_all(&dir);
}

fn form_open(app: &mut FontCraftApp, params: Value) -> Value {
    crate::dialogs::form::open_command(app, &params).unwrap()
}

#[test]
fn import_features_equals_the_command() {
    let dir = temp("features");
    let file = dir.join("features.fea");
    std::fs::write(&file, "languagesystem DFLT dflt;\n\n@Upper = [A B];\n\nfeature smcp {\n  sub a by A;\n} smcp;\n").unwrap();
    let mut app = plain();
    let opened = form_open(&mut app, json!({"command": "file.importFeatures"}));
    assert_eq!(opened["dialog"], "dialog.importFeatures");
    call(&mut app, "ui.dialog.set", json!({"field": "file", "value": file}));
    let done = call(&mut app, "ui.dialog.confirm", json!({}));
    assert_eq!(done["result"]["counts"], json!({"prefixes": 1, "classes": 1, "features": 1}));
    let mut direct = plain();
    direct.session.execute("file.importFeatures", &json!({"file": file})).unwrap();
    assert_eq!(app.session.font().unwrap().features, direct.session.font().unwrap().features);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_import_with_files_for_missing_glyphs_asks_and_add_glyphs_adds_them() {
    let dir = temp("outlines");
    let path = dir.join("newglyph.svg");
    std::fs::write(&path, r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1000 1000"><rect x="0" y="0" width="100" height="800"/></svg>"#)
        .unwrap();
    let file = path.to_string_lossy().into_owned();

    let mut app = plain();
    app.run_now(Msg::run("file.importOutlines", json!({"files": [file]})), false).unwrap();
    let asked = call(&mut app, "ui.dialog.inspect", json!({}));
    assert_eq!(asked["dialog"], "dialog.importOutlinesMissingGlyph", "the import's result asks the question");
    assert_eq!(asked["captions"], json!({"ok": "Add Glyphs", "cancel": "Skip"}));
    assert_eq!(asked["params"]["files"], json!([file]));
    call(&mut app, "ui.dialog.confirm", json!({}));
    assert!(app.session.font().unwrap().glyph("newglyph").is_some(), "Add Glyphs created the glyph");

    let mut skipped = plain();
    skipped.run_now(Msg::run("file.importOutlines", json!({"files": [file]})), false).unwrap();
    call(&mut skipped, "ui.dialog.cancel", json!({}));
    assert!(skipped.session.font().unwrap().glyph("newglyph").is_none(), "Skip leaves the file out");
    let _ = std::fs::remove_dir_all(&dir);
}
