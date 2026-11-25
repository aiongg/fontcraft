//! The outline, CFF, glyph-set and output options of font.compile (M2.13): the glyph subset, a
//! replaced .notdef, tables left out, post format 3, raw table files, the version bump and the
//! refused options.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};

use fontcraft_engine::Session;
use fontcraft_engine::model::{Glyph, Layer};
use serde_json::{Value, json};
use skrifa::raw::TableProvider;
use skrifa::raw::types::Tag;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-compile-outlines-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `A`, `acutecomb`, `B`, and `Aacute` made of the components `A` and `acutecomb`.
fn composites() -> Session {
    let mut s = Session::with_document();
    // A new font holds `.notdef`, `space` and the basic Latin letters; start from none.
    s.edit("fixture", |f| {
        f.glyphs.retain(|_| false);
        Ok(())
    })
    .unwrap();
    for name in ["A", "acutecomb", "B", "Aacute"] {
        run(&mut s, "glyph.newGlyph", json!({ "name": name }));
    }
    for base in ["A", "acutecomb"] {
        run(&mut s, "glyph.addComponent", json!({ "glyph": "Aacute", "component": base }));
    }
    s
}

/// Builds one TTF at `out` with the extra parameters `extra` and returns its bytes.
fn build(s: &mut Session, out: &Path, extra: Value) -> Vec<u8> {
    let mut p = json!({ "formats": ["ttf"], "destination": out, "timestamp": 0 });
    if let (Value::Object(m), Value::Object(more)) = (&mut p, extra) {
        m.extend(more);
    }
    run(s, "font.compile", p);
    std::fs::read(out).unwrap()
}

#[test]
fn glyphs_builds_the_named_glyphs_with_notdef_and_their_components() {
    let mut s = composites();
    let all = run(&mut s, "font.compile", json!({ "formats": ["ttf"] }));
    assert_eq!(all["fonts"][0]["glyphs"], 5, ".notdef and the four glyphs: {all}");
    let some = run(&mut s, "font.compile", json!({ "formats": ["ttf"], "glyphs": ["Aacute"] }));
    assert_eq!(some["fonts"][0]["glyphs"], 4, "B is left out; A and acutecomb stay as Aacute's components: {some}");
    let plan = run(&mut s, "font.compilePlan", json!({ "formats": ["ttf"], "glyphs": ["Aacute"], "sections": ["glyphOrder"] }));
    assert!(!plan["glyphOrder"].as_array().unwrap().contains(&json!("B")), "{plan}");
    let e = s.execute("font.compile", &json!({ "glyphs": ["Nothing"] })).unwrap_err().to_string();
    assert!(e.contains("Nothing"), "{e}");
}

