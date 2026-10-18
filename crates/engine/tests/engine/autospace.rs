//! Auto-spacing, Center in Width, Copy Metrics and the metrics beam of the sidebearing setters
//! (M13.20): `spacing.autoSpace` with each method, calibration and the options; centring with a
//! ratio; copying metrics onto several glyphs; one undo step each, and refusals that change
//! nothing.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::model::Anchor;
use fontcraft_engine::{EngineError, Session};
use fontcraft_spacing::{Field, metric, sidebearings_at_height};
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

/// `(left, right, width)` of a glyph's first-master layer.
fn lrw(s: &Session, glyph: &str) -> (f64, f64, f64) {
    let f = s.font().unwrap();
    let l = &f.glyph(glyph).unwrap().layers[0];
    let m = |field| metric(f, l, field).unwrap().unwrap();
    (m(Field::Left), m(Field::Right), m(Field::Width))
}

fn draw(s: &mut Session, glyph: &str, nodes: Value, width: f64) {
    run(s, "path.addPath", json!({"glyph": glyph, "nodes": nodes}));
    run(s, "spacing.setWidth", json!({"glyphs": [glyph], "value": width}));
}

/// A V: corners at (0, 500), (400, 500) and (200, 0). At height 250 its sides are at 100 and 300.
fn triangle() -> Value {
    json!([{"x": 0, "y": 500}, {"x": 400, "y": 500}, {"x": 200, "y": 0}])
}

fn lock(s: &mut Session, glyph: &str) {
    s.edit("lock", |f| {
        f.glyph_mut(glyph).unwrap().info.locked = true;
        Ok(())
    })
    .unwrap();
}

