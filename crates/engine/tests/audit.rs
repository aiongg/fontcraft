//! The audit commands (M3.17): running the outline checks, the result document, the kept run,
//! and fixes through the path commands. Fixtures are built in code.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use fontcraft_engine::model::Shape;
use fontcraft_engine::{EngineError, Session};
use fontcraft_testkit::rect;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn add(s: &mut Session, glyph: &str, nodes: Value, closed: bool) {
    run(s, "path.addPath", json!({"glyph": glyph, "nodes": nodes, "closed": closed}));
}

fn line(x: f64, y: f64) -> Value {
    json!({"x": x, "y": y})
}

fn curve(x: f64, y: f64) -> Value {
    json!({"x": x, "y": y, "type": "curve"})
}

fn off(x: f64, y: f64) -> Value {
    json!({"x": x, "y": y, "type": "offcurve"})
}

fn lines(list: &[(f64, f64)]) -> Value {
    Value::Array(list.iter().map(|&(x, y)| line(x, y)).collect())
}

/// How many results of `check` a result document lists.
fn count(doc: &Value, check: &str) -> usize {
    doc["results"].as_array().unwrap().iter().filter(|r| r["check"] == check).count()
}

fn font_json(s: &Session) -> Value {
    serde_json::to_value(s.font().unwrap()).unwrap()
}

fn layer_count(s: &Session, glyph: &str) -> usize {
    s.font().unwrap().glyph(glyph).unwrap().layers.len()
}

