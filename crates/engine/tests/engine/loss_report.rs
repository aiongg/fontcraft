//! The loss report before a save (M1.20, `roundtrip.lossReport`), `strict`, the members of a font
//! collection (`ttc.open`) and the view-only mark of a compiled font (`open.compiledIsViewOnly`).
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use fontcraft_engine::Session;
use fontcraft_model::{Font, Node, NodeKind, Path, Shape};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn lines(closed: bool, points: &[(f64, f64)]) -> Shape {
    Shape::Path(Path::new(closed, points.iter().map(|&(x, y)| Node::new(x, y, NodeKind::Line)).collect()))
}

/// A new document whose `I` has a corner component.
fn with_corner() -> Session {
    let mut font = Font::new("Losses");
    font.add_glyph("_corner.one", Vec::new(), 600.0).unwrap();
    font.glyph_mut("_corner.one").unwrap().layer_mut("m01").unwrap().shapes =
        vec![lines(false, &[(0.0, 50.0), (-50.0, 50.0), (-50.0, 0.0), (25.0, 0.0)])];
    font.glyph_mut("I").unwrap().layer_mut("m01").unwrap().shapes = vec![lines(true, &[(100.0, 0.0), (400.0, 0.0), (400.0, 500.0), (100.0, 500.0)])];
    let mut s = Session::new();
    s.add_document(font, None).unwrap();
    run(&mut s, "cornerComponent.add", json!({"glyph": "I", "points": [[0, 0]], "corner": "_corner.one"}));
    s
}

