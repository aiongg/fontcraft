//! Offset Curve (M3.12): `filter.offsetCurve` and its export string.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use fontcraft_engine::Session;
use fontcraft_testkit::{oval, rect};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn shapes(s: &mut Session, glyph: &str) -> Vec<Value> {
    run(s, "layer.inspect", json!({"glyph": glyph}))["shapes"].as_array().unwrap().clone()
}

/// The on-curve nodes of every path of a glyph's first master layer.
fn on_curve(s: &mut Session, glyph: &str) -> Vec<(f64, f64)> {
    shapes(s, glyph)
        .iter()
        .flat_map(|shape| shape["nodes"].as_array().unwrap().clone())
        .filter(|n| n["type"] != "offcurve")
        .map(|n| (n["x"].as_f64().unwrap(), n["y"].as_f64().unwrap()))
        .collect()
}

/// The box of the on-curve nodes: (left, bottom, right, top).
fn bounds(s: &mut Session, glyph: &str) -> (f64, f64, f64, f64) {
    on_curve(s, glyph)
        .into_iter()
        .fold((f64::MAX, f64::MAX, f64::MIN, f64::MIN), |(x0, y0, x1, y1), (x, y)| (x0.min(x), y0.min(y), x1.max(x), y1.max(y)))
}

fn font(s: &Session) -> Value {
    serde_json::to_value(s.font().unwrap()).unwrap()
}

/// `B`: a 100-unit square; `O`: an oval of four curves; `S`: an open line 100 units long.
fn fixture() -> Session {
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "B", "nodes": rect(0.0, 0.0, 100.0, 100.0)}));
    run(&mut s, "path.addPath", json!({"glyph": "O", "nodes": oval(250.0, 350.0, 200.0, 350.0)}));
    run(&mut s, "path.addPath", json!({"glyph": "S", "closed": false, "nodes": [{"x": 0, "y": 0}, {"x": 100, "y": 0}]}));
    s
}

#[test]
fn offset_curve_grows_shrinks_and_strokes() {
    let mut s = fixture();
    let v = run(&mut s, "filter.offsetCurve", json!({"glyph": "B", "horizontal": 10, "vertical": 10}));
    assert_eq!(v["glyphs"], json!(["B"]));
    assert_eq!(v["string"], "OffsetCurve;10;10;0;0.5;");
    assert_eq!(on_curve(&mut s, "B").len(), 4);
    assert_eq!(bounds(&mut s, "B"), (-10.0, -10.0, 110.0, 110.0));
    run(&mut s, "edit.undo", json!({}));

    run(&mut s, "filter.offsetCurve", json!({"glyph": "B", "horizontal": -10, "vertical": 20}));
    assert_eq!(bounds(&mut s, "B"), (10.0, -20.0, 90.0, 120.0));
    run(&mut s, "edit.undo", json!({}));

    // A closed path strokes into a frame: two paths.
    run(&mut s, "filter.offsetCurve", json!({"glyph": "B", "horizontal": 10, "vertical": 10, "makeStroke": true}));
    assert_eq!(shapes(&mut s, "B").len(), 2);
    assert_eq!(bounds(&mut s, "B"), (-10.0, -10.0, 110.0, 110.0));

    // An open path strokes into one closed path with its caps.
    run(&mut s, "filter.offsetCurve", json!({"glyph": "S", "horizontal": 10, "vertical": 10, "makeStroke": true, "capStyle": "square"}));
    let stroke = shapes(&mut s, "S");
    assert_eq!(stroke.len(), 1);
    assert_eq!(stroke[0]["closed"], true);
    assert_eq!(bounds(&mut s, "S"), (-10.0, -10.0, 110.0, 10.0));

    // Keep Compatible: the same nodes, moved.
    let before: Vec<Value> = shapes(&mut s, "O")[0]["nodes"].as_array().unwrap().clone();
    run(&mut s, "filter.offsetCurve", json!({"glyph": "O", "horizontal": 20, "vertical": 10, "keepCompatible": true}));
    let after: Vec<Value> = shapes(&mut s, "O")[0]["nodes"].as_array().unwrap().clone();
    assert_eq!(after.len(), before.len());
    assert!(before.iter().zip(&after).all(|(a, b)| a["type"] == b["type"] && a["smooth"] == b["smooth"]));
    assert_eq!(bounds(&mut s, "O"), (30.0, -10.0, 470.0, 710.0));
}

