//! Vertical origin and vertical width (M5.20): `spacing.setVerticalOrigin` and
//! `spacing.setVerticalWidth` with numbers and keys, one undo step each; the forms the fields
//! refuse; Update Metrics on a stale vertical-width key; the spacing shortcuts in top-to-bottom
//! text; and a `.glyphs` round trip of the keys.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_spacing::{Field, metric};
use fontcraft_testkit::{rect, snapshot};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn refused(s: &mut Session, id: &str, p: Value) -> String {
    let before = snapshot(s);
    let font = s.doc().unwrap().font.clone();
    let e = s.execute(id, &p).err().unwrap_or_else(|| panic!("{id} {p} should fail"));
    assert!(Arc::ptr_eq(&font, &s.doc().unwrap().font), "{id} failed but replaced the font");
    assert_eq!(snapshot(s), before, "{id} failed but changed the session");
    e.to_string()
}

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

/// One metric of a glyph's first-master layer.
fn m(s: &Session, glyph: &str, field: Field) -> f64 {
    let f = s.font().unwrap();
    metric(f, &f.glyph(glyph).unwrap().layers[0], field).unwrap().unwrap()
}

/// `(top, bottom, vertical origin, vertical width)` of a glyph's first-master layer.
fn vertical(s: &Session, glyph: &str) -> (f64, f64, f64, f64) {
    (m(s, glyph, Field::Top), m(s, glyph, Field::Bottom), m(s, glyph, Field::VertOrigin), m(s, glyph, Field::VertWidth))
}

/// `n` and `o`: rectangles from y 0 to 500, width 500.
fn session() -> Session {
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "n", "nodes": rect(50.0, 0.0, 430.0, 500.0)}));
    run(&mut s, "path.addPath", json!({"glyph": "o", "nodes": rect(40.0, 0.0, 450.0, 500.0)}));
    run(&mut s, "spacing.setWidth", json!({"glyphs": ["n", "o"], "value": 500}));
    s
}

#[test]
fn vertical_width_setter_takes_numbers_and_keys() {
    let mut s = session();
    let (top, bottom, origin, height) = vertical(&s, "n");
    let out = undo_restores(&mut s, "spacing.setVerticalWidth", json!({"glyphs": ["n"], "value": height + 100.0}));
    assert_eq!(out["values"]["n"].as_f64(), Some(height + 100.0), "{out}");
    assert_eq!(vertical(&s, "n"), (top, bottom + 100.0, origin, height + 100.0), "the bottom sidebearing takes the change");
    run(&mut s, "spacing.setVerticalWidth", json!({"glyphs": ["o"], "value": 1100}));
    let out = undo_restores(&mut s, "spacing.setVerticalWidth", json!({"glyphs": ["n"], "value": "=o-100"}));
    assert_eq!(out["key"], json!("=o-100"));
    assert_eq!(m(&s, "n", Field::VertWidth), 1000.0);
    assert_eq!(s.font().unwrap().glyph("n").unwrap().info.metrics_keys.vert_width.as_deref(), Some("=o-100"));
    run(&mut s, "spacing.setVerticalWidth", json!({"glyphs": ["n"], "value": "==o"}));
    assert_eq!(m(&s, "n", Field::VertWidth), 1100.0);
    assert_eq!(s.font().unwrap().glyph("n").unwrap().layers[0].info.metrics_keys.vert_width.as_deref(), Some("=o"));
    // A number replaces the key the field shows.
    run(&mut s, "spacing.setVerticalWidth", json!({"glyphs": ["n"], "value": 950}));
    assert_eq!(s.font().unwrap().glyph("n").unwrap().layers[0].info.metrics_keys.vert_width, None);
    for bad in ["=|o", "=|", "=o@100"] {
        let e = refused(&mut s, "spacing.setVerticalWidth", json!({"glyphs": ["n"], "value": bad}));
        assert!(e.contains("`value`"), "{bad}: {e}");
    }
    refused(&mut s, "spacing.setVerticalWidth", json!({"glyphs": ["n"], "value": 70_000}));
    refused(&mut s, "spacing.setVerticalWidth", json!({"glyphs": ["n"], "value": "=o*100"}));
}

