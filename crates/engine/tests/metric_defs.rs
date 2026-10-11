//! Metric and stem definitions (M7.6): the ten `font.metrics.*`, `font.stems.*` and
//! `font.master.setMetric` / `setStem` commands on a font with two masters, one undo step each,
//! refusals that change nothing, the italic angle feeding the sidebearings, a zone edit reaching
//! the alignment zones, and the default metrics of a new Arabic or Latin font.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::model::{AlignmentZone, metric};
use fontcraft_engine::{EngineError, Session};
use fontcraft_spacing::Field;
use fontcraft_testkit::{rect, snapshot};
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

/// A new font with a second master, `m02` called Bold, a copy of the first.
fn two_masters() -> Session {
    let mut s = Session::with_document();
    s.edit("second master", |font| {
        let mut m = font.default_master()?.clone();
        m.id = "m02".into();
        m.name = "Bold".into();
        Arc::make_mut(&mut font.masters).push(m);
        Ok(())
    })
    .unwrap();
    s
}

/// `(position, zone)` of every metric of `master`, missing values read as zero.
fn values(s: &Session, master: &str) -> Vec<(f64, f64)> {
    let font = s.font().unwrap();
    let m = font.master(master).unwrap();
    (0..font.metrics.len()).map(|i| m.metric_values.get(i).map_or((0.0, 0.0), |v| (v.pos, v.over))).collect()
}

fn stems(s: &Session, master: &str) -> Vec<f64> {
    let font = s.font().unwrap();
    let m = font.master(master).unwrap();
    (0..font.stems.len()).map(|i| m.stem_values.get(i).copied().unwrap_or(0.0)).collect()
}