#[test]
fn notdef_replace_draws_a_new_notdef() {
    let dir = temp("notdef");
    let mut s = composites();
    s.edit("fixture", |f| {
        let mut g = Glyph::new(".notdef");
        g.layers.push(Layer::new("m01", 123.0));
        f.glyphs.insert(0, g);
        Ok(())
    })
    .unwrap();
    let out = dir.join("Notdef.ttf");
    let advance = |bytes: &[u8]| {
        let font = skrifa::FontRef::new(bytes).unwrap();
        let hmtx = font.table_data(Tag::new(b"hmtx")).unwrap();
        u16::from_be_bytes([hmtx.as_bytes()[0], hmtx.as_bytes()[1]])
    };
    assert_eq!(advance(&build(&mut s, &out, json!({}))), 123, "the source's .notdef by default");
    assert_eq!(advance(&build(&mut s, &out, json!({ "notdef": "allow-missing" }))), 123);
    assert_ne!(advance(&build(&mut s, &out, json!({ "notdef": "replace" }))), 123, "a drawn .notdef");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn exclude_tables_leaves_the_tables_out() {
    let dir = temp("exclude");
    let mut s = composites();
    let out = dir.join("Exclude.ttf");
    let bytes = build(&mut s, &out, json!({ "stubDsig": true }));
    assert!(skrifa::FontRef::new(&bytes).unwrap().table_data(Tag::new(b"DSIG")).is_some());
    let bytes = build(&mut s, &out, json!({ "stubDsig": true, "excludeTables": ["DSIG", "kern"] }));
    let font = skrifa::FontRef::new(&bytes).unwrap();
    assert!(font.table_data(Tag::new(b"DSIG")).is_none(), "DSIG is left out");
    assert!(font.head().is_ok() && font.maxp().is_ok(), "the rest of the font is intact");
    let e = s.execute("font.compile", &json!({ "excludeTables": ["head"] })).unwrap_err().to_string();
    assert!(e.contains("head"), "{e}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn post_format_3_drops_the_glyph_names() {
    let dir = temp("post");
    let mut s = composites();
    let out = dir.join("Post.ttf");
    let version = |bytes: &[u8]| skrifa::FontRef::new(bytes).unwrap().post().unwrap().version();
    assert_eq!(version(&build(&mut s, &out, json!({}))), skrifa::raw::types::Version16Dot16::VERSION_2_0);
    assert_eq!(version(&build(&mut s, &out, json!({ "postFormat": 2 }))), skrifa::raw::types::Version16Dot16::VERSION_2_0);
    let bytes = build(&mut s, &out, json!({ "postFormat": 3 }));
    assert_eq!(version(&bytes), skrifa::raw::types::Version16Dot16::VERSION_3_0);
    assert_eq!(skrifa::FontRef::new(&bytes).unwrap().table_data(Tag::new(b"post")).unwrap().len(), 32, "the header only");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn tables_writes_raw_table_files_and_bump_version_raises_the_revision() {
    let dir = temp("raw");
    let mut s = composites();
    let r = run(
        &mut s,
        "font.compile",
        json!({ "formats": ["ttf"], "destination": dir, "timestamp": 0, "tables": ["head", "OS/2"], "bumpVersion": true }),
    );
    let tables = r["tables"].as_array().unwrap();
    assert_eq!(tables.len(), 2, "{r}");
    let font_file = r["fonts"][0]["files"][0]["path"].as_str().unwrap();
    let bytes = std::fs::read(font_file).unwrap();
    let font = skrifa::FontRef::new(&bytes).unwrap();
    let head_file = tables.iter().find(|t| t["table"] == "head").unwrap()["path"].as_str().unwrap();
    assert!(head_file.ends_with(".head"), "{head_file}");
    assert_eq!(std::fs::read(head_file).unwrap(), font.table_data(Tag::new(b"head")).unwrap().as_bytes());
    let revision = font.head().unwrap().font_revision().to_f64();
    assert!((revision - 1.005).abs() < 1e-3, "version 1.000 raised by 0.005: {revision}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn options_without_a_counterpart_are_refused_with_a_reason() {
    let mut s = composites();
    for (params, word) in [
        (json!({ "debugLookups": true }), "lookup names"),
        (json!({ "masterFinder": "{stem}.ttf" }), "compiled master"),
        (json!({ "emitDebug": true }), "intermediate"),
        (json!({ "cff": { "optimize": 1 }, "subroutinize": true }), "subroutines"),
        (json!({ "checkCompatibility": "fix" }), "repairs"),
    ] {
        let e = s.execute("font.compile", &params).unwrap_err().to_string();
        assert!(e.contains(word), "{params}: {e}");
    }
    // Open corners are erased as asked or not at all.
    for erase in [json!(true), json!(false), json!(null)] {
        run(&mut s, "font.compile", json!({ "formats": ["ttf"], "outlines": { "eraseOpenCorners": erase } }));
    }
}

/// A trial variable font: its setting removes a glyph and moves the outlines, the request writes
/// `post` 3 without `STAT`, a stub `DSIG` and a raw table, and the setting's fonts pack into one
/// collection.
#[test]
fn variable_fonts_take_the_options_of_static_ones() {
    let dir = temp("variable-options");
    let setting = "instances = (\n{\ncustomParameters = (\n{\nname = \"Remove Glyphs\";\nvalue = (\nsecond\n);\n},\n{\nname = Filter;\nvalue = \"Transformations;OffsetX:40\";\n}\n);\nname = Trial;\ntype = variable;\n},";
    let text = crate::export::TWO_MASTERS.replacen("instances = (", setting, 1);
    let path = dir.join("TestSans.glyphs");
    std::fs::write(&path, text).unwrap();
    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": path }));
    s.edit("Add", |font| font.add_glyph("second", vec![0xE000], 500.0).map(|_| ())).unwrap();
    let options = json!({
        "formats": ["variable", "variable-cff2"], "destination": dir.join("out"), "timestamp": 0, "postFormat": 3,
        "excludeTables": ["STAT"], "tables": ["fvar"], "stubDsig": true, "containers": ["plain", "woff2"],
    });
    let r = run(&mut s, "font.compile", options);
    assert_eq!(r["fonts"].as_array().unwrap().len(), 2, "{r}");
    assert_eq!(r["tables"].as_array().unwrap().len(), 2, "a raw fvar per font: {r}");
    let bytes = std::fs::read(r["fonts"][0]["files"][0]["path"].as_str().unwrap()).unwrap();
    let font = skrifa::FontRef::new(&bytes).unwrap();
    assert_eq!(font.post().unwrap().version(), skrifa::raw::types::Version16Dot16::VERSION_3_0);
    assert!(font.table_data(Tag::new(b"DSIG")).is_some() && font.table_data(Tag::new(b"STAT")).is_none());
    assert!(font.table_data(Tag::new(b"fvar")).is_some());
    assert!(skrifa::MetadataProvider::charmap(&font).map(0xE000u32).is_none(), "the setting removes the glyph");
    let a = skrifa::MetadataProvider::charmap(&font).map('A').unwrap();
    let bounds = skrifa::MetadataProvider::glyph_metrics(&font, skrifa::instance::Size::unscaled(), skrifa::instance::LocationRef::default())
        .bounds(a)
        .unwrap();
    assert_eq!(bounds.x_min, 140.0, "the setting's filter moves A by 40");

    let ttc = dir.join("Trial.ttc");
    let r = run(&mut s, "font.compile", json!({ "formats": ["variable"], "destination": ttc, "timestamp": 0 }));
    assert_eq!(r["collections"][0]["fonts"], 1, "{r}");
    assert_eq!(&std::fs::read(&ttc).unwrap()[..4], b"ttcf");
    let e = s.execute("font.compile", &json!({ "formats": ["variable"], "excludeTables": ["fvar"] })).unwrap_err().to_string();
    assert!(e.contains("fvar"), "{e}");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// `outlines.filters` runs a filter on static fonts, and `propagateAnchors` is taken either way.
#[test]
fn filters_run_inside_a_compile() {
    let dir = temp("filters");
    let mut s = composites();
    let nodes = json!([{"x": 60, "y": 0}, {"x": 560, "y": 0}, {"x": 560, "y": 700}, {"x": 60, "y": 700}]);
    run(&mut s, "path.addPath", json!({"glyph": "B", "nodes": nodes}));
    let x_max = |bytes: &[u8]| {
        let font = skrifa::FontRef::new(bytes).unwrap();
        font.head().unwrap().x_max()
    };
    let plain = build(&mut s, &dir.join("plain.ttf"), json!({}));
    let moved =
        build(&mut s, &dir.join("moved.ttf"), json!({ "outlines": { "filters": ["Transformations;OffsetX:2000"], "propagateAnchors": false } }));
    assert_eq!(x_max(&moved), x_max(&plain) + 2000);
    assert!(s.execute("font.compile", &json!({ "outlines": { "filters": [3] } })).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}
