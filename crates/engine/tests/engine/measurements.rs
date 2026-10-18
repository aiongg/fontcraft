//! The auto-meter and the measurements (M13.34): `autometer.measure`, `autometer.quickMeasure`,
//! `autometer.setStemWidth` and `font.recalculateMeasurements`.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use fontcraft_engine::Session;
use fontcraft_testkit::oval;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn add(s: &mut Session, glyph: &str, points: &[(i32, i32)]) {
    let nodes: Vec<Value> = points.iter().map(|(x, y)| json!({"x": x, "y": y})).collect();
    run(s, "path.addPath", json!({"glyph": glyph, "nodes": nodes}));
}

fn rect(s: &mut Session, glyph: &str, x0: i32, y0: i32, x1: i32, y1: i32) {
    add(s, glyph, &[(x0, y0), (x1, y0), (x1, y1), (x0, y1)]);
}

/// Every node of the glyph's first path as (x, y).
fn points(s: &mut Session, glyph: &str) -> Vec<(f64, f64)> {
    let layer = run(s, "layer.inspect", json!({"glyph": glyph}));
    layer["shapes"][0]["nodes"].as_array().unwrap().iter().map(|n| (n["x"].as_f64().unwrap(), n["y"].as_f64().unwrap())).collect()
}

/// `H`: 80-unit stems at 0 and 520, a 60-unit crossbar from 300 to 360, cap height 700.
fn with_h() -> Session {
    let mut s = Session::with_document();
    add(
        &mut s,
        "H",
        &[(0, 0), (80, 0), (80, 300), (520, 300), (520, 0), (600, 0), (600, 700), (520, 700), (520, 360), (80, 360), (80, 700), (0, 700)],
    );
    s
}

/// The index of the vertical stem whose first edge is at `x`.
fn vertical_at(found: &Value, x: f64) -> u64 {
    found["stems"]
        .as_array()
        .unwrap()
        .iter()
        .find(|st| st["direction"] == "vertical" && st["first"][0][0].as_f64() == Some(x))
        .unwrap_or_else(|| panic!("no vertical stem at {x}: {found}"))["index"]
        .as_u64()
        .unwrap()
}

#[test]
fn autometer_measure_labels_the_stems_counters_and_corners_of_h() {
    let mut s = with_h();
    run(&mut s, "font.stems.add", json!({"name": "lc", "orientation": "vertical", "values": {"m01": 80}}));
    let found = run(&mut s, "autometer.measure", json!({"glyph": "H"}));
    let stems = found["stems"].as_array().unwrap();
    let vertical: Vec<&Value> = stems.iter().filter(|st| st["direction"] == "vertical").collect();
    assert_eq!(vertical.len(), 2, "{found}");
    for st in &vertical {
        assert_eq!(st["width"], 80.0);
        assert_eq!(st["class"], "common");
        assert_eq!(st["editable"], true);
    }
    let crossbar = stems.iter().find(|st| st["direction"] == "horizontal").unwrap();
    assert_eq!(crossbar["width"], 60.0);
    assert_eq!(crossbar["class"], "other", "no horizontal stem value is defined");
    // The counter between the two bars, measured beside the crossbar.
    let counter = found["counters"].as_array().unwrap().iter().find(|c| c["direction"] == "vertical").unwrap();
    assert_eq!(counter["width"], 440.0);
    let y = counter["from"][1].as_f64().unwrap();
    assert!(!(300.0..=360.0).contains(&y), "the counter is drawn through the crossbar at {y}");
    // Twelve square corners.
    let corners = found["corners"].as_array().unwrap();
    assert_eq!(corners.len(), 12);
    assert!(corners.iter().all(|c| c["angle"] == 90.0), "{corners:?}");

    // A stem value 3 units off makes the bars near misses.
    run(&mut s, "font.master.setStem", json!({"masters": ["m01"], "stem": "lc", "value": 83}));
    let found = run(&mut s, "autometer.measure", json!({"glyph": "H", "counters": false, "corners": false}));
    let bar = found["stems"].as_array().unwrap().iter().find(|st| st["direction"] == "vertical").unwrap();
    assert_eq!(bar["class"], "uncommon");
    assert_eq!(bar["nearest"], 83.0);
    assert!(found["corners"].as_array().unwrap().is_empty());
}

