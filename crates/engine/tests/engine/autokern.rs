//! Auto-kerning, Kern to Distance, group building, pair rules and vertical kerning (M13.21):
//! exact gaps on simple shapes, the auto-kerning options, groups found from side profiles,
//! removal by rule, vertical pairs from horizontal ones; one undo step each, and refusals that
//! change nothing.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::{EngineError, Session};
use fontcraft_testkit::{oval, rect, snapshot};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// Runs a command that must fail, and checks that the font is the very same one.
fn refused(s: &mut Session, id: &str, p: Value) -> String {
    let before = snapshot(s);
    let font = s.doc().unwrap().font.clone();
    let e: EngineError = s.execute(id, &p).err().unwrap_or_else(|| panic!("{id} {p} should fail"));
    assert!(Arc::ptr_eq(&font, &s.doc().unwrap().font), "{id} failed but replaced the font");
    assert_eq!(snapshot(s), before, "{id} failed but changed the session");
    e.to_string()
}

/// Runs a command, checks it is one undo step that gives back the font, and redoes it.
fn undo_restores(s: &mut Session, id: &str, p: Value) -> Value {
    let before = s.font().unwrap().clone();
    let steps = s.doc().unwrap().history.undo.len();
    let out = run(s, id, p);
    assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1, "{id} is one undo step");
    run(s, "edit.undo", json!({}));
    assert_eq!(s.font().unwrap(), &before, "undo after {id} gives back the font");
    run(s, "edit.redo", json!({}));
    out
}

fn draw(s: &mut Session, glyph: &str, paths: &[Value], width: f64) {
    for nodes in paths {
        run(s, "path.addPath", json!({"glyph": glyph, "nodes": nodes}));
    }
    run(s, "spacing.setWidth", json!({"glyphs": [glyph], "value": width}));
}

/// The left-to-right pair stored in the first master under these spellings.
fn kern(s: &Session, left: &str, right: &str) -> Option<f64> {
    s.font().unwrap().kerning.ltr.get("m01", left, right)
}

/// A triangle standing on its tip: top corners at `x` and `x + 400` on height 500, the tip at
/// `x + 200` on the baseline. Its left side reaches `x` only at the top.
fn v_shape(x: f64) -> Value {
    json!([{"x": x, "y": 500}, {"x": x + 400.0, "y": 500}, {"x": x + 200.0, "y": 0}])
}

#[test]
fn kerning_kern_to_distance_gives_the_exact_gap() {
    let mut s = Session::with_document();
    draw(&mut s, "l", &[rect(50.0, 0.0, 150.0, 700.0)], 200.0);
    draw(&mut s, "i", &[rect(30.0, 0.0, 130.0, 500.0)], 160.0);
    draw(&mut s, "v", &[v_shape(10.0)], 420.0);
    let out = undo_restores(&mut s, "kerning.kernToDistance", json!({"pairs": [["l", "i"], ["l", "v"]], "distance": 20}));
    // l and i stand 50 + 30 = 80 apart at every height.
    assert_eq!(kern(&s, "l", "i"), Some(-60.0));
    // l and v come closest at v's top corner: 50 + 10 = 60, found between two sampled rows.
    assert_eq!(kern(&s, "l", "v"), Some(-40.0));
    let gap = out["pairs"][1]["gap"].as_f64().unwrap();
    assert!((gap - 60.0).abs() < 0.01, "{out}");
    assert_eq!(out["pairs"][0]["previous"], Value::Null);

    // Without pairs, every stored pair is set again; with apply false nothing is stored.
    let out = run(&mut s, "kerning.kernToDistance", json!({"distance": 0, "apply": false}));
    let values: Vec<f64> = out["pairs"].as_array().unwrap().iter().map(|p| p["value"].as_f64().unwrap()).collect();
    assert_eq!(values, [-80.0, -60.0], "{out}");
    assert_eq!(kern(&s, "l", "i"), Some(-60.0));

    assert!(refused(&mut s, "kerning.kernToDistance", json!({"pairs": [["l", "i"]]})).contains("distance"));
    refused(&mut s, "kerning.kernToDistance", json!({"pairs": [["l", "i"]], "distance": 10, "direction": "vertical"}));
    refused(&mut s, "kerning.kernToDistance", json!({"pairs": [["l"]], "distance": 10}));
    refused(&mut s, "kerning.kernToDistance", json!({"pairs": [["l", "nosuch"]], "distance": 10}));
    refused(&mut s, "kerning.kernToDistance", json!({"pairs": [], "distance": 10}));
}

