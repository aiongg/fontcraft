//! The export entry commands (`font.instance.*`) on fonts built in code: defaults, value checks,
//! warnings, undo, and the order of entries leaving exported files alone.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::BTreeMap;
use std::sync::Arc;

use base64::Engine as _;
use fontcraft_engine::{EngineError, Session};
use fontcraft_model::{Axis, Font, Instance};
use serde_json::{Value, json};
use skrifa::MetadataProvider;
use skrifa::raw::TableProvider;
use skrifa::string::StringId;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

/// The command's refusal message; panics when it succeeds.
fn refused(s: &mut Session, id: &str, p: Value) -> String {
    match s.execute(id, &p) {
        Err(EngineError::BadParams { message, .. }) => message,
        other => panic!("{id} {p}: expected a refusal, got {other:?}"),
    }
}

/// "Test Sans" with a weight axis and its one master at weight 400, plus `instances`.
fn font(instances: Vec<Instance>) -> Session {
    let mut f = Font::new("Test Sans");
    f.axes = Arc::new(vec![Axis { name: "Weight".into(), tag: "wght".into(), ..Axis::default() }]);
    for m in Arc::make_mut(&mut f.masters) {
        m.location = vec![400.0];
    }
    f.instances = Arc::new(instances);
    let mut s = Session::new();
    s.add_document(f, None).unwrap();
    s
}

fn instance(name: &str) -> Instance {
    Instance { name: name.into(), location: vec![400.0], ..Instance::default() }
}

fn list(s: &mut Session) -> Vec<Value> {
    run(s, "font.instance.list", json!({}))["instances"].as_array().unwrap().clone()
}

fn names(s: &mut Session) -> Vec<String> {
    list(s).iter().map(|i| i["name"].as_str().unwrap().to_string()).collect()
}

#[test]
fn font_instance_add_list_and_get() {
    let mut s = font(Vec::new());
    let out = run(&mut s, "font.instance.add", json!({}));
    assert_eq!(out["index"], 0);
    assert_eq!(out["name"], "Regular");
    assert_eq!(out["warnings"], json!([]));
    let bold = run(&mut s, "font.instance.add", json!({"styleName": "Bold", "weightClass": 700, "axes": {"wght": 700}}));
    assert_eq!(bold["index"], 1, "a new instance goes to the end of the list");

    let all = list(&mut s);
    assert_eq!(all.len(), 2);
    let regular = &all[0];
    assert_eq!(regular["name"], "Regular");
    assert_eq!(regular["type"], "static");
    assert_eq!(regular["active"], true);
    assert_eq!(regular["weightClass"], 400);
    assert_eq!(regular["widthClass"], 5);
    assert_eq!(regular["axes"], json!({"wght": 400}), "the first master's coordinates");
    assert_eq!(regular["isBold"], false);
    assert_eq!(regular["linkStyle"], Value::Null);
    assert_eq!(all[1]["weightClass"], 700);
    assert_eq!(all[1]["axes"], json!({"wght": 700}));

    assert_eq!(run(&mut s, "font.instance.list", json!({"type": "variable"}))["instances"], json!([]));
    assert_eq!(run(&mut s, "font.instance.list", json!({"type": "static"}))["instances"].as_array().unwrap().len(), 2);
    assert!(refused(&mut s, "font.instance.list", json!({"type": "other"})).contains("type"));

    let got = run(&mut s, "font.instance.get", json!({"instance": "Bold"}))["instance"].clone();
    assert_eq!(got["name"], "Bold");
    assert_eq!(got["index"], 1);
    assert_eq!(got["visible"], true);
    assert_eq!(got["properties"], json!({}));
    assert_eq!(got["customParameters"], json!([]));
    assert!(refused(&mut s, "font.instance.get", json!({"instance": "Black"})).contains("Black"));

    // One undo step per add.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(names(&mut s), ["Regular"]);
}

