//! TrueType Shift, Interpolate and Delta instructions, deleting, removing and listing them
//! (M10.7): selections checked against the touched state, Delta vertical and main-master only,
//! one undo step per command and refusals that leave the very font.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::model::{Instance, Path, Shape};
use fontcraft_hinting::{
    DeltaDir, Direction, NodeRef, Rounding, StemRef, TtInstruction, TtKind, TtPoint, ZoneRef, set_tt_instructions, tt_instructions,
};
use serde_json::{Value, json};

fn pt(path: usize, node: usize) -> TtPoint {
    TtPoint::Node(NodeRef::new(path, node))
}

fn snap(path: usize, node: usize) -> TtInstruction {
    TtInstruction::new(TtKind::Snap { zone: ZoneRef::Auto }, Direction::Vertical, pt(path, node))
}

/// Glyph `I` with two rectangles and the instructions `tt` on master `m01`; a second master
/// `m02` with its own layer, and an instance `Bold` at weight 700.
fn session(tt: &[TtInstruction]) -> Session {
    let mut s = Session::with_document();
    s.edit("setup", |font| {
        let mut m = font.masters[0].clone();
        m.id = "m02".into();
        m.name = "Black".into();
        m.location = vec![900.0];
        Arc::make_mut(&mut font.masters).push(m);
        Arc::make_mut(&mut font.instances).push(Instance { name: "Bold".into(), location: vec![700.0], ..Instance::default() });
        let mut g = font.glyph_mut("I")?;
        let layer = g.layer_mut("m01").unwrap();
        layer.shapes = vec![Shape::Path(Path::rect(100.0, 0.0, 200.0, 700.0)), Shape::Path(Path::rect(300.0, 0.0, 400.0, 700.0))];
        set_tt_instructions(layer, tt).unwrap();
        let mut other = layer.clone();
        other.id = "m02".into();
        other.master = "m02".into();
        g.layers.push(other);
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
    if p.get("glyphs").is_none() {
        p["glyph"] = json!("I");
    }
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
fn shift_copies_one_touched_origin_to_several_targets() {
    let mut s = session(&[snap(0, 0)]);
    let out = run(&mut s, "hint.tt.shiftPoints", json!({"nodes": [[0, 1], [0, 0], [1, 0]]}));
    assert_eq!(out["instructions"], json!([1, 2]));
    let list = tt(&s);
    for (t, target) in list[1..].iter().zip([pt(0, 1), pt(1, 0)]) {
        assert_eq!((&t.kind, t.origin, t.target, t.rounding), (&TtKind::Shift, Some(pt(0, 0)), target, Rounding::NoRounding));
    }
    // The explicit form, with a rounding.
    run(&mut s, "hint.tt.shiftPoints", json!({"origin": [0, 0], "targets": [[1, 1]], "rounding": "round"}));
    assert_eq!(tt(&s)[3].rounding, Rounding::Round);

    // Two touched points, no touched point, a touched target, an untouched origin.
    let e = refused(&mut s, "hint.tt.shiftPoints", json!({"nodes": [[0, 0], [0, 1], [0, 2]]}));
    assert!(e.contains("exactly one touched"), "{e}");
    refused(&mut s, "hint.tt.shiftPoints", json!({"nodes": [[0, 2], [0, 3]]}));
    refused(&mut s, "hint.tt.shiftPoints", json!({"origin": [0, 0], "targets": [[0, 2], [0, 1]]}));
    let e = refused(&mut s, "hint.tt.shiftPoints", json!({"origin": [0, 2], "targets": [[0, 3]]}));
    assert!(e.contains("not touched"), "{e}");
    // Touched vertically is untouched horizontally.
    refused(&mut s, "hint.tt.shiftPoints", json!({"nodes": [[0, 0], [0, 2]], "direction": "horizontal"}));
    refused(&mut s, "hint.tt.shiftPoints", json!({"nodes": [[0, 0]], "origin": [0, 0], "targets": [[0, 2]]}));
}

#[test]
fn interpolate_needs_two_touched_references() {
    let mut s = session(&[snap(0, 0), snap(0, 1)]);
    let out = run(&mut s, "hint.tt.interpolate", json!({"nodes": [[1, 0], [0, 1], [0, 0]]}));
    assert_eq!(out["instructions"], json!([2]));
    let t = &tt(&s)[2];
    assert_eq!((&t.kind, t.origin, t.other, t.target), (&TtKind::Interpolate, Some(pt(0, 1)), Some(pt(0, 0)), pt(1, 0)));
    assert_eq!(t.rounding, Rounding::NoRounding);
    run(&mut s, "hint.tt.interpolate", json!({"references": [[0, 0], [0, 1]], "target": [1, 1], "rounding": "up"}));

    // One touched reference; four points; a touched target.
    let e = refused(&mut s, "hint.tt.interpolate", json!({"nodes": [[0, 0], [0, 2], [0, 3]]}));
    assert!(e.contains("two touched references"), "{e}");
    refused(&mut s, "hint.tt.interpolate", json!({"nodes": [[0, 0], [0, 1], [0, 2], [0, 3]]}));
    refused(&mut s, "hint.tt.interpolate", json!({"references": [[0, 0], [0, 2]], "target": [0, 3]}));
    refused(&mut s, "hint.tt.interpolate", json!({"references": [[0, 0], [0, 1]], "target": [1, 0]}));
}

#[test]
fn delta_is_vertical_main_master_only_with_bounded_cells() {
    let mut s = session(&[snap(0, 0)]);
    // Touched or not, any number of points.
    let out = run(&mut s, "hint.tt.addDelta", json!({"nodes": [[0, 0], [1, 2]], "cells": {"Bold": {"12-13": "up", "20": "down"}}}));
    assert_eq!(out["instructions"], json!([1, 2]));
    let TtKind::Delta { cells } = &tt(&s)[1].kind else { panic!("not a Delta") };
    let bold = &cells["{700, 0, 0, 0, 0, 0}"];
    assert_eq!(bold.iter().map(|(k, v)| (*k, *v)).collect::<Vec<_>>(), vec![(12, DeltaDir::Up), (13, DeltaDir::Up), (20, DeltaDir::Down)]);

    // setDeltas changes named cells only; "none" clears one; an emptied column goes.
    run(&mut s, "hint.tt.setDeltas", json!({"instructions": [1], "cells": {"Bold": {"12": "down", "13": "none"}, "m02": {"9": "up"}}}));
    let TtKind::Delta { cells } = &tt(&s)[1].kind else { panic!("not a Delta") };
    assert_eq!(cells["{700, 0, 0, 0, 0, 0}"].len(), 2);
    assert_eq!(cells["{900, 0, 0, 0, 0, 0}"][&9], DeltaDir::Up);
    run(&mut s, "hint.tt.setDeltas", json!({"instructions": [1], "cells": {"{900, 0, 0, 0, 0, 0}": {"9": "none"}}}));
    let TtKind::Delta { cells } = &tt(&s)[1].kind else { panic!("not a Delta") };
    assert_eq!(cells.len(), 1);
    // A short key in braces names the same column as the stored form.
    run(&mut s, "hint.tt.setDeltas", json!({"instructions": [1], "cells": {"{700}": {"12": "up"}}}));
    let TtKind::Delta { cells } = &tt(&s)[1].kind else { panic!("not a Delta") };
    assert_eq!((cells.len(), cells["{700, 0, 0, 0, 0, 0}"][&12]), (1, DeltaDir::Up));

    let e = refused(&mut s, "hint.tt.addDelta", json!({"nodes": [[0, 1]], "direction": "horizontal"}));
    assert!(e.contains("vertical"), "{e}");
    let e = refused(&mut s, "hint.tt.addDelta", json!({"nodes": [[0, 1]], "layer": "m02"}));
    assert!(e.contains("main master"), "{e}");
    refused(&mut s, "hint.tt.setDeltas", json!({"instructions": [1], "layer": "m02", "cells": {"Bold": {"9": "up"}}}));
    for cells in [
        json!({"Bold": {"0": "up"}}),
        json!({"Bold": {"256": "up"}}),
        json!({"Bold": {"70000": "up"}}),
        json!({"Bold": {"20-10": "up"}}),
        json!({"Bold": {"12": "left"}}),
        json!({"Nope": {"12": "up"}}),
        json!({"Bold": 3}),
        json!({"{700, x}": {"12": "up"}}),
    ] {
        refused(&mut s, "hint.tt.addDelta", json!({"nodes": [[0, 1]], "cells": cells}));
    }
    // setDeltas acts on Delta instructions only.
    refused(&mut s, "hint.tt.setDeltas", json!({"instructions": [0], "cells": {"Bold": {"9": "up"}}}));
    refused(&mut s, "hint.tt.setDeltas", json!({"instructions": [1]}));
}

#[test]
fn moving_an_instruction_keeps_every_need_met() {
    let mut s = session(&[snap(0, 0), snap(0, 1)]);
    run(&mut s, "hint.tt.shiftPoints", json!({"nodes": [[0, 0], [1, 0]]}));
    // Nothing builds on Snap 1: its point can move.
    let out = run(&mut s, "hint.tt.moveInstruction", json!({"instruction": 1, "point": [1, 1]}));
    assert_eq!(out["instructions"], json!([1]));
    assert_eq!(tt(&s)[1].target, pt(1, 1));
    // The Shift's origin moves to the point Snap 1 touches now.
    run(&mut s, "hint.tt.moveInstruction", json!({"instruction": 2, "end": "origin", "point": [1, 1]}));
    assert_eq!((tt(&s)[2].origin, tt(&s)[2].target), (Some(pt(1, 1)), pt(1, 0)));

    let move_to = |instruction: usize, end: &str, point: Value| json!({"instruction": instruction, "end": end, "point": point});
    // Snap 1 now carries the Shift's origin.
    let e = refused(&mut s, "hint.tt.moveInstruction", move_to(1, "target", json!([0, 2])));
    assert!(e.contains("builds on instruction 1"), "{e}");
    // A Shift target must be untouched.
    let e = refused(&mut s, "hint.tt.moveInstruction", move_to(2, "target", json!([0, 0])));
    assert!(e.contains("cannot sit there"), "{e}");
    let e = refused(&mut s, "hint.tt.moveInstruction", move_to(2, "other", json!([0, 3])));
    assert!(e.contains("no such point"), "{e}");
    refused(&mut s, "hint.tt.moveInstruction", move_to(2, "target", json!([1, 0])));
    refused(&mut s, "hint.tt.moveInstruction", move_to(2, "middle", json!([0, 3])));
    refused(&mut s, "hint.tt.moveInstruction", move_to(3, "target", json!([0, 3])));
    refused(&mut s, "hint.tt.moveInstruction", move_to(0, "target", json!([0, 9])));
    // An intersection point is a point too.
    run(&mut s, "hint.tt.moveInstruction", move_to(2, "target", json!([0, 2, 1, 3])));
    assert_eq!(tt(&s)[2].target, TtPoint::Intersection { a: NodeRef::new(0, 2), b: NodeRef::new(1, 3) });
}

#[test]
fn deleting_an_instruction_reports_the_later_ones_that_needed_it() {
    let mut stem = TtInstruction::new(TtKind::Stem { stem: StemRef::Auto, triple: None }, Direction::Vertical, pt(0, 2));
    stem.origin = Some(pt(0, 1));
    let mut s = session(&[snap(0, 0), snap(0, 1), stem]);
    run(&mut s, "hint.tt.shiftPoints", json!({"nodes": [[0, 0], [1, 0]]}));
    run(&mut s, "hint.tt.interpolate", json!({"nodes": [[0, 0], [0, 1], [1, 1]]}));
    // Deleting the first Snap leaves the Shift without a touched origin and the Interpolate with
    // one untouched reference; the Stem does not depend on it.
    let out = run(&mut s, "hint.tt.deleteInstructions", json!({"instructions": [0]}));
    assert_eq!(out["deleted"], json!([0]));
    let unmet: Vec<u64> = out["unmet"].as_array().unwrap().iter().map(|u| u["instruction"].as_u64().unwrap()).collect();
    assert_eq!(unmet, vec![2, 3]);
    assert!(out["unmet"][0]["reason"].as_str().unwrap().contains("point [0, 0] is not touched"), "{out}");
    assert_eq!(tt(&s).len(), 4);
    refused(&mut s, "hint.tt.deleteInstructions", json!({"instructions": [4]}));
    refused(&mut s, "hint.tt.deleteInstructions", json!({"instructions": []}));
}

#[test]
fn deleting_a_triple_member_dissolves_its_triple_hint() {
    let stems: Vec<TtInstruction> = (0..3)
        .map(|i| {
            let mut t = TtInstruction::new(TtKind::Stem { stem: StemRef::Auto, triple: Some(0) }, Direction::Vertical, pt(i % 2, 2 + i / 2));
            t.origin = Some(pt(i % 2, i / 2));
            t
        })
        .collect();
    let mut s = session(&stems);
    let out = run(&mut s, "hint.tt.deleteInstructions", json!({"instructions": [1]}));
    assert_eq!(out["dissolved"], json!([0, 1]));
    assert!(tt(&s).iter().all(|t| matches!(t.kind, TtKind::Stem { triple: None, .. })));
}

#[test]
fn remove_hints_clears_the_layers_instructions() {
    let mut s = session(&[snap(0, 0), snap(0, 1)]);
    let out = run(&mut s, "hint.tt.removeHints", json!({"glyphs": ["I", "A"]}));
    assert_eq!(out, json!({"glyphs": ["I"], "removed": 2}));
    assert!(tt(&s).is_empty());
    let before = s.doc().unwrap().font.clone();
    assert_eq!(s.execute("hint.tt.removeHints", &json!({"glyph": "I"})).unwrap()["removed"], json!(0));
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before));
    refused(&mut s, "hint.tt.removeHints", json!({"glyphs": ["nosuch"]}));
}

#[test]
fn list_instructions_reports_every_field() {
    let mut s = session(&[snap(0, 0)]);
    s.execute("hint.tt.addStemHint", &json!({"glyph": "I", "nodes": [[0, 0], [0, 1]], "direction": "vertical"})).unwrap();
    s.execute("hint.tt.shiftPoints", &json!({"glyph": "I", "nodes": [[0, 0], [1, 0]]})).unwrap();
    s.execute("hint.tt.interpolate", &json!({"glyph": "I", "references": [[0, 0], [0, 1]], "target": [1, 1], "rounding": "gdiOnly"})).unwrap();
    s.execute("hint.tt.snapPoint", &json!({"glyph": "I", "nodes": ["lsb"], "direction": "horizontal", "zone": "none"})).unwrap();
    s.execute("hint.tt.addDelta", &json!({"glyph": "I", "nodes": [[1, 2]], "cells": {"Bold": {"14": "down"}}})).unwrap();
    let out = s.execute("hint.tt.listInstructions", &json!({"glyph": "I"})).unwrap();
    let bold = json!([{"column": "{700, 0, 0, 0, 0, 0}", "names": ["Bold"], "ppm": 14, "shift": "down"}]);
    assert_eq!(
        out,
        json!({
            "glyph": "I",
            "layer": "m01",
            "mainMaster": true,
            "instructions": [
                {"index": 0, "type": "snap", "direction": "vertical", "target": [0, 0], "zone": "auto", "rounding": "round"},
                {"index": 1, "type": "stem", "direction": "vertical", "origin": [0, 0], "target": [0, 1], "stem": "auto", "rounding": "none"},
                {"index": 2, "type": "shift", "direction": "vertical", "origin": [0, 0], "target": [1, 0], "rounding": "none"},
                {"index": 3, "type": "interpolate", "direction": "vertical", "references": [[0, 0], [0, 1]], "target": [1, 1], "rounding": "gdiOnly"},
                {"index": 4, "type": "snap", "direction": "horizontal", "target": "lsb", "zone": "none", "rounding": "round"},
                {"index": 5, "type": "delta", "direction": "vertical", "target": [1, 2], "deltas": bold},
            ],
            "unmet": [],
        })
    );
    let other = s.execute("hint.tt.listInstructions", &json!({"glyph": "I", "layer": "m02"})).unwrap();
    assert_eq!(
        (&other["mainMaster"], &other["instructions"]),
        (&json!(false), &json!([{"index": 0, "type": "snap", "direction": "vertical", "target": [0, 0], "zone": "auto", "rounding": "round"}]))
    );
}
