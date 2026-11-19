//! A UFO source's warnings (M1.11, `source.ufoUnsupportedFeatureWarning`) and saving a UFOZ
//! (`ufoz`): an edit that creates what a UFO cannot hold runs and warns; a UFOZ saves, reopens
//! and saves again to the same bytes.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use fontcraft_engine::Session;
use fontcraft_model::{Font, Node, NodeKind, Path, Shape, SourceFormat};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn lines(closed: bool, points: &[(f64, f64)]) -> Shape {
    Shape::Path(Path::new(closed, points.iter().map(|&(x, y)| Node::new(x, y, NodeKind::Line)).collect()))
}

/// A font with the corner glyph `_corner.one` and a rectangle in `I`, read from `format`.
fn font(format: SourceFormat) -> Font {
    let mut font = Font::new("Warnings");
    font.add_glyph("_corner.one", Vec::new(), 600.0).unwrap();
    font.glyph_mut("_corner.one").unwrap().layer_mut("m01").unwrap().shapes =
        vec![lines(false, &[(0.0, 50.0), (-50.0, 50.0), (-50.0, 0.0), (25.0, 0.0)])];
    font.glyph_mut("I").unwrap().layer_mut("m01").unwrap().shapes = vec![lines(true, &[(100.0, 0.0), (400.0, 0.0), (400.0, 500.0), (100.0, 500.0)])];
    font.source.format = format;
    if format != SourceFormat::Native {
        font.source.ufo_version = Some(3);
    }
    font
}

fn corner_warnings(r: &Value) -> Vec<String> {
    let list = r.get("warnings").and_then(Value::as_array).cloned().unwrap_or_default();
    list.iter().filter_map(Value::as_str).filter(|w| w.contains("corner components")).map(str::to_string).collect()
}

/// On a UFO the corner is added and the reply warns; undo warns of nothing, and the same edit
/// on a native document does not warn.
#[test]
fn a_corner_component_in_a_ufo_warns_without_blocking() {
    let add = json!({"glyph": "I", "points": [[0, 0]], "corner": "_corner.one"});
    let mut s = Session::new();
    s.add_document(font(SourceFormat::Ufo), None).unwrap();
    let r = run(&mut s, "cornerComponent.add", add.clone());
    assert_eq!(corner_warnings(&r).len(), 1, "{r}");
    assert_eq!(s.font().unwrap().glyph("I").unwrap().layer("m01").unwrap().info.hints.len(), 1, "the corner is added");
    let r = run(&mut s, "edit.undo", json!({}));
    assert!(corner_warnings(&r).is_empty(), "{r}");

    let mut s = Session::new();
    s.add_document(font(SourceFormat::Native), None).unwrap();
    let r = run(&mut s, "cornerComponent.add", add);
    assert!(corner_warnings(&r).is_empty(), "{r}");
}

/// Save As .ufoz writes one archive; the document reopens as a UFOZ, and saving it again gives
/// the same bytes.
#[test]
fn a_ufoz_saves_and_reopens_identical() {
    let dir = fontcraft_testkit::temp_dir("ufoz-save");
    let (first, second) = (dir.join("Font.ufoz"), dir.join("Again.ufoz"));
    let mut s = Session::new();
    s.add_document(font(SourceFormat::Ufo), None).unwrap();
    let r = run(&mut s, "file.saveAs", json!({"path": first.to_string_lossy()}));
    assert_eq!(r["format"], "ufoz", "{r}");
    assert_eq!(r["filesWritten"], 1, "{r}");
    let bytes = std::fs::read(&first).unwrap();
    let files = fontcraft_io_ufo::zip::read_archive(&bytes).unwrap();
    assert!(files.keys().all(|k| k.starts_with("Font.ufo/")), "{:?}", files.keys().collect::<Vec<_>>());
    assert!(files.contains_key("Font.ufo/metainfo.plist"));

    let mut s = Session::new();
    let r = run(&mut s, "file.open", json!({"path": first.to_string_lossy(), "keepGlyphNames": true}));
    assert_eq!(r["format"], "ufoz", "{r}");
    assert_eq!(r["viewOnly"], false, "{r}");
    let reopened = s.font().unwrap();
    assert_eq!(reopened.glyph("I").unwrap().layers.first().map(|l| l.shapes.len()), Some(1));
    // Saving in place keeps the format; Save As to another name writes the same UFO.
    let r = run(&mut s, "file.save", json!({}));
    assert_eq!(r["format"], "ufoz", "{r}");
    run(&mut s, "file.saveAs", json!({"path": second.to_string_lossy()}));
    let again = fontcraft_io_ufo::zip::read_ufoz(&std::fs::read(&second).unwrap()).unwrap();
    assert_eq!(again, fontcraft_io_ufo::zip::read_ufoz(&bytes).unwrap(), "the reopened UFOZ writes the same UFO");
}