#[test]
fn font_instance_add_refuses_bad_values_and_changes_nothing() {
    let mut s = font(vec![instance("Regular")]);
    for (params, needle) in [
        (json!({"weightClass": 0}), "weightClass"),
        (json!({"weightClass": 1001}), "weightClass"),
        (json!({"weightClass": 400.5}), "weightClass"),
        (json!({"widthClass": 0}), "widthClass"),
        (json!({"widthClass": 10}), "widthClass"),
        (json!({"axes": {"wdth": 100}}), "wdth"),
        (json!({"axes": {"wght": "heavy"}}), "wght"),
        (json!({"styleName": "  "}), "styleName"),
    ] {
        let message = refused(&mut s, "font.instance.add", params.clone());
        assert!(message.contains(needle), "{params}: {message}");
    }
    assert_eq!(names(&mut s), ["Regular"]);
    // The ends of the ranges are accepted.
    run(&mut s, "font.instance.add", json!({"styleName": "Hairline", "weightClass": 1, "widthClass": 1}));
    run(&mut s, "font.instance.add", json!({"styleName": "Ultra", "weightClass": 1000, "widthClass": 9}));
    assert_eq!(names(&mut s), ["Regular", "Hairline", "Ultra"]);
}

#[test]
fn font_instance_set_writes_several_entries_in_one_step() {
    let mut s = font(vec![instance("Regular"), instance("Bold"), instance("Black")]);
    let out = run(&mut s, "font.instance.set", json!({"instances": ["Regular", "Bold"], "widthClass": 3, "active": false}));
    assert_eq!(out["changed"], json!(["active", "widthClass"]));
    assert_eq!(out["instances"], json!([0, 1]));
    let all = list(&mut s);
    assert_eq!((all[0]["widthClass"].clone(), all[0]["active"].clone()), (json!(3), json!(false)));
    assert_eq!((all[1]["widthClass"].clone(), all[1]["active"].clone()), (json!(3), json!(false)));
    assert_eq!((all[2]["widthClass"].clone(), all[2]["active"].clone()), (json!(5), json!(true)));

    let out = run(&mut s, "font.instance.set", json!({"instances": ["Bold"], "styleName": "Semibold", "axes": {"wght": 600}}));
    assert_eq!(out["warnings"], json!([]));
    assert_eq!(names(&mut s), ["Regular", "Semibold", "Black"]);

    // One failing target or value fails the whole call.
    refused(&mut s, "font.instance.set", json!({"instances": ["Regular", "Thin"], "widthClass": 4}));
    refused(&mut s, "font.instance.set", json!({"instances": ["Regular"], "widthClass": 4, "weightClass": 2000}));
    refused(&mut s, "font.instance.set", json!({"instances": ["Regular"]}));
    assert_eq!(list(&mut s)[0]["widthClass"], 3);

    run(&mut s, "edit.undo", json!({}));
    assert_eq!(names(&mut s), ["Regular", "Bold", "Black"]);
    run(&mut s, "edit.undo", json!({}));
    let all = list(&mut s);
    assert!(all.iter().all(|i| i["widthClass"] == 5 && i["active"] == true), "{all:?}");
}

#[test]
fn font_instance_set_weight_class_and_width_class_leave_the_axes_alone() {
    let mut s = font(vec![instance("Regular")]);
    run(&mut s, "font.instance.setWeightClass", json!({"instances": ["Regular"], "weightClass": 350}));
    run(&mut s, "font.instance.setWidthClass", json!({"instances": ["Regular"], "widthClass": 7}));
    let regular = list(&mut s)[0].clone();
    assert_eq!(regular["weightClass"], 350, "values between the standard steps are accepted");
    assert_eq!(regular["widthClass"], 7);
    assert_eq!(regular["axes"], json!({"wght": 400}), "the classes do not move the instance");

    run(&mut s, "font.instance.set", json!({"instances": ["Regular"], "axes": {"wght": 900}}));
    let regular = list(&mut s)[0].clone();
    assert_eq!(regular["axes"], json!({"wght": 900}));
    assert_eq!(regular["weightClass"], 350, "the coordinates do not change the class");

    for (id, params) in [
        ("font.instance.setWeightClass", json!({"instances": ["Regular"], "weightClass": 0})),
        ("font.instance.setWeightClass", json!({"instances": ["Regular"], "weightClass": 1001})),
        ("font.instance.setWeightClass", json!({"instances": ["Regular"]})),
        ("font.instance.setWidthClass", json!({"instances": ["Regular"], "widthClass": 10})),
        ("font.instance.setWidthClass", json!({"instances": ["Regular"], "widthClass": 0})),
    ] {
        refused(&mut s, id, params);
    }
    let regular = list(&mut s)[0].clone();
    assert_eq!((regular["weightClass"].clone(), regular["widthClass"].clone()), (json!(350), json!(7)));
}

