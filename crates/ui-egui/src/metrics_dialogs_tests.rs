//! The metrics dialogs: the Export dialog's Metrics tab (`dialog.exportMetrics`) and the import
//! dialog's tabs (`dialog.importMetricsFile`, `dialog.importMetricsFont`) give what
//! `file.exportMetrics` and `file.importMetrics` give, open on the master the toolbar shows, and
//! Cancel changes nothing.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::PathBuf;
use std::sync::Arc;

use fontcraft_model::Layer;
use serde_json::{Value, json};

use crate::{FontcraftApp, control};

fn call(app: &mut FontcraftApp, method: &str, params: Value) -> Value {
    let r = control::handle(app, &json!({"id": 1, "method": method, "params": params}));
    assert_eq!(r["ok"], true, "{method}: {r}");
    r["result"].clone()
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-metrics-dialog-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Masters m01 Regular and m02 Bold; A drawn on m01; A–V −40 on m01 and −70 on m02.
fn app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.session
        .edit("setup", |font| {
            let mut m = font.default_master()?.clone();
            m.id = "m02".into();
            m.name = "Bold".into();
            Arc::make_mut(&mut font.masters).push(m);
            for i in 0..font.glyphs.len() {
                if let Some(mut g) = font.glyphs.get_mut_at(i) {
                    let width = g.layers[0].width;
                    g.layers.push(Layer::new("m02", width));
                }
            }
            Ok(())
        })
        .unwrap();
    app.run("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 0, "width": 400, "height": 700})).unwrap();
    app.run("kerning.setPair", &json!({"left": "A", "right": "V", "value": -40})).unwrap();
    app.run("kerning.setPair", &json!({"left": "A", "right": "V", "value": -70, "master": "m02"})).unwrap();
    app
}

fn same_font(a: &FontcraftApp, b: &FontcraftApp) -> bool {
    a.session.font().unwrap().same_as(b.session.font().unwrap())
}

#[test]
fn the_metrics_tab_writes_the_toolbar_master_as_file_export_metrics_does() {
    let dir = temp("export");
    let mut app = app();
    app.state.font_view.master = Some("m02".into());
    let opened = call(&mut app, "engine.execute", json!({"command": "file.export", "params": {"tab": "metrics", "params": {"destination": dir}}}));
    assert_eq!(opened["dialog"], "dialog.exportMetrics");
    assert_eq!(opened["captions"]["ok"], "Next…");
    assert_eq!(opened["params"]["master"], "m02", "export.metricsCurrentMaster: the master the toolbar shows");
    assert_eq!(opened["params"]["format"], "metrics");

    // Cancel writes nothing.
    call(&mut app, "ui.dialog.cancel", json!({}));
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);

    call(&mut app, "engine.execute", json!({"command": "file.export", "params": {"tab": "metrics", "params": {"destination": dir}}}));
    call(&mut app, "ui.dialog.set", json!({"field": "format", "value": "afm"}));
    let done = call(&mut app, "ui.dialog.confirm", json!({}));
    let path = PathBuf::from(done["result"]["path"].as_str().unwrap());
    assert_eq!(path, dir.join("Untitled-Bold.afm"));
    let written = std::fs::read_to_string(&path).unwrap();
    let direct = app.run("file.exportMetrics", &json!({"master": "m02", "format": "afm"})).unwrap();
    assert_eq!(json!(written), direct["text"], "the dialog writes what the command gives");
    assert!(written.contains("\nKPX A V -70\n"), "{written}");

    // A tab shown after another opens on the toolbar's master too, and takes a file name.
    call(&mut app, "engine.execute", json!({"command": "file.export", "params": {"params": {"destination": dir}}}));
    let metrics = call(&mut app, "ui.click", json!({"id": "dialog.exportMetrics"}));
    assert_eq!(metrics["params"]["master"], "m02");
    assert_eq!(metrics["params"]["destination"], json!(dir), "the folder comes along");
    call(&mut app, "ui.dialog.set", json!({"field": "fileName", "value": "Kerning"}));
    let done = call(&mut app, "ui.dialog.confirm", json!({}));
    assert_eq!(PathBuf::from(done["result"]["path"].as_str().unwrap()), dir.join("Kerning.tsv"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_import_file_tab_equals_file_import_metrics_and_cancel_changes_nothing() {
    let dir = temp("import");
    let file = dir.join("Regular.tsv");
    let text = app().run("file.exportMetrics", &json!({"format": "metrics"})).unwrap()["text"].as_str().unwrap().to_string();
    std::fs::write(&file, text).unwrap();

    let mut by_dialog = app();
    by_dialog.state.font_view.master = Some("m02".into());
    let before = by_dialog.session.doc().unwrap().font.clone();
    let opened = call(&mut by_dialog, "engine.execute", json!({"command": "dialog.open", "params": {"command": "file.importMetrics"}}));
    assert_eq!(opened["dialog"], "dialog.importMetricsFile", "the menu item opens the file tab");
    assert_eq!(opened["tabs"][1]["id"], "dialog.importMetricsFont");
    assert_eq!(opened["captions"]["ok"], "Import");
    assert_eq!(opened["params"]["master"], "m02", "the master the toolbar shows");
    assert_eq!(opened["params"]["importMetrics"], false);
    assert_eq!(opened["params"]["glyphs"], json!([]), "Selected Glyphs off: every glyph");
    call(&mut by_dialog, "ui.dialog.set", json!({"field": "file", "value": file}));
    call(&mut by_dialog, "ui.click", json!({"id": "cancel"}));
    assert!(Arc::ptr_eq(&by_dialog.session.doc().unwrap().font, &before), "Cancel changes nothing");

    call(&mut by_dialog, "engine.execute", json!({"command": "dialog.open", "params": {"dialog": "dialog.importMetricsFile"}}));
    call(&mut by_dialog, "ui.dialog.set", json!({"field": "file", "value": file}));
    call(&mut by_dialog, "ui.dialog.set", json!({"field": "importMetrics", "value": true}));
    let shown = call(&mut by_dialog, "ui.dialog.set", json!({"field": "mode", "value": "rsbAndWidth"}));
    assert_eq!(shown["params"]["importMetrics"], "rsbAndWidth");
    let steps = by_dialog.session.doc().unwrap().history.undo.len();
    let done = call(&mut by_dialog, "ui.dialog.confirm", json!({}));
    assert_eq!(done["result"]["kerning"]["pairs"], 1);
    assert_eq!(by_dialog.session.doc().unwrap().history.undo.len(), steps + 1, "one undo step");

    let mut direct = app();
    direct
        .run(
            "file.importMetrics",
            &json!({"file": file, "master": "m02", "importKerning": true, "importKerningGroups": true, "importMetrics": "rsbAndWidth", "keepAlignedComponents": false, "glyphs": []}),
        )
        .unwrap();
    assert!(same_font(&by_dialog, &direct), "the dialog gives the font the command gives");
    let pair = direct.run("kerning.getPair", &json!({"left": "A", "right": "V", "master": "m02"})).unwrap();
    assert_eq!(pair["value"], -40.0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn selected_glyphs_passes_the_font_view_selection() {
    let mut app = app();
    app.state.selected_glyphs = vec!["A".into()];
    let opened = call(&mut app, "engine.execute", json!({"command": "dialog.open", "params": {"command": "file.importMetrics"}}));
    assert_eq!(opened["params"]["glyphs"], json!([]), "unticked: every glyph");
    let ticked = call(&mut app, "ui.dialog.set", json!({"field": "selectedGlyphs", "value": true}));
    assert_eq!(ticked["params"]["glyphs"], json!(["A"]), "ticked: the selection");
    call(&mut app, "ui.dialog.cancel", json!({}));

    // Given in the command's shape, a list ticks the box.
    let given = call(
        &mut app,
        "engine.execute",
        json!({"command": "dialog.open", "params": {"dialog": "dialog.importMetricsFile", "params": {"glyphs": ["B"], "importMetrics": "widthCentered"}}}),
    );
    assert_eq!(given["params"]["glyphs"], json!(["B"]));
    assert_eq!(given["params"]["importMetrics"], "widthCentered");
    call(&mut app, "ui.dialog.cancel", json!({}));

    // While an Edit View tab shows, the Font View selection is still the one ticked.
    app.run("view.newTab", &json!({"glyphs": ["V"]})).unwrap();
    call(&mut app, "engine.execute", json!({"command": "dialog.open", "params": {"command": "file.importMetrics"}}));
    let ticked = call(&mut app, "ui.dialog.set", json!({"field": "selectedGlyphs", "value": true}));
    assert_eq!(ticked["params"]["glyphs"], json!(["A"]), "ticked in a tab: the Font View selection");
    call(&mut app, "ui.dialog.cancel", json!({}));
}

#[test]
fn the_import_font_tab_takes_another_open_document() {
    let setup = |app: &mut FontcraftApp| {
        let mut other = app.session.font().unwrap().clone();
        other.glyph_mut("A").unwrap().kern_groups.right = Some("A".into());
        Arc::make_mut(&mut other.kerning).ltr.set("m01", "@MMK_L_A", "V", -15.0);
        app.session.add_document(other, None).unwrap();
        app.run("document.activate", &json!({"index": 0})).unwrap();
    };
    let mut by_dialog = app();
    setup(&mut by_dialog);
    let opened = call(&mut by_dialog, "engine.execute", json!({"command": "dialog.open", "params": {"command": "file.importMetrics"}}));
    assert_eq!(opened["dialog"], "dialog.importMetricsFile");
    let font_tab = call(&mut by_dialog, "ui.click", json!({"id": "dialog.importMetricsFont"}));
    assert_eq!(font_tab["dialog"], "dialog.importMetricsFont");
    assert_eq!(font_tab["params"]["importKerning"], false, "kerning values start off for a font, whatever the file tab had");
    let source = font_tab["fields"].as_array().unwrap().iter().find(|f| f["id"] == "sourceDocument").unwrap().clone();
    assert_eq!(source["choices"], json!(["", "1"]), "{source}");
    call(&mut by_dialog, "ui.dialog.set", json!({"field": "sourceDocument", "value": "1"}));
    let shown = call(&mut by_dialog, "ui.dialog.set", json!({"field": "kerningValues", "value": true}));
    assert_eq!(shown["params"]["sourceDocument"], 1);
    assert!(shown["params"].get("file").is_none(), "{shown}");
    call(&mut by_dialog, "ui.dialog.confirm", json!({}));

    let mut direct = app();
    setup(&mut direct);
    direct
        .run("file.importMetrics", &json!({"sourceDocument": 1, "master": "m01", "importKerning": true, "importKerningGroups": true, "glyphs": []}))
        .unwrap();
    assert!(same_font(&by_dialog, &direct));
    let f = direct.session.font().unwrap();
    assert_eq!(f.glyph("A").unwrap().kern_groups.right.as_deref(), Some("A"));
}
