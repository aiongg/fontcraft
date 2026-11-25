//! What stops an export, listed at once (`file.exportCheck` and the errors of the export commands).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};

use fontcraft_engine::Session;
use serde_json::{Value, json};

use crate::export::TWO_MASTERS;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-export-check-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The test family; with `broken`, its `A` is not compatible.
fn two_masters(dir: &Path, broken: bool) -> Session {
    let path = dir.join("TestSans.glyphs");
    let text = if broken { TWO_MASTERS.replacen("(80,0,l),", "", 1) } else { TWO_MASTERS.to_string() };
    std::fs::write(&path, text).unwrap();
    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": path }));
    s
}

fn kinds(errors: &Value) -> Vec<&str> {
    errors.as_array().unwrap().iter().map(|e| e["kind"].as_str().unwrap()).collect()
}

/// Three bad names, a code point on two glyphs and feature code that does not compile are five
/// errors of one run, which writes nothing; the check lists the same without exporting.
#[test]
fn one_run_lists_every_problem_and_writes_nothing() {
    let dir = temp("names");
    let mut s = two_masters(&dir, false);
    s.edit("Add", |font| {
        for (name, cp) in [("a b", 0xE000), ("1one", 0xE001), ("d\u{e9}", 0xE002), ("second", 0x41)] {
            font.add_glyph(name, vec![cp], 500.0)?;
        }
        Ok(())
    })
    .unwrap();
    run(&mut s, "font.features.addFeature", json!({"tag": "liga", "code": "sub A by A;\nsub nothere by A;"}));
    let out = dir.join("out");
    for (command, tab) in [("file.exportStatic", "otf"), ("file.exportVariable", "variable"), ("file.exportAll", "otf")] {
        let error = s.execute(command, &json!({ "destination": out, "timestamp": 0 })).unwrap_err();
        let data = error.data().cloned().unwrap_or_else(|| panic!("{command}: {error}"));
        let found = kinds(&data["errors"]);
        assert_eq!(found.iter().filter(|k| **k == "glyphName").count(), 3, "{command}: {data}");
        assert!(found.contains(&"features"), "{command}: {data}");
        assert!(error.to_string().contains("`1one`") && error.to_string().contains("problems stop the export"), "{error}");
        assert!(!out.exists(), "{command} wrote something");
        let check = run(&mut s, "file.exportCheck", json!({ "tab": tab }));
        assert_eq!((&check["ok"], kinds(&check["errors"])), (&json!(false), found), "{command}: {check}");
    }
    let data = s.execute("file.exportVariable", &json!({})).unwrap_err().data().cloned().unwrap();
    let of = |kind: &str| data["errors"].as_array().unwrap().iter().find(|e| e["kind"] == kind).unwrap().clone();
    assert_eq!((&of("glyphName")["glyph"], &of("glyphName")["position"]), (&json!("a b"), &json!(2)));
    assert_eq!(of("codepoint")["glyphs"], json!(["A", "second"]));
    assert_eq!((&of("features")["entry"]["name"], &of("features")["line"]), (&json!("liga"), &json!(2)), "{data}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// An instance that cannot be interpolated is skipped with its reason and the glyph that keeps it
/// from being; the fonts at the masters are written, and the check says so before.
#[test]
fn a_skipped_instance_names_its_glyphs() {
    let dir = temp("skipped");
    let mut s = two_masters(&dir, true);
    let check = run(&mut s, "file.exportCheck", json!({ "params": { "destination": dir.join("out") } }));
    let reply = run(&mut s, "file.exportStatic", json!({ "destination": dir.join("out"), "timestamp": 0 }));
    assert_eq!((&check["ok"], &check["files"]), (&json!(true), &json!(reply["fonts"].as_array().unwrap().len())), "{check}");
    assert_eq!(check["skipped"], reply["skipped"], "{check} {reply}");
    let skipped = &reply["skipped"][0];
    assert_eq!((&skipped["instance"], &skipped["glyphs"][0]["glyph"]), (&json!("Medium"), &json!("A")), "{reply}");
    assert!(skipped["glyphs"][0]["layer"].is_string(), "{reply}");

    let only = s.execute("file.exportStatic", &json!({ "instances": ["Medium"] })).unwrap_err();
    assert_eq!(only.data().unwrap()["skipped"][0]["instance"], "Medium");
    let variable = run(&mut s, "file.exportCheck", json!({ "tab": "variable" }));
    assert_eq!((kinds(&variable["errors"]), &variable["errors"][0]["glyph"]), (vec!["incompatible"], &json!("A")), "{variable}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The errors of an export of two documents of one title say which of them they are about.
#[test]
fn errors_of_documents_with_one_title_name_the_document() {
    let dir = temp("titles");
    let mut s = Session::new();
    for folder in ["one", "two"] {
        let path = dir.join(folder).join("TestSans.glyphs");
        std::fs::create_dir_all(dir.join(folder)).unwrap();
        std::fs::write(&path, TWO_MASTERS).unwrap();
        run(&mut s, "file.open", json!({ "path": path }));
    }
    s.edit("Add", |font| {
        font.add_glyph("1one", vec![0xE001], 500.0)?;
        Ok(())
    })
    .unwrap();
    let id = s.active().unwrap().id;
    let error = s.execute("file.exportAll", &json!({ "destination": dir.join("out"), "timestamp": 0 })).unwrap_err();
    let data = error.data().cloned().unwrap();
    let bad = data["errors"].as_array().unwrap().iter().find(|e| e["kind"] == "glyphName").unwrap();
    assert_eq!((&bad["document"], &bad["documentId"]), (&json!("TestSans"), &json!(id)), "{data}");
    let _ = std::fs::remove_dir_all(&dir);
}
