//! Cap component commands (M13.40): `capComponent.add` on two neighbouring nodes, `decompose`,
//! `setFit`, `setGlyph`, `setScale` and `setAlignment`, their refusals, and the hint's `.glyphs`
//! round trip. The fitting itself is tested in `fontcraft_model::attach`.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use fontcraft_engine::Session;
use fontcraft_geom::Vec2;
use fontcraft_model::attach::{CapHint, cap_hint};
use fontcraft_model::corner::CornerAlignment;
use fontcraft_model::{Anchor, Font, Node, NodeKind, Path, Shape};
use fontcraft_testkit::snapshot;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn run_err(s: &mut Session, id: &str, p: Value) -> String {
    let before = snapshot(s);
    let e = s.execute(id, &p).err().unwrap_or_else(|| panic!("{id} {p} succeeded")).to_string();
    assert_eq!(snapshot(s), before, "{id} {p} failed but changed the session");
    e
}

fn lines(closed: bool, points: &[(f64, f64)]) -> Shape {
    Shape::Path(Path::new(closed, points.iter().map(|&(x, y)| Node::new(x, y, NodeKind::Line)).collect()))
}

/// `_cap.flag` and `_cap.wide` (origin caps), `_cap_flag` and `flag` (not cap glyphs), `l` (an
/// 80-unit stem and an open path) and `i` (a stem end of zero length).
fn session() -> Session {
    let mut font = Font::new("Caps");
    let flag = lines(false, &[(0.0, 20.0), (-40.0, -30.0), (100.0, -30.0), (100.0, 20.0)]);
    let glyphs: [(&str, Vec<Shape>, f64); 6] = [
        ("_cap.flag", vec![flag.clone()], 100.0),
        ("_cap.wide", vec![flag.clone()], 200.0),
        ("_cap_flag", vec![flag.clone()], 100.0),
        ("flag", vec![flag], 100.0),
        (
            "l",
            vec![
                lines(true, &[(100.0, 0.0), (180.0, 0.0), (180.0, 700.0), (100.0, 700.0)]),
                lines(false, &[(300.0, 0.0), (300.0, 100.0), (400.0, 100.0)]),
            ],
            280.0,
        ),
        ("i", vec![lines(true, &[(100.0, 0.0), (100.0, 0.0), (100.0, 500.0), (0.0, 500.0)])], 280.0),
    ];
    for (name, shapes, width) in glyphs {
        if font.glyph(name).is_none() {
            font.add_glyph(name, Vec::new(), width).unwrap();
        }
        let mut g = font.glyph_mut(name).unwrap();
        let l = g.layer_mut("m01").unwrap();
        l.shapes = shapes;
        l.width = width;
        if name.contains("flag") || name.contains("wide") {
            l.anchors.push(Anchor::new("origin", 0.0, 0.0));
        }
    }
    let mut s = Session::new();
    s.add_document(font, None).unwrap();
    s
}

fn caps(s: &Session, glyph: &str) -> Vec<CapHint> {
    let layer = s.font().unwrap().glyph(glyph).unwrap().layer("m01").unwrap();
    layer.info.hints.iter().map(|h| cap_hint(h).unwrap().unwrap()).collect()
}

fn add(s: &mut Session, points: Value) -> Value {
    run(s, "capComponent.add", json!({"glyph": "l", "points": points, "cap": "_cap.flag"}))
}

#[test]
fn cap_component_add_sits_on_the_first_node_in_path_order() {
    let mut s = session();
    let r = add(&mut s, json!([[0, 3], [0, 2]]));
    assert_eq!((r["caps"].clone(), r["replaced"].clone()), (json!([0]), json!(false)));
    assert_eq!(r["selection"], json!({"glyph": "l", "layer": "m01", "caps": [0]}));
    assert!(r.get("warning").is_none(), "{r}");
    // Model node 2 of a closed path is the file's node 1.
    assert_eq!(caps(&s, "l"), [CapHint::new("_cap.flag", 0, 1)]);
    // The same pair in the other order replaces it; the closing segment's first node is the last.
    let r = add(&mut s, json!([[0, 2], [0, 3]]));
    assert_eq!(r["replaced"], json!(true));
    add(&mut s, json!([[0, 0], [0, 3]]));
    assert_eq!(caps(&s, "l"), [CapHint::new("_cap.flag", 0, 1), CapHint::new("_cap.flag", 0, 2)]);
    // The outline has both caps fitted.
    let outline = s.font().unwrap().outline("l", "m01").unwrap();
    let ys: Vec<f64> = outline.elements().iter().filter_map(|e| e.end_point()).map(|p| p.y).collect();
    assert!(ys.contains(&730.0), "the top cap is drawn: {ys:?}");
}

