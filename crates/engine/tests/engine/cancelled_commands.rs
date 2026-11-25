//! What a cancel that arrives while a command runs, or after it did its work, leaves behind.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use fontcraft_engine::{EngineError, Result, Session, work};
use fontcraft_testkit::rect;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-cancelled-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Runs a command that is cancelled when it asks for the time: at its first edit, or when it has run.
fn cancelled_at_the_clock(s: &mut Session, id: &str, p: &Value) -> (Result<Value>, bool) {
    let token = work::Token::new();
    let stop = Arc::clone(&token);
    s.set_clock(Some(Arc::new(move || {
        stop.cancel();
        Some(0)
    })));
    let result = work::with(Arc::clone(&token), None, || s.execute(id, p));
    s.set_clock(None);
    (result, token.is_cancelled())
}

/// Overlapping squares in two glyphs.
fn overlaps() -> Session {
    let mut s = Session::with_document();
    for g in ["A", "B"] {
        run(&mut s, "path.addPath", json!({"glyph": g, "nodes": rect(0.0, 0.0, 100.0, 100.0), "closed": true}));
        run(&mut s, "path.addPath", json!({"glyph": g, "nodes": rect(50.0, 50.0, 150.0, 150.0), "closed": true}));
    }
    s
}

/// Runs the audit's fixes and cancels them at progress report `at`; how many reports came and the result.
fn fix_cancelled_at(s: &mut Session, at: Option<usize>) -> (usize, Result<Value>) {
    let token = work::Token::new();
    let reports = Rc::new(RefCell::new(0usize));
    let (seen, stop) = (Rc::clone(&reports), Arc::clone(&token));
    let report: work::Report = Rc::new(move |_, _, _: &str| {
        *seen.borrow_mut() += 1;
        if Some(*seen.borrow()) == at {
            stop.cancel();
        }
    });
    let result = work::with(token, Some(report), || s.execute("audit.fix", &json!({"profile": "outlines", "safeOnly": false})));
    let count = *reports.borrow();
    (count, result)
}

#[test]
fn audit_fixes_cancelled_at_any_of_their_stages_change_nothing() {
    let (all, done) = fix_cancelled_at(&mut overlaps(), None);
    let done = done.unwrap();
    assert!(all >= 2 && !done["applied"].as_array().unwrap().is_empty(), "{all} {done}");
    // While it looks for problems, and while it checks what remains.
    for at in [1, all] {
        let mut s = overlaps();
        let before = s.inspect();
        let (_, result) = fix_cancelled_at(&mut s, Some(at));
        assert!(matches!(result, Err(EngineError::Cancelled)), "at {at}: {result:?}");
        assert_eq!(s.inspect(), before);
    }
    // Inside a fix.
    let mut s = overlaps();
    let before = s.inspect();
    let (result, _) = cancelled_at_the_clock(&mut s, "audit.fix", &json!({"profile": "outlines", "safeOnly": false}));
    assert!(matches!(result, Err(EngineError::Cancelled)), "{result:?}");
    assert_eq!(s.inspect(), before);
}

#[test]
fn an_accepted_cancel_undoes_a_command_that_could_have_written_files_but_did_not() {
    let mut s = overlaps();
    let (result, accepted) = cancelled_at_the_clock(&mut s, "audit.fix", &json!({"profile": "outlines", "dryRun": true}));
    assert!(accepted && matches!(result, Err(EngineError::Cancelled)), "{accepted} {result:?}");
}

#[test]
fn a_cancel_after_files_or_extensions_changed_is_too_late() {
    let dir = folder("extensions");
    let mut s = Session::with_document();
    assert!(fontcraft_engine::extensions::open_folders(s.extensions_mut(), Some(&dir), false).is_empty());
    run(&mut s, "script.recordMacro", json!({"action": "start"}));
    run(&mut s, "glyph.newGlyph", json!({"name": "one.test"}));
    let (result, accepted) = cancelled_at_the_clock(&mut s, "script.recordMacro", &json!({"action": "stop", "name": "Kept"}));
    let path = PathBuf::from(result.unwrap()["path"].as_str().unwrap());
    assert!(!accepted && path.exists());
    assert!(s.extensions().recording.is_none());
    let (result, accepted) = cancelled_at_the_clock(&mut s, "pluginManager.remove", &json!({"id": "Kept"}));
    assert!(!accepted && !path.exists(), "{result:?}");
    assert!(PathBuf::from(result.unwrap()["movedTo"].as_str().unwrap()).exists());

    // Cancelled before it began, the command leaves the recording alone.
    let token = work::Token::new();
    token.cancel();
    assert!(work::with(token, None, || s.execute("script.recordMacro", &json!({"action": "start"}))).is_err());
    assert!(s.extensions().recording.is_none());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_macro_that_fails_or_is_cancelled_names_the_files_its_steps_wrote() {
    let dir = folder("macro");
    let mut s = Session::with_document();
    let before = s.inspect();
    let script = |file: &str, last: Value| {
        json!({"macro": {"title": "Save and edit", "steps": [
            {"command": "file.saveAs", "params": {"path": dir.join(file)}},
            {"command": "glyph.newGlyph", "params": last},
        ]}})
    };
    let failed = s.execute("script.run", &script("kept.fontcraft", json!({"name": 7}))).unwrap_err().to_string();
    assert!(failed.contains("stay: ") && failed.contains("kept.fontcraft"), "{failed}");
    assert!(dir.join("kept.fontcraft").exists());
    assert_eq!(s.inspect(), before);

    let (result, _) = cancelled_at_the_clock(&mut s, "script.run", &script("also-kept.fontcraft", json!({"name": "one.test"})));
    let cancelled = result.unwrap_err().to_string();
    assert!(cancelled.contains("stay: ") && cancelled.contains("also-kept.fontcraft"), "{cancelled}");
    assert_eq!(s.inspect(), before);
    std::fs::remove_dir_all(&dir).unwrap();
}
