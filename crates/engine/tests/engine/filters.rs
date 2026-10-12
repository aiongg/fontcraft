//! Filters (M3.12): the filter registry, `filter.apply` with export strings, `filter.list`,
//! `filter.repeatLastFilter`, and Round Corners.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use fontcraft_engine::{Session, command_specs};
use fontcraft_testkit::rect;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn shapes(s: &mut Session, glyph: &str) -> Value {
    run(s, "layer.inspect", json!({"glyph": glyph}))["shapes"].clone()
}

/// The on-curve nodes of the first path of a glyph's first master layer.
fn on_curve(s: &mut Session, glyph: &str) -> Vec<(f64, f64)> {
    shapes(s, glyph)[0]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["type"] != "offcurve")
        .map(|n| (n["x"].as_f64().unwrap(), n["y"].as_f64().unwrap()))
        .collect()
}

/// The active font, for comparing before and after.
fn font(s: &Session) -> Value {
    serde_json::to_value(s.font().unwrap()).unwrap()
}

/// `A`: a 200 × 700 bar from (100, 0); `L`: an L, counter-clockwise, whose node 3 at (50, 50) is
/// the inward corner; `B`: a 100-unit square.
fn fixture() -> Session {
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(100.0, 0.0, 300.0, 700.0)}));
    let l = json!([{"x": 0, "y": 0}, {"x": 200, "y": 0}, {"x": 200, "y": 50}, {"x": 50, "y": 50}, {"x": 50, "y": 200}, {"x": 0, "y": 200}]);
    run(&mut s, "path.addPath", json!({"glyph": "L", "nodes": l}));
    run(&mut s, "path.addPath", json!({"glyph": "B", "nodes": rect(0.0, 0.0, 100.0, 100.0)}));
    s
}

#[test]
fn round_corners_rounds_outward_corners_or_exactly_the_selected_ones() {
    let mut s = fixture();
    let v = run(&mut s, "filter.roundCorners", json!({"glyph": "A", "radius": 20}));
    assert_eq!(v["glyphs"], json!(["A"]));
    assert_eq!(v["string"], "RoundCorner;20;0;");
    assert_eq!(
        on_curve(&mut s, "A"),
        vec![(100.0, 20.0), (120.0, 0.0), (280.0, 0.0), (300.0, 20.0), (300.0, 680.0), (280.0, 700.0), (120.0, 700.0), (100.0, 680.0)]
    );
    // Four arcs: two handles each, on the grid.
    let nodes = shapes(&mut s, "A")[0]["nodes"].as_array().unwrap().clone();
    assert_eq!(nodes.len(), 16);
    assert!(nodes.iter().all(|n| n["x"].as_f64().unwrap().fract() == 0.0 && n["y"].as_f64().unwrap().fract() == 0.0));

    // Without a selection the inward corner of the L stays.
    run(&mut s, "filter.roundCorners", json!({"glyph": "L", "radius": 10}));
    let l = on_curve(&mut s, "L");
    assert_eq!(l.len(), 11);
    assert!(l.contains(&(50.0, 50.0)));
    run(&mut s, "edit.undo", json!({}));

    // A selected node is rounded whichever way it points, and only it.
    run(&mut s, "filter.roundCorners", json!({"glyph": "L", "radius": 10, "points": [[0, 3]]}));
    assert_eq!(on_curve(&mut s, "L"), vec![(0.0, 0.0), (200.0, 0.0), (200.0, 50.0), (60.0, 50.0), (50.0, 60.0), (50.0, 200.0), (0.0, 200.0)]);
    run(&mut s, "edit.undo", json!({}));

    // A selected segment rounds both of its end nodes.
    run(&mut s, "filter.roundCorners", json!({"glyph": "L", "radius": 10, "segments": [[0, 3]]}));
    let l = on_curve(&mut s, "L");
    assert_eq!(l.len(), 8);
    assert!(!l.contains(&(50.0, 50.0)) && !l.contains(&(200.0, 50.0)));
}

