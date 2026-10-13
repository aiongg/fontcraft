//! TrueType Snap and Stem instructions (M10.6): selections checked against the touched state,
//! default rounding per type, triple hints, and one undo step per command.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::model::{Path, Shape};
use fontcraft_hinting::{Direction, NodeRef, Rounding, StemRef, TtInstruction, TtKind, TtPoint, set_tt_instructions, tt_instructions};
use serde_json::{Value, json};

fn pt(path: usize, node: usize) -> TtPoint {
    TtPoint::Node(NodeRef::new(path, node))
}

/// Glyph `I` on master `m01` with two rectangles and the instructions `tt`.
fn session(tt: &[TtInstruction]) -> Session {
    let mut s = Session::with_document();
    s.edit("setup", |font| {
        let mut g = font.glyph_mut("I")?;
        let layer = g.layer_mut("m01").unwrap();
        layer.shapes = vec![Shape::Path(Path::rect(100.0, 0.0, 200.0, 700.0)), Shape::Path(Path::rect(300.0, 0.0, 400.0, 700.0))];
        set_tt_instructions(layer, tt).unwrap();
        Ok(())
    })
    .unwrap();
    s
}

fn tt(s: &Session) -> Vec<TtInstruction> {
    let layer = s.font().unwrap().glyph("I").unwrap().layer("m01").unwrap().clone();
    tt_instructions(&layer).unwrap().into_iter().map(|(_, t)| t).collect()
}

fn with_glyph(p: Value) -> Value {
    let mut p = p;
    p["glyph"] = json!("I");
    p
}

/// Runs `id`, checks that undo gives back the very font, runs it again and returns its result.
fn run(s: &mut Session, id: &str, p: Value) -> Value {
    let p = with_glyph(p);
    let before = s.doc().unwrap().font.clone();
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    assert!(!Arc::ptr_eq(&s.doc().unwrap().font, &before), "{id} changed nothing");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "{id}: undo does not give back the very font");
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// Runs `id` expecting a refusal that leaves the font pointer-equal; returns the message.
fn refused(s: &mut Session, id: &str, p: Value) -> String {
    let before = s.doc().unwrap().font.clone();
    let e = s.execute(id, &with_glyph(p)).expect_err(id).to_string();
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "{id}: a refusal changed the font");
    e
}

#[test]
fn snap_needs_untouched_points_and_rounds_by_default() {
    let mut s = session(&[]);
    let out = run(&mut s, "hint.tt.snapPoint", json!({"nodes": [[0, 0]]}));
    assert_eq!(out["instructions"], json!([0]));
    let snap = &tt(&s)[0];
    assert!(matches!(snap.kind, TtKind::Snap { .. }) && snap.rounding == Rounding::Round && snap.direction == Direction::Vertical);

    // One touched point in the selection refuses the whole selection.
    let e = refused(&mut s, "hint.tt.snapPoint", json!({"nodes": [[0, 1], [0, 0]]}));
    assert!(e.contains("touched"), "{e}");
    refused(&mut s, "hint.tt.snapPoint", json!({"nodes": [[0, 1], [0, 1]]}));
    refused(&mut s, "hint.tt.snapPoint", json!({"nodes": [[0, 9]]}));
    // The other direction has its own touched state.
    run(&mut s, "hint.tt.snapPoint", json!({"nodes": [[0, 0], "lsb"], "direction": "horizontal", "rounding": "none"}));
    assert_eq!(tt(&s).len(), 3);
    assert_eq!(tt(&s)[1].rounding, Rounding::NoRounding);
}