#[test]
fn kerning_auto_kern_targets_the_reference_gap_with_its_options() {
    // x-height 500 with 16 units of overshoot; clipping depth 150. n has 60 units each side, so
    // the reference gap of n n is 120.
    let mut s = Session::with_document();
    draw(&mut s, "n", &[rect(60.0, 0.0, 440.0, 500.0)], 500.0);
    draw(&mut s, "i", &[rect(20.0, 0.0, 80.0, 500.0)], 100.0);
    draw(&mut s, "v", &[v_shape(30.0)], 460.0);
    let pairs = json!([["n", "i"], ["n", "v"]]);
    let out = undo_restores(&mut s, "kerning.autoKern", json!({"pairs": pairs}));
    assert_eq!(out["targets"]["m01"], json!({"lowercase": 120.0, "capitals": 120.0}), "without H, capitals take the lowercase target");
    // n i: 60 + 20 = 80 of white, so 40 looser. n v: the rows run from 90 units apart at the
    // v's top corners to 240 where the clipping stops (a plain mean of 178); leaning toward the
    // closest rows, the gap is 160, so 40 tighter.
    assert_eq!(kern(&s, "n", "i"), Some(40.0));
    assert_eq!(kern(&s, "n", "v"), Some(-40.0), "{out}");
    assert_eq!(out["pairs"][0]["result"], "set");
    let gap = out["pairs"][1]["gap"].as_f64().unwrap();
    assert!((gap - 160.08).abs() < 0.01, "{out}");

    // Stored pairs are kept by default.
    let out = run(&mut s, "kerning.autoKern", json!({"pairs": [["n", "i"]], "separation": 100}));
    assert_eq!(out["pairs"][0]["result"], "kept");
    assert_eq!(kern(&s, "n", "i"), Some(40.0));
    // An explicit separation replaces the reference gap: 100 − 80.
    run(&mut s, "kerning.autoKern", json!({"pairs": [["n", "i"]], "separation": 100, "keepExisting": false}));
    assert_eq!(kern(&s, "n", "i"), Some(20.0));
    // Only closer: the loosening pair is dropped, and its stored pair removed.
    let out = run(&mut s, "kerning.autoKern", json!({"pairs": pairs, "onlyCloser": true, "keepExisting": false}));
    assert_eq!((out["pairs"][0]["result"].as_str(), out["pairs"][1]["result"].as_str()), (Some("removed"), Some("set")));
    assert_eq!((kern(&s, "n", "i"), kern(&s, "n", "v")), (None, Some(-40.0)));
    // Minimum value: against a separation of 100, n i's 20 is smaller in size than 35.
    run(&mut s, "kerning.autoKern", json!({"pairs": pairs, "separation": 100, "minKern": 35, "keepExisting": false}));
    assert_eq!((kern(&s, "n", "i"), kern(&s, "n", "v")), (None, Some(-60.0)));
    // Touching measures the closest approach: n and v come within 60 + 30 = 90 at v's top
    // corner, against 120 for n n, so the pair loosens. Apply false stores nothing.
    let out = run(&mut s, "kerning.autoKern", json!({"pairs": [["n", "v"]], "touching": true, "apply": false}));
    assert_eq!(out["pairs"][0]["value"], json!(30.0), "{out}");
    assert_eq!(kern(&s, "n", "v"), Some(-60.0), "the stored value stays");
    // Glyph lists make every pair of the two.
    let out = run(&mut s, "kerning.autoKern", json!({"glyphsLeft": ["n"], "glyphsRight": ["i", "v"], "apply": false, "keepExisting": false}));
    let values: Vec<f64> = out["pairs"].as_array().unwrap().iter().map(|p| p["value"].as_f64().unwrap()).collect();
    assert_eq!(values, [40.0, -40.0]);

    refused(&mut s, "kerning.autoKern", json!({}));
    refused(&mut s, "kerning.autoKern", json!({"glyphsLeft": ["n"]}));
    refused(&mut s, "kerning.autoKern", json!({"pairs": pairs, "direction": "vertical"}));
    refused(&mut s, "kerning.autoKern", json!({"pairs": pairs, "masters": ["nosuch"]}));
    refused(&mut s, "kerning.autoKern", json!({"pairs": pairs, "step": 0.1}));
    refused(&mut s, "kerning.autoKern", json!({"pairs": [["n", "@nosuch"]]}));
}

