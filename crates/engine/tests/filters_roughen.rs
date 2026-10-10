//! Roughen (M3.12): `filter.roughen` and the `Roughenizer` export string.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use fontcraft_engine::Session;
use fontcraft_testkit::rect;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn shapes(s: &mut Session, glyph: &str) -> Value {
    run(s, "layer.inspect", json!({"glyph": glyph}))["shapes"].clone()
}

fn nodes(s: &mut Session, glyph: &str) -> Vec<Value> {
    shapes(s, glyph)[0]["nodes"].as_array().unwrap().clone()
}

fn xy(n: &Value) -> (f64, f64) {
    (n["x"].as_f64().unwrap(), n["y"].as_f64().unwrap())
}

fn font(s: &Session) -> Value {
    serde_json::to_value(s.font().unwrap()).unwrap()
}

/// `A`: a 200 × 700 bar from (100, 0); `B`: a 100-unit square; `O`: an oval.
fn fixture() -> Session {
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(100.0, 0.0, 300.0, 700.0)}));
    run(&mut s, "path.addPath", json!({"glyph": "B", "nodes": rect(0.0, 0.0, 100.0, 100.0)}));
    run(&mut s, "path.addPath", json!({"glyph": "O", "nodes": fontcraft_testkit::oval(300.0, 300.0, 200.0, 200.0)}));
    s
}

fn roughened(params: Value) -> Session {
    let mut s = fixture();
    run(&mut s, "filter.roughen", params);
    s
}

#[test]
fn roughen_cuts_every_segment_and_moves_every_node_within_the_limits() {
    // Without moves: the 900-unit outline of the bar in pieces of 100 units, all on its edges.
    let mut flat = roughened(json!({"glyph": "A", "segmentLength": 100, "horizontal": 0, "vertical": 0}));
    let base = nodes(&mut flat, "A");
    assert_eq!(base.len(), 18);
    assert!(base.iter().all(|n| n["type"] != "offcurve" && n["type"] != "curve"));
    for n in &base {
        let (x, y) = xy(n);
        assert!(x == 100.0 || x == 300.0 || y == 0.0 || y == 700.0, "({x}, {y}) is off the outline");
    }
    assert!(base.iter().any(|n| xy(n) == (100.0, 0.0)) && base.iter().any(|n| xy(n) == (200.0, 0.0)));

    // With moves every node stays within the limits (plus half a unit for the grid).
    let mut moved = roughened(json!({"glyph": "A", "segmentLength": 100, "horizontal": 12, "vertical": 4, "seed": 5}));
    let out = nodes(&mut moved, "A");
    assert_eq!(out.len(), base.len());
    let mut changed = 0;
    for (n, b) in out.iter().zip(&base) {
        let ((x, y), (bx, by)) = (xy(n), xy(b));
        assert!((x - bx).abs() <= 12.5 && (y - by).abs() <= 4.5, "({x}, {y}) from ({bx}, {by})");
        assert!(x.fract() == 0.0 && y.fract() == 0.0, "off the grid");
        changed += usize::from((x, y) != (bx, by));
    }
    assert!(changed > 9, "only {changed} of 18 nodes moved");

    // Curves become straight pieces.
    let mut oval = roughened(json!({"glyph": "O", "segmentLength": 60, "horizontal": 0, "vertical": 0}));
    let out = nodes(&mut oval, "O");
    assert!(out.len() > 20 && out.iter().all(|n| n["type"] != "offcurve" && n["type"] != "curve"));
}

#[test]
fn one_seed_gives_one_font() {
    let params = |seed: u32| json!({"glyphs": ["A", "O"], "segmentLength": 80, "horizontal": 15, "vertical": 15, "seed": seed});
    let mut first = roughened(params(9));
    let mut second = roughened(params(9));
    let mut other = roughened(params(10));
    let get = |s: &mut Session| (shapes(s, "A"), shapes(s, "O"));
    assert_eq!(get(&mut first), get(&mut second));
    assert_eq!(font(&first), font(&second));
    assert_ne!(get(&mut first), get(&mut other));
    // The seed defaults to 0.
    let plain = json!({"glyph": "A", "segmentLength": 80, "horizontal": 15, "vertical": 15});
    let (mut a, mut b) =
        (roughened(plain.clone()), roughened(json!({"glyph": "A", "segmentLength": 80, "horizontal": 15, "vertical": 15, "seed": 0})));
    assert_eq!(shapes(&mut a, "A"), shapes(&mut b, "A"));
}