#[test]
fn stem_pairs_two_untouched_or_one_touched_and_refuses_the_rest() {
    let mut s = session(&[TtInstruction::new(TtKind::Snap { zone: Default::default() }, Direction::Vertical, pt(0, 0))]);
    // Two untouched: the first is the origin; one touched and one untouched: the touched one is.
    let out = run(&mut s, "hint.tt.addStemHint", json!({"nodes": [[0, 1], [0, 2], [0, 3], [0, 0]]}));
    assert_eq!(out["instructions"], json!([1, 2]));
    let list = tt(&s);
    assert_eq!((list[1].origin, list[1].target), (Some(pt(0, 1)), pt(0, 2)));
    assert_eq!((list[2].origin, list[2].target), (Some(pt(0, 0)), pt(0, 3)));
    assert!(list[1..].iter().all(|t| t.rounding == Rounding::NoRounding && matches!(t.kind, TtKind::Stem { stem: StemRef::Auto, .. })));

    let e = refused(&mut s, "hint.tt.addStemHint", json!({"nodes": [[1, 0], [1, 1], [1, 2]]}));
    assert!(e.contains("even"), "{e}");
    let e = refused(&mut s, "hint.tt.addStemHint", json!({"nodes": [[1, 0], [1, 1], [0, 0], [0, 1]]}));
    assert!(e.contains("both touched"), "{e}");
    refused(&mut s, "hint.tt.addStemHint", json!({"nodes": [[1, 0], [1, 1]], "stem": "no such stem"}));

    run(&mut s, "hint.tt.reverse", json!({"instructions": [1]}));
    assert_eq!((tt(&s)[1].origin, tt(&s)[1].target), (Some(pt(0, 2)), pt(0, 1)));
    refused(&mut s, "hint.tt.reverse", json!({"instructions": [0]}));
    run(&mut s, "hint.tt.setStemReference", json!({"instructions": [1, 2], "stem": "none"}));
    assert!(tt(&s)[1..].iter().all(|t| matches!(t.kind, TtKind::Stem { stem: StemRef::NoStem, .. })));
    run(&mut s, "hint.tt.setSnapZone", json!({"instructions": [0], "zone": "none"}));
    refused(&mut s, "hint.tt.setSnapZone", json!({"instructions": [1], "zone": "auto"}));
}

#[test]
fn a_triple_hint_links_three_vertical_stems_only() {
    let mut s = session(&[]);
    run(&mut s, "hint.tt.addStemHint", json!({"nodes": [[0, 0], [0, 1], [0, 2], [0, 3], [1, 0], [1, 1]]}));
    run(&mut s, "hint.tt.addStemHint", json!({"nodes": [[1, 2], [1, 3]], "direction": "horizontal"}));
    run(&mut s, "hint.tt.snapPoint", json!({"nodes": ["rsb"]}));
    refused(&mut s, "hint.tt.makeTripleHint", json!({"instructions": [0, 1]}));
    refused(&mut s, "hint.tt.makeTripleHint", json!({"instructions": [0, 1, 3]}));
    refused(&mut s, "hint.tt.makeTripleHint", json!({"instructions": [0, 1, 4]}));

    run(&mut s, "hint.tt.makeTripleHint", json!({"instructions": [2, 0, 1]}));
    let groups: Vec<Option<u32>> = tt(&s).iter().map(|t| if let TtKind::Stem { triple, .. } = t.kind { triple } else { None }).collect();
    assert_eq!(groups, vec![Some(0), Some(0), Some(0), None, None]);
    refused(&mut s, "hint.tt.makeTripleHint", json!({"instructions": [0, 1, 2]}));

    // Reversing a member dissolves the group.
    let out = run(&mut s, "hint.tt.reverse", json!({"instructions": [1]}));
    assert_eq!(out["dissolved"], json!([0, 1, 2]));
    assert!(tt(&s).iter().all(|t| !matches!(t.kind, TtKind::Stem { triple: Some(_), .. })));
}

#[test]
fn rounding_defaults_per_type_and_delta_has_none() {
    let mut delta = TtInstruction::new(TtKind::Delta { cells: Default::default() }, Direction::Vertical, pt(1, 0));
    delta.rounding = Rounding::Round;
    let mut s = session(&[delta]);
    run(&mut s, "hint.tt.snapPoint", json!({"nodes": [[0, 0]]}));
    run(&mut s, "hint.tt.addStemHint", json!({"nodes": [[0, 0], [0, 1]]}));
    let list = tt(&s);
    assert_eq!((list[1].rounding, list[2].rounding), (Rounding::Round, Rounding::NoRounding));
    // A Delta touches nothing, so its point can still be snapped.
    run(&mut s, "hint.tt.snapPoint", json!({"nodes": [[1, 0]]}));

    run(&mut s, "hint.tt.setRounding", json!({"instructions": [1, 2], "rounding": "gdiOnly"}));
    assert!(tt(&s)[1..3].iter().all(|t| t.rounding == Rounding::GdiOnly));
    refused(&mut s, "hint.tt.setRounding", json!({"instructions": [0, 1], "rounding": "up"}));
    refused(&mut s, "hint.tt.setRounding", json!({"instructions": [1], "rounding": "sideways"}));
    refused(&mut s, "hint.tt.setRounding", json!({"instructions": [9], "rounding": "up"}));
}