/// The left half of `oval(250, 250, 200, 250)`, closed by a vertical line at x 250.
fn left_bowl() -> Value {
    let k = 0.5523;
    let on = |x: f64, y: f64| json!({"x": x, "y": y, "type": "curve", "smooth": true});
    let off = |x: f64, y: f64| json!({"x": x, "y": y, "type": "offcurve"});
    json!([
        {"x": 250, "y": 500},
        off(250.0 - 200.0 * k, 500.0),
        off(50.0, 250.0 + 250.0 * k),
        on(50.0, 250.0),
        off(50.0, 250.0 - 250.0 * k),
        off(250.0 - 200.0 * k, 0.0),
        on(250.0, 0.0),
    ])
}

#[test]
fn kerning_build_groups_finds_round_and_straight_sides() {
    let mut s = Session::with_document();
    let o = oval(250.0, 250.0, 200.0, 250.0);
    draw(&mut s, "o", std::slice::from_ref(&o), 500.0);
    // e: the o with a bar inside, which no profile sees.
    draw(&mut s, "e", &[o, rect(60.0, 240.0, 440.0, 260.0)], 500.0);
    // c: the o's left half with two terminals on the right.
    draw(&mut s, "c", &[left_bowl(), rect(250.0, 400.0, 420.0, 480.0), rect(250.0, 20.0, 420.0, 100.0)], 470.0);
    // d and q: the same bowl with a stem on the right, rising above the x-height or falling
    // below the baseline; the parts in the overshoot zones are not compared.
    draw(&mut s, "d", &[left_bowl(), rect(250.0, 0.0, 390.0, 500.0), rect(390.0, 0.0, 450.0, 700.0)], 500.0);
    draw(&mut s, "q", &[left_bowl(), rect(250.0, 0.0, 390.0, 500.0), rect(390.0, -200.0, 450.0, 500.0)], 500.0);
    let p = json!({"glyphs": ["o", "e", "c", "d", "q"], "maxDifference": 10});
    let before = s.font().unwrap().clone();
    let out = run(&mut s, "kerning.buildGroups", p.clone());
    assert_eq!(s.font().unwrap(), &before, "without apply, only a proposal");
    // Glyph order: c, d, e, o, q. Every left side is the same bowl.
    assert_eq!(out["groups"]["left"], json!([{"name": "c", "glyphs": ["c", "d", "e", "o", "q"]}]), "{out}");
    // Right sides: stems with stems, rounds with rounds; c stays alone.
    assert_eq!(out["groups"]["right"], json!([{"name": "d", "glyphs": ["d", "q"]}, {"name": "e", "glyphs": ["e", "o"]}]), "{out}");

    let mut apply = p.clone();
    apply["apply"] = json!(true);
    let out = undo_restores(&mut s, "kerning.buildGroups", apply.clone());
    assert_eq!(out["glyphs"], json!(["c", "d", "e", "o", "q"]));
    let groups = |name: &str| {
        let g = &s.font().unwrap().glyph(name).unwrap().kern_groups;
        (g.left.clone(), g.right.clone())
    };
    assert_eq!(groups("c"), (Some("c".into()), None));
    assert_eq!(groups("q"), (Some("c".into()), Some("d".into())));
    assert_eq!(groups("o"), (Some("c".into()), Some("e".into())));
    // Again: every glyph keeps its groups, and c alone on the right gets none.
    let out = run(&mut s, "kerning.buildGroups", apply);
    assert_eq!(out["glyphs"], json!([]));
    assert_eq!(out["groups"], json!({"left": [], "right": []}));

    refused(&mut s, "kerning.buildGroups", json!({"glyphs": ["o"]}));
    refused(&mut s, "kerning.buildGroups", json!({"glyphs": ["o"], "maxDifference": 10, "side": "top"}));
    refused(&mut s, "kerning.buildGroups", json!({"glyphs": [], "maxDifference": 10}));
}

