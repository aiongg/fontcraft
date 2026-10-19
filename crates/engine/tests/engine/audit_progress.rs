//! Audit runs in steps and run ids (M12.64b): `audit.start`, `audit.step`, `audit.cancel`, and
//! the `runId` that audit.run, audit.results, audit.fix and audit.export share.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use fontcraft_engine::Session;
use fontcraft_testkit::rect;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn refused(s: &mut Session, id: &str, p: Value) -> String {
    match s.execute(id, &p) {
        Ok(v) => panic!("{id} should be refused: {v}"),
        Err(e) => e.to_string(),
    }
}

/// Three glyphs with outline problems: overlapping squares, an open line, a square with a
/// node in the middle of an edge.
fn font_with_problems() -> Session {
    let mut s = Session::with_document();
    for g in ["A", "B"] {
        run(&mut s, "path.addPath", json!({"glyph": g, "nodes": rect(0.0, 0.0, 100.0, 100.0), "closed": true}));
        run(&mut s, "path.addPath", json!({"glyph": g, "nodes": rect(50.0, 50.0, 150.0, 150.0), "closed": true}));
    }
    run(&mut s, "path.addPath", json!({"glyph": "C", "nodes": [{"x": 0, "y": 0}, {"x": 100, "y": 0}], "closed": false}));
    run(
        &mut s,
        "path.addPath",
        json!({"glyph": "D", "nodes": [{"x": 0, "y": 0}, {"x": 50, "y": 0}, {"x": 100, "y": 0}, {"x": 100, "y": 100}, {"x": 0, "y": 100}], "closed": true}),
    );
    s
}

/// A result document without its run id, for comparing two runs.
fn without_run_id(mut doc: Value) -> Value {
    doc.as_object_mut().unwrap().remove("runId");
    doc
}

#[test]
fn audit_start_and_audit_step_give_the_result_of_audit_run_and_keep_it_under_a_new_run_id() {
    let mut s = font_with_problems();
    let params = json!({"profile": "outlines"});
    let whole = run(&mut s, "audit.run", params.clone());
    let first_id = whole["runId"].as_str().unwrap().to_string();
    assert!(!whole["results"].as_array().unwrap().is_empty());

    let started = run(&mut s, "audit.start", params);
    let id = started["runId"].as_str().unwrap().to_string();
    assert_ne!(id, first_id, "every run has its own id");
    assert_eq!(started["done"], json!(0));
    let total = started["total"].as_u64().unwrap();
    assert!(total >= 4 + 3, "four layers and the three later stages: {started}");

    // Until the last step the document keeps the earlier run.
    let mut steps = 0;
    let report = loop {
        let out = run(&mut s, "audit.step", json!({"runId": id, "budget": 1}));
        steps += 1;
        assert!(steps <= total, "the run ends within its total");
        if out["finished"] == json!(true) {
            assert_eq!(out["done"].as_u64(), Some(total));
            break out["report"].clone();
        }
        assert!(out["done"].as_u64().unwrap() < total);
        assert_eq!(run(&mut s, "audit.results", json!({}))["runId"], json!(first_id));
    };
    assert_eq!(steps, total, "one layer per step, then one step per later stage");
    assert_eq!(report["runId"], json!(id));
    assert_eq!(without_run_id(report), without_run_id(whole), "the same results as one audit.run");

    // The finished run is the kept one; the earlier run's id is refused.
    assert_eq!(run(&mut s, "audit.results", json!({}))["runId"], json!(id));
    assert_eq!(run(&mut s, "audit.results", json!({"runId": id}))["runId"], json!(id));
    let msg = refused(&mut s, "audit.fix", json!({"results": ["r1"], "runId": first_id, "safeOnly": false}));
    assert!(msg.contains("no longer kept"), "{msg}");
    let path = std::env::temp_dir().join(format!("fontcraft-audit-run-id-{}.json", std::process::id()));
    let msg = refused(&mut s, "audit.export", json!({"path": path.to_string_lossy(), "runId": first_id}));
    assert!(msg.contains("no longer kept"), "{msg}");
    assert!(!path.exists(), "a refused export writes nothing");
    run(&mut s, "audit.export", json!({"path": path.to_string_lossy(), "runId": id}));
    let written: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(written["runId"], json!(id));
    let _ = std::fs::remove_file(&path);
    run(&mut s, "audit.fix", json!({"results": ["r1"], "runId": id, "safeOnly": false}));

    // The run is over: stepping it again is refused.
    let msg = refused(&mut s, "audit.step", json!({"runId": id}));
    assert!(msg.contains("in progress"), "{msg}");
}

#[test]
fn audit_cancel_drops_the_run_and_returns_its_partial_results_without_changing_anything() {
    let mut s = font_with_problems();
    let kept = run(&mut s, "audit.run", json!({"profile": "outlines"}))["runId"].clone();
    let font_before = serde_json::to_value(s.font().unwrap()).unwrap();
    let undo_before = s.doc().unwrap().history.undo.len();

    let id = run(&mut s, "audit.start", json!({"profile": "outlines"}))["runId"].as_str().unwrap().to_string();
    let out = run(&mut s, "audit.step", json!({"runId": id, "budget": 2}));
    assert_eq!(out["done"], json!(2));
    assert_eq!(out["finished"], json!(false));

    let cancelled = run(&mut s, "audit.cancel", json!({"runId": id}));
    assert_eq!(cancelled["cancelled"], json!(true));
    assert_eq!(cancelled["done"], json!(2));
    let partial = &cancelled["partial"];
    assert_eq!(partial["partial"], json!(true));
    assert_eq!(partial["runId"], json!(id));
    let glyphs: Vec<&str> = partial["results"].as_array().unwrap().iter().filter_map(|r| r["location"]["glyph"].as_str()).collect();
    assert!(glyphs.iter().all(|g| *g == "A" || *g == "B"), "only the two layers examined: {glyphs:?}");
    for r in partial["results"].as_array().unwrap() {
        assert_ne!(r["fix"]["params"].get("results"), Some(&json!([r["id"]])), "a partial run is not kept, so fixes name checks and glyphs");
    }

    assert_eq!(run(&mut s, "audit.results", json!({}))["runId"], kept, "the kept run is unchanged");
    assert_eq!(serde_json::to_value(s.font().unwrap()).unwrap(), font_before);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo_before);
    assert!(refused(&mut s, "audit.step", json!({"runId": id})).contains("in progress"));
    assert!(refused(&mut s, "audit.cancel", json!({"runId": id})).contains("in progress"));

    // A run started while another is in progress replaces it.
    let a = run(&mut s, "audit.start", json!({}))["runId"].clone();
    let b = run(&mut s, "audit.start", json!({}))["runId"].clone();
    assert!(refused(&mut s, "audit.step", json!({"runId": a})).contains("in progress"));
    run(&mut s, "audit.step", json!({"runId": b, "budget": 100000}));
    assert!(s.execute("audit.step", &json!({"runId": b, "budget": 0})).is_err(), "a budget below 1 is refused");
    assert!(s.execute("audit.start", &json!({"target": "/no/such/file.ttf"})).is_err(), "files are audited with audit.run");
}
