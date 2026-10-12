//! The stem filters (M13.24): `filter.adjustStems`, `filter.changeWeight`,
//! `filter.condenseExtend` and `filter.changeXHeight`.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use fontcraft_engine::Session;
use fontcraft_testkit::oval;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn layer(s: &mut Session, glyph: &str) -> Value {
    run(s, "layer.inspect", json!({"glyph": glyph}))
}

/// Every node of the glyph's paths as (x, y).
fn points(s: &mut Session, glyph: &str) -> Vec<(f64, f64)> {
    layer(s, glyph)["shapes"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|shape| shape["nodes"].as_array().cloned().unwrap_or_default())
        .map(|n| (n["x"].as_f64().unwrap(), n["y"].as_f64().unwrap()))
        .collect()
}

fn bounds(s: &mut Session, glyph: &str) -> (f64, f64, f64, f64) {
    let p = points(s, glyph);
    let f = |pick: fn(&(f64, f64)) -> f64, min: bool| {
        p.iter().map(pick).fold(if min { f64::INFINITY } else { f64::NEG_INFINITY }, if min { f64::min } else { f64::max })
    };
    (f(|p| p.0, true), f(|p| p.1, true), f(|p| p.0, false), f(|p| p.1, false))
}

fn width(s: &mut Session, glyph: &str) -> f64 {
    layer(s, glyph)["width"].as_f64().unwrap()
}

fn font(s: &Session) -> Value {
    serde_json::to_value(s.font().unwrap()).unwrap()
}

/// `H`: 80-unit stems at 0 and 520, a 60-unit crossbar from 300 to 360, cap height 700. `o`: a
/// bowl 80 units thick at the sides and 70 at the top and bottom. `x`: a bar from the baseline to
/// the x-height with a 15-unit serif step.
fn fixture() -> Session {
    let mut s = Session::with_document();
    let h = [(0, 0), (80, 0), (80, 300), (520, 300), (520, 0), (600, 0), (600, 700), (520, 700), (520, 360), (80, 360), (80, 700), (0, 700)];
    let nodes: Vec<Value> = h.iter().map(|(x, y)| json!({"x": x, "y": y})).collect();
    run(&mut s, "path.addPath", json!({"glyph": "H", "nodes": nodes}));
    run(&mut s, "path.addPath", json!({"glyph": "o", "nodes": oval(250.0, 260.0, 250.0, 260.0)}));
    // A negative radius mirrors the oval, so the counter runs clockwise.
    run(&mut s, "path.addPath", json!({"glyph": "o", "nodes": oval(250.0, 260.0, 170.0, -190.0)}));
    let x = [(0, 0), (200, 0), (200, 15), (140, 15), (140, 250), (140, 500), (60, 500), (60, 250), (60, 15), (0, 15)];
    let nodes: Vec<Value> = x.iter().map(|(x, y)| json!({"x": x, "y": y})).collect();
    run(&mut s, "path.addPath", json!({"glyph": "x", "nodes": nodes}));
    s
}

#[test]
fn adjust_stems_changes_the_stems_whose_width_a_rule_holds() {
    let mut s = fixture();
    let out = run(
        &mut s,
        "filter.adjustStems",
        json!({"glyphs": ["H", "o"], "vertical": [{"from": 70, "to": 90, "offset": 20}], "horizontal": [{"from": 50, "to": 65, "set": 40}]}),
    );
    assert_eq!(out["glyphs"], json!(["H", "o"]));
    // H: two vertical stems and the crossbar; o: its two sides (its 70-unit top and bottom are
    // outside the horizontal rule's range).
    assert_eq!(out["stems"], 5);
    let h = points(&mut s, "H");
    for x in [-10.0, 90.0, 510.0, 610.0] {
        assert!(h.iter().any(|p| p.0 == x), "H has no node at x = {x}: {h:?}");
    }
    assert!(h.contains(&(90.0, 310.0)) && h.contains(&(90.0, 350.0)), "the crossbar is 40 units: {h:?}");
    let o = points(&mut s, "o");
    for p in [(-10.0, 260.0), (90.0, 260.0), (410.0, 260.0), (510.0, 260.0), (250.0, 520.0), (250.0, 450.0)] {
        assert!(o.contains(&p), "o has no node at {p:?}: {o:?}");
    }

    // A stem out of every range, and a rule that would make a stem narrower than a unit.
    let mut s = fixture();
    let before = font(&s);
    let none = run(&mut s, "filter.adjustStems", json!({"glyph": "H", "vertical": [{"from": 100, "to": 200, "set": 50}]}));
    assert_eq!(none["glyphs"], json!([]));
    assert_eq!(none["skipped"][0]["reason"], "no stem matched");
    assert!(s.execute("filter.adjustStems", &json!({"glyph": "H", "vertical": [{"from": 70, "to": 90, "offset": -85}]})).is_err());
    for bad in [
        json!({"glyph": "H"}),
        json!({"glyph": "H", "vertical": [{"from": 70, "to": 90}]}),
        json!({"glyph": "H", "vertical": [{"from": 70, "to": 90, "set": 60, "offset": 1}]}),
        json!({"glyph": "H", "vertical": [{"from": 90, "to": 70, "set": 60}]}),
        json!({"glyph": "H", "vertical": [{"from": 70, "to": 90, "set": 60, "scale": 2}]}),
        json!({"glyph": "H", "vertical": "wide"}),
    ] {
        assert!(s.execute("filter.adjustStems", &bad).is_err(), "{bad}");
    }
    assert_eq!(font(&s), before, "a refused run changes nothing");
}

