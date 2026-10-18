//! `font.diff` on fonts the engine exports: a font equals itself, a changed glyph shows in the
//! outline tables with a diff, folders pair fonts by name, the texts can be written, and bad
//! parameters write nothing.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use fontcraft_engine::Session;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-diff-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A one-glyph font `A` of the given width, exported to `path`.
fn export(path: &Path, width: f64) {
    let mut s = Session::with_document();
    let rect = json!([{"x": 0, "y": 0}, {"x": width, "y": 0}, {"x": width, "y": 700}, {"x": 0, "y": 700}]);
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect}));
    run(&mut s, "file.exportStatic", json!({"destination": path, "timestamp": 0}));
}

#[test]
fn font_diff_reports_tables_by_presence_bytes_and_text() {
    let dir = temp("pair");
    let (narrow, wide) = (dir.join("narrow.ttf"), dir.join("wide.ttf"));
    export(&narrow, 500.0);
    export(&wide, 600.0);
    let mut s = Session::new();

    let same = run(&mut s, "font.diff", json!({"a": narrow, "b": narrow}));
    assert_eq!(same["identical"], true, "{same}");
    assert_eq!(same["differing"], json!([]));
    assert_eq!(same["tables"]["head"]["equal"], true);

    let out = run(&mut s, "font.diff", json!({"a": narrow, "b": wide}));
    assert_eq!(out["identical"], false);
    let glyf = &out["tables"]["glyf"];
    assert_eq!((glyf["a"].clone(), glyf["b"].clone(), glyf["status"].clone()), (json!(true), json!(true), json!("different")), "{glyf}");
    let lines: Vec<&str> = glyf["lines"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert!(lines.iter().any(|l| l.starts_with('-') && l.contains("x=\"500\"")), "{lines:#?}");
    assert!(lines.iter().any(|l| l.starts_with('+') && l.contains("x=\"600\"")), "{lines:#?}");
    assert!(out["differing"].as_array().unwrap().iter().any(|t| t == "head"), "the font bounding box differs");

    let offsets = run(&mut s, "font.diff", json!({"a": narrow, "b": wide, "detail": "offset", "tables": ["glyf", "name"]}));
    assert_eq!(offsets["tables"].as_object().unwrap().len(), 2);
    assert!(offsets["tables"]["glyf"]["firstOffset"].is_u64());
    assert!(offsets["tables"]["glyf"].get("lines").is_none());

    let ratio = run(&mut s, "font.diff", json!({"a": narrow, "b": wide, "similarity": true}));
    let r = ratio["tables"]["glyf"]["similarity"].as_f64().unwrap();
    assert!(r > 0.0 && r < 1.0 && ratio["tables"]["glyf"].get("lines").is_none(), "{}", ratio["tables"]["glyf"]);

    // The compiler normalisation keeps the outline difference.
    let compiler = run(&mut s, "font.diff", json!({"a": narrow, "b": wide, "normalize": "compiler"}));
    assert_eq!(compiler["tables"]["glyf"]["identical"], false);

    // The texts compared, written beside each other.
    let texts = dir.join("texts");
    let written = run(&mut s, "font.diff", json!({"a": narrow, "b": wide, "tables": ["glyf"], "outputDir": texts}));
    assert_eq!(written["files"].as_array().unwrap().len(), 2, "{written}");
    assert!(std::fs::read_to_string(texts.join("A.glyf.txt")).unwrap().contains("x=\"500\""));
    assert!(std::fs::read_to_string(texts.join("B.glyf.txt")).unwrap().contains("x=\"600\""));

    // Refusals write nothing.
    let refused = dir.join("refused");
    for bad in [
        json!({"a": narrow, "b": wide, "detail": "everything", "outputDir": refused}),
        json!({"a": narrow, "b": wide, "tables": ["glyf"], "excludeTables": ["name"], "outputDir": refused}),
        json!({"a": narrow, "b": wide, "detail": "hex", "normalize": "compiler", "outputDir": refused}),
        json!({"a": narrow, "b": wide, "tool": {"command": "meld"}, "outputDir": refused}),
        json!({"a": narrow, "b": wide, "offByOneBudget": 200, "outputDir": refused}),
        json!({"a": narrow, "b": dir.join("missing.ttf"), "outputDir": refused}),
    ] {
        assert!(s.execute("font.diff", &bad).is_err(), "{bad}");
        assert!(!refused.exists(), "{bad}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn font_diff_pairs_folders_by_file_name() {
    let dir = temp("folders");
    let (left, right) = (dir.join("left"), dir.join("right"));
    std::fs::create_dir_all(&left).unwrap();
    std::fs::create_dir_all(&right).unwrap();
    export(&left.join("Same.ttf"), 500.0);
    export(&right.join("Same.ttf"), 500.0);
    export(&left.join("Changed.ttf"), 500.0);
    export(&right.join("Changed.ttf"), 650.0);
    export(&left.join("Gone.ttf"), 500.0);
    std::fs::write(left.join("notes.txt"), "not a font").unwrap();

    let mut s = Session::new();
    let out = run(&mut s, "font.diff", json!({"a": left, "b": right, "detail": "tables"}));
    let names: Vec<&str> = out["pairs"].as_array().unwrap().iter().filter_map(|p| p["name"].as_str()).collect();
    assert_eq!(names, ["Changed.ttf", "Same.ttf"]);
    assert_eq!(out["onlyA"], json!(["Gone.ttf"]), "files that are not fonts are left out");
    assert_eq!(out["onlyB"], json!([]));
    assert_eq!(out["pairs"][0]["identical"], false);
    assert_eq!(out["pairs"][1]["identical"], true);
    assert_eq!(out["identical"], false);

    // A font and a folder: the folder's font of the same name.
    let one = run(&mut s, "font.diff", json!({"a": left.join("Same.ttf"), "b": right}));
    assert_eq!(one["identical"], true, "{one}");
    let _ = std::fs::remove_dir_all(&dir);
}
