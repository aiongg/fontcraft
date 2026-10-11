//! Spacing setters and shortcuts (M5.3): the five `spacing.set*` commands with numbers and every
//! form of metrics key, the twelve spacing shortcuts, one undo step per command, and refusals
//! (bad keys, linked masters) that change nothing.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::model::plist::Value as PValue;
use fontcraft_engine::model::{Anchor, Background, Font, Guide, Param, Path, Shape};
use fontcraft_engine::{EngineError, Session};
use fontcraft_spacing::{Field, metric};
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

/// `(left, right, width)` of a glyph's first-master layer.
fn lrw(s: &Session, glyph: &str) -> (f64, f64, f64) {
    let f = s.font().unwrap();
    let l = &f.glyph(glyph).unwrap().layers[0];
    let m = |field| metric(f, l, field).unwrap().unwrap();
    (m(Field::Left), m(Field::Right), m(Field::Width))
}

fn glyph_key(s: &Session, glyph: &str) -> Option<String> {
    s.font().unwrap().glyph(glyph).unwrap().info.metrics_keys.left.clone()
}

fn layer_key(s: &Session, glyph: &str) -> Option<String> {
    s.font().unwrap().glyph(glyph).unwrap().layers[0].info.metrics_keys.left.clone()
}

/// `n`: L50 R70 W500, with an anchor at x 250, a guide at x 100 and a background rectangle from
/// x 50. `o`: L40 R50 W500. `space` is empty.
fn session() -> Session {
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "n", "nodes": rect(50.0, 0.0, 430.0, 500.0)}));
    run(&mut s, "path.addPath", json!({"glyph": "o", "nodes": rect(40.0, 0.0, 450.0, 500.0)}));
    run(&mut s, "spacing.setWidth", json!({"glyphs": ["n", "o"], "value": 500}));
    s.edit("fixture", |f| {
        let mut g = f.glyph_mut("n").unwrap();
        let l = &mut g.layers[0];
        l.anchors.push(Anchor::new("top", 250.0, 500.0));
        l.info.guides.push(Guide { x: 100.0, y: 0.0, angle: 90.0, ..Guide::default() });
        l.info.background = Some(Background { shapes: vec![Shape::Path(Path::rect(50.0, 0.0, 430.0, 500.0))], ..Background::default() });
        Ok(())
    })
    .unwrap();
    s
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

/// Every key form in a field, one at a time, against `o` (L40 R50 W500).
fn keys_give(s: &mut Session, id: &str, glyph: &str, field: Field, cases: &[(&str, f64)]) {
    for (key, want) in cases {
        let out = run(s, id, json!({"glyphs": [glyph], "value": key}));
        assert_eq!(out["values"][glyph].as_f64(), Some(*want), "{id} {key}: {out}");
        let f = s.font().unwrap();
        let l = &f.glyph(glyph).unwrap().layers[0];
        assert_eq!(metric(f, l, field).unwrap(), Some(*want), "{id} {key}");
    }
}

