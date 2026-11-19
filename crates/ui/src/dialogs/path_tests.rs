//! The outline dialogs give the font their command gives; Shape Order lists the layer's shapes.

use std::sync::Arc;

use craft::host::Host;
use fontcraft_engine::Session;
use fontcraft_engine::model::Font;
use serde_json::{Value, json};

use crate::app::{APP, FontCraftApp, Msg, ViewMsg};
use crate::control;

fn call(app: &mut FontCraftApp, method: &str, params: Value) -> Value {
    control::dispatch(app, method, &params).unwrap_or_else(|e| panic!("{method}: {e}"))
}

fn fails(app: &mut FontCraftApp, method: &str, params: Value) -> String {
    control::dispatch(app, method, &params).expect_err(&format!("{method} should fail"))
}

fn open(app: &mut FontCraftApp, params: Value) -> Value {
    super::super::form::open_command(app, &params).unwrap()
}

fn font(app: &FontCraftApp) -> Arc<Font> {
    app.session.doc().unwrap().font.clone()
}

fn value(shown: &Value, key: &str) -> Value {
    shown["fields"].as_array().unwrap().iter().find(|f| f["id"] == key).map(|f| f["value"].clone()).unwrap_or(Value::Null)
}

/// `glyph` with the paths `paths` (each a list of corner points), shown in the glyph panels.
fn drawn(glyph: &str, paths: &[&[(i32, i32)]]) -> FontCraftApp {
    let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
    app.session.execute("glyph.newGlyph", &json!({"name": glyph})).ok();
    for nodes in paths {
        let nodes: Vec<Value> = nodes.iter().map(|(x, y)| json!({"x": x, "y": y})).collect();
        app.session.execute("path.addPath", &json!({"glyph": glyph, "nodes": nodes})).unwrap();
    }
    app.sent.push(Msg::View(ViewMsg::SelectGlyph(glyph.into())));
    app.drain();
    app
}

const SQUARE: &[(i32, i32)] = &[(100, 100), (300, 100), (300, 300), (100, 300)];
const H: &[(i32, i32)] =
    &[(0, 0), (80, 0), (80, 300), (520, 300), (520, 0), (600, 0), (600, 700), (520, 700), (520, 360), (80, 360), (80, 700), (0, 700)];

/// Opens `dialog`, sets `fields`, presses OK; the result must equal running `command` with `params` on the same glyph.
fn same_as_command(app: fn() -> FontCraftApp, dialog: &str, fields: Value, command: &str, params: Value) {
    let mut by_dialog = app();
    let before = font(&by_dialog);
    open(&mut by_dialog, json!({"dialog": dialog}));
    for (field, value) in fields.as_object().unwrap() {
        call(&mut by_dialog, "ui.dialog.set", json!({"field": field, "value": value}));
    }
    call(&mut by_dialog, "ui.dialog.confirm", json!({}));
    let mut direct = app();
    direct.session.execute(command, &params).unwrap();
    assert!(!font(&by_dialog).same_as(&before), "{dialog} changed nothing");
    assert!(font(&by_dialog).same_as(&font(&direct)), "{dialog} and {command} give the same font");
}

#[test]
fn transformations_previews_shapes_the_commands_objects_and_copies_the_filter_parameter() {
    let mut app = drawn("A", &[SQUARE]);
    let original = font(&app);
    let opened = open(&mut app, json!({"dialog": "dialog.transformations", "params": {"translate": {"x": 20}, "scale": {"x": 80}}}));
    assert_eq!(opened["params"]["glyph"], "A", "no points selected: the whole layer");
    assert_eq!(
        (opened["params"]["translate"].clone(), opened["params"]["scale"].clone()),
        (json!({"x": 20.0, "y": 0.0}), json!({"x": 80.0, "y": 80.0}))
    );
    assert_eq!(value(&opened, "scaleLocked"), true, "a scale without y opens locked");
    assert!(!font(&app).same_as(&original), "the canvas shows the preview");

    let shown = call(&mut app, "ui.dialog.set", json!({"field": "scaleY", "value": 60}));
    assert_eq!(value(&shown, "scaleX"), 60.0, "locked: the other scale follows");
    call(&mut app, "ui.click", json!({"id": "scaleLocked"}));
    let shown = call(&mut app, "ui.dialog.set", json!({"field": "scaleY", "value": 120}));
    assert_eq!(shown["params"]["scale"], json!({"x": 60.0, "y": 120.0}));
    call(&mut app, "ui.dialog.set", json!({"field": "translateX", "value": 0}));
    assert!(call(&mut app, "ui.dialog.inspect", json!({}))["params"].get("translate").is_none(), "a move of 0 is left out");
    assert!(fails(&mut app, "ui.dialog.set", json!({"field": "slant", "value": 90})).contains("from -89.9 to 89.9"));

    call(&mut app, "ui.dialog.set", json!({"field": "slant", "value": 10}));
    let copied = call(&mut app, "ui.click", json!({"id": "copyFilterParameter"}));
    assert!(copied["text"].as_str().unwrap().starts_with("Transformations;"), "{copied}");
    assert_eq!(call(&mut app, "ui.dialog.inspect", json!({}))["open"], true, "an action leaves the dialog open");
    let done = call(&mut app, "ui.dialog.confirm", json!({}));

    let mut direct = drawn("A", &[SQUARE]);
    direct.session.execute("path.transformations", &done["params"]).unwrap();
    assert!(font(&app).same_as(&font(&direct)));
}

