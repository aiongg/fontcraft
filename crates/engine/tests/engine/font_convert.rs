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
    // Beside the inputs: the first output, already staged when the second input fails, is removed.
    let listing = || std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect::<std::collections::BTreeSet<_>>();
    let before = listing();
    assert!(s.execute("font.convert", &json!({ "inputs": [ttf, junk], "to": "cff" })).is_err());
    assert_eq!(listing(), before, "no output and no temporary file is left");
    assert!(s.execute("font.convert", &json!({ "input": ttf, "to": "ufo" })).is_err(), "sources are written by file.saveAs");
    assert!(s.execute("font.convert", &json!({ "input": ttf, "to": "otf", "glyphs": ["A"] })).is_err(), "options not built yet are refused");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn font_convert_reads_ttx_text_and_bare_cff_tables() {
    let dir = std::env::temp_dir().join(format!("fontcraft-convert-programs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::with_document();
    let square = json!([{"x": 60, "y": 0}, {"x": 560, "y": 0}, {"x": 560, "y": 700}, {"x": 60, "y": 700}]);
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": square}));
    let ttf = dir.join("Font.ttf");
    run(&mut s, "file.exportStatic", json!({ "destination": ttf, "timestamp": 0 }));
    let cff_of = |path: PathBuf| fontcraft_tables::read(&std::fs::read(path).unwrap(), 0).unwrap().get(*b"CFF ").unwrap().to_vec();

    // A TTX dump converts as the font it was dumped from.
    let ttx = dir.join("Font.ttx");
    run(&mut s, "font.dumpTables", json!({ "font": ttf, "output": ttx }));
    run(&mut s, "font.convert", json!({ "input": ttf, "to": "otf", "output": dir.join("direct.otf") }));
    run(&mut s, "font.convert", json!({ "input": ttx, "to": "otf", "output": dir.join("via-ttx.otf") }));
    assert_eq!(cff_of(dir.join("via-ttx.otf")), cff_of(dir.join("direct.otf")));

    // A bare CFF table becomes a whole font again, with TrueType or CFF outlines.
    let bare = dir.join("Font.cff");
    run(&mut s, "font.convert", json!({ "input": ttf, "to": "cff", "output": bare }));
    let r = run(&mut s, "font.convert", json!({ "input": bare, "to": "ttf", "output": dir.join("from-cff.ttf") }));
    let font = fontcraft_tables::read(&std::fs::read(dir.join("from-cff.ttf")).unwrap(), 0).unwrap();
    assert!(font.get(*b"glyf").is_some() && font.get(*b"cmap").is_some() && font.get(*b"OS/2").is_some());
    let direct = fontcraft_tables::read(&std::fs::read(dir.join("direct.otf")).unwrap(), 0).unwrap();
    assert_eq!(r["results"][0]["glyphs"], json!(direct.get(*b"maxp").map(|m| u16::from_be_bytes([m[4], m[5]])).unwrap()));
    // Through OpenType (subroutinised) and back to a bare table: the charstrings are kept.
    run(&mut s, "font.convert", json!({ "input": bare, "to": "otf", "output": dir.join("from-cff.otf") }));
    assert!(cff_of(dir.join("from-cff.otf")).len() > 4);
    let back = dir.join("back.cff");
    run(&mut s, "font.convert", json!({ "input": dir.join("from-cff.otf"), "to": "cff", "cff": {"subroutinize": false}, "output": back }));
    run(&mut s, "font.convert", json!({ "input": bare, "to": "cff", "output": dir.join("again.cff") }));
    assert_eq!(std::fs::read(&back).unwrap(), std::fs::read(dir.join("again.cff")).unwrap());
    assert!(s.execute("font.convert", &json!({ "input": bare, "to": "otf", "face": 1, "output": dir.join("x.otf") })).is_err(), "one font: no face");
    let _ = std::fs::remove_dir_all(&dir);
}