#[test]
fn cap_component_add_refuses_wrong_targets_with_nothing_changed() {
    let mut s = session();
    let e = run_err(&mut s, "capComponent.add", json!({"glyph": "l", "points": [[0, 0], [0, 2]], "cap": "_cap.flag"}));
    assert!(e.contains("not neighbours"), "{e}");
    let e = run_err(&mut s, "capComponent.add", json!({"glyph": "l", "points": [[0, 0], [1, 0]], "cap": "_cap.flag"}));
    assert!(e.contains("two paths"), "{e}");
    let e = run_err(&mut s, "capComponent.add", json!({"glyph": "l", "points": [[0, 0]], "cap": "_cap.flag"}));
    assert!(e.contains("two nodes"), "{e}");
    for name in ["flag", "_cap_flag", "_cap."] {
        let e = run_err(&mut s, "capComponent.add", json!({"glyph": "l", "points": [[0, 0], [0, 1]], "cap": name}));
        assert!(e.contains("not a cap glyph"), "{name}: {e}");
    }
    let e = run_err(&mut s, "capComponent.add", json!({"glyph": "l", "points": [[0, 0], [0, 1]], "cap": "_cap.none"}));
    assert!(e.contains("no glyph"), "{e}");
    // An open path's two end nodes are not neighbours.
    let e = run_err(&mut s, "capComponent.add", json!({"glyph": "l", "points": [[1, 2], [1, 0]], "cap": "_cap.flag"}));
    assert!(e.contains("not neighbours"), "{e}");
    let e = run_err(&mut s, "capComponent.add", json!({"glyph": "l", "points": [[0, 0], [0, 9]], "cap": "_cap.flag"}));
    assert!(e.contains("resolve"), "{e}");
}

#[test]
fn cap_component_settings_write_the_stored_keys() {
    let mut s = session();
    add(&mut s, json!([[0, 2], [0, 3]]));
    run(&mut s, "capComponent.setFit", json!({"glyph": "l", "caps": [0], "fit": true}));
    run(&mut s, "capComponent.setAlignment", json!({"glyph": "l", "caps": [0], "mode": "right"}));
    run(&mut s, "capComponent.setScale", json!({"glyph": "l", "caps": [0], "scaleY": 50}));
    let r = run(&mut s, "capComponent.setGlyph", json!({"glyph": "l", "caps": [0], "cap": "_cap.wide"}));
    assert!(r.get("warning").is_none(), "{r}");
    let want = CapHint { scale: Vec2::new(1.0, 0.5), alignment: CornerAlignment::Right, fit: true, ..CapHint::new("_cap.wide", 0, 1) };
    assert_eq!(caps(&s, "l"), [want]);
    let hint = &s.font().unwrap().glyph("l").unwrap().layer("m01").unwrap().info.hints[0];
    assert_eq!(hint.value.as_dict().unwrap().get("options").and_then(|v| v.as_num()).map(|n| n.as_f64()), Some(9.0), "right (1) plus Fit (8)");
    // Alignment keeps Fit; Fit keeps the alignment.
    run(&mut s, "capComponent.setFit", json!({"glyph": "l", "caps": [0], "fit": false}));
    assert_eq!(caps(&s, "l")[0].alignment, CornerAlignment::Right);
    // Hostile values are refused with nothing changed.
    let e = run_err(&mut s, "capComponent.setScale", json!({"glyph": "l", "caps": [0], "scaleX": 1e9}));
    assert!(e.contains("10000 percent"), "{e}");
    run_err(&mut s, "capComponent.setScale", json!({"glyph": "l", "caps": [0], "scaleX": 0}));
    run_err(&mut s, "capComponent.setFit", json!({"glyph": "l", "caps": [0], "fit": "yes"}));
    run_err(&mut s, "capComponent.setAlignment", json!({"glyph": "l", "caps": [0], "mode": "up"}));
    run_err(&mut s, "capComponent.setGlyph", json!({"glyph": "l", "caps": [0], "cap": "flag"}));
    let e = run_err(&mut s, "capComponent.setFit", json!({"glyph": "l", "caps": [5], "fit": true}));
    assert!(e.contains("does not resolve"), "{e}");
}

