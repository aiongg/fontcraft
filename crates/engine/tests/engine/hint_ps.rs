//! PostScript hint commands (M10.4): adding linked, ghost and unlinked hints, ghost direction,
//! typed values and node links, deleting and clearing, the listing with its warnings, and the
//! font-wide hints with their check. Every change is one undo step; every refusal leaves the font
//! pointer-equal.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::model::{AlignmentZone, Node, NodeKind, Path, Shape, StemDef};
use fontcraft_hinting::{Edge, NodeRef, PsHint, PsHintKind, ps_hints};
use serde_json::{Value, json};

fn node(path: usize, node: usize) -> Edge {
    Edge::Node(NodeRef::new(path, node))
}

/// Glyph `I` on master `m01`: two bars, path 0 from (100, 0) to (180, 700) and path 1 from
/// (300, 0) to (380, 700), both drawn as `Path::rect` draws them (bottom left first,
/// anticlockwise), and path 2, a bar from (500, 0) to (600, 700) with an extra node at (600, 350).
/// The master has zones at the baseline (−10) and at 700 (+10), a horizontal stem of 30 and a
/// vertical stem of 80.
fn session() -> Session {
    let mut s = Session::with_document();
    s.edit("setup", |font| {
        let n = |x, y| Node::new(x, y, NodeKind::Line);
        let notched = Path::new(true, vec![n(500.0, 0.0), n(600.0, 0.0), n(600.0, 350.0), n(600.0, 700.0), n(500.0, 700.0)]);
        font.glyph_mut("I")?.layer_mut("m01").unwrap().shapes =
            vec![Shape::Path(Path::rect(100.0, 0.0, 180.0, 700.0)), Shape::Path(Path::rect(300.0, 0.0, 380.0, 700.0)), Shape::Path(notched)];
        Arc::make_mut(&mut font.stems).extend([
            StemDef { name: "crossbar".into(), horizontal: true, ..StemDef::default() },
            StemDef { name: "stem".into(), ..StemDef::default() },
        ]);
        for m in Arc::make_mut(&mut font.masters).iter_mut() {
            for v in &mut m.metric_values {
                v.over = 0.0;
            }
            m.zones = vec![AlignmentZone { pos: 0.0, size: -10.0 }, AlignmentZone { pos: 700.0, size: 10.0 }];
            m.stem_values = vec![30.0, 80.0];
        }
        Ok(())
    })
    .unwrap();
    s
}

fn hints(s: &Session) -> Vec<PsHint> {
    let layer = s.font().unwrap().glyph("I").unwrap().layer("m01").unwrap().clone();
    ps_hints(&layer).unwrap().into_iter().map(|(_, h)| h).collect()
}