#[test]
fn font_instance_style_name_warnings() {
    let mut s = font(vec![instance("Regular")]);
    let warnings = |out: &Value| out["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap().to_string()).collect::<Vec<_>>();

    let out = run(&mut s, "font.instance.set", json!({"instances": ["Regular"], "styleName": "Bold"}));
    assert!(warnings(&out).is_empty());
    // "Test Sans" and the style name together over 20 characters.
    let out = run(&mut s, "font.instance.set", json!({"instances": ["Bold"], "styleName": "Extra Condensed Bold"}));
    let w = warnings(&out);
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].contains("postscriptFontName"), "{w:?}");
    // Outside ASCII: warned, not refused.
    let out = run(&mut s, "font.instance.add", json!({"styleName": "Grâce"}));
    let w = warnings(&out);
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].contains("styleNames"), "{w:?}");
    assert_eq!(names(&mut s), ["Extra Condensed Bold", "Grâce"]);

    // An abbreviated PostScript name silences the length warning.
    let mut long = instance("Regular");
    long.properties.set("postscriptFontName", "TestSans-XCnBd", false);
    let mut s = font(vec![long]);
    let out = run(&mut s, "font.instance.set", json!({"instances": ["Regular"], "styleName": "Extra Condensed Bold"}));
    assert!(warnings(&out).is_empty());
}

#[test]
fn font_instance_set_active_remove_and_reorder() {
    let mut s = font(vec![instance("Regular"), instance("Bold"), instance("Black")]);
    assert_eq!(run(&mut s, "font.instance.setActive", json!({"instances": ["Bold"], "active": false}))["instances"], json!([1]));
    let active: Vec<Value> = run(&mut s, "font.instance.list", json!({"activeOnly": true}))["instances"].as_array().unwrap().clone();
    assert_eq!(active.iter().map(|i| i["name"].clone()).collect::<Vec<_>>(), [json!("Regular"), json!("Black")]);
    assert_eq!(active[1]["index"], 2, "indexes are positions in the whole list");
    refused(&mut s, "font.instance.setActive", json!({"instances": ["Bold"]}));

    assert_eq!(run(&mut s, "font.instance.reorder", json!({"instance": "Black", "toIndex": 0}))["index"], 0);
    assert_eq!(names(&mut s), ["Black", "Regular", "Bold"]);
    refused(&mut s, "font.instance.reorder", json!({"instance": "Black", "toIndex": 3}));
    refused(&mut s, "font.instance.reorder", json!({"instance": "Black"}));

    assert_eq!(run(&mut s, "font.instance.remove", json!({"instances": ["Regular", "Black"]}))["removed"], 2);
    assert_eq!(names(&mut s), ["Bold"]);
    refused(&mut s, "font.instance.remove", json!({"instances": ["Regular"]}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(names(&mut s), ["Black", "Regular", "Bold"], "remove is one undo step");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(names(&mut s), ["Regular", "Bold", "Black"]);
}

#[test]
fn font_instance_set_style_linking() {
    let mut s = font(vec![instance("Regular"), instance("Bold Italic")]);
    let out =
        run(&mut s, "font.instance.setStyleLinking", json!({"instances": ["Bold Italic"], "isBold": true, "isItalic": true, "linkStyle": "Regular"}));
    assert_eq!(out["changed"], json!(["isBold", "isItalic", "linkStyle"]));
    assert_eq!(out["warnings"], json!([]));
    let got = run(&mut s, "font.instance.get", json!({"instance": "Bold Italic"}))["instance"].clone();
    assert_eq!((got["isBold"].clone(), got["isItalic"].clone(), got["linkStyle"].clone()), (json!(true), json!(true), json!("Regular")));
    // An empty base clears the link.
    run(&mut s, "font.instance.setStyleLinking", json!({"instances": ["Bold Italic"], "linkStyle": ""}));
    assert_eq!(list(&mut s)[1]["linkStyle"], Value::Null);
    refused(&mut s, "font.instance.setStyleLinking", json!({"instances": ["Bold Italic"]}));
    refused(&mut s, "font.instance.setStyleLinking", json!({"instances": ["Bold Italic"], "isBold": "yes"}));

    // A base with another family name is linked, with a warning.
    let mut regular = instance("Regular");
    regular.properties.set("familyNames", "Test Sans Display", true);
    let mut s = font(vec![regular, instance("Italic")]);
    let out = run(&mut s, "font.instance.setStyleLinking", json!({"instances": ["Italic"], "isItalic": true, "linkStyle": "Regular"}));
    let w = out["warnings"].as_array().unwrap();
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].as_str().unwrap().contains("Test Sans Display"), "{w:?}");
    assert_eq!(list(&mut s)[1]["linkStyle"], "Regular");
}