#[test]
fn metric_definitions_and_values_on_two_masters() {
    let mut s = two_masters();
    let listed = run(&mut s, "font.metrics.list", json!({}));
    let categories: Vec<&str> = listed["metrics"].as_array().unwrap().iter().map(|m| m["category"].as_str().unwrap()).collect();
    assert_eq!(categories, [metric::ASCENDER, metric::CAP_HEIGHT, metric::X_HEIGHT, metric::BASELINE, metric::DESCENDER]);
    assert!(listed.get("master").is_none() && listed["metrics"][0].get("position").is_none(), "no master, no values: {listed}");

    // Adding gives every master a zero value; the readable label names the category.
    let out = run(&mut s, "font.metrics.add", json!({"category": "Cap Height", "name": "Small Cap"}));
    assert_eq!(out["index"], 5);
    assert_eq!((out["metric"]["category"].as_str(), out["metric"]["name"].as_str()), (Some(metric::CAP_HEIGHT), Some("Small Cap")));
    assert_eq!(values(&s, "m01").len(), 6);
    assert_eq!((values(&s, "m01")[5], values(&s, "m02")[5]), ((0.0, 0.0), (0.0, 0.0)));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.font().unwrap().metrics.len(), 5, "adding a metric is one undo step");
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(s.font().unwrap().metrics.len(), 6);

    // Both masters in one step, then one of them; a negative zone is a zone below the line.
    run(&mut s, "font.master.setMetric", json!({"masters": ["m01", "Bold"], "metric": "Small Cap", "position": 520, "zone": 12}));
    assert_eq!((values(&s, "m01")[5], values(&s, "m02")[5]), ((520.0, 12.0), (520.0, 12.0)));
    run(&mut s, "font.master.setMetric", json!({"masters": ["m02"], "metric": 5, "position": 540}));
    run(&mut s, "font.master.setMetric", json!({"masters": ["m01"], "metric": "baseline", "zone": -20}));
    assert_eq!((values(&s, "m01")[5], values(&s, "m02")[5], values(&s, "m01")[3]), ((520.0, 12.0), (540.0, 12.0), (0.0, -20.0)));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(values(&s, "m01")[3], (0.0, -16.0), "setting a value is one undo step");
    let bold = run(&mut s, "font.metrics.list", json!({"master": "m02"}));
    assert_eq!(
        (bold["master"].as_str(), bold["metrics"][5]["position"].as_f64(), bold["metrics"][5]["zone"].as_f64()),
        (Some("m02"), Some(540.0), Some(12.0))
    );

    // A new zone reaches the alignment zones the hinting and Edit View read.
    let zones = {
        let font = s.font().unwrap();
        font.alignment_zones(font.master("m01").unwrap())
    };
    assert!(zones.contains(&AlignmentZone { pos: 520.0, size: 12.0 }), "{zones:?}");

    // Editing the definition keeps the values.
    run(&mut s, "font.metrics.set", json!({"metric": "Small Cap", "category": "x-height", "scope": "name == 'a'"}));
    let def = s.font().unwrap().metrics[5].clone();
    assert_eq!((def.kind.as_str(), def.name.as_deref(), def.filter.as_deref()), (metric::X_HEIGHT, Some("Small Cap"), Some("name == 'a'")));
    assert_eq!(values(&s, "m02")[5], (540.0, 12.0));
    run(&mut s, "font.metrics.set", json!({"metric": "Small Cap", "scope": ""}));
    assert_eq!(s.font().unwrap().metrics[5].filter, None, "an empty scope removes it");
    // A category picks the plain metric over the named one of the same category.
    run(&mut s, "font.master.setMetric", json!({"masters": ["m01"], "metric": "x-Height", "zone": 14}));
    assert_eq!((values(&s, "m01")[2], values(&s, "m01")[5]), ((500.0, 14.0), (520.0, 12.0)));
    run(&mut s, "edit.undo", json!({}));

    let e = refused(&mut s, "font.metrics.set", json!({"metric": "Small Cap", "name": ""}));
    assert!(e.contains("already"), "a second plain x-height: {e}");
    let e = refused(&mut s, "font.metrics.add", json!({"category": "x-height"}));
    assert!(e.contains("already"), "{e}");
    let e = refused(&mut s, "font.metrics.add", json!({"category": "shoulder"}));
    assert!(e.contains("categories are"), "{e}");
    refused(&mut s, "font.metrics.add", json!({"category": ""}));
    let e = refused(&mut s, "font.metrics.add", json!({"category": "ascender", "name": "Tall", "scope": "name == 'a"}));
    assert!(e.contains("scope"), "{e}");
    let e = refused(&mut s, "font.master.setMetric", json!({"masters": ["m01"], "metric": "baseline", "position": 5}));
    assert!(e.contains("always at 0"), "{e}");
    refused(&mut s, "font.master.setMetric", json!({"masters": ["m01"], "metric": "ascender"}));
    refused(&mut s, "font.master.setMetric", json!({"masters": ["m01"], "metric": "ascender", "position": f64::MAX}));
    refused(&mut s, "font.master.setMetric", json!({"masters": ["m09"], "metric": "ascender", "position": 700}));
    refused(&mut s, "font.master.setMetric", json!({"masters": ["m01", "m09"], "metric": "ascender", "position": 700}));
    refused(&mut s, "font.master.setMetric", json!({"masters": ["m01"], "metric": "Nothing", "position": 700}));
    refused(&mut s, "font.master.setMetric", json!({"masters": ["m01"], "metric": 99, "position": 700}));
    let e = refused(&mut s, "font.metrics.set", json!({"metric": "ascender", "category": "baseline", "name": "Low"}));
    assert!(e.contains("always at 0"), "the ascender's values do not fit a baseline: {e}");

    // Removing takes the values from every master.
    let out = run(&mut s, "font.metrics.remove", json!({"metric": "Small Cap"}));
    assert_eq!((out["removed"]["index"].as_u64(), out["metrics"].as_u64()), (Some(5), Some(5)));
    assert_eq!((values(&s, "m01").len(), s.font().unwrap().masters[1].metric_values.len()), (5, 5));
    run(&mut s, "font.metrics.remove", json!({"metric": "cap height"}));
    let categories: Vec<String> = s.font().unwrap().metrics.iter().map(|d| d.kind.clone()).collect();
    assert_eq!(categories, [metric::ASCENDER, metric::X_HEIGHT, metric::BASELINE, metric::DESCENDER]);
    assert_eq!(values(&s, "m02")[1], (500.0, 16.0), "the x-height's value moves up with it");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(values(&s, "m02")[1], (700.0, 16.0), "removing is one undo step");
}

#[test]
fn the_italic_angle_feeds_the_sidebearings() {
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(100.0, 0.0, 200.0, 700.0)}));
    let lsb = |s: &Session| {
        let font = s.font().unwrap();
        let layer = &font.glyph("A").unwrap().layers[0];
        fontcraft_spacing::metric(font, layer, Field::Left).unwrap().unwrap()
    };
    assert!((lsb(&s) - 100.0).abs() < 1e-6);
    run(&mut s, "font.metrics.add", json!({"category": "italic angle"}));
    run(&mut s, "font.master.setMetric", json!({"masters": ["m01"], "metric": "Italic Angle", "position": 10}));
    assert!((lsb(&s) - 100.0).abs() > 10.0, "the left sidebearing is measured along the slant: {}", lsb(&s));
    let listed = run(&mut s, "font.metrics.list", json!({"master": "m01"}));
    assert!(listed["metrics"][5].get("zone").is_none(), "the italic angle has one field: {listed}");
    refused(&mut s, "font.master.setMetric", json!({"masters": ["m01"], "metric": "italic angle", "position": 90}));
    refused(&mut s, "font.master.setMetric", json!({"masters": ["m01"], "metric": "italic angle", "zone": 4}));
    run(&mut s, "font.master.setMetric", json!({"masters": ["m01"], "metric": "italic angle", "position": 0}));
    assert!((lsb(&s) - 100.0).abs() < 1e-6, "upright again");
}

