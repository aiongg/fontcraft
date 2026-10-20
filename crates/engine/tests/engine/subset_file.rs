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
    let flat = s.execute("font.subset", &json!({ "font": ttf, "keep": {"text": "A"}, "desubroutinize": true, "output": dir.join("flat.ttf") }));
    assert!(flat.is_ok(), "desubroutinize leaves a TrueType font as it is: {flat:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn font_subset_takes_the_layout_options() {
    let dir = std::env::temp_dir().join(format!("fontcraft-subset-layout-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::with_document();
    let nodes = json!([{"x": 60, "y": 0}, {"x": 560, "y": 0}, {"x": 560, "y": 700}, {"x": 60, "y": 700}]);
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": nodes}));
    let ttf = dir.join("Font.ttf");
    run(&mut s, "file.exportStatic", json!({ "destination": ttf, "timestamp": 0 }));
    for (i, extra) in [
        json!({"layoutFeatures": "*", "layoutScripts": ["latn", "DFLT.dflt"], "layoutClosure": false}),
        json!({"layoutFeatures": {"add": ["smcp"], "remove": ["kern"]}}),
        json!({"layoutFeatures": [], "layoutScripts": "*"}),
    ]
    .into_iter()
    .enumerate()
    {
        let mut p = json!({ "font": ttf, "keep": {"text": "A"}, "output": dir.join(format!("{i}.ttf")) });
        p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let r = run(&mut s, "font.subset", p);
        assert_eq!(r["retained"], json!([".notdef", "A"]));
    }
    let bad =
        s.execute("font.subset", &json!({ "font": ttf, "keep": {"text": "A"}, "layoutFeatures": {"keep": ["kern"]}, "output": dir.join("bad.ttf") }));
    assert!(bad.is_err(), "an edit takes add and remove only");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn font_subset_writes_the_order_and_container_asked_for() {
    let dir = std::env::temp_dir().join(format!("fontcraft-subset-order-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::with_document();
    let nodes = json!([{"x": 60, "y": 0}, {"x": 560, "y": 0}, {"x": 560, "y": 700}, {"x": 60, "y": 700}]);
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": nodes}));
    let exported = dir.join("Exported.ttf");
    run(&mut s, "file.exportStatic", json!({ "destination": exported, "timestamp": 0 }));
    // The same font with its tables stored in reverse order.
    let mut font = fontcraft_tables::read(&std::fs::read(&exported).unwrap(), 0).unwrap();
    font.order = font.tags().into_iter().rev().collect();
    let ttf = dir.join("Font.ttf");
    std::fs::write(&ttf, fontcraft_tables::write(&font, &fontcraft_tables::WriteOptions { order: fontcraft_tables::TableOrder::Keep }).unwrap())
        .unwrap();

    let order_of = |path: &std::path::Path| fontcraft_tables::read(&std::fs::read(path).unwrap(), 0).unwrap().order;
    let kept = run(&mut s, "font.subset", json!({ "font": ttf, "keep": {"text": "A"}, "output": dir.join("kept.ttf") }));
    let kept = order_of(&PathBuf::from(kept["output"].as_str().unwrap()));
    let sorted = run(&mut s, "font.subset", json!({ "font": ttf, "keep": {"text": "A"}, "canonicalOrder": true, "output": dir.join("sorted.ttf") }));
    let sorted = order_of(&PathBuf::from(sorted["output"].as_str().unwrap()));
    assert_eq!(sorted.first(), Some(b"head"), "the recommended order starts with head: {sorted:?}");
    assert_ne!(kept, sorted, "the input's order by default");

    let r = run(&mut s, "font.subset", json!({ "font": ttf, "keep": {"text": "A"}, "container": "woff2" }));
    let woff2 = PathBuf::from(r["output"].as_str().unwrap());
    assert_eq!(woff2, dir.join("Font.subset.woff2"));
    assert_eq!(&std::fs::read(&woff2).unwrap()[..4], b"wOF2");
    // A WOFF2 input gives a WOFF2 output unless told otherwise.
    let r = run(&mut s, "font.subset", json!({ "font": woff2, "keep": {"text": "A"}, "outputDir": dir.join("again") }));
    assert!(r["output"].as_str().unwrap().ends_with("Font.subset.subset.woff2"), "{r}");
    let r = run(&mut s, "font.subset", json!({ "font": woff2, "keep": {"text": "A"}, "container": "plain", "outputDir": dir.join("plain") }));
    assert!(r["output"].as_str().unwrap().ends_with(".ttf"), "{r}");
    let zopfli = s.execute("font.subset", &json!({ "font": ttf, "keep": {"text": "A"}, "container": "woff", "zopfli": true }));
    assert!(zopfli.is_err(), "zopfli is refused, not ignored, where it would apply");

    let r =
        run(&mut s, "font.subset", json!({ "font": ttf, "keep": {"text": "A"}, "dropTables": {"add": ["gasp"]}, "output": dir.join("nogasp.ttf") }));
    assert!(r["dropped"].as_array().unwrap().iter().any(|d| d["tag"] == "gasp") || font.get(*b"gasp").is_none(), "{r}");
    let _ = std::fs::remove_dir_all(&dir);
}
