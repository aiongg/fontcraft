//! The `features` group of font.compile and font.compilePlan (M2.12): feature writers, a
//! replacement feature file, the debug file, the GSUB stub and the refused options.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};

use fontcraft_engine::Session;
use serde_json::{Value, json};
use skrifa::raw::TableProvider;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-compile-features-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A font whose kerning moves `left.k` against `right.k` by -50, with no feature code.
fn kerned() -> Session {
    let mut s = Session::with_document();
    for name in ["left.k", "right.k", "both.k"] {
        run(&mut s, "glyph.newGlyph", json!({ "name": name }));
    }
    run(&mut s, "kerning.setPair", json!({ "left": "left.k", "right": "right.k", "value": -50 }));
    s
}

/// Builds one TTF with the `features` group and answers its feature tags by table.
fn build(s: &mut Session, out: &Path, features: Value) -> Vec<(String, String)> {
    run(s, "font.compile", json!({ "formats": ["ttf"], "destination": out, "timestamp": 0, "features": features }));
    let listed = run(s, "layout.listFeatures", json!({ "font": out }));
    listed["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["table"].as_str().unwrap().to_string(), f["feature"].as_str().unwrap().to_string()))
        .collect()
}

fn has(tags: &[(String, String)], table: &str, feature: &str) -> bool {
    tags.iter().any(|(t, f)| t == table && f == feature)
}

#[test]
fn feature_writers_none_leaves_the_kerning_out() {
    let dir = temp("writers");
    let mut s = kerned();
    let out = dir.join("Kerned.ttf");
    assert!(has(&build(&mut s, &out, json!({})), "GPOS", "kern"), "the kern writer runs by default");
    assert!(has(&build(&mut s, &out, json!({ "writers": ["kern", "..."] })), "GPOS", "kern"));
    assert!(!has(&build(&mut s, &out, json!({ "writers": ["none"] })), "GPOS", "kern"));
    assert!(!has(&build(&mut s, &out, json!({ "writers": "none" })), "GPOS", "kern"));
    assert!(!has(&build(&mut s, &out, json!({ "writers": ["mark", "curs"] })), "GPOS", "kern"), "only the writers listed run");
    // With no layout left, the stub gives an empty GSUB.
    build(&mut s, &out, json!({ "writers": ["none"], "stubGsub": true }));
    let bytes = std::fs::read(&out).unwrap();
    let font = skrifa::FontRef::new(&bytes).unwrap();
    assert!(font.gpos().is_err(), "no GPOS");
    assert_eq!(font.gsub().unwrap().feature_list().unwrap().feature_records().len(), 0, "an empty GSUB");

    let bad = s.execute("font.compile", &json!({ "features": { "writers": ["frobnicate"] } })).unwrap_err().to_string();
    assert!(bad.contains("frobnicate"), "{bad}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_plans_feature_code_equals_the_debug_file_and_replaces_the_sources() {
    let dir = temp("debug");
    let mut s = kerned();
    let fea = dir.join("liga.fea");
    std::fs::write(&fea, "languagesystem DFLT dflt;\nfeature liga {\n    sub left.k right.k by both.k;\n} liga;\n").unwrap();
    let debug = dir.join("debug.fea");
    let out = dir.join("Liga.ttf");
    let features = json!({ "file": fea, "debugFile": debug });

    let plan = run(&mut s, "font.compilePlan", json!({ "formats": ["ttf"], "sections": ["features"], "features": features }));
    let planned = plan["outputs"][0]["features"].as_str().unwrap().to_string();
    assert!(planned.contains("sub left.k right.k by both.k;"), "{planned}");
    assert!(!debug.exists(), "a plan writes nothing");

    let tags = build(&mut s, &out, features);
    assert!(has(&tags, "GSUB", "liga"), "the file's code is compiled: {tags:?}");
    assert!(has(&tags, "GPOS", "kern"), "the writers still run: {tags:?}");
    assert_eq!(std::fs::read_to_string(&debug).unwrap(), planned);

    // Without code the plan says so.
    let mut plain = kerned();
    let plan = run(&mut plain, "font.compilePlan", json!({ "formats": ["ttf"], "sections": ["features"] }));
    assert!(plan["outputs"][0]["features"].is_null(), "{plan}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn per_master_layout_and_expanded_instance_features_are_refused_with_a_reason() {
    let mut s = kerned();
    for (features, word) in [
        (json!({ "variable": false }), "per master"),
        (json!({ "expandToInstances": true }), "instance"),
        (json!({ "namesOverride": false }), "name records"),
        (json!({ "frobnicate": true }), "frobnicate"),
    ] {
        let e = s.execute("font.compile", &json!({ "features": features })).unwrap_err().to_string();
        assert!(e.contains(word), "{features}: {e}");
    }
}