#[test]
fn the_export_string_and_the_parameters_give_the_same_font() {
    let forms = [
        ("filter.roughen", json!({"glyphs": ["A", "O"], "segmentLength": 100, "horizontal": 10, "vertical": 5})),
        ("filter.roughen", json!({"glyphs": ["A", "O"], "segmentLength": 100, "horizontal": 10, "vertical": 5, "angle": 0, "seed": 0})),
        ("filter.apply", json!({"glyphs": ["A", "O"], "string": "Roughenizer; 100; 10; 5"})),
        ("filter.apply", json!({"glyphs": ["A", "O"], "string": "roughenizer;100;10;5;0;"})),
        ("filter.apply", json!({"glyphs": ["A", "O"], "filter": "Roughenizer", "args": ["100", "10", "5"]})),
        ("filter.apply", json!({"glyphs": ["A", "O"], "filter": "filter.roughen", "args": {"segmentLength": 100, "horizontal": 10, "vertical": 5}})),
        ("filter.apply", json!({"string": "Roughenizer;100;10;5;0;include:A,O;"})),
        ("filter.apply", json!({"string": "Roughenizer;100;10;5;", "exclude": ["B"]})),
    ];
    let mut results = Vec::new();
    for (id, params) in &forms {
        let mut s = fixture();
        let v = run(&mut s, id, params.clone());
        if *id == "filter.roughen" {
            assert_eq!(v["string"], "Roughenizer;100;10;5;0;");
        }
        results.push((shapes(&mut s, "A"), shapes(&mut s, "O"), shapes(&mut s, "B")));
    }
    let untouched = {
        let mut s = fixture();
        (shapes(&mut s, "A"), shapes(&mut s, "B"))
    };
    assert_ne!(results[0].0, untouched.0, "the filter changed A");
    for (i, r) in results.iter().enumerate() {
        assert_eq!(r, &results[0], "form {i}: {}", forms[i].1);
        assert_eq!(r.2, untouched.1, "form {i} left B alone");
    }
    // A seed given as an argument matches the command with that seed; the string cannot carry one.
    let mut a = fixture();
    run(
        &mut a,
        "filter.apply",
        json!({"glyph": "A", "filter": "Roughenizer", "args": {"segmentLength": 100, "horizontal": 10, "vertical": 5, "seed": 3}}),
    );
    let mut b = roughened(json!({"glyph": "A", "segmentLength": 100, "horizontal": 10, "vertical": 5, "seed": 3}));
    assert_eq!(shapes(&mut a, "A"), shapes(&mut b, "A"));
    assert_ne!(shapes(&mut a, "A"), results[0].0);
}

#[test]
fn the_angle_turns_the_box_of_moves() {
    let mut flat = roughened(json!({"glyph": "A", "segmentLength": 100, "horizontal": 0, "vertical": 0}));
    let base = nodes(&mut flat, "A");
    let mut s = roughened(json!({"glyph": "A", "segmentLength": 100, "horizontal": 12, "vertical": 2, "angle": 90, "seed": 1}));
    for (n, b) in nodes(&mut s, "A").iter().zip(&base) {
        let ((x, y), (bx, by)) = (xy(n), xy(b));
        assert!((x - bx).abs() <= 2.5 && (y - by).abs() <= 12.5, "({x}, {y}) from ({bx}, {by})");
    }
    let mut via_string = fixture();
    run(&mut via_string, "filter.apply", json!({"glyph": "A", "string": "Roughenizer;100;12;2;90"}));
    let mut via_command = roughened(json!({"glyph": "A", "segmentLength": 100, "horizontal": 12, "vertical": 2, "angle": 90}));
    assert_eq!(shapes(&mut via_string, "A"), shapes(&mut via_command, "A"));
}

#[test]
fn filter_list_describes_roughen() {
    let mut s = fixture();
    let v = run(&mut s, "filter.list", json!({}));
    let f = v["filters"].as_array().unwrap().iter().find(|f| f["name"] == "Roughenizer").unwrap().clone();
    assert_eq!(f["command"], "filter.roughen");
    assert_eq!(f["label"], "Roughen");
    assert_eq!(f["selection"], false);
    assert_eq!(f["string"], "Roughenizer;50;10;10;0;");
    let params = f["params"].as_array().unwrap();
    assert_eq!(params.iter().map(|p| p["key"].as_str().unwrap()).collect::<Vec<_>>(), ["segmentLength", "horizontal", "vertical", "angle", "seed"]);
    assert_eq!(params[4]["kind"], "integer");
    assert_eq!(params[4]["slot"], Value::Null);
}

#[test]
fn hostile_parameters_and_strings_are_errors_that_change_nothing() {
    let mut s = fixture();
    run(&mut s, "path.addPath", json!({"glyph": "H", "nodes": rect(0.0, 0.0, 200_000.0, 200_000.0)}));
    let before = font(&s);
    let ok = json!({"glyph": "A", "segmentLength": 50, "horizontal": 5, "vertical": 5});
    let with = |key: &str, value: Value| {
        let mut p = ok.clone();
        p[key] = value;
        p
    };
    let params = [
        with("segmentLength", json!(0)),
        with("segmentLength", json!(0.5)),
        with("segmentLength", json!(-3)),
        with("segmentLength", json!("abc")),
        with("segmentLength", json!(1e12)),
        with("horizontal", json!(-1)),
        with("horizontal", json!(1e12)),
        with("vertical", json!(-0.1)),
        with("angle", json!(361)),
        with("seed", json!(-1)),
        with("seed", json!(1.5)),
        with("seed", json!(1e300)),
        json!({"glyph": "A", "horizontal": 5, "vertical": 5}),
        json!({"glyph": "A", "segmentLength": 50, "vertical": 5}),
        json!({"glyph": "Missing", "segmentLength": 50, "horizontal": 5, "vertical": 5}),
        // 800000 units of outline in pieces of 1 unit: more nodes than a layer may hold.
        json!({"glyph": "H", "segmentLength": 1, "horizontal": 0, "vertical": 0}),
    ];
    for p in params {
        assert!(s.execute("filter.roughen", &p).is_err(), "{p}");
        assert_eq!(font(&s), before, "{p} changed the font");
    }
    let strings = [
        "Roughenizer",
        "Roughenizer;50",
        "Roughenizer;50;10",
        "Roughenizer;0;10;10",
        "Roughenizer;NaN;10;10",
        "Roughenizer;50;inf;10",
        "Roughenizer;50;-1;10",
        "Roughenizer;50;10;10;400",
        "Roughenizer;50;10;10;0;1",
        "Roughenizer;50;10;10;seed:3",
        "Roughenizer;SegmentLength:50;10;10",
    ];
    for text in strings {
        assert!(s.execute("filter.apply", &json!({"glyph": "A", "string": text})).is_err(), "{text}");
        assert_eq!(font(&s), before, "{text} changed the font");
    }
}
