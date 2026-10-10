//! Hatch Outline (M3.12): the filter on a fixture, its export string, and hostile settings.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use fontcraft_engine::Session;
use fontcraft_testkit::rect;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn shapes(s: &mut Session, glyph: &str) -> Vec<Value> {
    run(s, "layer.inspect", json!({"glyph": glyph}))["shapes"].as_array().unwrap().clone()
}

/// Every node of a path as (x, y).
fn points(shape: &Value) -> Vec<(f64, f64)> {
    shape["nodes"].as_array().unwrap().iter().map(|n| (n["x"].as_f64().unwrap(), n["y"].as_f64().unwrap())).collect()
}

fn font(s: &Session) -> Value {
    serde_json::to_value(s.font().unwrap()).unwrap()
}

/// `O`: a 200-unit square with a clockwise counter from 80 to 120; `B`: a 100-unit square;
/// `Big`: a bar 50 000 units tall.
fn fixture() -> Session {
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "O", "nodes": rect(0.0, 0.0, 200.0, 200.0)}));
    let counter = json!([{"x": 80, "y": 80}, {"x": 80, "y": 120}, {"x": 120, "y": 120}, {"x": 120, "y": 80}]);
    run(&mut s, "path.addPath", json!({"glyph": "O", "nodes": counter}));
    run(&mut s, "path.addPath", json!({"glyph": "B", "nodes": rect(0.0, 0.0, 100.0, 100.0)}));
    run(&mut s, "path.addPath", json!({"glyph": "Big", "nodes": rect(0.0, 0.0, 100.0, 50_000.0)}));
    s
}

#[test]
fn hatch_outline_gives_open_lines_or_closed_strokes_cut_to_the_outline() {
    let mut s = fixture();
    let open = json!({"glyph": "O", "originY": 10, "stepWidth": 40, "angle": 0, "offsetPath": false});
    let v = run(&mut s, "filter.hatchOutline", open);
    assert_eq!(v["glyphs"], json!(["O"]));
    assert_eq!(v["string"], "HatchOutlineFilter;OriginX:0;OriginY:10;StepWidth:40;Angle:0;");
    let paths = shapes(&mut s, "O");
    assert!(paths.iter().all(|p| p["closed"] == false));
    // Lines at y = 10, 50, 90, 130, 170; the one at 90 passes the counter in two pieces.
    assert_eq!(
        paths.iter().map(points).collect::<Vec<_>>(),
        vec![
            vec![(0.0, 10.0), (200.0, 10.0)],
            vec![(0.0, 50.0), (200.0, 50.0)],
            vec![(0.0, 90.0), (80.0, 90.0)],
            vec![(120.0, 90.0), (200.0, 90.0)],
            vec![(0.0, 130.0), (200.0, 130.0)],
            vec![(0.0, 170.0), (200.0, 170.0)],
        ]
    );
    run(&mut s, "edit.undo", json!({}));

    let v = run(&mut s, "filter.hatchOutline", json!({"glyph": "O", "originY": 10, "stepWidth": 40, "angle": 0, "strokeWidth": 10}));
    assert_eq!(v["string"], "HatchOutlineFilter;OriginX:0;OriginY:10;StepWidth:40;Angle:0;Offset:10;");
    let paths = shapes(&mut s, "O");
    assert_eq!(paths.len(), 6);
    for p in &paths {
        assert_eq!(p["closed"], true);
        let pts = points(p);
        assert!(pts.iter().all(|(x, y)| x.fract() == 0.0 && y.fract() == 0.0), "{pts:?}");
        let (y0, y1) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), (_, y)| (a.min(*y), b.max(*y)));
        assert_eq!(y1 - y0, 10.0, "{pts:?}");
    }
    // The other glyphs are not touched.
    assert_eq!(shapes(&mut s, "B").len(), 1);
}

