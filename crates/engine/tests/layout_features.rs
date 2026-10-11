//! layout.listFeatures and layout.decompileFeatures on a font exported from a document.

use std::path::PathBuf;

use fontcraft_engine::Session;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-layout-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A font whose GPOS kerns `left.k` against `right.k` by -50.
fn kerned_font(dir: &std::path::Path) -> (Session, PathBuf) {
    let mut s = Session::with_document();
    run(&mut s, "glyph.newGlyph", json!({ "name": "left.k" }));
    run(&mut s, "glyph.newGlyph", json!({ "name": "right.k" }));
    run(&mut s, "kerning.setPair", json!({ "left": "left.k", "right": "right.k", "value": -50 }));
    let ttf = dir.join("Kerned.ttf");
    run(&mut s, "file.exportStatic", json!({ "destination": ttf, "timestamp": 0 }));
    (s, ttf)
}

#[test]
fn layout_list_features_and_decompile_features_read_an_exported_font() {
    let dir = temp("kern");
    let (mut s, ttf) = kerned_font(&dir);

    let listed = run(&mut s, "layout.listFeatures", json!({ "font": ttf, "lookups": true }));
    let kern = listed["features"].as_array().unwrap().iter().find(|f| f["feature"] == "kern").expect("a kern feature");
    assert_eq!(kern["table"], "GPOS");
    assert_eq!(kern["systems"][0]["lookups"][0]["type"], 2, "a pair lookup");
    let by_script = run(&mut s, "layout.listFeatures", json!({ "font": ttf, "by": "script", "table": "GPOS" }));
    assert!(by_script["systems"].as_array().unwrap().iter().all(|sys| sys["table"] == "GPOS"));

    let d = run(&mut s, "layout.decompileFeatures", json!({ "font": ttf }));
    let text = d["text"].as_str().unwrap();
    assert!(text.contains("pos left.k right.k -50;"), "{text}");
    assert!(text.contains("feature kern {"), "{text}");
    assert!(d["unsupported"].is_array());

    // Written to a file beside the font, never over it.
    let written = run(&mut s, "layout.decompileFeatures", json!({ "font": ttf, "outputDir": dir }));
    let out = PathBuf::from(written["output"].as_str().unwrap());
    assert_eq!(out, dir.join("Kerned.fea"));
    assert_eq!(std::fs::read_to_string(&out).unwrap(), text, "the same text as returned");

    // Bad parameters and inputs fail with a message and write nothing.
    let fea = dir.join("Kerned.fea");
    assert!(s.execute("layout.decompileFeatures", &json!({ "font": ttf, "tables": ["kern"] })).is_err());
    assert!(s.execute("layout.decompileFeatures", &json!({ "font": ttf, "names": "source" })).is_err(), "source names need a GOADB");
    assert!(s.execute("layout.decompileFeatures", &json!({ "font": ttf, "inPlace": true })).is_err());
    assert!(s.execute("layout.listFeatures", &json!({ "font": fea })).is_err(), "feature code is not a compiled font");
    assert!(s.execute("layout.listFeatures", &json!({ "font": ttf, "by": "lookup" })).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}