#[test]
fn change_weight_keeps_heights_with_method_lcg() {
    let mut s = fixture();
    let w = width(&mut s, "H");
    run(&mut s, "filter.changeWeight", json!({"glyph": "H", "amount": 20, "method": "lcg", "counters": "squish"}));
    assert_eq!(bounds(&mut s, "H"), (-10.0, 0.0, 610.0, 700.0), "the cap height and baseline stay");
    assert_eq!(width(&mut s, "H"), w, "squish keeps the width");

    // The default counters with lcg keep the counter: the right stem moves over by the amount.
    let mut s = fixture();
    run(&mut s, "filter.changeWeight", json!({"glyph": "H", "amount": 20}));
    assert_eq!(bounds(&mut s, "H"), (-10.0, 0.0, 630.0, 700.0));
    let h = points(&mut s, "H");
    assert!(h.iter().any(|p| p.0 == 90.0) && h.iter().any(|p| p.0 == 530.0), "the counter stays 440 wide: {h:?}");
    assert_eq!(width(&mut s, "H"), w + 20.0);

    // cjk only expands.
    let mut s = fixture();
    run(&mut s, "filter.changeWeight", json!({"glyph": "H", "amount": 20, "method": "cjk"}));
    assert_eq!(bounds(&mut s, "H"), (-10.0, -10.0, 610.0, 710.0));

    // A lowercase glyph keeps the x-height.
    let mut s = fixture();
    run(&mut s, "filter.changeWeight", json!({"glyph": "x", "amount": 10, "method": "lcg"}));
    let (_, y0, _, y1) = bounds(&mut s, "x");
    assert_eq!((y0, y1), (0.0, 500.0));

    let mut s = fixture();
    for bad in [
        json!({"glyph": "H"}),
        json!({"glyph": "H", "amount": 20, "method": "custom"}),
        json!({"glyph": "H", "amount": 20, "method": "heavy"}),
        json!({"glyph": "H", "amount": 20, "zones": {"bottom": 700, "top": 0}}),
        json!({"glyph": "H", "amount": 5000}),
        json!({"glyph": "H", "amount": 20, "counters": "keep"}),
    ] {
        assert!(s.execute("filter.changeWeight", &bad).is_err(), "{bad}");
    }
}

#[test]
fn condense_keeps_stem_widths_within_one_unit() {
    let mut s = fixture();
    let w = width(&mut s, "H");
    run(&mut s, "filter.condenseExtend", json!({"glyph": "H", "counters": {"scale": 80}}));
    let h = points(&mut s, "H");
    let mut xs: Vec<f64> = h.iter().map(|p| p.0).collect();
    xs.sort_by(f64::total_cmp);
    xs.dedup();
    assert_eq!(xs, vec![0.0, 80.0, 432.0, 512.0], "the stems stay 80 wide and the counter is 80% of 440");
    assert_eq!(width(&mut s, "H"), w - 88.0);

    // Round stems keep their widths too.
    let mut s = fixture();
    run(&mut s, "filter.condenseExtend", json!({"glyph": "o", "counters": {"scale": 50}, "sidebearings": {"add": 10}}));
    let o = points(&mut s, "o");
    assert!(o.contains(&(10.0, 260.0)) && o.contains(&(90.0, 260.0)), "{o:?}");
    assert!(o.contains(&(260.0, 260.0)) && o.contains(&(340.0, 260.0)), "the 340-unit counter halves: {o:?}");

    for bad in [json!({"glyph": "H"}), json!({"glyph": "H", "counters": {"scale": 0}}), json!({"glyph": "H", "counters": {"size": 3}})] {
        assert!(s.execute("filter.condenseExtend", &bad).is_err(), "{bad}");
    }
}

#[test]
fn change_x_height_keeps_serifs_below_the_serif_height() {
    let mut s = fixture();
    run(&mut s, "filter.changeXHeight", json!({"glyph": "x", "to": 540, "serifHeight": 20}));
    let x = points(&mut s, "x");
    assert!(x.contains(&(0.0, 0.0)) && x.contains(&(0.0, 15.0)) && x.contains(&(140.0, 15.0)), "the serif stays: {x:?}");
    assert!(x.contains(&(140.0, 540.0)) && x.contains(&(60.0, 540.0)), "the x-height moves: {x:?}");
    // 250 sits in the stretched band: 20 + 230 × 500 / 460.
    assert!(x.contains(&(140.0, 270.0)), "{x:?}");

    // Locked glyphs are skipped in a list and refused alone.
    run(&mut s, "glyph.toggleLocked", json!({"glyph": "H", "locked": true}));
    let out = run(&mut s, "filter.changeXHeight", json!({"glyphs": ["H", "x"], "to": 500}));
    assert_eq!(out["skipped"][0]["glyph"], "H");
    assert!(s.execute("filter.changeXHeight", &json!({"glyph": "H", "to": 500})).is_err());
    assert!(s.execute("filter.changeXHeight", &json!({"glyph": "x", "to": 30, "serifHeight": 20})).is_err());
}