/// The report names the kind and its glyphs, the native format loses nothing, and a document already saved as a UFO is
/// not asked about the same data again.
#[test]
fn the_report_names_what_a_ufo_keeps_to_itself_and_only_once() {
    let dir = fontcraft_testkit::temp_dir("loss-report");
    let mut s = with_corner();
    let r = run(&mut s, "file.lossReport", json!({"format": "ufo"}));
    let corner = |list: &Value| list.as_array().unwrap().iter().find(|e| e["what"].as_str().unwrap().starts_with("corner components")).cloned();
    assert_eq!(corner(&r["fresh"]).unwrap()["glyphs"], json!(["I"]), "{r}");
    assert_eq!(corner(&r["privateData"]).unwrap()["entries"], 1, "{r}");
    let native = run(&mut s, "file.lossReport", json!({"format": "fontcraft"}));
    assert_eq!((&native["losses"], &native["fresh"]), (&json!([]), &json!([])));

    let saved = run(&mut s, "file.saveAs", json!({"path": dir.join("Font.ufo")}));
    assert_eq!(saved["losses"], r["losses"], "the report says what the save says");
    run(&mut s, "spacing.setWidth", json!({"glyphs": ["I"], "value": 610}));
    let again = run(&mut s, "file.lossReport", json!({}));
    assert_eq!((again["format"].as_str(), &again["fresh"]), (Some("ufo"), &json!([])), "{again}");
    assert!(corner(&again["privateData"]).is_some());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// `strict` refuses a save that leaves data out, and writes nothing.
#[test]
fn strict_refuses_a_lossy_save() {
    let dir = fontcraft_testkit::temp_dir("loss-strict");
    let v1 = r#"{"format":"fontcraft","version":1,"font":{"family_name":"Old","units_per_em":1000,"fromTheFuture":1,
        "masters":[{"id":"m01","name":"Regular","ascender":800,"cap_height":700,"x_height":500,"descender":-200}],
        "glyphs":[{"name":"A","layers":[{"master":"m01","width":600}]}]}}"#;
    std::fs::write(dir.join("Old.fontcraft"), v1).unwrap();
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": dir.join("Old.fontcraft")}));
    let target = dir.join("Old.glyphs");
    let err = s.execute("file.saveAs", &json!({"path": target, "strict": true})).unwrap_err().to_string();
    assert!(err.contains("strict") && err.contains("keys read from that format"), "{err}");
    assert!(!target.exists());
    run(&mut s, "file.saveAs", json!({"path": target, "strict": false}));
    assert!(target.exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A compiled font saved as a native file is an ordinary document: Save writes the native file, and it opens editable.
/// Each member of a collection opens as its own document.
#[test]
fn a_compiled_font_becomes_editable_and_collection_members_open_apart() {
    let dir = fontcraft_testkit::temp_dir("loss-compiled");
    let mut s = Session::with_document();
    let (ttf, ttc) = (dir.join("One.ttf"), dir.join("Two.ttc"));
    run(&mut s, "file.exportStatic", json!({"destination": ttf, "timestamp": 0}));
    run(&mut s, "collection.pack", json!({"fonts": [ttf, ttf], "output": ttc}));
    let members = run(&mut s, "collection.list", json!({"file": ttc}));
    assert_eq!(members["members"].as_array().unwrap().len(), 2);
    assert_eq!(members["members"][1]["family"], "Untitled", "{members}");
    assert!(members["members"][1]["glyphs"].as_u64().unwrap() > 0);

    let err = s.execute("file.open", &json!({"path": ttc})).unwrap_err().to_string();
    assert!(err.contains("collection of 2"), "{err}");
    let before = s.documents().len();
    let second = run(&mut s, "file.open", json!({"path": ttc, "face": 1}));
    assert_eq!((second["face"].as_u64(), second["viewOnly"].as_bool()), (Some(1), Some(true)), "{second}");
    assert!(second["title"].as_str().unwrap().starts_with("Untitled"), "{second}");
    run(&mut s, "file.open", json!({"path": ttc, "face": 0}));
    assert_eq!(s.documents().len(), before + 2, "a second member is not the document already open");
    let shown = run(&mut s, "file.open", json!({"path": ttc, "face": 1}));
    assert_eq!((shown["alreadyOpen"].as_bool(), s.documents().len()), (Some(true), before + 2));

    let native = dir.join("Two.fontcraft");
    run(&mut s, "file.saveAs", json!({"path": native}));
    assert!(!s.active().unwrap().view_only());
    run(&mut s, "spacing.setWidth", json!({"glyphs": ["A"], "value": 610}));
    let saved = run(&mut s, "file.save", json!({}));
    assert_eq!(saved["format"], "fontcraft");
    let mut s = Session::new();
    let opened = run(&mut s, "file.open", json!({"path": native}));
    assert_eq!((opened["format"].as_str(), opened["viewOnly"].as_bool()), (Some("fontcraft"), Some(false)), "{opened}");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Save As from a package to a UFO leaves the package's other files out and says so, and they are not written by the
/// next save either.
#[test]
fn a_packages_files_stay_out_of_a_ufo() {
    let dir = fontcraft_testkit::temp_dir("package-to-ufo");
    let (pkg, ufo) = (dir.join("Font.glyphspackage"), dir.join("Font.ufo"));
    let mut s = Session::with_document();
    s.execute("file.saveAs", &json!({ "path": pkg })).unwrap();
    std::fs::write(pkg.join("notes.txt"), b"kept beside the glyphs").unwrap();
    let mut s = Session::new();
    s.execute("file.open", &json!({ "path": pkg })).unwrap();
    let side = |losses: &Value| losses.as_array().unwrap().iter().any(|l| l["files"].to_string().contains("notes.txt"));
    let report = s.execute("file.lossReport", &json!({ "format": "ufo" })).unwrap();
    assert!(side(&report["losses"]) && side(&report["fresh"]), "{report}");
    let refused = s.execute("file.saveAs", &json!({ "path": ufo, "strict": true })).unwrap_err().to_string();
    assert!(refused.contains("beside the glyphs") && !ufo.exists(), "{refused}");
    let saved = s.execute("file.saveAs", &json!({ "path": ufo })).unwrap();
    assert!(side(&saved["losses"]) && !ufo.join("notes.txt").exists(), "{saved}");
    let again = s.execute("file.save", &json!({})).unwrap();
    assert!(!side(&again["losses"]) && !ufo.join("notes.txt").exists(), "{again}");
    let _ = std::fs::remove_dir_all(&dir);
}
