//! The custom-parameter commands: `font.customParameters.list`, `.describe`, `.add`, `.set`,
//! `.remove`, the catalogue checks behind them, and precedence at export.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use base64::Engine as _;
use fontcraft_engine::model::{Font, Instance};
use fontcraft_engine::{EngineError, Session};
use serde_json::{Value, json};
use skrifa::raw::TableProvider;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

/// The refusal message of a command that must fail, after checking the font did not change.
fn refused(s: &mut Session, id: &str, p: Value) -> String {
    let before = s.font().unwrap().clone();
    let e = match s.execute(id, &p) {
        Err(e) => e,
        Ok(v) => panic!("{id} {p} succeeded: {v}"),
    };
    let after = s.font().unwrap();
    let same =
        Arc::ptr_eq(&before.info, &after.info) && Arc::ptr_eq(&before.masters, &after.masters) && Arc::ptr_eq(&before.instances, &after.instances);
    assert!(same, "{id} {p} changed the font");
    match e {
        EngineError::BadParams { message, .. } => message,
        other => other.to_string(),
    }
}

fn font_params(s: &mut Session) -> Vec<Value> {
    run(s, "font.customParameters.list", json!({"scope": "font"}))["parameters"].as_array().unwrap().clone()
}

/// A font with one master and one export at the master, so every scope has one target.
fn session_with_instance() -> Session {
    let mut font = Font::new("Precedence Sans");
    Arc::make_mut(&mut font.instances).push(Instance { name: "Regular".into(), ..Instance::default() });
    let mut s = Session::new();
    s.add_document(font, None).unwrap();
    s
}

#[test]
fn add_checks_each_kind_and_refuses_bad_values() {
    let mut s = Session::with_document();
    let add = |name: &str, value: Value| json!({"scope": "font", "name": name, "value": value});
    // Numbers, also in text, are stored as numbers.
    assert_eq!(run(&mut s, "font.customParameters.add", add("typoAscender", json!("800")))["value"], json!(800));
    assert_eq!(run(&mut s, "font.customParameters.add", add("Use Typo Metrics", json!(1)))["value"], json!(true));
    assert_eq!(run(&mut s, "font.customParameters.add", add("glyphOrder", json!("A\nB")))["value"], json!(["A", "B"]));
    assert_eq!(run(&mut s, "font.customParameters.add", add("gasp Table", json!({"8": 10, "65535": 15})))["value"]["8"], json!(10));
    assert_eq!(run(&mut s, "font.customParameters.add", add("Name Table Entry", json!("25; Prefix")))["value"], json!("25; Prefix"));
    // Without a value, the kind's empty value.
    assert_eq!(run(&mut s, "font.customParameters.add", json!({"scope": "font", "name": "Write lastChange"}))["value"], json!(false));
    assert_eq!(font_params(&mut s).len(), 6);

    for (name, value, needle) in [
        ("typoAscender", json!("tall"), "a number"),
        ("Use Typo Metrics", json!(["on"]), "a boolean"),
        ("glyphOrder", json!({"A": 1}), "a list"),
        ("gasp Table", json!(8), "a dictionary"),
        ("Name Table Entry", json!("1; Family"), "name ID 1"),
        ("versionString", json!("1.0"), "Version"),
        ("Scale to UPM", json!(8), "16"),
        ("Mystery Parameter", Value::Null, "value"),
    ] {
        let message = refused(&mut s, "font.customParameters.add", add(name, value));
        assert!(message.contains(needle), "{name}: {message}");
    }
    assert_eq!(font_params(&mut s).len(), 6);
}