#[test]
fn autometer_set_stem_width_moves_both_edges_of_one_stem() {
    let mut s = with_h();
    let found = run(&mut s, "autometer.measure", json!({"glyph": "H"}));
    let left = vertical_at(&found, 0.0);
    let out = run(&mut s, "autometer.setStemWidth", json!({"glyph": "H", "stem": left, "width": 90}));
    assert_eq!(out["was"], 80.0);
    assert_eq!(out["width"], 90.0);
    assert_eq!(out["moved"], 6);
    let p = points(&mut s, "H");
    // The left bar grew by 5 on each side; the right bar stayed.
    assert_eq!(p[0], (-5.0, 0.0));
    assert_eq!(p[1], (85.0, 0.0));
    assert_eq!(p[2], (85.0, 300.0));
    assert_eq!(p[4], (520.0, 0.0));
    assert_eq!(p[9], (85.0, 360.0));
    assert_eq!(p[11], (-5.0, 700.0));
    let found = run(&mut s, "autometer.measure", json!({"glyph": "H"}));
    let widths: Vec<f64> =
        found["stems"].as_array().unwrap().iter().filter(|st| st["direction"] == "vertical").map(|st| st["width"].as_f64().unwrap()).collect();
    assert!(widths.contains(&90.0) && widths.contains(&80.0), "{widths:?}");
    // One undo step puts it back.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(points(&mut s, "H")[0], (0.0, 0.0));
}

#[test]
fn autometer_set_stem_width_snaps_and_keeps_baseline_edges() {
    let mut s = with_h();
    run(&mut s, "font.stems.add", json!({"name": "lc", "orientation": "vertical", "values": {"m01": 83}}));
    let found = run(&mut s, "autometer.measure", json!({"glyph": "H"}));
    let right = vertical_at(&found, 520.0);
    let out = run(&mut s, "autometer.setStemWidth", json!({"glyph": "H", "stem": right, "snap": true}));
    assert_eq!(out["width"], 83.0);
    let p = points(&mut s, "H");
    assert_eq!(p[5].0 - p[4].0, 83.0, "{p:?}");

    // A bar standing on the baseline grows upwards only.
    rect(&mut s, "E", 0, 0, 300, 50);
    let found = run(&mut s, "autometer.measure", json!({"glyph": "E"}));
    let bar = found["stems"].as_array().unwrap().iter().find(|st| st["direction"] == "horizontal").unwrap()["index"].clone();
    run(&mut s, "autometer.setStemWidth", json!({"glyph": "E", "stem": bar, "by": 10}));
    let ys: Vec<f64> = points(&mut s, "E").iter().map(|p| p.1).collect();
    assert_eq!(ys, vec![0.0, 0.0, 60.0, 60.0]);

    // Nothing to snap to, and a stem that is not there: refused, nothing changed.
    assert!(s.execute("autometer.setStemWidth", &json!({"glyph": "E", "stem": bar, "snap": true})).is_err());
    assert!(s.execute("autometer.setStemWidth", &json!({"glyph": "E", "stem": 99, "width": 40})).is_err());
    assert!(s.execute("autometer.setStemWidth", &json!({"glyph": "E", "stem": bar, "width": 40, "by": 2})).is_err());
    assert_eq!(points(&mut s, "E")[2], (300.0, 60.0));
}

