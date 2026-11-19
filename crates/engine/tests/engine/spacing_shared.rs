//! Spacing and kerning fields for several glyphs (M5.22): the group and metric setters change
//! every glyph in `glyphs` in one undo step, and `spacing.getSharedMetrics` shows a value only
//! where all the glyphs share it.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use fontcraft_engine::Session;
use fontcraft_testkit::rect;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// Runs a command and checks that it added exactly one undo step.
fn one_step(s: &mut Session, id: &str, p: Value) -> Value {
    let steps = s.doc().unwrap().history.undo.len();
    let out = run(s, id, p);
    assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1, "{id} is one undo step");
    out
}

fn shared(s: &mut Session, glyphs: &[&str]) -> Value {
    run(s, "spacing.getSharedMetrics", json!({ "glyphs": glyphs }))
}

/// `H` L50 R50, `O` L30 R30, `D` L50 R40, all 600 wide.
fn session() -> Session {
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "H", "nodes": rect(50.0, 0.0, 550.0, 700.0)}));
    run(&mut s, "path.addPath", json!({"glyph": "O", "nodes": rect(30.0, 0.0, 570.0, 700.0)}));
    run(&mut s, "path.addPath", json!({"glyph": "D", "nodes": rect(50.0, 0.0, 560.0, 700.0)}));
    run(&mut s, "spacing.setWidth", json!({"glyphs": ["H", "O", "D"], "value": 600}));
    s
}

#[test]
fn fields_for_several_glyphs_show_only_shared_values() {
    let mut s = session();
    let hod = ["H", "O", "D"];
    let out = shared(&mut s, &hod);
    assert_eq!(out["glyphs"], json!(hod), "{out}");
    assert_eq!(out["fields"]["left"], json!({"key": null, "value": null, "mixed": true}), "{out}");
    assert_eq!(out["fields"]["width"], json!({"key": null, "value": 600.0, "mixed": false}), "{out}");
    assert_eq!(out["groups"]["left"], json!({"value": null, "mixed": false}), "no glyph has a group");

    // Typing `H` into the left Group field with the three glyphs selected: one step, all three.
    one_step(&mut s, "kerning.setLeftGroup", json!({"glyphs": hod, "group": "H"}));
    run(&mut s, "kerning.setRightGroup", json!({"glyphs": ["O"], "group": "O"}));
    let out = shared(&mut s, &hod);
    assert_eq!(out["groups"]["left"], json!({"value": "H", "mixed": false}), "{out}");
    assert_eq!(out["groups"]["right"], json!({"value": null, "mixed": true}), "{out}");

    // A typed left sidebearing goes to all three in one step.
    let set = one_step(&mut s, "spacing.setLeftSidebearing", json!({"glyphs": hod, "value": 40}));
    assert_eq!(set["glyphs"], json!(hod), "{set}");
    let out = shared(&mut s, &hod);
    assert_eq!(out["fields"]["left"], json!({"key": null, "value": 40.0, "mixed": false}), "{out}");

    // Keys: a key on some glyphs and none on another is mixed, whatever the values; a key every
    // glyph shares is shown.
    run(&mut s, "spacing.setLeftSidebearing", json!({"glyphs": ["O", "D"], "value": "=H"}));
    let out = shared(&mut s, &hod);
    assert_eq!(out["fields"]["left"]["mixed"], json!(true), "{out}");
    let out = shared(&mut s, &["O", "D"]);
    assert_eq!(out["fields"]["left"], json!({"key": "=H", "value": 40.0, "mixed": false}), "{out}");

    for bad in [json!({"glyphs": []}), json!({"glyphs": ["H"], "layer": "nowhere"}), json!({})] {
        assert!(s.execute("spacing.getSharedMetrics", &bad).is_err(), "{bad} should be refused");
    }
}