#[test]
fn the_export_string_and_the_parameters_give_the_same_font() {
    let forms = [
        ("filter.roundCorners", json!({"glyphs": ["A", "L"], "radius": 30, "visualCorrections": true})),
        ("filter.apply", json!({"glyphs": ["A", "L"], "string": "RoundCorner; 30; 1"})),
        ("filter.apply", json!({"glyphs": ["A", "L"], "filter": "RoundCorner", "args": ["30", "1"]})),
        ("filter.apply", json!({"glyphs": ["A", "L"], "filter": "filter.roundCorners", "args": {"radius": 30, "visualCorrections": true}})),
        ("filter.apply", json!({"string": "RoundCorner;30;1;include:A,L;"})),
        ("filter.apply", json!({"string": "RoundCorner;30;1;", "exclude": ["B"]})),
    ];
    let mut results = Vec::new();
    for (id, params) in &forms {
        let mut s = fixture();
        run(&mut s, id, params.clone());
        results.push((shapes(&mut s, "A"), shapes(&mut s, "L"), shapes(&mut s, "B")));
    }
    let untouched = {
        let mut s = fixture();
        shapes(&mut s, "B")
    };
    assert_ne!(results[0].0, fixture_shapes("A"), "the filter changed A");
    for (i, r) in results.iter().enumerate() {
        assert_eq!(r, &results[0], "form {i}: {}", forms[i].1);
        assert_eq!(r.2, untouched, "form {i} left B alone");
    }
}

fn fixture_shapes(glyph: &str) -> Value {
    let mut s = fixture();
    shapes(&mut s, glyph)
}

#[test]
fn filter_apply_runs_transformations_through_its_command() {
    let text = "Transformations;ScaleX:50;OffsetY:10;Origin:4;";
    let mut a = fixture();
    let v = run(&mut a, "filter.apply", json!({"glyphs": ["A"], "string": text}));
    assert_eq!(v["command"], "path.transformations");
    let mut b = fixture();
    run(&mut b, "path.transformations", json!({"glyphs": ["A"], "string": text}));
    assert_eq!(shapes(&mut a, "A"), shapes(&mut b, "A"));
    assert_ne!(shapes(&mut a, "A"), fixture_shapes("A"));
}

#[test]
fn filter_list_names_every_filter_with_its_schema() {
    let mut s = fixture();
    let v = run(&mut s, "filter.list", json!({}));
    let filters = v["filters"].as_array().unwrap();
    let round = filters.iter().find(|f| f["name"] == "RoundCorner").unwrap();
    assert_eq!(round["command"], "filter.roundCorners");
    assert_eq!(round["label"], "Round Corners");
    assert_eq!(round["menu"], json!(["Filter"]));
    assert_eq!(round["selection"], true);
    assert_eq!(round["string"], "RoundCorner;20;0;");
    let params = round["params"].as_array().unwrap();
    assert_eq!(params.iter().map(|p| p["key"].as_str().unwrap()).collect::<Vec<_>>(), ["radius", "visualCorrections"]);
    assert_eq!(params[0]["kind"], "number");
    assert_eq!(params[0]["required"], true);
    assert_eq!(params[0]["slot"], json!({"position": 0}));
    assert_eq!(params[1]["kind"], "bool");
    assert_eq!(params[1]["default"], false);
    let transformations = filters.iter().find(|f| f["name"] == "Transformations").unwrap();
    assert_eq!(transformations["command"], "path.transformations");
    // Every listed filter has its command.
    for f in filters {
        let id = f["command"].as_str().unwrap();
        assert!(command_specs().iter().any(|c| c.id == id), "{id}");
    }
}