#[test]
fn vertical_origin_setter_takes_numbers_and_keys() {
    let mut s = session();
    let (top, bottom, _, height) = vertical(&s, "n");
    let out = undo_restores(&mut s, "spacing.setVerticalOrigin", json!({"glyphs": ["n"], "value": -50}));
    assert_eq!(out["values"]["n"].as_f64(), Some(-50.0), "{out}");
    assert_eq!(vertical(&s, "n"), (top + 50.0, bottom - 50.0, -50.0, height), "the box moves up and keeps its height");
    run(&mut s, "spacing.setVerticalOrigin", json!({"glyphs": ["o"], "value": 30}));
    undo_restores(&mut s, "spacing.setVerticalOrigin", json!({"glyphs": ["n"], "value": "=o+10"}));
    assert_eq!(m(&s, "n", Field::VertOrigin), 40.0);
    assert_eq!(s.font().unwrap().glyph("n").unwrap().info.metrics_keys.vert_origin.as_deref(), Some("=o+10"));
    run(&mut s, "spacing.setVerticalOrigin", json!({"glyphs": ["n"], "value": "o"}));
    assert_eq!(m(&s, "n", Field::VertOrigin), 30.0);
    for bad in ["=|o", "=o@100"] {
        refused(&mut s, "spacing.setVerticalOrigin", json!({"glyphs": ["n"], "value": bad}));
    }
    refused(&mut s, "spacing.setVerticalOrigin", json!({"glyphs": ["n"], "value": 40_000}));
}

#[test]
fn update_metrics_sets_a_stale_vertical_width_key() {
    let mut s = session();
    run(&mut s, "spacing.setVerticalWidth", json!({"glyphs": ["n"], "value": "o"}));
    run(&mut s, "spacing.setVerticalWidth", json!({"glyphs": ["o"], "value": 1200}));
    let out = run(&mut s, "spacing.listOutOfSyncMetrics", json!({"glyphs": ["n"]}));
    assert_eq!(out["count"], json!(1), "{out}");
    assert_eq!(out["findings"][0]["field"], json!("vertWidth"));
    assert_eq!(out["findings"][0]["value"], json!(1200.0));
    let got = run(&mut s, "spacing.getMetrics", json!({"glyph": "n"}));
    assert_eq!(got["keys"]["vertWidth"]["inSync"], json!(false), "{got}");
    run(&mut s, "glyph.updateMetrics", json!({"glyphs": ["n"]}));
    assert_eq!(m(&s, "n", Field::VertWidth), 1200.0);
    assert_eq!(run(&mut s, "spacing.listOutOfSyncMetrics", json!({"glyphs": ["n"]}))["count"], json!(0));
}

#[test]
fn spacing_shortcuts_in_top_to_bottom_text_change_the_vertical_sidebearings() {
    let mut s = session();
    let width = m(&s, "n", Field::Width);
    let (left, right) = (m(&s, "n", Field::Left), m(&s, "n", Field::Right));
    let (top, bottom, _, height) = vertical(&s, "n");
    let out = undo_restores(&mut s, "spacing.increaseLeftSidebearing", json!({"glyphs": ["n"], "amount": 5, "vertical": true}));
    assert_eq!(out["verticalWidths"]["n"].as_f64(), Some(height + 5.0), "{out}");
    assert_eq!((m(&s, "n", Field::Top), m(&s, "n", Field::Bottom)), (top + 5.0, bottom), "the top sidebearing grows");
    undo_restores(&mut s, "spacing.increaseRightSidebearingBy10", json!({"glyphs": ["n"], "vertical": true}));
    assert_eq!((m(&s, "n", Field::Top), m(&s, "n", Field::Bottom)), (top + 5.0, bottom + 10.0), "the bottom sidebearing grows");
    undo_restores(&mut s, "spacing.shiftOutlineRight", json!({"glyphs": ["n"], "vertical": true}));
    assert_eq!(vertical(&s, "n").0, top + 6.0, "the outline moves down");
    assert_eq!(vertical(&s, "n").1, bottom + 9.0);
    assert_eq!(vertical(&s, "n").3, height + 15.0, "a shift keeps the vertical width");
    assert_eq!((m(&s, "n", Field::Left), m(&s, "n", Field::Right), m(&s, "n", Field::Width)), (left, right, width), "the horizontal metrics stay");
    refused(&mut s, "spacing.decreaseLeftSidebearing", json!({"glyphs": ["n"], "vertical": "yes"}));
}

#[test]
fn vertical_keys_survive_a_glyphs_round_trip() {
    let mut s = session();
    run(&mut s, "spacing.setVerticalWidth", json!({"glyphs": ["n"], "value": "=o+20"}));
    run(&mut s, "spacing.setVerticalOrigin", json!({"glyphs": ["n"], "value": "==o"}));
    let font = s.font().unwrap();
    let back = fontcraft_io_glyphs::read(&fontcraft_io_glyphs::write(font).unwrap()).unwrap();
    let (a, b) = (font.glyph("n").unwrap(), back.glyph("n").unwrap());
    assert_eq!(b.info.metrics_keys.vert_width, a.info.metrics_keys.vert_width);
    assert_eq!(b.layers[0].info.metrics_keys.vert_origin, a.layers[0].info.metrics_keys.vert_origin);
    assert_eq!(b.layers[0].info.vert_width, a.layers[0].info.vert_width);
    assert_eq!(b.layers[0].info.vert_origin, a.layers[0].info.vert_origin);
}
