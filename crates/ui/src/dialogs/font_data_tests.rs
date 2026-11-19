//! Math Table, Baselines and Languages: the text boxes become the commands' parameters and open on what is stored.

use std::sync::Arc;

use craft::host::Host;
use fontcraft_engine::Session;
use fontcraft_engine::cmd::math_base::{baselines, glyph_math, math_constants};
use serde_json::{Value, json};

use crate::app::{APP, FontCraftApp};
use crate::control;

fn call(app: &mut FontCraftApp, method: &str, params: Value) -> Value {
    control::dispatch(app, method, &params).unwrap_or_else(|e| panic!("{method}: {e}"))
}

fn open(app: &mut FontCraftApp, dialog: &str, params: Value) -> Value {
    super::super::form::open_command(app, &json!({"dialog": dialog, "params": params})).unwrap()
}

fn field(shown: &Value, id: &str) -> Value {
    shown["fields"].as_array().unwrap().iter().find(|f| f["id"] == id).map(|f| f["value"].clone()).unwrap_or(Value::Null)
}

fn math_app() -> FontCraftApp {
    let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
    for name in ["integral", "integral.size1"] {
        app.session.execute("glyph.newGlyph", &json!({"name": name, "unicodes": [], "width": 500})).unwrap();
        app.session.execute("path.addRectangle", &json!({"glyph": name, "x": 50, "y": 0, "width": 300, "height": 900})).unwrap();
    }
    let data =
        json!({"constants": {"AxisHeight": 250}, "glyphs": {"integral": {"italicCorrection": 30, "vVariants": ["integral", "integral.size1"]}}});
    app.session.execute("font.setMathData", &data).unwrap();
    app
}

#[test]
fn math_table_opens_on_the_glyph_and_sets_its_data() {
    let mut app = math_app();
    let shown = open(&mut app, "dialog.mathTable", json!({"glyph": "integral"}));
    assert_eq!(field(&shown, "constants"), json!("AxisHeight 250"), "{shown}");
    assert_eq!(field(&shown, "italicCorrection"), json!("30"));
    assert_eq!(field(&shown, "vVariants"), json!("integral integral.size1"));
    assert_eq!(field(&shown, "extendedShape"), json!("no"));

    call(&mut app, "ui.dialog.set", json!({"field": "constants", "value": "AxisHeight 260\nMathLeading 150"}));
    call(&mut app, "ui.dialog.set", json!({"field": "vAssembly", "value": "integral.size1 0 0 100\nintegral 1 100 100"}));
    call(&mut app, "ui.dialog.set", json!({"field": "extendedShape", "value": "yes"}));
    call(&mut app, "ui.dialog.confirm", json!({}));

    let font = app.session.doc().unwrap().font.clone();
    let master = font.default_master().unwrap().id.clone();
    let m = glyph_math(&font, &master, "integral").unwrap();
    assert!(m.extended_shape);
    assert_eq!(m.italic_correction, Some(30.0));
    assert_eq!(m.variants.v_assembly.iter().map(|p| p.flags).collect::<Vec<_>>(), [0, 1]);
    let constants = math_constants(&font, &master);
    assert!(constants.contains(&("MathLeading".to_string(), 150.0)) && constants.contains(&("AxisHeight".to_string(), 260.0)), "{constants:?}");

    // A typo is the command's message, and OK keeps the dialog open.
    open(&mut app, "dialog.mathTable", json!({}));
    call(&mut app, "ui.dialog.set", json!({"field": "constants", "value": "AxisHight 1"}));
    let e = control::dispatch(&mut app, "ui.dialog.confirm", &json!({})).unwrap_err();
    assert!(e.contains("AxisHight"), "{e}");
    assert!(app.dialogs.open.is_some());
}

#[test]
fn baselines_writes_the_base_block_and_opens_on_it() {
    let mut app = math_app();
    let shown = open(&mut app, "dialog.baselines", json!({}));
    assert_eq!(field(&shown, "hTags"), json!(""), "{shown}");
    call(&mut app, "ui.dialog.set", json!({"field": "hTags", "value": "romn ideo"}));
    call(&mut app, "ui.dialog.set", json!({"field": "hScripts", "value": "latn romn 0 -120"}));
    call(&mut app, "ui.dialog.set", json!({"field": "hExtents", "value": "latn dflt -300 900"}));
    call(&mut app, "ui.dialog.confirm", json!({}));
    let font = app.session.doc().unwrap().font.clone();
    let (_, base) = baselines(&font).unwrap().unwrap();
    let h = base.horizontal.unwrap();
    assert_eq!(h.tags, ["ideo", "romn"]);
    assert_eq!(h.scripts.first().map(|s| s.values.clone()), Some(vec![-120, 0]));
    assert!(base.vertical.is_none());

    let shown = open(&mut app, "dialog.baselines", json!({}));
    assert_eq!(field(&shown, "hTags"), json!("ideo romn"));
    assert_eq!(field(&shown, "hScripts"), json!("latn romn -120 0"));
}

#[test]
fn languages_shows_the_ticked_scripts_and_hides_the_others() {
    let mut app = math_app();
    let shown = open(&mut app, "dialog.languagesList", json!({}));
    assert_eq!(field(&shown, "scripts"), json!(["cyrillic", "greek", "latin"]), "the default scripts are ticked");
    call(&mut app, "ui.dialog.set", json!({"field": "scripts", "value": ["greek"]}));
    let done = call(&mut app, "ui.dialog.confirm", json!({}));
    assert_eq!(done["params"]["show"], json!(["greek"]));
    assert!(done["params"]["hide"].as_array().unwrap().contains(&json!("latin")));
    let shown = open(&mut app, "dialog.languagesList", json!({}));
    assert_eq!(field(&shown, "scripts"), json!(["greek"]), "it opens on the scripts the settings list");
    call(&mut app, "ui.dialog.cancel", json!({}));
}