/// T's right group `T`, o and e's left group `o`; a group pair, two exceptions, a small pair.
fn rule_font() -> Session {
    let mut s = Session::with_document();
    run(&mut s, "kerning.setRightGroup", json!({"glyphs": ["T"], "group": "T"}));
    run(&mut s, "kerning.setLeftGroup", json!({"glyphs": ["o", "e"], "group": "o"}));
    for (left, right, value) in [("@T", "@o", -80), ("T", "e", -78), ("T", "o", -50), ("A", "V", -5), ("A", "W", 30)] {
        run(&mut s, "kerning.setPair", json!({"left": left, "right": right, "value": value}));
    }
    s
}

#[test]
fn kerning_remove_pairs_by_rule_by_size_and_class_value() {
    let mut s = rule_font();
    let out = undo_restores(&mut s, "kerning.removePairsByRule", json!({"smallerThan": 10, "exceptionsWithin": 5}));
    assert_eq!(
        out["removed"],
        json!([
            {"master": "m01", "left": "T", "right": "e", "value": -78.0, "rule": "exceptionsWithin", "classValue": -80.0},
            {"master": "m01", "left": "A", "right": "V", "value": -5.0, "rule": "smallerThan"},
        ]),
        "{out}"
    );
    assert_eq!(kern(&s, "T", "e"), None);
    assert_eq!(kern(&s, "T", "o"), Some(-50.0), "30 units from its class value");
    assert_eq!(kern(&s, "@MMK_L_T", "@MMK_R_o"), Some(-80.0), "a group pair is not an exception");
    let applied = run(&mut s, "kerning.getPair", json!({"left": "T", "right": "e"}));
    assert_eq!(applied["value"], json!(-80.0));

    // Limited to pairs naming e or its group: T o stays although it is within 50.
    let mut s = rule_font();
    let out = run(&mut s, "kerning.removePairsByRule", json!({"exceptionsWithin": 50, "glyphs": ["e"]}));
    let removed: Vec<(&str, &str)> =
        out["removed"].as_array().unwrap().iter().map(|r| (r["left"].as_str().unwrap(), r["right"].as_str().unwrap())).collect();
    assert_eq!(removed, [("T", "e")]);
    assert_eq!(kern(&s, "T", "o"), Some(-50.0));

    refused(&mut s, "kerning.removePairsByRule", json!({}));
    refused(&mut s, "kerning.removePairsByRule", json!({"smallerThan": -1}));
    refused(&mut s, "kerning.removePairsByRule", json!({"smallerThan": 5, "glyphs": []}));
}

#[test]
fn kerning_vertical_from_horizontal_pairs_the_alternates() {
    let mut s = Session::with_document();
    for name in ["A.vert", "V.vert", "W.vert"] {
        run(&mut s, "glyph.newGlyph", json!({"name": name}));
    }
    run(&mut s, "kerning.setLeftGroup", json!({"glyphs": ["V", "W"], "group": "V"}));
    for (left, right, value) in [("A", "V", -60), ("A", "@V", -30), ("A", "T", -40)] {
        run(&mut s, "kerning.setPair", json!({"left": left, "right": right, "value": value}));
    }
    run(&mut s, "kerning.setPair", json!({"left": "X", "right": "Y", "value": 10, "direction": "vertical"}));
    let out = undo_restores(&mut s, "kerning.verticalFromHorizontal", json!({}));
    assert_eq!(out["masters"]["m01"], json!({"cleared": 1, "created": 2}), "{out}");
    let vertical = &s.font().unwrap().kerning.vertical;
    // A V keeps its exception; W takes the group value; T has no alternate; X Y is cleared.
    assert_eq!(vertical.get("m01", "A.vert", "V.vert"), Some(-60.0));
    assert_eq!(vertical.get("m01", "A.vert", "W.vert"), Some(-30.0));
    assert_eq!(vertical.get("m01", "X", "Y"), None);
    assert_eq!(vertical.pair_count(), 2);
    refused(&mut s, "kerning.verticalFromHorizontal", json!({"masters": ["nosuch"]}));
}