#[test]
fn spacing_auto_space_optical_calibrates_on_n_and_o() {
    // x-height 500 and baseline 0, both with 16 units of overshoot: the lowercase band runs from
    // -16 to 516. n has 60 units each side; o is round, 50 units at its extremes.
    let mut s = Session::with_document();
    draw(&mut s, "n", rect(60.0, 0.0, 440.0, 500.0), 500.0);
    draw(&mut s, "o", oval(250.0, 250.0, 200.0, 266.0), 500.0);
    draw(&mut s, "l", rect(0.0, 0.0, 80.0, 700.0), 80.0);
    draw(&mut s, "I", rect(0.0, 0.0, 100.0, 700.0), 100.0);
    let out = undo_restores(&mut s, "spacing.autoSpace", json!({"glyphs": ["n", "o", "l", "I"]}));
    let area = out["areas"]["m01"]["lowercase"].as_f64().unwrap();
    // The o's curves hold more white than its 50 units, so the calibrated area lies between
    // the n's 60 and the o's mean white (clipped at 75 units inward).
    assert!(area > 60.0 && area < 90.0, "{out}");
    // Without H and O, capitals take the lowercase area.
    assert_eq!(out["areas"]["m01"]["capitals"].as_f64(), Some(area), "{out}");
    let a = area.round();
    // Straight sides get the area itself; the o gives back what the n gains, so the pair keeps
    // its total.
    assert_eq!(lrw(&s, "n"), (a, a, 380.0 + 2.0 * a));
    assert_eq!(lrw(&s, "l"), (a, a, 80.0 + 2.0 * a));
    assert_eq!(lrw(&s, "I"), (a, a, 100.0 + 2.0 * a));
    let (ol, or, _) = lrw(&s, "o");
    assert!(ol < 50.0 && (ol - or).abs() <= 1.0, "o: {ol} {or}");
    assert!((109.0..=111.0).contains(&(a + ol)), "n {a} + o {ol}");
    assert_eq!(out["values"]["n"]["m01"], json!({"left": a, "right": a}));
    let changed: Vec<&str> = out["glyphs"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    assert_eq!(changed, ["n", "o", "l", "I"]);

    // An explicit separation is twice the area per side.
    run(&mut s, "spacing.autoSpace", json!({"glyphs": ["n"], "separation": 100}));
    assert_eq!(lrw(&s, "n"), (50.0, 50.0, 480.0));
    // Without glyphs: every exported letter and figure; the empty ones are skipped.
    let out = run(&mut s, "spacing.autoSpace", json!({"area": 40}));
    assert_eq!(lrw(&s, "n"), (40.0, 40.0, 460.0));
    assert!(out["skipped"].as_array().unwrap().iter().any(|v| v["glyph"] == "x" && v["reason"] == "the layer is empty"), "{out}");
}

#[test]
fn spacing_auto_space_tight_custom_beam_keys_and_limits() {
    let mut s = Session::with_document();
    draw(&mut s, "v", triangle(), 600.0);
    draw(&mut s, "x", rect(20.0, 0.0, 320.0, 500.0), 600.0);
    draw(&mut s, "z", rect(10.0, 0.0, 310.0, 500.0), 600.0);
    draw(&mut s, "k", rect(10.0, 0.0, 310.0, 500.0), 600.0);
    draw(&mut s, "w", rect(10.0, 0.0, 310.0, 500.0), 600.0);
    undo_restores(&mut s, "spacing.autoSpace", json!({"glyphs": ["v", "x"], "method": "tight"}));
    assert_eq!(lrw(&s, "v"), (0.0, 0.0, 400.0));
    assert_eq!(lrw(&s, "x"), (0.0, 0.0, 300.0));
    run(&mut s, "spacing.autoSpace", json!({"glyphs": ["x"], "method": "custom", "percentOfUpm": 10}));
    assert_eq!(lrw(&s, "x"), (100.0, 100.0, 500.0));
    run(&mut s, "spacing.autoSpace", json!({"glyphs": ["x"], "method": "custom", "percentOfUpm": 10, "max": 80}));
    assert_eq!(lrw(&s, "x"), (80.0, 80.0, 460.0));
    run(&mut s, "spacing.autoSpace", json!({"glyphs": ["x"], "method": "tight", "min": 15}));
    assert_eq!(lrw(&s, "x"), (15.0, 15.0, 330.0));

    // At the beam, the V's sides are 200 units apart: tight leaves 200 units of width.
    run(&mut s, "spacing.autoSpace", json!({"glyphs": ["v"], "method": "tight", "beam": 250}));
    let f = s.font().unwrap();
    let l = &f.glyph("v").unwrap().layers[0];
    assert_eq!(sidebearings_at_height(f, l, 250.0).unwrap(), Some((0.0, 0.0)));
    assert_eq!(l.width, 200.0);
    let out = run(&mut s, "spacing.autoSpace", json!({"glyphs": ["v"], "method": "tight", "beam": 900}));
    assert_eq!(out["skipped"][0]["reason"], "the outline does not reach the beam", "{out}");

    // A side with a key keeps its value; a width key, a lock skip the layer.
    run(&mut s, "spacing.setLeftSidebearing", json!({"glyphs": ["z"], "value": "=x"}));
    run(&mut s, "spacing.setWidth", json!({"glyphs": ["w"], "value": "=x"}));
    lock(&mut s, "k");
    let before_w = lrw(&s, "w");
    let before_k = lrw(&s, "k");
    let out = run(&mut s, "spacing.autoSpace", json!({"glyphs": ["z", "w", "k"], "method": "tight"}));
    assert_eq!(lrw(&s, "z"), (15.0, 0.0, 315.0), "the left key keeps 15 from x");
    assert_eq!(lrw(&s, "w"), before_w);
    assert_eq!(lrw(&s, "k"), before_k);
    let reasons: Vec<(&str, &str)> =
        out["skipped"].as_array().unwrap().iter().map(|v| (v["glyph"].as_str().unwrap(), v["reason"].as_str().unwrap())).collect();
    assert_eq!(reasons, [("w", "its width has a metrics key"), ("k", "the glyph is locked")]);
}

#[test]
fn spacing_auto_space_refusals_change_nothing() {
    let mut s = Session::with_document();
    draw(&mut s, "x", rect(20.0, 0.0, 320.0, 500.0), 600.0);
    for (p, says) in [
        (json!({"glyphs": ["x"], "beam": 250}), "beam"),
        (json!({"glyphs": ["x"], "area": 50, "separation": 100}), "not both"),
        (json!({"glyphs": ["x"], "method": "loose"}), "method"),
        (json!({"glyphs": ["x"], "masters": ["nope"]}), "nope"),
        (json!({"glyphs": ["x"], "masters": []}), "empty"),
        (json!({"glyphs": []}), "empty"),
        (json!({"glyphs": ["x"], "step": 0.5}), "step"),
        (json!({"glyphs": ["x"], "min": 50, "max": 10}), "min"),
        (json!({"glyphs": ["x"], "percentOfUpm": 1e9, "method": "custom"}), "percentOfUpm"),
        // No n, o, H or O has ink to calibrate on.
        (json!({"glyphs": ["x"]}), "calibrate"),
    ] {
        let e = refused(&mut s, "spacing.autoSpace", p.clone());
        assert!(e.contains(says), "{p}: {e}");
    }
    run(&mut s, "spacing.autoSpace", json!({"glyphs": ["x"], "area": 30}));
    assert_eq!(lrw(&s, "x"), (30.0, 30.0, 360.0));
}

#[test]
fn spacing_center_in_width_centres_and_takes_a_ratio() {
    let mut s = Session::with_document();
    draw(&mut s, "c", rect(100.0, 0.0, 400.0, 500.0), 600.0);
    s.edit("anchor", |f| {
        f.glyph_mut("c").unwrap().layers[0].anchors.push(Anchor::new("top", 250.0, 500.0));
        Ok(())
    })
    .unwrap();
    let out = undo_restores(&mut s, "spacing.centerInWidth", json!({"glyphs": ["c"]}));
    assert_eq!(lrw(&s, "c"), (150.0, 150.0, 600.0));
    assert_eq!(out["values"]["c"]["m01"], json!({"left": 150.0, "right": 150.0}));
    assert_eq!(s.font().unwrap().glyph("c").unwrap().layers[0].anchors[0].x, 300.0, "the anchor moves along");
    // 1:2 puts a third of the 300 units left; a number is left ÷ right.
    run(&mut s, "spacing.centerInWidth", json!({"glyphs": ["c"], "ratio": "1:2"}));
    assert_eq!(lrw(&s, "c"), (100.0, 200.0, 600.0));
    run(&mut s, "spacing.centerInWidth", json!({"glyph": "c", "ratio": 2}));
    assert_eq!(lrw(&s, "c"), (200.0, 100.0, 600.0));
    run(&mut s, "spacing.centerInWidth", json!({"glyphs": ["c"], "ratio": 0.5}));
    assert_eq!(lrw(&s, "c"), (100.0, 200.0, 600.0));
    // Empty glyphs are skipped.
    let out = run(&mut s, "spacing.centerInWidth", json!({"glyphs": ["c", "d"]}));
    assert_eq!(out["skipped"][0]["glyph"], "d", "{out}");
    for ratio in [json!("1:x"), json!("0:0"), json!(-1), json!("2"), json!(f64::MAX)] {
        refused(&mut s, "spacing.centerInWidth", json!({"glyphs": ["c"], "ratio": ratio}));
    }
    lock(&mut s, "c");
    let e = refused(&mut s, "spacing.centerInWidth", json!({"glyph": "c"}));
    assert!(e.contains("locked"), "{e}");
}

#[test]
fn edit_copy_metrics_onto_three_glyphs() {
    let mut s = Session::with_document();
    draw(&mut s, "n", rect(50.0, 0.0, 430.0, 500.0), 500.0);
    draw(&mut s, "a", rect(0.0, 0.0, 200.0, 500.0), 300.0);
    draw(&mut s, "b", rect(100.0, 0.0, 150.0, 300.0), 300.0);
    draw(&mut s, "c", rect(-20.0, 0.0, 600.0, 500.0), 700.0);
    run(&mut s, "spacing.setLeftSidebearing", json!({"glyphs": ["c"], "value": "=a"}));
    assert!(s.font().unwrap().glyph("c").unwrap().info.metrics_keys.left.is_some());

    // Without `to`, only the values.
    let font = s.doc().unwrap().font.clone();
    let out = run(&mut s, "edit.copyMetrics", json!({"from": "n", "what": ["rsb", "lsb", "lsb"]}));
    assert_eq!(out["values"], json!({"m01": {"lsb": 50.0, "rsb": 70.0}}));
    assert!(Arc::ptr_eq(&font, &s.doc().unwrap().font));

    let out = undo_restores(&mut s, "edit.copyMetrics", json!({"from": "n", "what": ["lsb", "rsb"], "to": ["a", "b", "c"]}));
    assert_eq!(out["glyphs"], json!(["a", "b", "c"]));
    assert_eq!(lrw(&s, "a"), (50.0, 70.0, 320.0));
    assert_eq!(lrw(&s, "b"), (50.0, 70.0, 170.0));
    assert_eq!(lrw(&s, "c"), (50.0, 70.0, 740.0));
    assert_eq!(s.font().unwrap().glyph("c").unwrap().info.metrics_keys.left, None, "a copied value replaces the key");

    run(&mut s, "edit.copyMetrics", json!({"from": "n", "what": ["width", "verticalAdvance"], "to": ["a"]}));
    assert_eq!(lrw(&s, "a"), (50.0, 250.0, 500.0));
    // The font's ascender 800 to descender -200.
    assert_eq!(s.font().unwrap().glyph("a").unwrap().layers[0].info.vert_width, Some(1000.0));

    for p in [
        json!({"from": "n", "what": ["height"], "to": ["a"]}),
        json!({"from": "n", "what": [], "to": ["a"]}),
        json!({"from": "nope", "what": ["width"], "to": ["a"]}),
        json!({"from": "n", "what": ["width"], "to": ["nope"]}),
        json!({"from": "d", "what": ["lsb"], "to": ["a"]}),
        json!({"from": "n", "what": ["width"], "to": ["a"], "layers": ["nope"]}),
    ] {
        refused(&mut s, "edit.copyMetrics", p);
    }
}

#[test]
fn spacing_set_sidebearings_measure_at_the_beam() {
    let mut s = Session::with_document();
    draw(&mut s, "v", triangle(), 600.0);
    let out = run(&mut s, "spacing.setLeftSidebearing", json!({"glyphs": ["v"], "value": 30, "beam": 250}));
    assert_eq!(out["values"]["v"], 30.0);
    run(&mut s, "spacing.setRightSidebearing", json!({"glyphs": ["v"], "value": 40, "beam": 250}));
    let f = s.font().unwrap();
    let l = &f.glyph("v").unwrap().layers[0];
    assert_eq!(sidebearings_at_height(f, l, 250.0).unwrap(), Some((30.0, 40.0)));
    assert_eq!(lrw(&s, "v"), (-70.0, -60.0, 270.0), "the bounds stick out past the beam's values");
    let out = run(&mut s, "spacing.setLeftSidebearing", json!({"glyphs": ["v"], "value": 30, "beam": 600}));
    assert_eq!(out["skipped"], json!(["v"]));
    for (id, p) in [
        ("spacing.setLeftSidebearing", json!({"glyphs": ["v"], "value": "=n", "beam": 250})),
        ("spacing.setWidth", json!({"glyphs": ["v"], "value": 500, "beam": 250})),
        ("spacing.setRightSidebearing", json!({"glyphs": ["v"], "value": 10, "beam": 1e300})),
    ] {
        refused(&mut s, id, p);
    }
}