#[test]
fn spacing_set_left_sidebearing_moves_everything_and_stores_keys() {
    let mut s = session();
    let out = undo_restores(&mut s, "spacing.setLeftSidebearing", json!({"glyphs": ["n"], "value": 80}));
    assert_eq!(out["values"]["n"], json!(80.0));
    assert_eq!(lrw(&s, "n"), (80.0, 70.0, 530.0), "the right sidebearing stays");
    let f = s.font().unwrap();
    let l = &f.glyph("n").unwrap().layers[0];
    assert_eq!(l.anchors[0].x, 280.0);
    assert_eq!(l.info.guides[0].x, 130.0);
    let Some(Shape::Path(bg)) = l.info.background.as_ref().map(|b| &b.shapes[0]) else { panic!("background") };
    assert_eq!(bg.nodes.iter().map(|n| n.x).fold(f64::MAX, f64::min), 80.0);

    // Each key form: bare name, `=name`, calculation, mirrored, self-mirrored, constant, height.
    keys_give(
        &mut s,
        "spacing.setLeftSidebearing",
        "n",
        Field::Left,
        &[("o", 40.0), ("=o", 40.0), ("=o+5", 45.0), ("=o*1.5", 60.0), ("=|o", 50.0), ("=64", 64.0), ("=o@250", 40.0)],
    );
    assert_eq!(glyph_key(&s, "n").as_deref(), Some("=o@250"));
    run(&mut s, "spacing.setLeftSidebearing", json!({"glyphs": ["n"], "value": "=|"}));
    let (l, r, _) = lrw(&s, "n");
    assert_eq!(l, r, "`=|` centres the outline");

    // A local key goes on the layer and hides the glyph's key there; a typed number removes the
    // key the field shows, a glyph key typed over a local one removes the local one.
    let out = run(&mut s, "spacing.setLeftSidebearing", json!({"glyphs": ["n"], "value": "==o-10"}));
    assert_eq!(out["key"], "==o-10");
    assert_eq!((layer_key(&s, "n").as_deref(), lrw(&s, "n").0), (Some("=o-10"), 30.0));
    run(&mut s, "spacing.setLeftSidebearing", json!({"glyphs": ["n"], "value": 33}));
    assert_eq!((layer_key(&s, "n"), glyph_key(&s, "n").as_deref()), (None, Some("=|")));
    run(&mut s, "spacing.setLeftSidebearing", json!({"glyphs": ["n"], "value": "==o"}));
    run(&mut s, "spacing.setLeftSidebearing", json!({"glyphs": ["n"], "value": "=o"}));
    assert_eq!((layer_key(&s, "n"), glyph_key(&s, "n").as_deref()), (None, Some("=o")));
    run(&mut s, "spacing.setLeftSidebearing", json!({"glyphs": ["n"], "value": 50}));
    assert_eq!(glyph_key(&s, "n"), None);

    // Several glyphs in one step; an empty layer is skipped.
    let out = undo_restores(&mut s, "spacing.setLeftSidebearing", json!({"glyphs": ["n", "o", "space"], "value": 20}));
    assert_eq!((out["glyphs"].clone(), out["skipped"].clone()), (json!(["n", "o"]), json!(["space"])));
    assert_eq!((lrw(&s, "n").0, lrw(&s, "o").0), (20.0, 20.0));
}

#[test]
fn spacing_set_right_sidebearing_changes_the_width() {
    let mut s = session();
    undo_restores(&mut s, "spacing.setRightSidebearing", json!({"glyphs": ["n"], "value": 100}));
    assert_eq!(lrw(&s, "n"), (50.0, 100.0, 530.0));
    keys_give(
        &mut s,
        "spacing.setRightSidebearing",
        "n",
        Field::Right,
        &[("o", 50.0), ("=o-10", 40.0), ("=|o", 40.0), ("==o/2", 25.0), ("=-12", -12.0)],
    );
    assert_eq!(lrw(&s, "n").0, 50.0, "the left sidebearing stays");
}

#[test]
fn spacing_set_width_takes_numbers_and_keys() {
    let mut s = session();
    undo_restores(&mut s, "spacing.setWidth", json!({"glyphs": ["n"], "value": 620}));
    assert_eq!(lrw(&s, "n"), (50.0, 190.0, 620.0), "the right sidebearing takes the change");
    keys_give(&mut s, "spacing.setWidth", "n", Field::Width, &[("o", 500.0), ("=o*2", 1000.0), ("=600", 600.0), ("==o", 500.0)]);
    let e = refused(&mut s, "spacing.setWidth", json!({"glyphs": ["n"], "value": "=|o"}));
    assert!(e.contains("`value`") && e.contains("byte"), "{e}");
    refused(&mut s, "spacing.setWidth", json!({"glyphs": ["n"], "value": "=o@100"}));
    refused(&mut s, "spacing.setWidth", json!({"glyphs": ["n"], "value": 70_000}));
    refused(&mut s, "spacing.setWidth", json!({"glyphs": ["n"], "value": "=o*200"}));
}

