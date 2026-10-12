//! Changes another program makes to an open document's files (`file.reviewExternalChanges`,
//! `io.watchSourceOnDisk`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use fontcraft_engine::Session;
use fontcraft_engine::cmd::external_changes::changed_on_disk;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

/// A new font saved as a package (one file per glyph) in a fresh folder, then opened in a
/// session of its own.
fn saved(name: &str) -> (PathBuf, PathBuf, Session) {
    let dir = std::env::temp_dir().join(format!("fontcraft-external-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Test.glyphspackage");
    let mut first = Session::new();
    run(&mut first, "file.new", json!({}));
    run(&mut first, "file.saveAs", json!({ "path": path }));
    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": path }));
    (dir, path, s)
}

fn parts(r: &Value) -> u64 {
    r["parts"].as_array().unwrap().len() as u64
}

/// Another program opens the file, runs `steps` and saves.
fn other_program(path: &Path, steps: &[(&str, Value)]) {
    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": path }));
    for (id, p) in steps {
        run(&mut s, id, p.clone());
    }
    run(&mut s, "file.save", json!({}));
}

fn width(s: &Session, name: &str) -> f64 {
    s.font().unwrap().glyphs.get(name).unwrap().layers.first().unwrap().width
}

fn listed(r: &Value) -> Vec<&str> {
    r["glyphs"].as_array().unwrap().iter().map(|g| g["glyph"].as_str().unwrap()).collect()
}

fn entry<'a>(r: &'a Value, glyph: &str) -> &'a Value {
    r["glyphs"].as_array().unwrap().iter().find(|g| g["glyph"] == glyph).unwrap()
}

#[test]
fn a_glyph_file_changed_on_disk_is_listed_and_one_item_is_taken_as_one_undo_step() {
    let (dir, path, mut s) = saved("accept");
    let (a, b) = (width(&s, "A"), width(&s, "B"));
    assert!(changed_on_disk(&s).is_empty());
    other_program(&path, &[("spacing.setWidth", json!({ "glyphs": ["A", "B"], "value": 1234 })), ("glyph.newGlyph", json!({ "name": "A.alt" }))]);
    assert_eq!(changed_on_disk(&s).len(), 1, "the stamps notice the change");

    let r = run(&mut s, "file.reviewExternalChanges", json!({}));
    assert_eq!(r["changed"], true);
    let mut names = listed(&r);
    names.sort_unstable();
    assert_eq!(names, ["A", "A.alt", "B"]);
    assert_eq!(entry(&r, "A.alt")["change"], "added");
    let layer = &entry(&r, "A")["layers"][0];
    assert_eq!(layer["change"], "changed");
    assert_eq!(layer["name"], "Regular");
    assert_eq!(width(&s, "A"), a, "a review without accept changes nothing");

    let undo = s.doc().unwrap().history.undo.len();
    let r = run(&mut s, "file.reviewExternalChanges", json!({ "accept": [{ "glyph": "A", "layer": layer["layer"] }] }));
    assert_eq!((r["accepted"].as_u64(), r["rejected"].as_u64(), r["saveNeedsForce"].as_bool()), (Some(1), Some(2 + parts(&r)), Some(true)));
    assert_eq!(width(&s, "A"), 1234.0);
    assert_eq!(width(&s, "B"), b, "the rejected change is not taken");
    assert!(s.font().unwrap().glyphs.get("A.alt").is_none());
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step");

    // Saving now would overwrite what the other program wrote to B and A.alt.
    let err = s.execute("file.save", &json!({})).unwrap_err().to_string();
    assert!(err.contains("changed on disk"), "{err}");

    run(&mut s, "edit.undo", json!({}));
    assert_eq!(width(&s, "A"), a);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_documents_own_edits_are_not_listed_and_survive_taking_every_change() {
    let (dir, path, mut s) = saved("own");
    run(&mut s, "spacing.setWidth", json!({ "glyphs": ["Z"], "value": 777 }));
    other_program(&path, &[("spacing.setWidth", json!({ "glyphs": ["A"], "value": 1234 }))]);

    let r = run(&mut s, "file.reviewExternalChanges", json!({}));
    assert_eq!(listed(&r), ["A"]);
    assert_eq!((r["modified"].as_bool(), entry(&r, "A")["conflict"].as_bool()), (Some(true), Some(false)));

    let r = run(&mut s, "file.reviewExternalChanges", json!({ "accept": "all" }));
    assert_eq!((r["rejected"].as_u64(), r["saveNeedsForce"].as_bool()), (Some(0), Some(false)));
    assert_eq!((width(&s, "A"), width(&s, "Z")), (1234.0, 777.0));
    assert!(s.doc().unwrap().modified(), "the document's own edit is still unsaved");
    assert!(changed_on_disk(&s).is_empty());

    // Nothing on disk is left out, so saving writes both without asking.
    run(&mut s, "file.save", json!({}));
    let mut again = Session::new();
    run(&mut again, "file.open", json!({ "path": path }));
    assert_eq!((width(&again, "A"), width(&again, "Z")), (1234.0, 777.0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_glyph_both_sides_changed_is_marked_and_rejecting_all_keeps_the_document() {
    let (dir, path, mut s) = saved("conflict");
    run(&mut s, "spacing.setWidth", json!({ "glyphs": ["B"], "value": 777 }));
    other_program(&path, &[("spacing.setWidth", json!({ "glyphs": ["B"], "value": 1234 }))]);

    let r = run(&mut s, "file.reviewExternalChanges", json!({ "accept": "none" }));
    assert_eq!(listed(&r), ["B"]);
    assert_eq!(entry(&r, "B")["conflict"], true);
    assert_eq!((r["accepted"].as_u64(), r["rejected"].as_u64(), r["saveNeedsForce"].as_bool()), (Some(0), Some(1 + parts(&r)), Some(true)));
    assert_eq!(width(&s, "B"), 777.0);
    assert!(s.execute("file.save", &json!({})).is_err(), "saving after a refusal asks first");
    run(&mut s, "file.save", json!({ "force": true }));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_file_only_touched_lists_nothing_and_saves_without_asking() {
    let (dir, path, mut s) = saved("touched");
    let info = std::fs::read_dir(&path).unwrap().flatten().map(|e| e.path()).find(|p| p.is_file()).unwrap();
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(10);
    std::fs::File::options().write(true).open(&info).unwrap().set_modified(later).unwrap();
    assert_eq!(changed_on_disk(&s).len(), 1);

    let r = run(&mut s, "file.reviewExternalChanges", json!({}));
    assert_eq!((r["changed"].as_bool(), r["saveNeedsForce"].as_bool()), (Some(false), Some(false)));
    assert!(changed_on_disk(&s).is_empty(), "the review refreshed the stamp");
    run(&mut s, "file.save", json!({}));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_item_that_did_not_change_is_refused_and_nothing_changes() {
    let (dir, path, mut s) = saved("refused");
    other_program(&path, &[("spacing.setWidth", json!({ "glyphs": ["A"], "value": 1234 }))]);
    let err = s.execute("file.reviewExternalChanges", &json!({ "accept": [{ "glyph": "B" }] })).unwrap_err().to_string();
    assert!(err.contains("did not change on disk"), "{err}");
    assert_ne!(width(&s, "A"), 1234.0);
    let _ = std::fs::remove_dir_all(&dir);
}