#[test]
fn autometer_quick_measure_across_and_along_an_axis() {
    let mut s = with_h();
    let across = run(&mut s, "autometer.quickMeasure", json!({"glyph": "H", "at": [30, 150]}));
    assert_eq!(across["distance"], 80.0);
    assert_eq!(across["from"], json!([0.0, 150.0]));
    assert_eq!(across["to"], json!([80.0, 150.0]));
    let up = run(&mut s, "autometer.quickMeasure", json!({"glyph": "H", "at": [40, 150], "axis": "vertical"}));
    assert_eq!(up["distance"], 700.0);
    // In the counter, across to the other bar.
    let counter = run(&mut s, "autometer.quickMeasure", json!({"glyph": "H", "at": [300, 150], "axis": "horizontal"}));
    assert_eq!(counter["distance"], 440.0);
    assert!(s.execute("autometer.quickMeasure", &json!({"glyph": "H", "at": [-100, 150], "axis": "horizontal"})).is_err());
}

#[test]
fn font_recalculate_measurements_of_a_synthetic_master() {
    let mut s = Session::with_document();
    rect(&mut s, "x", 0, 0, 400, 480);
    add(
        &mut s,
        "H",
        &[(0, 0), (80, 0), (80, 300), (420, 300), (420, 0), (500, 0), (500, 690), (420, 690), (420, 360), (80, 360), (80, 690), (0, 690)],
    );
    rect(&mut s, "n", 0, 0, 70, 480);
    rect(&mut s, "n", 300, 0, 370, 480);
    run(&mut s, "path.addPath", json!({"glyph": "o", "nodes": oval(250.0, 240.0, 230.0, 252.0)}));
    run(&mut s, "path.addPath", json!({"glyph": "O", "nodes": oval(300.0, 345.0, 280.0, 357.0)}));
    run(&mut s, "font.stems.add", json!({"name": "lc", "orientation": "vertical"}));
    run(&mut s, "font.stems.add", json!({"name": "uc", "orientation": "vertical"}));
    let out = run(&mut s, "font.recalculateMeasurements", json!({}));
    let values = &out["masters"][0]["values"];
    assert_eq!(values["xHeight"], 480.0);
    assert_eq!(values["capHeight"], 690.0);
    assert_eq!(values["xOvershoot"], 12.0);
    assert_eq!(values["capOvershoot"], 12.0);
    assert_eq!(values["baselineOvershoot"], -12.0);
    assert_eq!(values["verticalStems"], json!([70.0, 80.0]));
    let skipped: Vec<&str> = out["skipped"].as_array().unwrap().iter().map(|k| k["field"].as_str().unwrap()).collect();
    for field in ["ascender", "descender", "horizontalStems"] {
        assert!(skipped.contains(&field), "{field} should be skipped: {skipped:?}");
    }
    let master = run(&mut s, "font.master.get", json!({"master": "m01"}));
    assert_eq!(master["metrics"]["x-height"]["position"], 480.0);
    assert_eq!(master["metrics"]["x-height"]["zone"], 12.0);
    assert_eq!(master["metrics"]["cap height"]["position"], 690.0);
    assert_eq!(master["metrics"]["baseline"]["zone"], -12.0);
    assert_eq!(master["stems"]["lc"], 70.0);
    assert_eq!(master["stems"]["uc"], 80.0);
    // One undo step.
    run(&mut s, "edit.undo", json!({}));
    let master = run(&mut s, "font.master.get", json!({"master": "m01"}));
    assert_eq!(master["metrics"]["x-height"]["position"], 500.0);

    // Only the fields asked for.
    let out = run(&mut s, "font.recalculateMeasurements", json!({"fields": ["capHeight"]}));
    assert_eq!(out["masters"][0]["values"].as_object().unwrap().len(), 1);
    assert!(s.execute("font.recalculateMeasurements", &json!({"fields": ["width"]})).is_err());
    assert!(s.execute("font.recalculateMeasurements", &json!({"masters": ["nope"]})).is_err());
}

#[test]
fn font_recalculate_measurements_refuses_an_empty_font() {
    let mut s = Session::with_document();
    let before = serde_json::to_value(s.font().unwrap()).unwrap();
    assert!(s.execute("font.recalculateMeasurements", &json!({})).is_err());
    assert_eq!(serde_json::to_value(s.font().unwrap()).unwrap(), before);
}
