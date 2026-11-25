use std::sync::Arc;

use craft::host::Host;
use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::Session;
use fontcraft_engine::model::{Layer, LayerSide};
use serde_json::{Value, json};

use crate::app::{APP, FontCraftApp, Msg, ViewMsg};
use crate::{control, dialogs};

fn call(app: &mut FontCraftApp, method: &str, params: Value) -> Value {
    control::dispatch(app, method, &params).unwrap_or_else(|e| panic!("{method}: {e}"))
}

fn open(app: &mut FontCraftApp, params: Value) -> Value {
    dialogs::form::open_command(app, &params).unwrap()
}

fn square(session: &mut Session, glyph: &str, size: u32) {
    session.execute("path.addRectangle", &json!({"glyph": glyph, "x": 0, "y": 0, "width": size, "height": size})).unwrap();
}

fn app() -> FontCraftApp {
    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
    app.sent.push(Msg::View(ViewMsg::SelectGlyph("A".into())));
    app.drain();
    app
}

/// An open document with unsaved edits as the source: its masters have other names, so nothing changes until the person
/// pairs them.
#[test]
fn assign_background_takes_an_open_document_and_asks_for_unmatched_masters() {
    let mut app = app();
    app.session
        .edit("masters", |font| {
            let mut m = font.default_master()?.clone();
            m.id = "src2".into();
            m.name = "Heavy".into();
            let masters = Arc::make_mut(&mut font.masters);
            masters[0].name = "Light".into();
            masters.push(m);
            font.glyph_mut("A")?.layers.push(Layer::new("src2", 700.0));
            Ok(())
        })
        .unwrap();
    app.session.execute("path.addRectangle", &json!({"glyph": "A", "layer": "src2", "x": 0, "y": 0, "width": 70, "height": 70})).unwrap();
    app.session.execute("file.new", &json!({})).unwrap();
    app.drain();
    app.sent.push(Msg::View(ViewMsg::SelectGlyph("A".into())));
    app.drain();

    let opened = open(&mut app, json!({"command": "path.assignBackground", "params": {"glyphs": ["A"]}}));
    assert_eq!(opened["dialog"], "dialog.assignBackground");
    call(&mut app, "ui.dialog.set", json!({"field": "source", "value": "0"}));
    let asked = call(&mut app, "ui.dialog.confirm", json!({}));
    assert_eq!(asked["result"]["unmatched"].as_array().unwrap().len(), 1, "{asked}");
    let first = app.session.font().unwrap().masters[0].id.clone();
    let background = |app: &FontCraftApp| {
        let glyph = app.session.font().unwrap().glyph("A").unwrap().clone();
        glyph.layer(&first).unwrap().parts(LayerSide::Background).shapes.len()
    };
    assert_eq!(background(&app), 0, "nothing changed yet");
    let asking = call(&mut app, "ui.dialog.inspect", json!({}));
    assert_eq!(asking["dialog"], "dialog.assignBackgroundMasters");

    call(&mut app, "ui.dialog.set", json!({"field": format!("master:{first}"), "value": "src2"}));
    let done = call(&mut app, "ui.dialog.confirm", json!({}));
    assert_eq!(done["result"]["glyphs"], json!(["A"]), "{done}");
    assert_eq!(background(&app), 1, "the outlines of the chosen master");
}

/// A layer name copies each glyph's layer of that name, whatever its id; a glyph without it is skipped.
#[test]
fn copy_layer_to_layer_goes_by_name() {
    let mut app = app();
    for glyph in ["A", "B"] {
        square(&mut app.session, glyph, 50);
        app.session.execute("layers.addBackupLayer", &json!({"glyphs": [glyph], "name": "old"})).unwrap();
        square(&mut app.session, glyph, 90);
    }
    open(&mut app, json!({"command": "edit.copyLayerToLayer", "params": {"glyphs": ["A", "B", "C"]}}));
    call(&mut app, "ui.dialog.set", json!({"field": "fromLayer", "value": "name:old"}));
    let first = app.session.font().unwrap().masters[0].id.clone();
    call(&mut app, "ui.dialog.set", json!({"field": "toLayer", "value": format!("id:{first}")}));
    let done = call(&mut app, "ui.dialog.confirm", json!({}));
    assert_eq!(done["result"]["copied"], json!(["A", "B"]), "{done}");
    assert_eq!(done["result"]["skipped"][0]["glyph"], "C");
    let font = app.session.font().unwrap();
    assert_eq!(font.glyph("B").unwrap().layer(&first).unwrap().shapes.len(), 1, "the square drawn after the backup is gone");
}

/// Compare Layers in Font View selects exactly the glyphs that differ.
#[test]
fn compare_layers_selects_the_glyphs_that_differ() {
    let mut app = app();
    for glyph in ["A", "B", "C"] {
        square(&mut app.session, glyph, 50);
        app.session.execute("layers.addBackupLayer", &json!({"glyphs": [glyph], "name": "old"})).unwrap();
    }
    square(&mut app.session, "C", 90);
    app.sent.push(Msg::Shell(CommandRef::with_params("fontView.show", json!({}))));
    app.sent.push(Msg::Shell(CommandRef::with_params("fontView.setSelection", json!({"glyphs": ["A", "B", "C"]}))));
    app.drain();
    open(&mut app, json!({"command": "font.compareLayers", "params": {"glyphs": ["A", "B", "C"]}}));
    call(&mut app, "ui.dialog.set", json!({"field": "secondLayer", "value": "name:old"}));
    call(&mut app, "ui.dialog.confirm", json!({}));
    app.drain();
    assert_eq!(app.view.font_view.selected, ["C"]);
    assert!(app.status.contains("1 glyph differs: C"), "{}", app.status);
}
