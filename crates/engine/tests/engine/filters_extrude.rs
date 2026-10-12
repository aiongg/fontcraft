//! Extrude (M3.12): `filter.extrude`, `filter.apply` with named parameters, the refused export
//! string, and hostile parameters.
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
    assert!(v.get("string").is_none(), "Extrude has no export string: {v}");
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
fn filter_apply_with_parameters_matches_the_command() {
    let mut direct = fixture();
    run(&mut direct, "filter.extrude", json!({"glyphs": ["A", "O"], "offset": 40, "angle": -30, "dontSubtract": true}));
    let mut applied = fixture();
    run(&mut applied, "filter.apply", json!({"filter": "Extrude", "args": {"offset": 40, "angle": -30, "dontSubtract": true}, "exclude": ["B"]}));
    assert_ne!(shapes(&mut direct, "A"), shapes(&mut fixture(), "A"), "the filter changed A");
    for glyph in ["A", "O", "B"] {
        assert_eq!(shapes(&mut applied, glyph), shapes(&mut direct, glyph), "{glyph}");
    }
    assert_eq!(shapes(&mut applied, "B"), shapes(&mut fixture(), "B"));
}

/// No written source shows an export string for Extrude, so it has none: every string or
/// positional form is refused and leaves the font as it was.
#[test]
fn an_extrude_string_is_refused_and_changes_nothing() {
    let mut s = fixture();
    let before = font(&s);
    for params in [
        json!({"string": "Extrude; 40; -30; 1"}),
        json!({"glyphs": ["A", "O"], "string": "extrude;40;-30;1;include:A,O;"}),
        json!({"glyph": "A", "filter": "Extrude", "args": ["40", "-30", "1"]}),
        json!({"glyph": "A", "filter": "filter.extrude"}),
    ] {
        let e = s.execute("filter.apply", &params).unwrap_err().to_string();
        assert!(e.contains("no export string form"), "{params}: {e}");
        assert_eq!(font(&s), before, "{params} changed the font");
    }
    let list = run(&mut s, "filter.list", json!({}));
    let extrude = list["filters"].as_array().unwrap().iter().find(|f| f["name"] == "Extrude").unwrap();
    assert!(extrude.get("string").is_none(), "{extrude}");
}

#[test]
fn hostile_parameters_are_errors_that_change_nothing() {
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
}
