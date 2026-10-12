//! Status definitions and glyph status: the definitions and the statuses survive `.glyphs` (both
//! versions) and UFO, setting a status on many glyphs is one undo step, and refused requests
//! change nothing.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::cmd::status::{StatusDef, definitions, effective_status, stored_status};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

/// Runs a command that must fail and checks that the font is the same object as before.
fn refused(s: &mut Session, id: &str, p: Value) -> String {
    let before = s.doc().unwrap().font.clone();
    let e = s.execute(id, &p).expect_err(id).to_string();
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "{id} {p}: a refused command changed the font");
    e
}

fn statuses() -> Value {
    json!([
        {"code": 0, "name": "In progress", "color": [1, 0, 0, 1]},
        {"code": 1, "name": "Review", "color": [1, 0.75, 0], "isDefault": true},
        {"code": 7, "name": "Final", "color": [0, 0.5, 0.25, 0.5]},
    ])
}

fn session() -> Session {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"familyName": "Status Sans"}));
    run(&mut s, "font.setStatusDefinitions", json!({"statuses": statuses()}));
    s
}

fn saved_state(s: &Session) -> (Vec<StatusDef>, Vec<Option<i64>>) {
    let font = s.font().unwrap();
    (definitions(font), ["A", "B", "a", "b"].iter().map(|g| stored_status(font, g)).collect())
}

#[test]
fn font_set_status_definitions_and_glyph_set_status_survive_glyphs_and_ufo() {
    let dir = std::env::temp_dir().join(format!("fontcraft-status-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let with_statuses = || {
        let mut s = session();
        let r = run(&mut s, "glyph.setStatus", json!({"glyphs": ["A", "B"], "status": 7}));
        assert_eq!(r, json!({"glyphs": ["A", "B"], "status": 7}));
        run(&mut s, "glyph.setStatus", json!({"glyphs": ["a"], "status": "in progress"}));
        s
    };
    let expected = saved_state(&with_statuses());
    assert_eq!(expected.0.len(), 3);
    assert_eq!(expected.0[1], StatusDef { code: 1, name: "Review".into(), color: [1.0, 0.75, 0.0, 1.0], is_default: true });
    assert_eq!(expected.1, vec![Some(7), Some(7), Some(0), None]);
    for (file, params) in [("Status2.glyphs", json!({"version": 2})), ("Status3.glyphs", json!({"version": 3})), ("Status.ufo", json!({}))] {
        // Each save starts from the unsaved font: a font saved in one version refuses another.
        let mut s = with_statuses();
        let path = dir.join(file);
        let mut p = params.clone();
        p["path"] = json!(path);
        run(&mut s, "file.saveAs", p);
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": path}));
        assert_eq!(saved_state(&back), expected, "{file}");
        let font = back.font().unwrap();
        assert_eq!(effective_status(font, "b").map(|d| d.code), Some(1), "{file}: a glyph without a status is in the default one");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn glyph_set_status_on_many_glyphs_is_one_undo_step() {
    let mut s = session();
    let names: Vec<String> = s.font().unwrap().glyphs.iter().map(|g| g.name.clone()).take(40).collect();
    assert!(names.len() > 10, "the new font has enough glyphs");
    let before = s.doc().unwrap().font.clone();
    let steps = s.doc().unwrap().history.undo.len();
    run(&mut s, "glyph.setStatus", json!({"glyphs": names, "status": "Final"}));
    assert!(names.iter().all(|n| stored_status(s.font().unwrap(), n) == Some(7)));
    assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1);
    run(&mut s, "edit.undo", json!({}));
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "one undo gives back the font");

    // Clearing goes back to the default status; clearing again changes nothing.
    run(&mut s, "glyph.setStatus", json!({"glyphs": ["A"], "status": 0}));
    run(&mut s, "glyph.setStatus", json!({"glyphs": ["A"], "status": null}));
    assert_eq!(stored_status(s.font().unwrap(), "A"), None);
    assert_eq!(effective_status(s.font().unwrap(), "A").map(|d| d.name), Some("Review".into()));
    let cleared = s.doc().unwrap().font.clone();
    run(&mut s, "glyph.setStatus", json!({"glyphs": ["A"], "status": null}));
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &cleared));
}

#[test]
fn status_requests_the_definitions_do_not_allow_are_refused() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"familyName": "Status Sans"}));
    assert!(refused(&mut s, "glyph.setStatus", json!({"glyphs": ["A"], "status": 1})).contains("no status definitions"));
    let def = |code: i64, name: &str, default: bool| json!({"code": code, "name": name, "color": [0, 0, 0], "isDefault": default});
    refused(&mut s, "font.setStatusDefinitions", json!({"statuses": [def(1, "a", false), def(1, "b", false)]}));
    refused(&mut s, "font.setStatusDefinitions", json!({"statuses": [def(1, "a", false), def(2, "a", false)]}));
    refused(&mut s, "font.setStatusDefinitions", json!({"statuses": [def(1, "a", true), def(2, "b", true)]}));
    refused(&mut s, "font.setStatusDefinitions", json!({"statuses": [{"code": 1, "name": "a", "color": [2, 0, 0]}]}));
    refused(&mut s, "font.setStatusDefinitions", json!({"statuses": [{"code": -1, "name": "a", "color": [0, 0, 0]}]}));
    let many: Vec<Value> = (0..65).map(|i| def(i, &format!("s{i}"), false)).collect();
    refused(&mut s, "font.setStatusDefinitions", json!({"statuses": many}));

    run(&mut s, "font.setStatusDefinitions", json!({"statuses": statuses()}));
    refused(&mut s, "glyph.setStatus", json!({"glyphs": ["A"], "status": 3}));
    refused(&mut s, "glyph.setStatus", json!({"glyphs": ["A"], "status": "Unknown"}));
    refused(&mut s, "glyph.setStatus", json!({"glyphs": ["A"]}));
    refused(&mut s, "glyph.setStatus", json!({"glyphs": ["no such glyph"], "status": 1}));

    // Redefining keeps stored codes and counts those the new list lacks.
    run(&mut s, "glyph.setStatus", json!({"glyphs": ["A", "B"], "status": 7}));
    let r = run(&mut s, "font.setStatusDefinitions", json!({"statuses": [def(0, "Draft", false)]}));
    assert_eq!(r["undefined"], json!(2));
    assert_eq!(stored_status(s.font().unwrap(), "A"), Some(7));
    assert!(effective_status(s.font().unwrap(), "A").is_none());
    run(&mut s, "font.setStatusDefinitions", json!({"statuses": []}));
    assert!(definitions(s.font().unwrap()).is_empty());
}
