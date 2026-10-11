//! The spacing dialogs driven over the control channel: `dialog.transformMetrics` gives the font
//! `glyph.transformMetrics` gives with the same answers, `dialog.removeGlyphMetricsKeys` is the
//! metrics-keys question of Remove Glyph, and the smart-filter rule editor offers the spacing
//! rules (`smartFilter.rule.hasMetricsKeys`, `smartFilter.rule.metricsKeysOutOfSync`,
//! `smartFilter.rule.hasKerningGroups`).

use std::sync::Arc;

use fontcraft_model::Font;
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

fn set(app: &mut FontcraftApp, field: &str, value: Value) -> Value {
    call(app, "ui.dialog.set", json!({"field": field, "value": value}))
}

fn font(app: &FontcraftApp) -> Arc<Font> {
    app.session.doc().unwrap().font.clone()
}

/// A and B each hold a 200-unit square at x = 100, shown in Font View.
fn squares_app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    for glyph in ["A", "B"] {
        app.run("path.addRectangle", &json!({"glyph": glyph, "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    }
    app
}

fn direct(params: Value) -> Arc<Font> {
    let mut app = squares_app();
    app.run("glyph.transformMetrics", &params).unwrap();
    font(&app)
}

#[test]
fn transform_metrics_dialog_equals_the_command() {
    // From the menu on the Font View selection; opening changes nothing.
    let mut app = squares_app();
    app.state.selected_glyphs = vec!["A".into(), "B".into()];
    let before = font(&app);
    let opened = call(&mut app, "ui.menu.invoke", json!({"command": "glyph.transformMetrics"}));
    assert_eq!(opened["dialog"], "dialog.transformMetrics", "{opened}");
    let p = &opened["params"];
    assert_eq!(p["glyphs"], json!(["A", "B"]), "the Font View selection: {p}");
    assert_eq!((p["lsb"].clone(), p["rsb"].clone(), p["relative"].clone(), p.get("width")), (json!(0.0), json!(0.0), json!(true), None));
    assert!(font(&app).same_as(&before), "the dialog opens on a change of nothing");

    // Width mode, on both sides.
    set(&mut app, "useWidth", json!(true));
    set(&mut app, "width", json!(500));
    let shown = set(&mut app, "onBothSides", json!(true));
    let p = &shown["params"];
    assert_eq!((p["width"].clone(), p["onBothSides"].clone()), (json!(500.0), json!(true)), "{p}");
    assert!(p.get("lsb").is_none() && p.get("rsb").is_none() && p.get("relative").is_none(), "the other mode's values are not sent: {p}");
    call(&mut app, "ui.dialog.confirm", json!({}));
    let want = direct(json!({"glyphs": ["A", "B"], "width": 500, "onBothSides": true}));
    assert!(font(&app).same_as(&want), "Width mode gives the font the command gives");
    assert_eq!(font(&app).glyph("B").unwrap().layers[0].width, 500.0);

    // Sidebearing mode, the left side only, absolute.
    let mut app = squares_app();
    open(&mut app, "dialog.transformMetrics", json!({"glyphs": ["A", "B"]}));
    set(&mut app, "useRsb", json!(false));
    set(&mut app, "relative", json!(false));
    set(&mut app, "lsb", json!(40));
    call(&mut app, "ui.dialog.confirm", json!({}));
    assert!(font(&app).same_as(&direct(json!({"glyphs": ["A", "B"], "lsb": 40, "relative": false}))));

    // Parameters in the command's shape fill the checkboxes; Cancel changes nothing.
    let mut app = squares_app();
    let before = font(&app);
    let opened = open(&mut app, "dialog.transformMetrics", json!({"glyphs": ["A"], "rsb": 30}));
    let value = |key: &str| opened["fields"].as_array().unwrap().iter().find(|f| f["id"] == key).map(|f| f["value"].clone()).unwrap();
    assert_eq!((value("useWidth"), value("useLsb"), value("useRsb")), (json!(false), json!(false), json!(true)), "{opened}");
    assert!(!font(&app).same_as(&before), "the preview adds 30 units");
    call(&mut app, "ui.dialog.cancel", json!({}));
    assert!(Arc::ptr_eq(&font(&app), &before), "Cancel leaves the very font the dialog opened on");

    // Neither side checked: the command's message, and OK waits.
    let mut app = squares_app();
    open(&mut app, "dialog.transformMetrics", json!({"glyphs": ["A"]}));
    set(&mut app, "useLsb", json!(false));
    let shown = set(&mut app, "useRsb", json!(false));
    assert!(shown["error"].as_str().is_some_and(|e| e.contains("lsb")), "{shown}");
}

/// [`squares_app`] where C's left metrics key is `=A` and nothing uses A as a component.
fn keyed_app() -> FontcraftApp {
    let mut app = squares_app();
    app.session
        .edit("key", |f| {
            f.glyph_mut("C")?.info.metrics_keys.left = Some("=A".into());
            Ok(())
        })
        .unwrap();
    app
}

#[test]
fn remove_glyph_asks_about_metrics_keys_dissolve_or_keep() {
    let mut app = keyed_app();
    let opened = open(&mut app, "dialog.removeGlyph", json!({"glyphs": ["A"]}));
    assert_eq!(opened["fields"].as_array().unwrap().len(), 1, "{opened}");
    assert_eq!(opened["fields"][0]["id"], "metricsKeys");
    assert!(opened["fields"][0]["label"].as_str().unwrap().contains('C'), "the question names the glyph whose key points at A");
    assert_eq!(opened["buttons"], json!(["ok", "cancel"]), "no components: no Decompose");
    assert_eq!(opened["params"], json!({"glyphs": ["A"], "metricsKeys": "dissolve"}), "Dissolve is the default");

    set(&mut app, "metricsKeys", json!("keep"));
    call(&mut app, "ui.dialog.confirm", json!({}));
    let mut want = keyed_app();
    want.run("glyph.removeGlyph", &json!({"glyphs": ["A"], "metricsKeys": "keep"})).unwrap();
    assert!(font(&app).same_as(&font(&want)), "Keep gives the font the command gives");
    assert_eq!(font(&app).glyph("C").unwrap().info.metrics_keys.left.as_deref(), Some("=A"), "the key still points at the removed glyph");

    let mut app = keyed_app();
    open(&mut app, "dialog.removeGlyph", json!({"glyphs": ["A"]}));
    call(&mut app, "ui.dialog.confirm", json!({}));
    let mut want = keyed_app();
    want.run("glyph.removeGlyph", &json!({"glyphs": ["A"], "metricsKeys": "dissolve"})).unwrap();
    assert!(font(&app).same_as(&font(&want)), "Dissolve gives the font the command gives");
    assert!(font(&app).glyph("C").unwrap().info.metrics_keys.left.is_none());
}

#[test]
fn smart_filter_rules_for_spacing_are_offered_and_query() {
    let offered: Vec<&str> = crate::dialogs::rule_tree::PROPERTIES.iter().map(|(id, _)| *id).collect();
    for id in ["hasMetricsKeys", "metricsKeysOutOfSync", "hasKerningGroups"] {
        assert!(offered.contains(&id), "the rule editor offers {id}");
        assert_eq!(crate::dialogs::rule_tree::ops_of(id), ["is", "isNot"], "{id} is a yes-or-no rule");
    }
    let mut app = keyed_app();
    app.session
        .edit("group", |f| {
            f.glyph_mut("B")?.kern_groups.left = Some("B".into());
            Ok(())
        })
        .unwrap();
    let query = |app: &mut FontcraftApp, property: &str| {
        let rules = json!({"all": [{"property": property, "op": "is", "value": true}]});
        call(app, "engine.execute", json!({"command": "fontView.queryGlyphs", "params": {"rules": rules}}))["glyphs"].clone()
    };
    assert_eq!(query(&mut app, "hasMetricsKeys"), json!(["C"]));
    assert_eq!(query(&mut app, "hasKerningGroups"), json!(["B"]));
}