/// Each exported font, by instance name, as base64.
fn export(s: &mut Session) -> BTreeMap<String, String> {
    let out = run(s, "file.exportStatic", json!({"timestamp": 0}));
    out["fonts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["instance"].as_str().unwrap().to_string(), f["files"][0]["dataBase64"].as_str().unwrap().to_string()))
        .collect()
}

#[test]
fn reordering_export_entries_changes_no_exported_byte() {
    let mut s = font(Vec::new());
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": [{"x": 60, "y": 0}, {"x": 560, "y": 0}, {"x": 560, "y": 700}, {"x": 60, "y": 700}]}));
    run(&mut s, "font.instance.add", json!({}));
    run(&mut s, "font.instance.add", json!({"styleName": "Bold", "weightClass": 700}));
    run(&mut s, "font.instance.setStyleLinking", json!({"instances": ["Bold"], "isBold": true, "linkStyle": "Regular"}));
    let before = export(&mut s);
    assert_eq!(before.keys().collect::<Vec<_>>(), ["Bold", "Regular"]);

    run(&mut s, "font.instance.reorder", json!({"instance": "Bold", "toIndex": 0}));
    assert_eq!(names(&mut s), ["Bold", "Regular"]);
    assert_eq!(export(&mut s), before, "the order of the entries is organisational only");

    // Linked styles of one family: same name ID 1, and the bold one is marked bold.
    let decode = |b64: &str| base64::engine::general_purpose::STANDARD.decode(b64).unwrap();
    let family = |bytes: &[u8]| {
        let f = skrifa::FontRef::new(bytes).unwrap();
        f.localized_strings(StringId::FAMILY_NAME).english_or_first().unwrap().to_string()
    };
    let regular = decode(&before["Regular"]);
    let bold = decode(&before["Bold"]);
    assert_eq!(family(&regular), "Test Sans");
    assert_eq!(family(&bold), "Test Sans");
    let bold_font = skrifa::FontRef::new(&bold).unwrap();
    assert_eq!(bold_font.os2().unwrap().fs_selection().bits() & 0x21, 0x20, "BOLD set, ITALIC clear");
    let regular_font = skrifa::FontRef::new(&regular).unwrap();
    assert_eq!(regular_font.os2().unwrap().fs_selection().bits() & 0x20, 0);
}