#[test]
fn the_export_string_and_the_parameters_give_the_same_font() {
    let same = |forms: &[(&str, Value)]| {
        let mut results = Vec::new();
        for (id, params) in forms {
            let mut s = fixture();
            run(&mut s, id, params.clone());
            results.push((shapes(&mut s, "B"), shapes(&mut s, "O"), shapes(&mut s, "S")));
        }
        let untouched = {
            let mut s = fixture();
            (shapes(&mut s, "B"), shapes(&mut s, "O"), shapes(&mut s, "S"))
        };
        assert_ne!(results[0].0, untouched.0, "the filter changed B");
        for (i, r) in results.iter().enumerate() {
            assert_eq!(r, &results[0], "form {i}: {}", forms[i].1);
        }
    };
    same(&[
        ("filter.offsetCurve", json!({"glyphs": ["B", "O", "S"], "horizontal": 12, "vertical": 8, "makeStroke": true, "position": 25})),
        ("filter.apply", json!({"glyphs": ["B", "O", "S"], "string": "OffsetCurve; 12; 8; 1; 0.25"})),
        ("filter.apply", json!({"glyphs": ["B", "O", "S"], "filter": "OffsetCurve", "args": ["12", "8", "1", "0.25"]})),
        (
            "filter.apply",
            json!({"glyphs": ["B", "O", "S"], "filter": "filter.offsetCurve", "args": {"horizontal": 12, "vertical": 8, "makeStroke": true, "position": 25}}),
        ),
        ("filter.apply", json!({"string": "OffsetCurve;12;8;1;0.25;include:B,O,S;"})),
    ]);
    same(&[
        ("filter.offsetCurve", json!({"glyphs": ["B", "O", "S"], "horizontal": 10, "vertical": 10, "autoStroke": true})),
        ("filter.apply", json!({"glyphs": ["B", "O", "S"], "string": "OffsetCurve;10;10;0;auto"})),
    ]);
    same(&[
        ("filter.offsetCurve", json!({"glyphs": ["B", "O"], "horizontal": -5, "vertical": 15})),
        ("filter.apply", json!({"glyphs": ["B", "O"], "string": "OffsetCurve;-5;15"})),
    ]);
}

#[test]
fn hostile_strings_and_arguments_are_errors_that_change_nothing() {
    let mut s = fixture();
    let before = font(&s);
    for text in [
        "OffsetCurve",
        "OffsetCurve;10",
        "OffsetCurve;10;10;1;2",
        "OffsetCurve;10;10;1;auto;1",
        "OffsetCurve;NaN;0",
        "OffsetCurve;1e9;0",
        "OffsetCurve;10;10;Position:1",
    ] {
        assert!(s.execute("filter.apply", &json!({"string": text})).is_err(), "{text}");
        assert_eq!(font(&s), before, "{text} changed the font");
    }
    for p in [
        json!({"glyph": "B", "horizontal": 10}),
        json!({"glyph": "B", "horizontal": 10, "vertical": 10, "position": 150}),
        json!({"glyph": "B", "horizontal": 10, "vertical": 10, "capStyle": "wavy"}),
        json!({"glyph": "B", "horizontal": "abc", "vertical": 10}),
        json!({"glyph": "B", "horizontal": 1e300, "vertical": 10}),
        json!({"glyph": "B", "horizontal": 10, "vertical": 10, "keepCompatible": 3}),
        json!({"glyph": "B", "horizontal": 10, "vertical": 10, "points": [[0, 0]]}),
    ] {
        assert!(s.execute("filter.offsetCurve", &p).is_err(), "{p}");
        assert_eq!(font(&s), before, "{p} changed the font");
    }
}

#[test]
fn a_vertical_distance_left_out_is_the_horizontal_one() {
    let mut left_out = fixture();
    let mut given = fixture();
    let v = run(&mut left_out, "filter.offsetCurve", json!({"glyphs": ["B", "O", "S"], "horizontal": 12}));
    assert_eq!(v["string"], "OffsetCurve;12;12;0;0.5;");
    run(&mut given, "filter.offsetCurve", json!({"glyphs": ["B", "O", "S"], "horizontal": 12, "vertical": 12}));
    assert_eq!(font(&left_out), font(&given));
    assert_eq!(bounds(&mut left_out, "B"), (-12.0, -12.0, 112.0, 112.0));

    // The export string with one value gives the same font.
    let mut from_string = fixture();
    run(&mut from_string, "filter.apply", json!({"glyphs": ["B", "O", "S"], "string": "OffsetCurve;12"}));
    assert_eq!(font(&from_string), font(&given));
}