#[test]
fn spacing_set_top_and_bottom_sidebearings_change_the_vertical_box() {
    let mut s = session();
    let tb = |s: &Session| {
        let f = s.font().unwrap();
        let l = &f.glyph("n").unwrap().layers[0];
        (metric(f, l, Field::Top).unwrap().unwrap(), metric(f, l, Field::Bottom).unwrap().unwrap())
    };
    let (_, bottom) = tb(&s);
    undo_restores(&mut s, "spacing.setTopSidebearing", json!({"glyphs": ["n"], "value": 120}));
    assert_eq!(tb(&s), (120.0, bottom), "the bottom sidebearing stays");
    undo_restores(&mut s, "spacing.setBottomSidebearing", json!({"glyphs": ["n"], "value": 30}));
    assert_eq!(tb(&s), (120.0, 30.0));
    assert_eq!(lrw(&s, "n"), (50.0, 70.0, 500.0), "the horizontal metrics stay");
    // Keys: `|` is the opposite vertical side; `@` is refused.
    run(&mut s, "spacing.setTopSidebearing", json!({"glyphs": ["n"], "value": "=|"}));
    assert_eq!(tb(&s), (30.0, 30.0));
    run(&mut s, "spacing.setBottomSidebearing", json!({"glyphs": ["n"], "value": "=o"}));
    let f = s.font().unwrap();
    let o = metric(f, &f.glyph("o").unwrap().layers[0], Field::Bottom).unwrap().unwrap();
    assert_eq!(tb(&s).1, o);
    refused(&mut s, "spacing.setTopSidebearing", json!({"glyphs": ["n"], "value": "=o@10"}));
}

#[test]
fn spacing_setters_refuse_bad_keys_and_evaluation_errors() {
    let mut s = session();
    for (value, needle) in [("=o+", "byte 3"), ("=o+1+2", "byte"), ("=", "byte"), ("=nope", "nope"), ("=o/0", "zero"), ("=space", "space")] {
        let e = refused(&mut s, "spacing.setLeftSidebearing", json!({"glyphs": ["n"], "value": value}));
        assert!(e.contains("`value`") && e.contains(needle), "{value}: {e}");
    }
    refused(&mut s, "spacing.setLeftSidebearing", json!({"glyphs": ["n"], "value": "x".repeat(1_000_000)}));
    refused(&mut s, "spacing.setLeftSidebearing", json!({"glyphs": ["n"], "value": 1e9}));
    refused(&mut s, "spacing.setLeftSidebearing", json!({"glyphs": ["n", "nope"], "value": 10}));
    refused(&mut s, "spacing.setLeftSidebearing", json!({"glyphs": [], "value": 10}));
    refused(&mut s, "spacing.setLeftSidebearing", json!({"glyphs": ["n"], "layer": "nope", "value": 10}));
    // The whole list is refused when one glyph fails: `o`, which comes first, is not changed.
    run(&mut s, "spacing.setWidth", json!({"glyphs": ["n"], "value": 440}));
    refused(&mut s, "spacing.decreaseRightSidebearing", json!({"glyphs": ["o", "n"], "amount": 460}));
}

/// A second master `m02` that takes its metrics from the first.
fn with_linked_master(s: &mut Session) {
    s.edit("fixture", |f: &mut Font| {
        let mut m = f.masters[0].clone();
        m.id = "m02".into();
        m.name = "Linked".into();
        m.params.0.push(Param::new("Link Metrics With First Master", PValue::Bool(true)));
        Arc::make_mut(&mut f.masters).push(m);
        Ok(())
    })
    .unwrap();
}

#[test]
fn spacing_commands_refuse_a_linked_master() {
    let mut s = session();
    with_linked_master(&mut s);
    for id in ["spacing.setLeftSidebearing", "spacing.setWidth", "spacing.setTopSidebearing"] {
        let e = refused(&mut s, id, json!({"glyphs": ["n"], "layer": "m02", "value": 10}));
        assert!(e.contains("Linked") && e.contains("Regular"), "{e}");
    }
    for id in ["spacing.increaseLeftSidebearing", "spacing.shiftOutlineRightBy10"] {
        let e = refused(&mut s, id, json!({"glyphs": ["n"], "layer": "m02"}));
        assert!(e.contains("`layer`"), "{e}");
    }
}