fn undo_steps(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

fn overlapping_squares(s: &mut Session, glyph: &str) {
    add(s, glyph, rect(0.0, 0.0, 100.0, 100.0), true);
    add(s, glyph, rect(50.0, 50.0, 150.0, 150.0), true);
}

/// A square with a node in the middle of its bottom edge.
fn square_with_extra_node(s: &mut Session, glyph: &str) {
    add(s, glyph, lines(&[(0.0, 0.0), (50.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]), true);
}

#[test]
fn each_fix_leaves_its_check_silent() {
    let mut s = Session::with_document();
    overlapping_squares(&mut s, "A");
    add(&mut s, "B", lines(&[(0.0, 0.0), (100.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]), true);
    add(&mut s, "C", json!([line(0.0, 0.0), off(30.0, 0.0), off(70.0, 0.0), curve(100.0, 0.0), line(100.0, 100.0), line(0.0, 100.0)]), true);
    square_with_extra_node(&mut s, "D");
    // Clockwise: wrong for an outer contour.
    add(&mut s, "E", lines(&[(0.0, 0.0), (0.0, 100.0), (100.0, 100.0), (100.0, 0.0)]), true);
    // The curve's rightmost point (175, 100) lies between its nodes.
    add(&mut s, "F", json!([line(0.0, 0.0), line(100.0, 0.0), off(200.0, 0.0), off(200.0, 200.0), curve(100.0, 200.0), line(0.0, 200.0)]), true);
    add(&mut s, "G", lines(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)]), false);
    // Starts at the top right instead of the bottom left.
    add(&mut s, "H", lines(&[(100.0, 100.0), (0.0, 100.0), (0.0, 0.0), (100.0, 0.0)]), true);
    add(&mut s, "I", rect(0.0, 0.0, 100.0, 100.0), true);
    s.edit("fractional coordinates", |font| {
        let mut g = font.glyph_mut("I")?;
        if let Some(Shape::Path(p)) = g.layers[0].shapes.first_mut() {
            p.nodes[1].x = 100.4;
            p.nodes[2].x = 100.4;
        }
        Ok(())
    })
    .unwrap();

    let cases = [
        ("check.outlines.overlap", "A", "path.removeOverlap"),
        ("check.outlines.coincidentPoints", "B", "path.tidyUpPaths"),
        ("check.outlines.flatCurve", "C", "path.tidyUpPaths"),
        ("check.outlines.collinearLines", "D", "path.tidyUpPaths"),
        ("check.outlines.pathDirection", "E", "path.correctPathDirection"),
        ("check.outlines.missingExtremes", "F", "path.addExtremes"),
        ("check.outlines.openPath", "G", "path.closeOpenPaths"),
        ("check.outlines.startPointPosition", "H", "path.makeNodeFirst"),
        ("check.outlines.nodesOffGrid", "I", "path.roundCoordinates"),
    ];
    for (check, glyph, command) in cases {
        let select = json!({"profile": "strict", "checks": [check], "glyphs": [glyph]});
        let doc = run(&mut s, "audit.run", select.clone());
        assert!(count(&doc, check) >= 1, "{check}: the fixture is reported: {doc}");

        let before = font_json(&s);
        let (layers, steps) = (layer_count(&s, glyph), undo_steps(&s));
        let mut p = select.clone();
        p["safeOnly"] = json!(false);
        let r = run(&mut s, "audit.fix", p);
        assert_eq!(r["applied"][0]["command"], json!(command), "{check}: {r}");
        assert_eq!(count(&r["remaining"], check), 0, "{check}: reported after the fix: {r}");
        assert_eq!(layer_count(&s, glyph), layers + 1, "{check}: the unfixed layer is kept as a backup");
        assert_eq!(r["backups"][0]["glyph"], json!(glyph));
        assert_eq!(undo_steps(&s), steps + 1, "{check}: one undo step");

        let again = run(&mut s, "audit.run", select);
        assert_eq!(count(&again, check), 0, "{check}: the backup layer is not audited");

        run(&mut s, "edit.undo", json!({}));
        assert_eq!(font_json(&s), before, "{check}: undo restores the layer and drops the backup");
        run(&mut s, "edit.redo", json!({}));
    }
}

#[test]
fn results_are_kept_and_fixed_by_id() {
    let mut s = Session::with_document();
    assert!(s.execute("audit.results", &json!({})).is_err(), "nothing has run yet");
    overlapping_squares(&mut s, "A");

    let doc = run(&mut s, "audit.run", json!({"checks": ["check.outlines.overlap"], "glyphs": ["A"]}));
    assert_eq!(doc["schema"], json!("fontcraft.audit/1"));
    assert_eq!(doc["summary"]["warn"], json!(1));
    let r = &doc["results"][0];
    assert_eq!(r["id"], json!("r1"));
    assert_eq!(r["location"]["glyph"], json!("A"));
    assert_eq!(r["location"]["master"], json!("m01"));
    assert_eq!(r["location"]["shapes"], json!([0, 1]));
    assert_eq!(r["fix"]["available"], json!(true));
    assert_eq!(r["fix"]["params"], json!({"results": ["r1"]}));
    assert_eq!(r["stale"], json!(false));

    // A dry run plans and changes nothing.
    let before = font_json(&s);
    let dry = run(&mut s, "audit.fix", json!({"results": ["r1"], "dryRun": true}));
    assert_eq!(dry["planned"][0]["command"], json!("path.removeOverlap"));
    assert_eq!(dry["applied"], json!([]));
    assert_eq!(font_json(&s), before);

    assert!(s.execute("audit.fix", &json!({"results": ["r9"]})).is_err(), "an unknown id");
    assert!(s.execute("audit.fix", &json!({"results": ["r1"], "checks": ["*"]})).is_err(), "ids and filters together");

    let steps = undo_steps(&s);
    let fixed = run(&mut s, "audit.fix", json!({"results": ["r1"]}));
    assert_eq!(fixed["applied"][0]["command"], json!("path.removeOverlap"));
    assert_eq!(undo_steps(&s), steps + 1);

    let kept = run(&mut s, "audit.results", json!({}));
    assert_eq!(kept["results"][0]["stale"], json!(true), "the glyph changed since the run");
    let again = run(&mut s, "audit.fix", json!({"results": ["r1"]}));
    assert_eq!(again["applied"], json!([]));
    assert!(again["skipped"][0]["reason"].as_str().unwrap().contains("changed"), "{again}");

    // A design-changing fix waits for safeOnly: false.
    add(&mut s, "E", lines(&[(0.0, 0.0), (0.0, 100.0), (100.0, 100.0), (100.0, 0.0)]), true);
    let before = font_json(&s);
    let safe = run(&mut s, "audit.fix", json!({"checks": ["check.outlines.pathDirection"], "glyphs": ["E"]}));
    assert_eq!(safe["applied"], json!([]));
    assert!(safe["skipped"][0]["reason"].as_str().unwrap().contains("safeOnly"), "{safe}");
    assert_eq!(font_json(&s), before);
}

#[test]
fn max_results_collapses_a_capped_check_but_not_its_fix() {
    let mut s = Session::with_document();
    let glyphs = ["a", "b", "c", "d", "e"];
    for g in glyphs {
        square_with_extra_node(&mut s, g);
    }
    let select = |max: Option<u64>| {
        let mut p = json!({"checks": ["check.outlines.collinearLines"], "glyphs": glyphs});
        if let Some(max) = max {
            p["maxResults"] = json!(max);
        }
        p
    };
    let doc = run(&mut s, "audit.run", select(None));
    assert_eq!(count(&doc, "check.outlines.collinearLines"), 5);
    let doc = run(&mut s, "audit.run", select(Some(5)));
    assert_eq!(count(&doc, "check.outlines.collinearLines"), 5, "five is not more than five");

    let doc = run(&mut s, "audit.run", select(Some(3)));
    let results = doc["results"].as_array().unwrap();
    assert_eq!(results.len(), 1, "{doc}");
    assert_eq!(results[0]["code"], json!("too-many-results"));
    assert_eq!(results[0]["severity"], json!("info"));
    assert_eq!(results[0]["data"]["count"], json!(5.0));

    let fixed = run(&mut s, "audit.fix", select(Some(3)));
    assert_eq!(fixed["applied"].as_array().unwrap().len(), 5, "every glyph is fixed: {fixed}");
    assert_eq!(count(&fixed["remaining"], "check.outlines.collinearLines"), 0);
}

#[test]
fn hostile_patterns_and_settings_are_errors() {
    let mut s = Session::with_document();
    overlapping_squares(&mut s, "A");
    let before = font_json(&s);
    let bad = [
        json!({"checks": ["x".repeat(10_000)]}),
        json!({"checks": ["check.outlines.["]}),
        json!({"checks": ["check.*.overlap"]}),
        json!({"checks": [""]}),
        json!({"checks": [1]}),
        json!({"checks": vec!["*"; 2_000]}),
        json!({"exclude": ["**nothing**"]}),
        json!({"groups": ["nope"]}),
        json!({"scope": ["cloud"]}),
        json!({"profile": "everything"}),
        json!({"minSeverity": "loud"}),
        json!({"maxResults": -1}),
        json!({"maxResults": 1.5}),
        json!({"config": {"check.outlines.flatCurve": {"tolerance": 1e9}}}),
        json!({"config": {"check.outlines.flatCurve": {"nope": 1}}}),
        json!({"config": {"check.outlines.flatCurve": 3}}),
        json!({"glyphs": ["no-such-glyph"]}),
        json!({"masters": ["m99"]}),
        json!({"target": "/etc/passwd"}),
    ];
    for p in &bad {
        assert!(s.execute("audit.run", p).is_err(), "audit.run {p}");
        assert!(s.execute("audit.fix", p).is_err(), "audit.fix {p}");
    }
    assert_eq!(font_json(&s), before);
    run(&mut s, "audit.run", json!({}));
    for p in [json!({"checks": ["check.outlines.["]}), json!({"glyphs": vec!["x"; 200_000]}), json!({"minSeverity": 3})] {
        assert!(s.execute("audit.results", &p).is_err(), "audit.results {p}");
    }

    // `*` is every check and a single string is a list of one.
    let all = run(&mut s, "audit.run", json!({"checks": ["*"], "glyphs": ["A"]}));
    assert_eq!(count(&all, "check.outlines.overlap"), 1);
    let one = run(&mut s, "audit.run", json!({"checks": "check.outlines.overlap", "glyphs": ["A"]}));
    assert_eq!(one["results"].as_array().unwrap().len(), 1);
    let none = run(&mut s, "audit.run", json!({"exclude": ["*"]}));
    assert_eq!(none["results"], json!([]));
    assert_eq!(none["checks"], json!([]));
    let ok = run(&mut s, "audit.run", json!({"checks": ["check.outlines.flatCurve"], "config": {"check.outlines.flatCurve": {"tolerance": 1}}}));
    assert_eq!(ok["config"], json!({"check.outlines.flatCurve": {"tolerance": 1}}));
}

#[test]
fn a_locked_glyph_is_skipped_and_left_alone() {
    let mut s = Session::with_document();
    overlapping_squares(&mut s, "A");
    overlapping_squares(&mut s, "B");
    run(&mut s, "glyph.toggleLocked", json!({"glyphs": ["A"], "locked": true}));
    let before = s.font().unwrap().glyph("A").cloned().unwrap();
    let steps = undo_steps(&s);

    let r = run(&mut s, "audit.fix", json!({"checks": ["check.outlines.overlap"], "glyphs": ["A", "B"]}));
    assert_eq!(r["skipped"][0]["glyph"], json!("A"));
    assert!(r["skipped"][0]["reason"].as_str().unwrap().contains("locked"), "{r}");
    assert_eq!(r["applied"][0]["glyph"], json!("B"));
    assert_eq!(s.font().unwrap().glyph("A").unwrap().layers, before.layers, "no fix and no backup layer");
    assert_eq!(count(&r["remaining"], "check.outlines.overlap"), 1, "A still overlaps");
    assert_eq!(undo_steps(&s), steps + 1);

    // Only the locked glyph: nothing changes, no undo step.
    let r = run(&mut s, "audit.fix", json!({"checks": ["check.outlines.overlap"], "glyphs": ["A"]}));
    assert_eq!(r["applied"], json!([]));
    assert_eq!(undo_steps(&s), steps + 1);
    assert!(matches!(s.execute("path.removeOverlap", &json!({"glyph": "A"})), Err(EngineError::Locked(_))));
}

#[test]
fn the_registry_is_listed_and_described() {
    let mut s = Session::with_document();
    let list = run(&mut s, "audit.list", json!({}));
    let checks = list["checks"].as_array().unwrap();
    assert!(checks.iter().all(|c| c["id"].as_str().unwrap().starts_with("check.outlines.")));
    let by_id = |id: &str| checks.iter().find(|c| c["id"] == id).unwrap().clone();
    assert_eq!(by_id("check.outlines.missingExtremes")["fixAvailable"], json!(true));
    assert_eq!(by_id("check.outlines.tinyContour")["fixAvailable"], json!(false), "no command deletes contours yet");
    assert_eq!(by_id("check.outlines.noContours")["fixable"], json!(false));
    let fixable = run(&mut s, "audit.list", json!({"fixable": true}));
    assert!(fixable["checks"].as_array().unwrap().iter().all(|c| c["fixable"] == json!(true)));

    let d = run(&mut s, "audit.describe", json!({"check": "check.outlines.flatCurve"}));
    assert_eq!(d["thresholds"][0]["name"], json!("tolerance"));
    assert_eq!(d["profiles"], json!(["outlines", "universal", "strict"]));
    assert_eq!(d["fix"]["command"], json!("path.tidyUpPaths"));
    assert!(s.execute("audit.describe", &json!({"check": "check.outlines.*"})).is_err());

    let profiles = run(&mut s, "audit.profile.list", json!({}));
    let p = profiles["profiles"].as_array().unwrap();
    assert_eq!(p.len(), 4);
    let n = |id: &str| p.iter().find(|x| x["id"] == id).unwrap()["checks"].as_u64().unwrap();
    assert_eq!(n("strict") as usize, checks.len());
    assert!(n("outlines") <= n("universal") && n("universal") <= n("strict"));
    assert_eq!(p.iter().filter(|x| x["default"] == json!(true)).count(), 1);
}