#[test]
fn hostile_strings_and_arguments_are_errors_that_change_nothing() {
    let mut s = fixture();
    let before = font(&s);
    let long = format!("RoundCorner;{}", "9".repeat(2 << 20));
    let many = "RoundCorner;20;".repeat(300);
    let strings = [
        "",
        ";;;",
        "Nope;1",
        "RoundCorner",
        "RoundCorner;NaN",
        "RoundCorner;1e999",
        "RoundCorner;-1",
        "RoundCorner;20;2",
        "RoundCorner;20;0;9",
        "RoundCorner;Radius:20",
        "RoundCorner;20;include:A;exclude:B",
        "RoundCorner;20;include:A;include:B",
        "RoundCorner;20;include:",
        "Transformations;LSB:+10",
        "Transformations;Slant:95",
        "Transformations;Bogus:1",
        long.as_str(),
        many.as_str(),
    ];
    for text in strings {
        assert!(s.execute("filter.apply", &json!({"string": text})).is_err(), "{:.60}", text);
        assert_eq!(font(&s), before, "{text:.60} changed the font");
    }
    let params = [
        ("filter.apply", json!({"filter": "RoundCorner", "args": {"radius": "abc"}})),
        ("filter.apply", json!({"filter": "RoundCorner", "args": {"radius": 20, "bogus": 1}})),
        ("filter.apply", json!({"filter": "RoundCorner", "args": ["20;1"]})),
        ("filter.apply", json!({"filter": "RoundCorner", "args": 20})),
        ("filter.apply", json!({"filter": "RoundCorner", "args": ["20"], "string": "RoundCorner;20"})),
        ("filter.apply", json!({"string": "RoundCorner;20;include:A", "include": ["B"]})),
        ("filter.roundCorners", json!({"glyph": "A", "radius": -1})),
        ("filter.roundCorners", json!({"glyph": "A"})),
        ("filter.roundCorners", json!({"glyph": "A", "radius": 20, "points": [[0, 99]]})),
        ("filter.roundCorners", json!({"glyphs": ["A"], "radius": 20, "points": [[0, 0]]})),
        ("filter.roundCorners", json!({"glyph": "A", "radius": 20, "shapes": [0]})),
        ("filter.roundCorners", json!({"glyph": "Missing", "radius": 20})),
    ];
    for (id, p) in params {
        assert!(s.execute(id, &p).is_err(), "{id} {p}");
        assert_eq!(font(&s), before, "{id} {p} changed the font");
    }
}

#[test]
fn repeat_last_filter_runs_the_last_filter_on_new_targets() {
    let mut s = fixture();
    assert!(s.execute("filter.repeatLastFilter", &json!({"glyph": "A"})).is_err(), "no filter has run yet");
    run(&mut s, "filter.roundCorners", json!({"glyph": "B", "radius": 10}));
    let v = run(&mut s, "filter.repeatLastFilter", json!({"glyph": "A"}));
    assert_eq!(v["repeated"]["command"], "filter.roundCorners");
    assert_eq!(on_curve(&mut s, "A")[0], (100.0, 10.0));

    run(&mut s, "path.addPath", json!({"glyph": "C", "nodes": rect(0.0, 0.0, 100.0, 100.0)}));
    run(&mut s, "filter.apply", json!({"glyph": "L", "string": "RoundCorner;5;0;"}));
    // Other commands in between do not count; the repeat of a repeat is the same filter.
    run(&mut s, "path.addPath", json!({"glyph": "D", "nodes": rect(0.0, 0.0, 100.0, 100.0)}));
    let v = run(&mut s, "filter.repeatLastFilter", json!({"glyph": "C"}));
    assert_eq!(v["repeated"]["command"], "filter.apply");
    assert_eq!(on_curve(&mut s, "C")[0], (0.0, 5.0));
    let v = run(&mut s, "filter.repeatLastFilter", json!({"glyphs": ["D"]}));
    assert_eq!(v["repeated"]["params"]["string"], "RoundCorner;5;0;");
    assert_eq!(on_curve(&mut s, "D")[0], (0.0, 5.0));
}