fn with_glyph(p: Value) -> Value {
    let mut p = p;
    if p.get("glyph").is_none() {
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

fn query(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

#[test]
fn add_horizontal_hint_links_node_pairs_and_makes_ghosts_from_one_node() {
    let mut s = session();
    // Two nodes: one stem hint linked to both, the first as origin.
    let out = run(&mut s, "hint.ps.addHorizontalHint", json!({"nodes": [[0, 0], [0, 3]]}));
    assert_eq!(out["hints"], json!([0]));
    let h = &hints(&s)[0];
    assert!(h.horizontal && h.kind == PsHintKind::Stem);
    assert_eq!((h.origin, h.other), (node(0, 0), Some(node(0, 3))));

    // Several pairs, each on a path of its own: one hint per path.
    let out = run(&mut s, "hint.ps.addHorizontalHint", json!({"nodes": [[0, 1], [1, 0], [0, 2], [1, 3]]}));
    assert_eq!(out["hints"], json!([1, 2]));
    let list = hints(&s);
    assert_eq!((list[1].origin, list[1].other), (node(0, 1), Some(node(0, 2))));
    assert_eq!((list[2].origin, list[2].other), (node(1, 0), Some(node(1, 3))));

    // Explicit pairs may span paths.
    run(&mut s, "hint.ps.addVerticalHint", json!({"nodes": [[[0, 1], [1, 0]]]}));
    let h = &hints(&s)[3];
    assert!(!h.horizontal);
    assert_eq!((h.origin, h.other), (node(0, 1), Some(node(1, 0))));

    // One node: a ghost hint on it, its direction from the nearer zone.
    run(&mut s, "hint.ps.addHorizontalHint", json!({"nodes": [[0, 3]]}));
    run(&mut s, "hint.ps.addHorizontalHint", json!({"nodes": [[1, 0]]}));
    let list = hints(&s);
    assert_eq!((list[4].kind, list[4].origin, list[4].other), (PsHintKind::GhostTop, node(0, 3), None));
    assert_eq!((list[5].kind, list[5].origin), (PsHintKind::GhostBottom, node(1, 0)));

    // Hints may overlap: the same stem again is accepted.
    run(&mut s, "hint.ps.addHorizontalHint", json!({"nodes": [[0, 0], [0, 3]]}));
    assert_eq!(hints(&s).len(), 7);

    // Wrong selections change nothing.
    let e = refused(&mut s, "hint.ps.addHorizontalHint", json!({"nodes": [[0, 0], [0, 1], [0, 2]]}));
    assert!(e.contains("path 0"), "{e}");
    refused(&mut s, "hint.ps.addHorizontalHint", json!({"nodes": [[0, 0], [0, 0]]}));
    refused(&mut s, "hint.ps.addHorizontalHint", json!({"nodes": [[0, 9]]}));
    refused(&mut s, "hint.ps.addHorizontalHint", json!({"nodes": [[3, 0]]}));
    refused(&mut s, "hint.ps.addHorizontalHint", json!({"nodes": [[0, 0], [0, 3]], "origin": 5}));
    refused(&mut s, "hint.ps.addHorizontalHint", json!({"nodes": ["lsb"]}));
    refused(&mut s, "hint.ps.addHorizontalHint", json!({"glyph": "nope"}));
}

#[test]
fn add_vertical_hint_refuses_one_node_and_centres_an_unlinked_hint() {
    let mut s = session();
    let e = refused(&mut s, "hint.ps.addVerticalHint", json!({"nodes": [[0, 0]]}));
    assert!(e.contains("ghost"), "{e}");

    // Nothing selected: centred on the outline's box (x 100 to 600, y 0 to 700), sized to the
    // master's first stem of the direction.
    run(&mut s, "hint.ps.addVerticalHint", json!({}));
    run(&mut s, "hint.ps.addHorizontalHint", json!({}));
    run(&mut s, "hint.ps.addVerticalHint", json!({"origin": 120, "size": -20}));
    let list = hints(&s);
    assert_eq!((list[0].horizontal, list[0].origin, list[0].other), (false, Edge::Value(310.0), Some(Edge::Value(390.0))));
    assert_eq!((list[1].horizontal, list[1].origin, list[1].other), (true, Edge::Value(335.0), Some(Edge::Value(365.0))));
    assert_eq!((list[2].origin, list[2].other), (Edge::Value(120.0), Some(Edge::Value(100.0))));

    refused(&mut s, "hint.ps.addVerticalHint", json!({"origin": 1.0e9}));
}

#[test]
fn ghost_hints_take_a_direction_and_turn_back_into_stems() {
    let mut s = session();
    run(&mut s, "hint.ps.addHorizontalHint", json!({"origin": 690, "size": 10}));
    run(&mut s, "hint.ps.addHorizontalHint", json!({"origin": 20, "size": 30}));
    run(&mut s, "hint.ps.addVerticalHint", json!({"nodes": [[0, 0], [0, 1]]}));

    // Without a direction the nearer zone decides: 690 is near the top zone, 20 the baseline.
    run(&mut s, "hint.ps.makeGhostHint", json!({"hints": [0, 1]}));
    let list = hints(&s);
    assert_eq!((list[0].kind, list[0].origin, list[0].other), (PsHintKind::GhostTop, Edge::Value(690.0), None));
    assert_eq!((list[1].kind, list[1].origin), (PsHintKind::GhostBottom, Edge::Value(20.0)));
    let e = refused(&mut s, "hint.ps.makeGhostHint", json!({"hints": [2]}));
    assert!(e.contains("vertical"), "{e}");
    refused(&mut s, "hint.ps.makeGhostHint", json!({"hints": [0], "direction": "up"}));

    run(&mut s, "hint.ps.setGhostDirection", json!({"hints": [0], "direction": "bottom"}));
    assert_eq!(hints(&s)[0].kind, PsHintKind::GhostBottom);
    let e = refused(&mut s, "hint.ps.setGhostDirection", json!({"hints": [2], "direction": "top"}));
    assert!(e.contains("stem hint"), "{e}");
    refused(&mut s, "hint.ps.setGhostDirection", json!({"hints": [0]}));
    refused(&mut s, "hint.ps.setGhostDirection", json!({"hints": [9], "direction": "top"}));

    // A size makes a ghost a stem hint again.
    run(&mut s, "hint.ps.setHintValues", json!({"hint": 0, "size": 15}));
    let h = &hints(&s)[0];
    assert_eq!((h.kind, h.origin, h.other), (PsHintKind::Stem, Edge::Value(690.0), Some(Edge::Value(705.0))));
}

#[test]
fn typed_values_unlink_ends_and_attach_links_them_again() {
    let mut s = session();
    run(&mut s, "hint.ps.addHorizontalHint", json!({"nodes": [[0, 0], [0, 3]]}));

    // A typed origin unlinks the origin only.
    run(&mut s, "hint.ps.setHintValues", json!({"hint": 0, "origin": 10}));
    let h = &hints(&s)[0];
    assert_eq!((h.origin, h.other), (Edge::Value(10.0), Some(node(0, 3))));
    // A typed size unlinks the other end, at origin + size.
    run(&mut s, "hint.ps.setHintValues", json!({"hint": 0, "size": 50}));
    assert_eq!(hints(&s)[0].other, Some(Edge::Value(60.0)));
    // With both ends numbers, a typed origin keeps the size.
    run(&mut s, "hint.ps.setHintValues", json!({"hint": 0, "origin": 0}));
    assert_eq!((hints(&s)[0].origin, hints(&s)[0].other), (Edge::Value(0.0), Some(Edge::Value(50.0))));
    refused(&mut s, "hint.ps.setHintValues", json!({"hint": 0}));
    refused(&mut s, "hint.ps.setHintValues", json!({"hint": 1, "origin": 3}));

    run(&mut s, "hint.ps.attachHintToNode", json!({"hint": 0, "end": "target", "node": [0, 3]}));
    assert_eq!(hints(&s)[0].other, Some(node(0, 3)));
    run(&mut s, "hint.ps.attachHintToNode", json!({"hint": 0, "end": "origin", "node": [0, 1]}));
    assert_eq!(hints(&s)[0].origin, node(0, 1));
    // Null unlinks and keeps the position: node 1 of path 0 is at y 0.
    run(&mut s, "hint.ps.attachHintToNode", json!({"hint": 0, "end": "origin", "node": null}));
    assert_eq!(hints(&s)[0].origin, Edge::Value(0.0));

    run(&mut s, "hint.ps.addHorizontalHint", json!({"nodes": [[1, 3]]}));
    let e = refused(&mut s, "hint.ps.attachHintToNode", json!({"hint": 1, "end": "target", "node": [0, 0]}));
    assert!(e.contains("ghost"), "{e}");
    refused(&mut s, "hint.ps.attachHintToNode", json!({"hint": 0, "end": "origin"}));
    refused(&mut s, "hint.ps.attachHintToNode", json!({"hint": 0, "end": "middle", "node": [0, 0]}));
    refused(&mut s, "hint.ps.attachHintToNode", json!({"hint": 0, "end": "origin", "node": [0, 7]}));
}

#[test]
fn delete_and_clear_remove_hints() {
    let mut s = session();
    run(&mut s, "hint.ps.addHorizontalHint", json!({"nodes": [[0, 0], [0, 3]]}));
    run(&mut s, "hint.ps.addVerticalHint", json!({"nodes": [[0, 0], [0, 1]]}));
    let out = run(&mut s, "hint.ps.deleteHints", json!({"hints": [0, 0]}));
    assert_eq!(out["removed"], json!(1));
    let list = hints(&s);
    assert_eq!(list.len(), 1);
    assert!(!list[0].horizontal);
    refused(&mut s, "hint.ps.deleteHints", json!({"hints": []}));
    refused(&mut s, "hint.ps.deleteHints", json!({"hints": [1]}));

    refused(&mut s, "hint.ps.clearGlyphHints", json!({"glyphs": ["I"], "layers": ["nope"]}));
    let out = run(&mut s, "hint.ps.clearGlyphHints", json!({"glyphs": ["I"], "layers": "all"}));
    assert_eq!(out["cleared"], json!([{"glyph": "I", "layer": "m01"}]));
    assert!(hints(&s).is_empty());
    let listed = query(&mut s, "hint.ps.listHints", json!({"glyph": "I"}));
    assert_eq!((listed["hintSource"].clone(), listed["sourceLayer"].clone()), (json!(false), Value::Null));
}

#[test]
fn list_hints_reports_edges_links_source_and_warnings() {
    let mut s = session();
    run(&mut s, "hint.ps.addHorizontalHint", json!({"nodes": [[0, 0], [0, 3]]}));
    run(&mut s, "hint.ps.addHorizontalHint", json!({"origin": 335, "size": 30}));
    let out = run(&mut s, "hint.ps.addHorizontalHint", json!({"nodes": [[2, 2], [2, 3]]}));
    // Node 2 of path 2 lies halfway up a straight side: no extreme in y.
    let w = out["warnings"].as_array().unwrap();
    assert!(w.len() == 1 && w[0].as_str().unwrap().contains("not an extreme"), "{w:?}");
    run(&mut s, "hint.ps.addHorizontalHint", json!({"nodes": [[0, 3]]}));

    let listed = query(&mut s, "hint.ps.listHints", json!({"glyph": "I"}));
    assert_eq!(listed["layer"], json!("m01"));
    assert_eq!((listed["hintSource"].clone(), listed["sourceLayer"].clone()), (json!(true), json!("m01")));
    let h = &listed["hints"];
    assert_eq!(h[0]["direction"], json!("horizontal"));
    assert_eq!((h[0]["kind"].clone(), h[0]["origin"].clone(), h[0]["size"].clone()), (json!("stem"), json!(0.0), json!(700.0)));
    assert_eq!(h[0]["nodes"], json!({"origin": [0, 0], "target": [0, 3]}));
    assert_eq!(h[0]["warnings"], json!([]));
    // Horizontal hints act only on nodes at exactly their edges: 335 and 365 match no node.
    assert_eq!(h[1]["nodes"], json!({"origin": null, "target": null}));
    assert_eq!(h[1]["warnings"].as_array().unwrap().len(), 2);
    assert!(h[1]["warnings"][0].as_str().unwrap().contains("y = 335"), "{}", h[1]["warnings"]);
    assert_eq!(h[2]["warnings"].as_array().unwrap().len(), 1);
    assert_eq!((h[3]["kind"].clone(), h[3]["size"].clone()), (json!("ghostTop"), Value::Null));
    assert_eq!(listed["warnings"].as_array().unwrap().len(), 3);

    assert!(s.execute("hint.ps.listHints", &json!({"glyph": "I", "layer": "nope"})).is_err());
    assert!(s.execute("hint.ps.listHints", &json!({})).is_err());
}

#[test]
fn font_hints_are_the_private_dict_values_and_their_check() {
    let mut s = session();
    let out = query(&mut s, "hint.ps.getFontHints", json!({}));
    assert_eq!(out["master"], json!("m01"));
    assert_eq!(out["blueValues"], json!([-10, 0, 700, 710]));
    assert_eq!(out["otherBlues"], json!([]));
    assert_eq!(out["familyBlues"], json!([]));
    assert_eq!((out["stdHW"].clone(), out["stdVW"].clone()), (json!(30), json!(80)));
    assert_eq!((out["stemSnapH"].clone(), out["stemSnapV"].clone()), (json!([30]), json!([80])));
    // Without blueScale, blueShift and blueFuzz the handbook's defaults apply.
    assert_eq!((out["blueScale"].clone(), out["blueShift"].clone(), out["blueFuzz"].clone()), (json!(0.037), json!(7), json!(1)));
    assert_eq!(query(&mut s, "hint.ps.getFontHints", json!({"master": "m01"}))["blueValues"], out["blueValues"]);
    assert!(s.execute("hint.ps.getFontHints", &json!({"master": "nope"})).is_err());
    assert!(s.execute("hint.ps.getFontHints", &json!({"master": "m01", "instance": 0})).is_err());

    let checked = query(&mut s, "hint.ps.validateFontHints", json!({}));
    assert_eq!(checked["errors"], json!(0), "{checked}");
    s.edit("overlap", |font| {
        Arc::make_mut(&mut font.masters)[0].zones.push(AlignmentZone { pos: 695.0, size: 10.0 });
        Ok(())
    })
    .unwrap();
    let checked = query(&mut s, "hint.ps.validateFontHints", json!({"masters": ["m01"]}));
    let findings = checked["findings"].as_array().unwrap();
    assert!(findings.iter().any(|f| f["rule"] == json!("zones.overlap") && f["severity"] == json!("error")), "{checked}");
    assert!(s.execute("hint.ps.validateFontHints", &json!({"masters": ["nope"]})).is_err());
}
