//! The Math Table and Baselines dialogs driven over the control channel: each opens on the
//! font's data and OK gives the font the command gives with the same answers.

use fontcraft_engine::cmd::math_base::{baselines, glyph_math};
use serde_json::{Value, json};

use crate::{FontcraftApp, control};

fn call(app: &mut FontcraftApp, method: &str, params: Value) -> Value {
    let r = control::handle(app, &json!({"id": 1, "method": method, "params": params}));
    assert_eq!(r["ok"], true, "{method}: {r}");
    r["result"].clone()
}

fn open(app: &mut FontcraftApp, dialog: &str, params: Value) -> Value {
    call(app, "engine.execute", json!({"command": "dialog.open", "params": {"dialog": dialog, "params": params}}))
}

fn field(shown: &Value, id: &str) -> Value {
    shown["fields"].as_array().unwrap().iter().find(|f| f["id"] == id).map(|f| f["value"].clone()).unwrap_or(Value::Null)
}

fn math_app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    for name in ["integral", "integral.size1"] {
        app.run("glyph.newGlyph", &json!({"name": name, "unicodes": [], "width": 500})).unwrap();
        app.run("path.addRectangle", &json!({"glyph": name, "x": 50, "y": 0, "width": 300, "height": 900})).unwrap();
    }
    app.run(
        "font.setMathData",
        &json!({"constants": {"AxisHeight": 250}, "glyphs": {"integral": {"italicCorrection": 30, "vVariants": ["integral", "integral.size1"]}}}),
    )
    .unwrap();
    app
}

#[test]
fn math_table_dialog_opens_on_the_glyph_and_sets_its_data() {
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
    assert_eq!(m.variants.v_assembly.len(), 2);
    assert_eq!(m.variants.v_assembly[1].flags, 1);
    let constants = fontcraft_engine::cmd::math_base::math_constants(&font, &master);
    assert!(constants.contains(&("MathLeading".to_string(), 150.0)) && constants.contains(&("AxisHeight".to_string(), 260.0)), "{constants:?}");

    // A typo is the command's message, and OK keeps the dialog open.
    open(&mut app, "dialog.mathTable", json!({}));
    let shown = call(&mut app, "ui.dialog.set", json!({"field": "constants", "value": "AxisHight 1"}));
    assert!(shown["error"].as_str().is_some_and(|e| e.contains("AxisHight")), "{shown}");
}

#[test]
fn baselines_dialog_writes_the_base_block() {
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
    assert_eq!(h.scripts[0].values, [-120, 0]);
    assert_eq!((h.extents[0].min, h.extents[0].max), (-300, 900));
    assert!(base.vertical.is_none());

    // It opens again on what it wrote, in table order.
    let shown = open(&mut app, "dialog.baselines", json!({}));
    assert_eq!(field(&shown, "hTags"), json!("ideo romn"));
    assert_eq!(field(&shown, "hScripts"), json!("latn romn -120 0"));
}
