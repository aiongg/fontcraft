//! The masters and interpolation dialogs, filled as an agent fills them (`ui.dialog.*`): each gives the font its
//! command gives, and Cancel changes nothing.

use std::sync::Arc;

use craft::host::Host;
use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::Session;
use fontcraft_engine::model::Font;
use serde_json::{Value, json};

use crate::app::{APP, FontCraftApp, Msg, ViewMsg};
use crate::control;

fn call(app: &mut FontCraftApp, method: &str, params: Value) -> Value {
    control::dispatch(app, method, &params).unwrap_or_else(|e| panic!("{method}: {e}"))
}

fn open(app: &mut FontCraftApp, dialog: &str, params: Value) -> Result<Value, String> {
    super::super::form::open_command(app, &json!({"dialog": dialog, "params": params}))
}

fn font(app: &FontCraftApp) -> Arc<Font> {
    app.session.doc().unwrap().font.clone()
}

fn undo_steps(app: &FontCraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

/// Masters Light (wght 30) and Bold (wght 120); glyph `n` with a rectangle per master, shown in the glyph panels.
fn two_masters() -> FontCraftApp {
    let mut s = Session::with_document();
    let mut run = |id: &str, params: Value| {
        s.execute(id, &params).unwrap_or_else(|e| panic!("{id}: {e}"));
    };
    run("font.axes.add", json!({"name": "Weight", "tag": "wght"}));
    run("font.master.add", json!({}));
    run("font.master.setAxisLocation", json!({"masters": [0], "location": [30]}));
    run("font.master.setAxisLocation", json!({"masters": [1], "location": [120]}));
    run("glyph.newGlyph", json!({"name": "n.test"}));
    let ids: Vec<String> = s.font().unwrap().masters.iter().map(|m| m.id.clone()).collect();
    for (id, width) in ids.iter().zip([100, 300]) {
        s.execute("path.addRectangle", &json!({"glyph": "n.test", "layer": id, "x": 50, "y": 0, "width": width, "height": 500})).unwrap();
    }
    let mut app = FontCraftApp::with_edit_tab(s, Arc::new(Host::builder(APP).build()));
    app.sent.push(Msg::View(ViewMsg::SelectGlyph("n.test".into())));
    app.drain();
    app
}

/// The font after running `id` with `params` directly on a fresh [`two_masters`].
fn direct(id: &str, params: &Value) -> Arc<Font> {
    let mut app = two_masters();
    app.session.execute(id, params).unwrap_or_else(|e| panic!("{id}: {e}"));
    font(&app)
}

#[test]
fn the_axis_location_dialog_previews_and_gives_the_font_its_command_gives() {
    let mut app = two_masters();
    app.session.execute("font.customParameters.add", &json!({"scope": "master", "targets": 1, "name": "Axis Location"})).unwrap();
    let error = open(&mut app, "dialog.axisLocation", json!({"targets": 0})).unwrap_err();
    assert!(error.contains("add it first"), "a master without the parameter has nothing to edit: {error}");
    let error = open(&mut app, "dialog.axisLocation", json!({})).unwrap_err();
    assert!(error.contains("targets"), "two masters: the dialog needs to know which: {error}");

    let opened = open(&mut app, "dialog.axisLocation", json!({"targets": 1})).unwrap();
    assert_eq!(opened["params"]["value"], json!([{"Axis": "Weight", "Location": 120.0}]), "an empty value starts at the internal coordinate");
    call(&mut app, "ui.dialog.set", json!({"field": "Weight", "value": 700}));
    let done = call(&mut app, "ui.dialog.confirm", json!({}));
    assert_eq!(done["params"]["value"], json!([{"Axis": "Weight", "Location": 700.0}]));

    let mut expected = two_masters();
    expected.session.execute("font.customParameters.add", &json!({"scope": "master", "targets": 1, "name": "Axis Location"})).unwrap();
    expected.session.execute("font.customParameters.set", &done["params"]).unwrap();
    assert_eq!(font(&app).masters, font(&expected).masters, "the dialog gives the font the command gives");

    // Opened again, it shows the stored value; Cancel leaves the very font it opened on.
    let original = font(&app);
    let again = open(&mut app, "dialog.axisLocation", json!({"targets": 1})).unwrap();
    assert_eq!(again["params"]["value"][0]["Location"], 700.0);
    call(&mut app, "ui.dialog.set", json!({"field": "Weight", "value": 50}));
    call(&mut app, "ui.dialog.cancel", json!({}));
    assert!(Arc::ptr_eq(&font(&app), &original));
}

#[test]
fn set_master_locations_by_table_runs_on_ok_as_its_command_does() {
    let mut app = two_masters();
    let opened = open(&mut app, "dialog.setMasterLocations", json!({})).unwrap();
    assert_eq!(opened["params"]["axis"], "wght");
    assert_eq!(opened["params"]["scale"], 1.0, "without a table the scale and offset apply");

    // A table that does not read reaches the command as typed: the dialog stays open with the refusal.
    let original = font(&app);
    call(&mut app, "ui.dialog.set", json!({"field": "table", "value": "30 to 100"}));
    let refused = control::dispatch(&mut app, "ui.dialog.confirm", &json!({}));
    assert!(refused.is_err() || call(&mut app, "ui.dialog.inspect", json!({}))["open"] == true, "{refused:?}");
    assert!(Arc::ptr_eq(&font(&app), &original), "a refused OK changes nothing");

    call(&mut app, "ui.dialog.set", json!({"field": "table", "value": "30:100, 120:900"}));
    let steps = undo_steps(&app);
    let done = call(&mut app, "ui.dialog.confirm", json!({}));
    assert_eq!(done["params"]["map"], json!([{"from": 30.0, "to": 100.0}, {"from": 120.0, "to": 900.0}]));
    assert_eq!(undo_steps(&app), steps + 1, "one undo step");
    let expected = direct("font.setMasterLocations", &json!({"axis": "wght", "map": [{"from": 30, "to": 100}, {"from": 120, "to": 900}]}));
    let locations = |f: &Font| f.masters.iter().map(|m| m.location.clone()).collect::<Vec<_>>();
    assert_eq!(locations(&font(&app)), locations(&expected));
    assert_eq!(locations(&font(&app)), [vec![100.0], vec![900.0]]);
}

#[test]
fn the_metric_icon_and_variation_dialogs_give_the_font_their_commands_give() {
    let mut app = two_masters();
    // Add Metric starts on a category the font has no plain metric of, and leaves an empty name out.
    let opened = open(&mut app, "fontInfo.addMetric", json!({})).unwrap();
    let category = opened["params"]["category"].clone();
    call(&mut app, "ui.dialog.set", json!({"field": "scope", "value": "script == \"greek\""}));
    let done = call(&mut app, "ui.dialog.confirm", json!({}));
    assert!(done["params"].get("name").is_none(), "{}", done["params"]);
    let expected = direct("font.metrics.add", &json!({"category": category, "scope": "script == \"greek\""}));
    assert_eq!(font(&app).metrics, expected.metrics);

    // The icon picker: a typed glyph wins over the built-in icon.
    open(&mut app, "fontInfo.masterIconPicker", json!({"masters": [0, 1]})).unwrap();
    call(&mut app, "ui.dialog.set", json!({"field": "glyph", "value": "n.test"}));
    let done = call(&mut app, "ui.dialog.confirm", json!({}));
    assert_eq!(done["params"], json!({"masters": [0, 1], "glyph": "n.test"}));
    assert!(font(&app).masters.iter().all(|m| m.icon.as_deref() == Some("n.test")));
    assert!(open(&mut app, "fontInfo.masterIconPicker", json!({})).is_err(), "the picker needs its masters");

    // Adjust by Variation previews on the shown glyph; Cancel puts the font back.
    let original = font(&app);
    // The command declares no glyph targets, so the dialog takes the glyphs it is given.
    let opened = open(&mut app, "dialog.adjustByVariation", json!({"glyphs": ["n.test"]})).unwrap();
    assert_eq!(opened["params"]["glyphs"], json!(["n.test"]));
    call(&mut app, "ui.dialog.set", json!({"field": "wght", "value": 45}));
    assert!(!Arc::ptr_eq(&font(&app), &original), "the preview changes the glyph");
    call(&mut app, "ui.dialog.cancel", json!({}));
    assert!(Arc::ptr_eq(&font(&app), &original));

    // Add Variation: a new axis and a master on it, one undo step.
    let steps = undo_steps(&app);
    open(&mut app, "dialog.addVariation", json!({})).unwrap();
    call(&mut app, "ui.dialog.set", json!({"field": "location", "value": 75}));
    let done = call(&mut app, "ui.dialog.confirm", json!({}));
    assert_eq!(done["params"]["axis"], "wdth", "the first registered axis the font lacks");
    assert_eq!(undo_steps(&app), steps + 1);
    assert_eq!(font(&app).masters.len(), 3);
    assert_eq!(font(&app).axes.len(), 2);
}

/// Intermediate on a corner glyph's layer asks a width and a height, and names the layer as the corner fitting reads
/// it; the font needs no axes (`corner.scaleInterpolatesWithIntermediateLayers`).
#[test]
fn a_corner_glyph_s_layer_is_given_a_size() {
    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
    app.session.execute("glyph.newGlyph", &json!({"name": "_corner.s"})).unwrap();
    app.session
        .edit("setup", |font| {
            let mut wide = font.glyph("_corner.s").unwrap().layers[0].clone();
            wide.id = "wide".into();
            font.glyph_mut("_corner.s")?.layers.push(wide);
            Ok(())
        })
        .unwrap();
    app.drain();
    let edit = |app: &mut FontCraftApp| {
        let command =
            CommandRef::with_params("layers.editSpecialLayerSettings", json!({"glyph": "_corner.s", "layer": "wide", "part": "intermediate"}));
        app.apply_msg(Msg::Shell(command)).unwrap();
    };
    edit(&mut app);
    call(&mut app, "ui.dialog.set", json!({"field": "width", "value": 150}));
    let done = call(&mut app, "ui.dialog.confirm", json!({}));
    assert_eq!(done["params"]["name"], json!("Width 150 Height 100"), "{done}");
    let name = |app: &FontCraftApp| font(app).glyph("_corner.s").unwrap().layers.iter().find(|l| l.id == "wide").unwrap().info.name.clone();
    assert_eq!(name(&app).as_deref(), Some("Width 150 Height 100"));
    edit(&mut app);
    call(&mut app, "ui.dialog.set", json!({"field": "height", "value": 80}));
    call(&mut app, "ui.dialog.confirm", json!({}));
    assert_eq!(name(&app).as_deref(), Some("Width 150 Height 80"), "the fields start from the layer's size");
    let master = font(&app).masters[0].id.clone();
    assert!(open(&mut app, "popover.cornerLayerSize", json!({"glyph": "_corner.s", "layer": master})).is_err());
}