#[test]
fn spacing_shortcuts_move_the_named_edge() {
    // (command, params beyond the glyph, change of left, right and width)
    let cases: [(&str, Value, (f64, f64, f64)); 14] = [
        ("spacing.increaseLeftSidebearing", json!({}), (1.0, 0.0, 1.0)),
        ("spacing.increaseLeftSidebearing", json!({"amount": 5}), (5.0, 0.0, 5.0)),
        ("spacing.increaseLeftSidebearingBy10", json!({}), (10.0, 0.0, 10.0)),
        ("spacing.decreaseLeftSidebearing", json!({}), (-1.0, 0.0, -1.0)),
        ("spacing.decreaseLeftSidebearingBy10", json!({}), (-10.0, 0.0, -10.0)),
        ("spacing.increaseRightSidebearing", json!({}), (0.0, 1.0, 1.0)),
        ("spacing.increaseRightSidebearingBy10", json!({}), (0.0, 10.0, 10.0)),
        ("spacing.decreaseRightSidebearing", json!({"amount": 2.5}), (0.0, -2.5, -2.5)),
        ("spacing.decreaseRightSidebearingBy10", json!({}), (0.0, -10.0, -10.0)),
        ("spacing.shiftOutlineLeft", json!({}), (-1.0, 1.0, 0.0)),
        ("spacing.shiftOutlineLeftBy10", json!({}), (-10.0, 10.0, 0.0)),
        ("spacing.shiftOutlineRight", json!({}), (1.0, -1.0, 0.0)),
        ("spacing.shiftOutlineRightBy10", json!({}), (10.0, -10.0, 0.0)),
        ("spacing.shiftOutlineRight", json!({"amount": 3}), (3.0, -3.0, 0.0)),
    ];
    let mut s = session();
    for (id, extra, (dl, dr, dw)) in cases {
        let (l0, r0, w0) = lrw(&s, "n");
        let mut p = json!({"glyphs": ["n"]});
        p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let out = undo_restores(&mut s, id, p);
        assert_eq!(lrw(&s, "n"), (l0 + dl, r0 + dr, w0 + dw), "{id} {extra}");
        assert_eq!(out["widths"]["n"].as_f64(), Some(w0 + dw), "{id}");
    }
    refused(&mut s, "spacing.increaseLeftSidebearing", json!({"glyphs": ["n"], "amount": 1e9}));
    refused(&mut s, "spacing.decreaseRightSidebearing", json!({"glyphs": ["n"], "amount": 600}));
    refused(&mut s, "spacing.increaseRightSidebearing", json!({"glyphs": ["n"], "layer": "nope"}));
}

#[test]
fn spacing_shortcuts_keep_metrics_keys() {
    let mut s = session();
    run(&mut s, "spacing.setLeftSidebearing", json!({"glyphs": ["n"], "value": "=o"}));
    run(&mut s, "spacing.increaseLeftSidebearingBy10", json!({"glyphs": ["n"]}));
    assert_eq!((glyph_key(&s, "n").as_deref(), lrw(&s, "n").0), (Some("=o"), 50.0), "the key stays, now out of sync");
}

#[test]
fn spacing_shift_keeps_width() {
    let mut s = session();
    run(&mut s, "spacing.shiftOutlineLeftBy10", json!({"glyphs": ["n", "o"]}));
    run(&mut s, "spacing.shiftOutlineRight", json!({"glyphs": ["n", "o"], "amount": 7}));
    assert_eq!(lrw(&s, "n"), (47.0, 73.0, 500.0));
    assert_eq!(lrw(&s, "o"), (37.0, 53.0, 500.0));
    let f = s.font().unwrap();
    assert_eq!(f.glyph("n").unwrap().layers[0].anchors[0].x, 247.0, "anchors move with the outline");
}