#[test]
fn the_export_string_and_the_parameters_give_the_same_font() {
    let closed = [
        ("filter.hatchOutline", json!({"glyphs": ["O", "B"], "originX": 5, "originY": 10, "stepWidth": 30, "angle": 30, "strokeWidth": 8})),
        ("filter.apply", json!({"glyphs": ["O", "B"], "string": "HatchOutlineFilter; OriginX: 5; OriginY: 10; StepWidth: 30; Angle: 30; Offset: 8"})),
        (
            "filter.apply",
            json!({"glyphs": ["O", "B"], "filter": "HatchOutlineFilter", "args": ["OriginX: 5", "OriginY: 10", "StepWidth: 30", "Angle: 30", "Offset: 8"]}),
        ),
        (
            "filter.apply",
            json!({"glyphs": ["O", "B"], "filter": "filter.hatchOutline", "args": {"originX": 5, "originY": 10, "stepWidth": 30, "angle": 30, "offsetPath": true, "strokeWidth": 8}}),
        ),
        ("filter.apply", json!({"string": "hatchoutlinefilter;originx:5;originy:10;stepwidth:30;angle:30;offset:8;include:O,B;"})),
    ];
    let open = [
        ("filter.hatchOutline", json!({"glyphs": ["O", "B"], "originX": 5, "originY": 10, "stepWidth": 30, "angle": 30, "offsetPath": false})),
        ("filter.apply", json!({"glyphs": ["O", "B"], "string": "HatchOutlineFilter; OriginX: 5; OriginY: 10; StepWidth: 30; Angle: 30"})),
        ("filter.apply", json!({"glyphs": ["O", "B"], "string": "HatchOutlineFilter; OriginX: 5; OriginY: 10; StepWidth: 30; Angle: 30; Offset: 0"})),
    ];
    let untouched = shapes(&mut fixture(), "O");
    let mut seen = Vec::new();
    for forms in [&closed[..], &open[..]] {
        let mut results = Vec::new();
        for (id, params) in forms {
            let mut s = fixture();
            run(&mut s, id, params.clone());
            results.push((shapes(&mut s, "O"), shapes(&mut s, "B"), shapes(&mut s, "Big")));
        }
        assert_ne!(results[0].0, untouched, "the filter changed O");
        for (i, r) in results.iter().enumerate() {
            assert_eq!(r, &results[0], "form {i}: {}", forms[i].1);
        }
        seen.push(results[0].clone());
    }
    assert_ne!(seen[0], seen[1], "Offset makes closed strokes, its absence open lines");
}

#[test]
fn hostile_settings_are_errors_that_change_nothing() {
    let mut s = fixture();
    let before = font(&s);
    for text in [
        "HatchOutlineFilter; 20",
        "HatchOutlineFilter; StepWidth: 0",
        "HatchOutlineFilter; StepWidth: -20",
        "HatchOutlineFilter; StepWidth: NaN",
        "HatchOutlineFilter; Angle: inf",
        "HatchOutlineFilter; Angle: 1e9",
        "HatchOutlineFilter; OriginX: 1e300",
        "HatchOutlineFilter; Offset: -1",
        "HatchOutlineFilter; Offset: 0.5",
        "HatchOutlineFilter; Offset: thick",
        "HatchOutlineFilter; OffsetPath: 1",
        "HatchOutlineFilter; StepWidth: 20; StepWidth: 30",
        // 50 000 lines for the bar.
        "HatchOutlineFilter; StepWidth: 1; Angle: 0; include: Big",
        "HatchOutlineFilter; StepWidth: 1; Angle: 0; Offset: 1; include: Big",
    ] {
        assert!(s.execute("filter.apply", &json!({"string": text})).is_err(), "{text}");
        assert_eq!(font(&s), before, "{text} changed the font");
    }
    for p in [
        json!({"glyph": "O", "stepWidth": "abc"}),
        json!({"glyph": "O", "stepWidth": 0.5}),
        json!({"glyph": "O", "offsetPath": 3}),
        json!({"glyph": "O", "strokeWidth": 0}),
        json!({"glyph": "O", "originX": 1e300}),
        json!({"glyph": "O", "angle": f64::MAX}),
        json!({"glyph": "O", "points": [[0, 0]]}),
        json!({"glyph": "Big", "stepWidth": 1}),
        json!({"glyph": "Missing"}),
    ] {
        assert!(s.execute("filter.hatchOutline", &p).is_err(), "{p}");
        assert_eq!(font(&s), before, "{p} changed the font");
    }
}
