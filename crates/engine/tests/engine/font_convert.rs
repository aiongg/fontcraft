//! `font.convert` on compiled font files.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::PathBuf;

use fontcraft_engine::Session;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

#[test]
fn font_convert_changes_the_outline_flavour_and_writes_nothing_on_failure() {
    let dir = std::env::temp_dir().join(format!("fontcraft-convert-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::with_document();
    let square = json!([{"x": 60, "y": 0}, {"x": 560, "y": 0}, {"x": 560, "y": 700}, {"x": 60, "y": 700}]);
    let curve = json!([
        {"x": 100, "y": 0}, {"x": 300, "y": 0, "type": "offcurve"}, {"x": 400, "y": 150, "type": "offcurve"}, {"x": 400, "y": 350, "type": "curve"},
        {"x": 400, "y": 700}, {"x": 100, "y": 700}
    ]);
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": square}));
    run(&mut s, "path.addPath", json!({"glyph": "B", "nodes": curve}));
    let ttf = dir.join("Font.ttf");
    run(&mut s, "file.exportStatic", json!({ "destination": ttf, "timestamp": 0 }));
    let original = std::fs::read(&ttf).unwrap();

    // TrueType to CFF, beside the input with the new extension.
    let r = run(&mut s, "font.convert", json!({ "input": ttf, "to": "otf" }));
    let otf = PathBuf::from(r["results"][0]["output"].as_str().unwrap());
    assert_eq!(otf, dir.join("Font.otf"));
    assert_eq!(std::fs::read(&ttf).unwrap(), original, "the input is never changed");
    let font = fontcraft_tables::read(&std::fs::read(&otf).unwrap(), 0).unwrap();
    assert!(font.get(*b"CFF ").is_some() && font.get(*b"glyf").is_none());
    assert_eq!(r["results"][0]["glyphs"], json!(font.get(*b"maxp").map(|m| u16::from_be_bytes([m[4], m[5]])).unwrap()));

    // CFF back to TrueType in another folder; CFF to CFF2 wrapped in WOFF2.
    let out_dir = dir.join("out");
    let r = run(&mut s, "font.convert", json!({ "inputs": [otf], "to": "ttf", "outputDir": out_dir, "curves": {"maxError": 0.001} }));
    let back = PathBuf::from(r["results"][0]["output"].as_str().unwrap());
    assert_eq!(back, out_dir.join("Font.ttf"));
    let font = fontcraft_tables::read(&std::fs::read(&back).unwrap(), 0).unwrap();
    assert!(font.get(*b"glyf").is_some() && font.get(*b"CFF ").is_none());
    let r = run(&mut s, "font.convert", json!({ "input": otf, "to": "otf-cff2", "container": "woff2", "output": dir.join("Font.cff2.woff2") }));
    let woff2 = std::fs::read(r["results"][0]["output"].as_str().unwrap()).unwrap();
    assert_eq!(&woff2[..4], b"wOF2");
    let font = fontcraft_tables::read(&woff2, 0).unwrap();
    assert!(font.get(*b"CFF2").is_some());

    // A run with one bad input writes none of its outputs.
    let junk = dir.join("notes.txt");
    std::fs::write(&junk, "not a font").unwrap();
    let failed = s.execute("font.convert", &json!({ "inputs": [ttf, junk], "to": "cff", "outputDir": dir.join("none") }));
    assert!(failed.is_err());
    assert!(!dir.join("none").exists(), "a failed conversion writes nothing");
    assert!(s.execute("font.convert", &json!({ "input": ttf, "to": "ufo" })).is_err(), "sources are written by file.saveAs");
    assert!(s.execute("font.convert", &json!({ "input": ttf, "to": "otf", "glyphs": ["A"] })).is_err(), "options not built yet are refused");
    let _ = std::fs::remove_dir_all(&dir);
}