#[test]
fn decompose_cap_writes_the_drawn_outline_and_undoes() {
    let mut s = session();
    add(&mut s, json!([[0, 2], [0, 3]]));
    add(&mut s, json!([[0, 0], [0, 1]]));
    let drawn = s.font().unwrap().outline("l", "m01").unwrap();
    let before = s.font().unwrap().glyph("l").unwrap().clone();
    let r = run(&mut s, "capComponent.decompose", json!({"glyph": "l", "caps": [0]}));
    assert_eq!(r["decomposed"], json!(1));
    assert_eq!(caps(&s, "l").len(), 1, "the other cap stays");
    assert_eq!(s.font().unwrap().outline("l", "m01").unwrap(), drawn, "the moved cap still sits on its nodes");
    run(&mut s, "capComponent.decompose", json!({"glyph": "l"}));
    assert!(caps(&s, "l").is_empty());
    assert_eq!(s.font().unwrap().outline("l", "m01").unwrap(), drawn);
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.font().unwrap().glyph("l").unwrap(), &before, "undo gives back the glyph");
    let e = run_err(&mut s, "capComponent.decompose", json!({"glyph": "i"}));
    assert!(e.contains("no cap components"), "{e}");
}

#[test]
fn a_zero_length_stem_end_warns_and_refuses_to_decompose() {
    let mut s = session();
    let r = run(&mut s, "capComponent.add", json!({"glyph": "i", "points": [[0, 0], [0, 1]], "cap": "_cap.flag"}));
    assert!(r["warning"].as_str().is_some_and(|w| w.contains("zero length")), "{r}");
    let e = run_err(&mut s, "capComponent.decompose", json!({"glyph": "i"}));
    assert!(e.contains("zero length"), "{e}");
}

#[test]
fn the_cap_hint_round_trips_through_a_glyphs_file() {
    let mut s = session();
    add(&mut s, json!([[0, 2], [0, 3]]));
    run(&mut s, "capComponent.setAlignment", json!({"glyph": "l", "caps": [0], "mode": "both"}));
    run(&mut s, "capComponent.setScale", json!({"glyph": "l", "caps": [0], "scaleX": 150, "scaleY": 75}));
    let font = s.font().unwrap();
    let text = fontcraft_io_glyphs::write(font).unwrap();
    assert!(text.contains("type = Cap;"), "{text}");
    let back = fontcraft_io_glyphs::read(&text).unwrap();
    let hints = |f: &Font| f.glyph("l").unwrap().layer("m01").unwrap().info.hints.clone();
    assert_eq!(hints(&back), hints(font));
    assert_eq!(fontcraft_io_glyphs::write(&back).unwrap(), text);
}

/// A copied cap keeps its glyph, scale, alignment and Fit, and goes onto the straight segment whose
/// two nodes are selected; one node, or two nodes of no segment, are refused with nothing changed
/// (`cap.copyPasteOntoSegments`).
#[test]
fn a_copied_cap_pastes_onto_the_two_nodes_of_a_segment() {
    let mut s = session();
    add(&mut s, json!([[0, 3], [0, 2]]));
    run(&mut s, "capComponent.setFit", json!({"glyph": "l", "caps": [0], "fit": true}));
    run(&mut s, "capComponent.setAlignment", json!({"glyph": "l", "caps": [0], "mode": "both"}));
    let text = run(&mut s, "edit.copy", json!({"glyph": "l", "caps": [0]}))["json"].as_str().unwrap().to_string();
    // The bottom segment runs from model node 0, which the file numbers 3, to model node 1.
    let r = run(&mut s, "edit.paste", json!({"glyph": "l", "json": text, "points": [[0, 0], [0, 1]]}));
    assert_eq!((r["caps"].clone(), r["flavour"].clone()), (json!([1]), json!("caps")));
    let caps = caps(&s, "l");
    assert_eq!(caps.len(), 2);
    assert_eq!((caps[1].shape, caps[1].node), (0, 3));
    assert_eq!((caps[1].fit, caps[1].alignment, caps[1].scale), (caps[0].fit, caps[0].alignment, caps[0].scale));
    assert!(caps[1].fit && caps[1].alignment == CornerAlignment::Both);
    let e = run_err(&mut s, "edit.paste", json!({"glyph": "l", "json": text, "points": [[0, 0]]}));
    assert!(e.contains("no segment to paste"), "{e}");
    let e = run_err(&mut s, "edit.paste", json!({"glyph": "l", "json": text, "points": [[0, 0], [1, 0]]}));
    assert!(e.contains("no segment to paste"), "{e}");
    let e = run_err(&mut s, "edit.copy", json!({"glyph": "l", "caps": [0], "brushes": [0]}));
    assert!(e.contains("one kind at a time"), "{e}");
}