#[test]
fn stem_definitions_and_values_on_two_masters() {
    let mut s = two_masters();
    let out = run(&mut s, "font.stems.add", json!({"orientation": "vertical", "values": {"m01": 80, "Bold": 140}}));
    assert_eq!((out["index"].as_u64(), out["stem"]["name"].as_str(), out["stem"]["orientation"].as_str()), (Some(0), Some("Stem"), Some("vertical")));
    let out = run(&mut s, "font.stems.add", json!({"orientation": "horizontal", "scope": "name == 'H'"}));
    assert_eq!((out["stem"]["name"].as_str(), out["stem"]["scope"].as_str()), (Some("Stem.1"), Some("name == 'H'")));
    assert_eq!((stems(&s, "m01"), stems(&s, "m02")), (vec![80.0, 0.0], vec![140.0, 0.0]));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.font().unwrap().stems.len(), 1, "adding a stem is one undo step");
    run(&mut s, "edit.redo", json!({}));

    let e = refused(&mut s, "font.stems.set", json!({"stem": 1, "name": "Stem"}));
    assert!(e.contains("already"), "{e}");
    refused(&mut s, "font.stems.set", json!({"stem": 1, "name": ""}));
    refused(&mut s, "font.stems.add", json!({"name": "Stem", "orientation": "vertical"}));
    refused(&mut s, "font.stems.add", json!({}));
    refused(&mut s, "font.stems.add", json!({"orientation": "diagonal"}));
    refused(&mut s, "font.stems.add", json!({"orientation": "vertical", "values": {"m09": 50}}));
    refused(&mut s, "font.stems.add", json!({"orientation": "vertical", "values": {"m01": -1}}));
    refused(&mut s, "font.master.setStem", json!({"masters": ["m01"], "stem": "Stem", "value": 20000}));
    refused(&mut s, "font.master.setStem", json!({"masters": ["m01"], "stem": "Nothing", "value": 20}));

    run(&mut s, "font.stems.set", json!({"stem": "Stem.1", "name": "Bar", "orientation": "vertical", "scope": ""}));
    let bar = s.font().unwrap().stems[1].clone();
    assert_eq!((bar.name.as_str(), bar.horizontal, bar.filter), ("Bar", false, None));
    run(&mut s, "font.master.setStem", json!({"masters": ["m01", "m02"], "stem": "Bar", "value": 60}));
    run(&mut s, "font.master.setStem", json!({"masters": ["m02"], "stem": "Bar", "value": 66.5}));
    let listed = run(&mut s, "font.stems.list", json!({"master": "Bold"}));
    let widths: Vec<f64> = listed["stems"].as_array().unwrap().iter().map(|st| st["value"].as_f64().unwrap()).collect();
    assert_eq!((listed["master"].as_str(), widths), (Some("m02"), vec![140.0, 66.5]));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(stems(&s, "m02"), [140.0, 60.0], "setting a width is one undo step");

    let out = run(&mut s, "font.stems.remove", json!({"stem": "Stem"}));
    assert_eq!((out["removed"]["name"].as_str(), out["stems"].as_u64()), (Some("Stem"), Some(1)));
    assert_eq!((stems(&s, "m01"), stems(&s, "m02")), (vec![60.0], vec![60.0]));
    assert_eq!(s.font().unwrap().masters[0].stem_values.len(), 1, "the width left the master");
    run(&mut s, "font.stems.add", json!({"orientation": "horizontal"}));
    assert_eq!(s.font().unwrap().stems[1].name, "Stem", "the default name is free again");
}

#[test]
fn new_fonts_take_the_default_metrics_of_their_scripts() {
    let mut s = Session::new();
    run(&mut s, "file.newFromGlyphSets", json!({"scripts": [{"script": "arabic"}]}));
    let listed = run(&mut s, "font.metrics.list", json!({"master": "m01"}));
    let got: Vec<(String, f64)> = listed["metrics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| (m["name"].as_str().or(m["label"].as_str()).unwrap().to_string(), m["position"].as_f64().unwrap()))
        .collect();
    let want =
        [("Ascender", 800.0), ("Alef Height", 700.0), ("Joining Line", 150.0), ("Baseline", 0.0), ("Descender", -200.0), ("Meem Depth", -250.0)];
    assert_eq!(got, want.map(|(n, v)| (n.to_string(), v)));

    run(&mut s, "file.newFromGlyphSets", json!({"scripts": [{"script": "latin"}, {"script": "arabic"}]}));
    let kinds: Vec<String> = s.font().unwrap().metrics.iter().map(|d| d.kind.clone()).collect();
    assert_eq!(kinds, [metric::ASCENDER, metric::CAP_HEIGHT, metric::X_HEIGHT, metric::BASELINE, metric::DESCENDER, metric::ITALIC_ANGLE]);
    let font = s.font().unwrap();
    assert_eq!(font.metric_pos(font.default_master().unwrap(), metric::ITALIC_ANGLE, 99.0), 0.0);
}
