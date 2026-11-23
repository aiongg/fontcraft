//! Find and replace in feature code (Font Info › Features find bar): font.features.find and
//! font.features.replace.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::{EngineError, Session};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn refused(s: &mut Session, id: &str, p: Value, needle: &str) {
    let before = Arc::clone(&s.doc().unwrap().font);
    match s.execute(id, &p) {
        Err(EngineError::BadParams { message, .. }) => assert!(message.contains(needle), "{id} {p}: {message}"),
        other => panic!("{id} {p}: expected a refusal naming `{needle}`, got {other:?}"),
    }
    assert!(Arc::ptr_eq(&before, &s.doc().unwrap().font), "{id} {p}: a refused command changed the font");
}

fn code_of(s: &mut Session, kind: &str, name: &str) -> String {
    run(s, "font.features.get", json!({"kind": kind, "name": name}))["code"].as_str().unwrap().to_string()
}

fn font_with_entries() -> Session {
    let mut s = Session::with_document();
    run(&mut s, "font.features.addClass", json!({"name": "Low", "code": "a b a.sc"}));
    run(&mut s, "font.features.addFeature", json!({"tag": "liga", "code": "sub a by b;\nsub A by a;"}));
    run(&mut s, "font.features.addFeature", json!({"tag": "smcp", "code": "sub a by a.sc;"}));
    s.edit("automatic", |font| {
        let f = Arc::make_mut(&mut font.features);
        f.features.iter_mut().filter(|e| e.name == "smcp").for_each(|e| e.automatic = true);
        Ok(())
    })
    .unwrap();
    s
}

#[test]
fn find_lists_matches_across_entries_with_lines_and_honours_case_and_whole_word() {
    let mut s = font_with_entries();
    let r = run(&mut s, "font.features.find", json!({"query": "a", "wholeWord": true, "matchCase": true}));
    let places: Vec<String> = r["matches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| format!("{}:{}:{}:{}", m["name"].as_str().unwrap(), m["start"], m["end"], m["line"]))
        .collect();
    assert_eq!(places, ["Low:0:1:1", "liga:4:5:1", "liga:21:22:2", "smcp:4:5:1"]);
    let r = run(&mut s, "font.features.find", json!({"query": "A", "wholeWord": true}));
    assert_eq!(r["count"], 5, "{r}");
    let r = run(&mut s, "font.features.find", json!({"query": "zz", "kind": "feature", "name": "liga", "code": "zz zz"}));
    assert_eq!(r["count"], 2, "the editor's unsaved text is searched: {r}");
}

#[test]
fn replace_all_across_entries_is_one_undo_step_and_skips_automatic_entries() {
    let mut s = font_with_entries();
    let before = Arc::clone(&s.doc().unwrap().font);
    let r = run(&mut s, "font.features.replace", json!({"query": "a", "wholeWord": true, "matchCase": true, "replacement": "x", "all": true}));
    assert_eq!(r["replaced"], 3, "{r}");
    assert_eq!(r["skipped"][0]["name"], "smcp");
    assert_eq!(code_of(&mut s, "class", "Low"), "x b a.sc");
    assert_eq!(code_of(&mut s, "feature", "liga"), "sub x by b;\nsub A by x;");
    assert_eq!(code_of(&mut s, "feature", "smcp"), "sub a by a.sc;");
    run(&mut s, "edit.undo", json!({}));
    assert!(Arc::ptr_eq(&before, &s.doc().unwrap().font), "Replace All is one undo step");
}

#[test]
fn replace_one_match_expands_groups_and_refuses_what_it_cannot_do() {
    let mut s = font_with_entries();
    let r = run(
        &mut s,
        "font.features.replace",
        json!({"query": "sub (\\w+) by", "regex": true, "replacement": "sub $1 $1 by", "kind": "feature", "name": "liga", "start": 12}),
    );
    assert_eq!(r["replaced"], 1);
    assert_eq!(code_of(&mut s, "feature", "liga"), "sub a by b;\nsub A A by a;");
    assert_eq!(r["caret"], 22);
    refused(&mut s, "font.features.replace", json!({"query": "(", "regex": true, "replacement": "", "all": true}), "regular expression");
    refused(&mut s, "font.features.replace", json!({"query": "a", "replacement": "", "kind": "feature", "name": "liga", "start": 1}), "no match");
    refused(
        &mut s,
        "font.features.replace",
        json!({"query": "a", "replacement": "b", "kind": "feature", "name": "smcp", "all": true}),
        "automatically",
    );
    refused(&mut s, "font.features.replace", json!({"query": "a", "replacement": "b"}), "start");
}