#[test]
fn transform_metrics_sends_the_fields_of_its_mode() {
    let app = || drawn("A", &[SQUARE]);
    same_as_command(
        app,
        "dialog.transformMetrics",
        json!({"lsb": 20, "useRsb": false}),
        "glyph.transformMetrics",
        json!({"glyphs": ["A"], "lsb": 20, "relative": true}),
    );
    same_as_command(
        app,
        "dialog.transformMetrics",
        json!({"useWidth": true, "width": 700, "onBothSides": true}),
        "glyph.transformMetrics",
        json!({"glyphs": ["A"], "width": 700, "onBothSides": true}),
    );
}

#[test]
fn each_stem_filter_dialog_gives_its_commands_font() {
    let h = || drawn("H", &[H]);
    same_as_command(
        h,
        "dialog.filterAdjustStems",
        json!({"verticalFrom": 70, "verticalTo": 90, "verticalValue": 10}),
        "filter.adjustStems",
        json!({"glyph": "H", "vertical": [{"from": 70, "to": 90, "offset": 10}]}),
    );
    same_as_command(
        h,
        "dialog.filterChangeWeight",
        json!({"amount": 20, "method": "custom", "top": 700, "serifHeight": 10}),
        "filter.changeWeight",
        json!({"glyph": "H", "amount": 20, "method": "custom", "zones": {"bottom": 0, "top": 700, "serifHeight": 10}}),
    );
    same_as_command(
        h,
        "dialog.filterCondenseExtend",
        json!({"counterScale": 80, "sidebearingAdd": 5}),
        "filter.condenseExtend",
        json!({"glyph": "H", "counters": {"scale": 80}, "sidebearings": {"add": 5}}),
    );
    same_as_command(
        h,
        "dialog.filterChangeXHeight",
        json!({"to": 540, "serifHeight": 20}),
        "filter.changeXHeight",
        json!({"glyph": "H", "to": 540, "serifHeight": 20}),
    );
}

fn dotted() -> FontCraftApp {
    let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
    let square = |x: i32, y: i32| -> Vec<Value> {
        [(x, y), (x + 40, y), (x + 40, y + 40), (x, y + 40)].iter().map(|(x, y)| json!({"x": x, "y": y})).collect()
    };
    let diamond: Vec<Value> = [(20, 0), (40, 20), (20, 40), (0, 20)].iter().map(|(x, y)| json!({"x": x, "y": y})).collect();
    for (glyph, nodes) in [("a", square(0, 0)), ("b", diamond), ("i", square(0, 100)), ("i", square(300, 600)), ("j", square(100, 600))] {
        app.session.execute("glyph.newGlyph", &json!({"name": glyph})).ok();
        app.session.execute("path.addPath", &json!({"glyph": glyph, "nodes": nodes})).unwrap();
    }
    app
}

#[test]
fn find_all_lists_the_matches_and_replace_all_replaces_them() {
    let mut app = dotted();
    let before = font(&app);
    let opened = open(&mut app, json!({"command": "path.findOutlinePattern", "params": {"patternGlyph": "a"}}));
    assert_eq!(opened["captions"]["ok"], "Find All");
    call(&mut app, "ui.dialog.confirm", json!({}));
    assert!(Arc::ptr_eq(&font(&app), &before), "Find All changes nothing");

    let found = call(&mut app, "ui.dialog.inspect", json!({}));
    assert_eq!(found["command"], "path.replaceOutlinePattern", "{found}");
    assert_eq!(value(&found, "matches").as_array().map(Vec::len), Some(3), "{found}");
    call(&mut app, "ui.dialog.set", json!({"field": "replacementGlyph", "value": "b"}));
    call(&mut app, "ui.dialog.confirm", json!({}));

    let mut direct = dotted();
    direct.session.execute("path.replaceOutlinePattern", &json!({"patternGlyph": "a", "replacementGlyph": "b"})).unwrap();
    assert!(font(&app).same_as(&font(&direct)));

    // Edit › Replace Outline… copies the matches from its actions menu.
    let mut app = dotted();
    let opened = open(&mut app, json!({"dialog": "dialog.findOutline", "params": {"patternGlyph": "a", "replacementGlyph": "b"}}));
    assert_eq!(opened["command"], "path.replaceOutlinePattern");
    let copied = call(&mut app, "ui.click", json!({"id": "copyOutlineMatches"}));
    assert_eq!(copied["text"].as_str().unwrap().lines().count(), 3, "{copied}");
    call(&mut app, "ui.dialog.cancel", json!({}));
}

#[test]
fn shape_order_lists_the_shapes_and_previews_the_order() {
    let three = || drawn("A", &[SQUARE, &[(150, 100), (250, 100), (250, 200), (150, 200)], &[(400, 100), (450, 100), (450, 150), (400, 150)]]);
    let mut app = three();
    let original = font(&app);
    let opened = open(&mut app, json!({"command": "filter.shapeOrder"}));
    assert_eq!(opened["dialog"], "dialog.shapeOrder");
    let field = &opened["fields"][0];
    assert_eq!(field["labels"], json!(["0: Path (4 nodes)", "1: Path (4 nodes)", "2: Path (4 nodes)"]), "{field}");
    assert_eq!(field["value"], json!([0, 1, 2]));
    for bad in [json!([0, 1]), json!([0, 0, 1]), json!([0, 1, 3])] {
        fails(&mut app, "ui.dialog.set", json!({"field": "order", "value": bad}));
    }
    call(&mut app, "ui.dialog.set", json!({"field": "order", "value": [2, 0, 1]}));
    assert!(!font(&app).same_as(&original), "the canvas shows the new order");
    call(&mut app, "ui.dialog.confirm", json!({}));
    let mut direct = three();
    direct.session.execute("filter.shapeOrder", &json!({"glyph": "A", "order": [2, 0, 1]})).unwrap();
    assert!(font(&app).same_as(&font(&direct)));
}
