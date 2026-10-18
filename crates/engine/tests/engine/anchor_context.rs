//! Set Context of contextual anchors (M8.21): the context lands in the anchor's user data under
//! `GPOS_Context`, an empty one removes it, and bad requests change nothing.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::model::Font;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn refused(s: &mut Session, id: &str, p: Value, needle: &str) {
    let before = s.doc().unwrap().font.clone();
    let err = s.execute(id, &p).expect_err(id).to_string();
    assert!(err.contains(needle), "{id}: expected an error naming {needle:?}, got {err:?}");
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "{id}: a refused command changes nothing");
}

/// The stored user data value of `anchor` in `A`'s first master layer under `GPOS_Context`.
fn stored(f: &Font, anchor: &str) -> Option<String> {
    let g = f.glyph("A").unwrap();
    let a = g.layers.iter().find(|l| l.id == "m01").unwrap().anchors.iter().find(|a| a.name == anchor).unwrap();
    a.info.user_data.get("GPOS_Context").map(|v| v.as_str().unwrap().to_string())
}

fn undo_depth(s: &Session) -> usize {
    s.inspect()["documents"][0]["undo"].as_array().map_or(0, Vec::len)
}

#[test]
fn anchor_set_context_stores_the_context_in_the_anchor_user_data() {
    let mut s = Session::with_document();
    run(&mut s, "anchor.add", json!({"glyph": "A", "name": "top", "x": 300, "y": 700}));
    run(&mut s, "anchor.add", json!({"glyph": "A", "name": "*top.after", "x": 320, "y": 720}));
    let depth = undo_depth(&s);
    let r = run(&mut s, "anchor.setContext", json!({"glyph": "A", "anchor": "*top.after", "context": "lookupflag IgnoreMarks; V *\nW *"}));
    assert_eq!(r["changed"], json!(true));
    assert_eq!(r["selection"]["anchors"], json!(["*top.after"]));
    assert_eq!(stored(s.font().unwrap(), "*top.after").as_deref(), Some("lookupflag IgnoreMarks; V *\nW *"));
    assert_eq!(undo_depth(&s), depth + 1, "one undo step");

    // The same context again changes nothing.
    let r = run(&mut s, "anchor.setContext", json!({"glyph": "A", "anchor": "*top.after", "context": "lookupflag IgnoreMarks; V *\nW *"}));
    assert_eq!(r["changed"], json!(false));
    assert_eq!(undo_depth(&s), depth + 1);

    // An empty context removes the key; undo brings it back.
    run(&mut s, "anchor.setContext", json!({"glyph": "A", "anchor": "*top.after", "context": "  "}));
    assert_eq!(stored(s.font().unwrap(), "*top.after"), None);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(stored(s.font().unwrap(), "*top.after").as_deref(), Some("lookupflag IgnoreMarks; V *\nW *"));
}

#[test]
fn anchor_set_context_refuses_plain_and_missing_anchors_and_bad_text() {
    let mut s = Session::with_document();
    run(&mut s, "anchor.add", json!({"glyph": "A", "name": "top", "x": 300, "y": 700}));
    run(&mut s, "anchor.add", json!({"glyph": "A", "name": "*top", "x": 320, "y": 720}));
    refused(&mut s, "anchor.setContext", json!({"glyph": "A", "anchor": "top", "context": "V *"}), "not contextual");
    refused(&mut s, "anchor.setContext", json!({"glyph": "A", "anchor": "*bottom", "context": "V *"}), "no anchor");
    refused(&mut s, "anchor.setContext", json!({"glyph": "A", "anchor": "*top", "context": "V\u{0} *"}), "control characters");
    refused(&mut s, "anchor.setContext", json!({"glyph": "A", "anchor": "*top", "context": "x".repeat(16_385)}), "16384");
    refused(&mut s, "anchor.setContext", json!({"glyph": "A", "anchor": "*top"}), "context");
    refused(&mut s, "anchor.setContext", json!({"glyph": "A", "anchor": "*top", "context": "V *", "anchors": ["*top"]}), "target lists");
}
