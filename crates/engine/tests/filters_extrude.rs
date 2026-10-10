//! Extrude (M3.12): `filter.extrude`, its export string through `filter.apply`, and hostile
//! parameters.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use fontcraft_engine::Session;
use fontcraft_testkit::{oval, rect};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn shapes(s: &mut Session, glyph: &str) -> Value {
    run(s, "layer.inspect", json!({"glyph": glyph}))["shapes"].clone()
}

/// Every node of every path of a glyph's first master layer, as (x, y).
fn points(s: &mut Session, glyph: &str) -> Vec<(f64, f64)> {
    shapes(s, glyph)
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|shape| shape["nodes"].as_array().cloned().unwrap_or_default())
        .map(|n| (n["x"].as_f64().unwrap(), n["y"].as_f64().unwrap()))
        .collect()
}

/// (min x, min y, max x, max y) of a glyph's nodes.
fn bounds(s: &mut Session, glyph: &str) -> (f64, f64, f64, f64) {
    points(s, glyph)
        .into_iter()
        .fold((f64::MAX, f64::MAX, f64::MIN, f64::MIN), |(x0, y0, x1, y1), (x, y)| (x0.min(x), y0.min(y), x1.max(x), y1.max(y)))
}

fn font(s: &Session) -> Value {
    serde_json::to_value(s.font().unwrap()).unwrap()
}

/// `A`: a 200 × 700 bar from (100, 0); `B`: a 100-unit square; `O`: a circle of radius 200
/// around (250, 250).
fn fixture() -> Session {
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(100.0, 0.0, 300.0, 700.0)}));
    run(&mut s, "path.addPath", json!({"glyph": "B", "nodes": rect(0.0, 0.0, 100.0, 100.0)}));
    run(&mut s, "path.addPath", json!({"glyph": "O", "nodes": oval(250.0, 250.0, 200.0, 200.0)}));
    s
}

#[test]
fn extrude_leaves_the_shadow_or_keeps_the_original_area() {
    let mut s = fixture();
    let v = run(&mut s, "filter.extrude", json!({"glyph": "B", "offset": 50, "angle": 0}));
    assert_eq!(v["glyphs"], json!(["B"]));
    assert_eq!(v["string"], "Extrude;50;0;0;");
    assert_eq!(bounds(&mut s, "B"), (100.0, 0.0, 150.0, 100.0));
    run(&mut s, "edit.undo", json!({}));

    // Straight up, with the original kept: the square grows to 150 high.
    run(&mut s, "filter.extrude", json!({"glyph": "B", "offset": 50, "angle": 90, "dontSubtract": true}));
    assert_eq!(bounds(&mut s, "B"), (0.0, 0.0, 100.0, 150.0));

    // A curved shape: the shadow lies on the side of the sweep, every point on the grid.
    run(&mut s, "filter.extrude", json!({"glyph": "O", "offset": 100, "angle": -45}));
    let (x0, y0, x1, y1) = bounds(&mut s, "O");
    assert!(x0 > 50.0 && y1 < 450.0 && x1 > 450.0 && y0 < 50.0, "{:?}", (x0, y0, x1, y1));
    assert!(points(&mut s, "O").iter().all(|(x, y)| x.fract() == 0.0 && y.fract() == 0.0));
}

#[test]
fn the_export_string_and_the_parameters_give_the_same_font() {
    let forms = [
        ("filter.extrude", json!({"glyphs": ["A", "O"], "offset": 40, "angle": -30, "dontSubtract": true})),
        ("filter.apply", json!({"glyphs": ["A", "O"], "string": "Extrude; 40; -30; 1"})),
        ("filter.apply", json!({"glyphs": ["A", "O"], "filter": "extrude", "args": ["40", "-30", "1"]})),
        ("filter.apply", json!({"glyphs": ["A", "O"], "filter": "filter.extrude", "args": {"offset": 40, "angle": -30, "dontSubtract": true}})),
        ("filter.apply", json!({"string": "Extrude;40;-30;1;include:A,O;"})),
        ("filter.apply", json!({"string": "Extrude;40;-30;1;", "exclude": ["B"]})),
    ];
    let untouched = shapes(&mut fixture(), "B");
    let original = shapes(&mut fixture(), "A");
    let mut results = Vec::new();
    for (id, params) in &forms {
        let mut s = fixture();
        run(&mut s, id, params.clone());
        results.push((shapes(&mut s, "A"), shapes(&mut s, "O"), shapes(&mut s, "B")));
    }
    assert_ne!(results[0].0, original, "the filter changed A");
    for (i, r) in results.iter().enumerate() {
        assert_eq!(r, &results[0], "form {i}: {}", forms[i].1);
        assert_eq!(r.2, untouched, "form {i} left B alone");
    }
}

#[test]
fn hostile_parameters_and_strings_are_errors_that_change_nothing() {
    let mut s = fixture();
    let before = font(&s);
    let params = [
        json!({"glyph": "B", "offset": -1, "angle": 0}),
        json!({"glyph": "B", "offset": "abc", "angle": 0}),
        json!({"glyph": "B", "offset": 1e7, "angle": 0}),
        json!({"glyph": "B", "offset": 50, "angle": 400}),
        json!({"glyph": "B", "offset": 50}),
        json!({"glyph": "B", "angle": 0}),
        json!({"glyph": "B", "offset": 50, "angle": 0, "dontSubtract": 2}),
        // The sweep would carry the bar beyond the coordinate limit.
        json!({"glyph": "A", "offset": 1_000_000, "angle": 0}),
        json!({"glyphs": ["B", "A"], "offset": 1_000_000, "angle": 0, "dontSubtract": true}),
    ];
    for p in params {
        assert!(s.execute("filter.extrude", &p).is_err(), "{p}");
        assert_eq!(font(&s), before, "{p} changed the font");
    }
    for text in [
        "Extrude",
        "Extrude;50",
        "Extrude;NaN;0",
        "Extrude;50;inf",
        "Extrude;-5;0",
        "Extrude;50;0;2",
        "Extrude;50;0;0;9",
        "Extrude;Offset:50;0",
        "Extrude;1000000;0",
    ] {
        assert!(s.execute("filter.apply", &json!({"string": text})).is_err(), "{text}");
        assert_eq!(font(&s), before, "{text} changed the font");
    }
}
