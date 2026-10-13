//! `font.subset` on compiled font files.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::PathBuf;

use fontcraft_engine::Session;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

#[test]
fn font_subset_keeps_the_selection_and_reports_the_map() {
    let dir = std::env::temp_dir().join(format!("fontcraft-subset-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::with_document();
    let nodes = json!([{"x": 60, "y": 0}, {"x": 560, "y": 0}, {"x": 560, "y": 700}, {"x": 60, "y": 700}]);
    for glyph in ["A", "B"] {
        run(&mut s, "path.addPath", json!({"glyph": glyph, "nodes": nodes}));
    }
    let ttf = dir.join("Font.ttf");
    run(&mut s, "file.exportStatic", json!({ "destination": ttf, "timestamp": 0 }));
    let original = std::fs::read(&ttf).unwrap();

    let r = run(&mut s, "font.subset", json!({ "font": ttf, "keep": {"text": "B"} }));
    let out = PathBuf::from(r["output"].as_str().unwrap());
    assert_eq!(out, dir.join("Font.subset.ttf"));
    assert_eq!(r["retained"], json!([".notdef", "B"]));
    assert_eq!(r["glyphMap"].as_array().unwrap().len(), 2);
    assert_eq!(std::fs::read(&ttf).unwrap(), original, "the input is never changed");
    let font = fontcraft_tables::read(&std::fs::read(&out).unwrap(), 0).unwrap();
    assert_eq!(u16::from_be_bytes([font.get(*b"maxp").unwrap()[4], font.get(*b"maxp").unwrap()[5]]), 2);

    let missing = s.execute("font.subset", &json!({ "font": ttf, "keep": {"glyphs": ["nosuch"]}, "output": dir.join("x.ttf") }));
    assert!(missing.is_err(), "a missing glyph is refused by default");
    assert!(!dir.join("x.ttf").exists(), "a failed command writes nothing");
    assert!(s.execute("font.subset", &json!({ "font": ttf, "keep": {"text": "A"}, "desubroutinize": true })).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