#[test]
fn name_properties_scopes_aliases_and_unknown_names() {
    let mut s = session_with_instance();
    // A name property lands in the properties, not in the parameter list.
    let out = run(&mut s, "font.customParameters.add", json!({"scope": "font", "name": "designers", "value": "Ann"}));
    assert_eq!(out["property"], json!(true));
    assert_eq!(s.font().unwrap().info.properties.value("designers"), Some("Ann"));
    assert!(font_params(&mut s).is_empty());
    assert!(refused(&mut s, "font.customParameters.add", json!({"scope": "font", "name": "designer", "value": "Bo"})).contains("already"));
    assert!(refused(&mut s, "font.customParameters.add", json!({"scope": "master", "name": "designers", "value": "Bo"})).contains("masters"));
    run(&mut s, "font.customParameters.set", json!({"scope": "font", "name": "designers", "value": {"dflt": "Ann", "DEU": "Anna"}}));
    run(&mut s, "font.customParameters.remove", json!({"scope": "font", "name": "designers"}));
    assert!(s.font().unwrap().info.properties.get("designers").is_none());

    // A parameter whose entry names its scopes is refused elsewhere.
    let message = refused(&mut s, "font.customParameters.add", json!({"scope": "master", "name": "glyphOrder", "value": ["A"]}));
    assert!(message.contains("master"), "{message}");

    // The long spelling is stored under the short one, and both find the row.
    let out = run(&mut s, "font.customParameters.add", json!({"scope": "font", "name": "openTypeOS2TypoAscender", "value": 790}));
    assert_eq!(out["name"], json!("typoAscender"));
    run(&mut s, "font.customParameters.set", json!({"scope": "font", "name": "openTypeOS2TypoAscender", "value": 795}));
    assert_eq!(font_params(&mut s)[0]["value"], json!(795));
    // `description` and `openTypeNameDescription` are one entry of the catalogue.
    for query in ["description", "openTypeNameDescription"] {
        let found = run(&mut s, "font.customParameters.describe", json!({"query": query}));
        let names: Vec<&str> =
            found["parameters"].as_array().unwrap().iter().filter_map(|p| p["name"].as_str()).filter(|n| n.contains("escription")).collect();
        assert_eq!(names, ["descriptions"], "{query}");
    }
    let master_only = run(&mut s, "font.customParameters.describe", json!({"scope": "master"}));
    assert!(master_only["parameters"].as_array().unwrap().iter().all(|p| p["scopes"].as_array().unwrap().contains(&json!("master"))));

    // An unknown name is kept with its value, marked, and survives saving and opening.
    run(&mut s, "font.customParameters.add", json!({"scope": "instance", "name": "Master Name", "value": "Display"}));
    let rows = run(&mut s, "font.customParameters.list", json!({"scope": "instance"}));
    assert_eq!(rows["parameters"][0]["known"], json!(false));
    let dir = std::env::temp_dir().join(format!("fontcraft-custom-params-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("unknown.fontcraft").to_string_lossy().into_owned();
    run(&mut s, "file.saveAs", json!({"path": path}));
    let mut other = Session::new();
    run(&mut other, "file.open", json!({"path": path}));
    let rows = run(&mut other, "font.customParameters.list", json!({"scope": "instance"}));
    assert_eq!(rows["parameters"][0]["name"], json!("Master Name"));
    assert_eq!(rows["parameters"][0]["value"], json!("Display"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn duplicates_index_checkbox_and_undo() {
    let mut s = Session::with_document();
    let add = |name: &str, value: Value| json!({"scope": "font", "name": name, "value": value});
    run(&mut s, "font.customParameters.add", add("typoAscender", json!(800)));
    run(&mut s, "font.customParameters.add", add("typoAscender", json!(810)));
    let rows = font_params(&mut s);
    assert_eq!((rows[0]["duplicate"].clone(), rows[1]["duplicate"].clone()), (json!(false), json!(true)));
    assert_eq!(rows[1]["index"], json!(1));
    // Two rows of one name: set and remove need `index`.
    assert!(refused(&mut s, "font.customParameters.set", json!({"scope": "font", "name": "typoAscender", "value": 1})).contains("index"));
    run(&mut s, "font.customParameters.set", json!({"scope": "font", "name": "typoAscender", "index": 1, "enabled": false}));
    let rows = font_params(&mut s);
    assert_eq!(rows[1]["enabled"], json!(false));
    assert!(rows[1].get("duplicate").is_none(), "a disabled row is not counted");
    run(&mut s, "font.customParameters.remove", json!({"scope": "font", "name": "typoAscender", "index": 0}));
    assert_eq!(font_params(&mut s)[0]["value"], json!(810));

    // Each command is one undo step.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(font_params(&mut s).len(), 2);
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert!(font_params(&mut s).is_empty());
}

#[test]
fn several_targets_change_in_one_step() {
    let mut font = Font::new("Two Exports");
    for name in ["Regular", "Bold"] {
        Arc::make_mut(&mut font.instances).push(Instance { name: name.into(), ..Instance::default() });
    }
    let mut s = Session::new();
    s.add_document(font, None).unwrap();
    // With two exports the target must be named.
    assert!(refused(&mut s, "font.customParameters.add", json!({"scope": "instance", "name": "typoAscender", "value": 800})).contains("targets"));
    run(
        &mut s,
        "font.customParameters.add",
        json!({"scope": "instance", "targets": ["Regular", {"index": 1}], "name": "typoAscender", "value": 800}),
    );
    assert!(s.font().unwrap().instances.iter().all(|m| m.params.get("typoAscender").is_some()));
    let bold = run(&mut s, "font.customParameters.list", json!({"scope": "instance", "target": "Bold"}));
    assert_eq!(bold["target"]["index"], json!(1));
    assert_eq!(bold["parameters"][0]["value"], json!(800));
    run(&mut s, "edit.undo", json!({}));
    assert!(s.font().unwrap().instances.iter().all(|m| m.params.is_empty()));
}

#[test]
fn hostile_values_are_refused() {
    let mut s = Session::with_document();
    let huge = "x".repeat(5 << 20);
    assert!(refused(&mut s, "font.customParameters.add", json!({"scope": "font", "name": "note", "value": huge})).contains("more than"));
    let many: Vec<Value> = (0..100_000).map(|i| json!(format!("g{i}"))).collect();
    assert!(refused(&mut s, "font.customParameters.add", json!({"scope": "font", "name": "glyphOrder", "value": many})).contains("more than"));
    assert!(refused(&mut s, "font.customParameters.add", json!({"scope": "font", "name": "Mystery", "value": [1, null]})).contains("null"));
}

/// sTypoAscender of the one font `file.exportStatic` builds.
fn typo_ascender(s: &mut Session) -> i16 {
    let out = run(s, "file.exportStatic", json!({"timestamp": 1_700_000_000}));
    let bytes = base64::engine::general_purpose::STANDARD.decode(out["dataBase64"].as_str().unwrap()).unwrap();
    skrifa::FontRef::new(&bytes).unwrap().os2().unwrap().s_typo_ascender()
}

#[test]
fn precedence_export_over_master_over_font_and_the_checkbox() {
    let mut s = session_with_instance();
    for (scope, value) in [("font", 700), ("master", 750), ("instance", 790)] {
        run(&mut s, "font.customParameters.add", json!({"scope": scope, "name": "typoAscender", "value": value}));
    }
    assert_eq!(typo_ascender(&mut s), 790);
    // A disabled row is kept but not used: the master's value counts.
    run(&mut s, "font.customParameters.set", json!({"scope": "instance", "name": "typoAscender", "enabled": false}));
    assert_eq!(typo_ascender(&mut s), 750);
    run(&mut s, "font.customParameters.remove", json!({"scope": "master", "name": "typoAscender"}));
    assert_eq!(typo_ascender(&mut s), 700);
}
