//! Corner component commands (M6.9): `cornerComponent.add` on nodes and extra nodes,
//! `setGlyph`, `setScale` (mirroring included), `setAlignment`, and Copy and Paste of corners
//! onto other nodes, with their refusals.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use fontcraft_engine::Session;
use fontcraft_geom::Vec2;
use fontcraft_model::corner::{CornerAlignment, CornerHint, CornerTarget, corner_hint, insert_corners};
use fontcraft_model::{Font, Node, NodeKind, Path, Shape};
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

fn path(closed: bool, nodes: &[(f64, f64, NodeKind)]) -> Shape {
    Shape::Path(Path::new(closed, nodes.iter().map(|&(x, y, k)| Node::new(x, y, k)).collect()))
}

fn lines(closed: bool, points: &[(f64, f64)]) -> Shape {
    path(closed, &points.iter().map(|&(x, y)| (x, y, NodeKind::Line)).collect::<Vec<_>>())
}

/// Two corner glyphs, `I` (a rectangle, an open path and a path with a curve) and `T` (two
/// crossing rectangles), in the model's node order.
fn session() -> Session {
    let mut font = Font::new("Corners");
    let glyphs: [(&str, Vec<Shape>); 5] = [
        ("_corner.one", vec![lines(false, &[(0.0, 50.0), (-50.0, 50.0), (-50.0, 0.0), (25.0, 0.0)])]),
        ("_corner.two", vec![lines(false, &[(0.0, 30.0), (-30.0, 30.0), (-30.0, 0.0), (15.0, 0.0)])]),
        ("notACorner", vec![lines(true, &[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)])]),
        (
            "I",
            vec![
                lines(true, &[(100.0, 0.0), (400.0, 0.0), (400.0, 500.0), (100.0, 500.0)]),
                lines(false, &[(500.0, 0.0), (500.0, 100.0), (600.0, 100.0)]),
                path(
                    true,
                    &[
                        (0.0, 600.0, NodeKind::Line),
                        (50.0, 650.0, NodeKind::OffCurve),
                        (100.0, 650.0, NodeKind::OffCurve),
                        (150.0, 600.0, NodeKind::Curve),
                    ],
                ),
            ],
        ),
        (
            "T",
            vec![
                lines(true, &[(0.0, 400.0), (500.0, 400.0), (500.0, 500.0), (0.0, 500.0)]),
                lines(true, &[(200.0, 0.0), (300.0, 0.0), (300.0, 600.0), (200.0, 600.0)]),
            ],
        ),
    ];
    for (name, shapes) in glyphs {
        // `I` and `T` come with the new font.
        if font.glyph(name).is_none() {
            font.add_glyph(name, Vec::new(), 600.0).unwrap();
        }
        font.glyph_mut(name).unwrap().layer_mut("m01").unwrap().shapes = shapes;
    }
    let mut s = Session::new();
    s.add_document(font, None).unwrap();
    s
}

fn corners(s: &Session, glyph: &str) -> Vec<CornerHint> {
    let layer = s.font().unwrap().glyph(glyph).unwrap().layer("m01").unwrap();
    layer.info.hints.iter().map(|h| corner_hint(h).unwrap().unwrap()).collect()
}

fn keys(s: &Session, glyph: &str, hint: usize) -> Vec<String> {
    let layer = s.font().unwrap().glyph(glyph).unwrap().layer("m01").unwrap();
    layer.info.hints[hint].value.as_dict().unwrap().entries.iter().map(|(k, _)| k.as_str().to_string()).collect()
}

fn undo_steps(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

/// The corner sits on the selected node, counted in the file's order, and the outline has it
/// fitted; a second corner on the same node takes its place.
#[test]
fn corner_component_add_attaches_a_corner_to_each_node() {
    let mut s = session();
    let r = run(&mut s, "cornerComponent.add", json!({"glyph": "I", "points": [[0, 0]], "corner": "_corner.one"}));
    assert_eq!((r["corners"].clone(), r["added"].clone(), r["replaced"].clone()), (json!([0]), json!(1), json!(0)));
    assert_eq!(r["selection"], json!({"glyph": "I", "layer": "m01", "corners": [0]}));
    assert!(r.get("warning").is_none(), "{r}");
    // Model node 0 of a closed path is the file's last node.
    assert_eq!(corners(&s, "I"), [CornerHint::new("_corner.one", 0, 3)]);
    assert_eq!(undo_steps(&s), 1);

    let font = s.font().unwrap();
    let fitted = insert_corners(font, font.glyph("I").unwrap().layer("m01").unwrap()).unwrap().unwrap();
    let Shape::Path(host) = &fitted[0] else { panic!() };
    let points: Vec<(f64, f64)> = host.nodes.iter().map(|n| (n.x, n.y)).collect();
    assert_eq!(points.len(), 7, "{points:?}");
    for p in [(100.0, 50.0), (50.0, 50.0), (50.0, 0.0), (125.0, 0.0)] {
        assert!(points.contains(&p), "{p:?} in {points:?}");
    }

    // Two nodes at once; node 0 already holds a corner, which is replaced in place.
    let r = run(&mut s, "cornerComponent.add", json!({"glyph": "I", "points": [[0, 0], [0, 2]], "corner": "_corner.two"}));
    assert_eq!((r["corners"].clone(), r["added"].clone(), r["replaced"].clone()), (json!([0, 1]), json!(1), json!(1)));
    assert_eq!(corners(&s, "I"), [CornerHint::new("_corner.two", 0, 3), CornerHint::new("_corner.two", 0, 1)]);
    assert_eq!(undo_steps(&s), 2);
}

#[test]
fn corner_component_add_refuses_wrong_nodes_and_glyphs() {
    let mut s = session();
    let add = |glyph: &str, points: Value, corner: &str| json!({"glyph": glyph, "points": points, "corner": corner});
    let cases = [
        (add("I", json!([[1, 0]]), "_corner.one"), "ends an open path"),
        (add("I", json!([[1, 2]]), "_corner.one"), "ends an open path"),
        (add("I", json!([[2, 1]]), "_corner.one"), "off-curve"),
        (add("I", json!([[0, 0]]), "notACorner"), "not a corner glyph"),
        (add("I", json!([[0, 0]]), "_corner.missing"), "no glyph"),
        (add("_corner.two", json!([[0, 1]]), "_corner.two"), "itself"),
        (json!({"glyph": "I", "corner": "_corner.one"}), "points"),
        (json!({"glyph": "I", "shapes": [0], "points": [[0, 0]], "corner": "_corner.one"}), "no other target list"),
        (json!({"glyph": "I", "side": "background", "points": [[0, 0]], "corner": "_corner.one"}), "foreground"),
    ];
    for (params, why) in cases {
        let e = run_err(&mut s, "cornerComponent.add", params.clone());
        assert!(e.contains(why), "{params}: {e}");
    }
    // The middle node of the open path has a segment on each side.
    run(&mut s, "cornerComponent.add", add("I", json!([[1, 1]]), "_corner.one"));
    assert_eq!(corners(&s, "I"), [CornerHint::new("_corner.one", 1, 1)]);
}

/// An extra node is named by its position: the two segments crossing there are stored, the
/// earlier path first.
#[test]
fn corner_component_add_attaches_a_corner_to_an_extra_node() {
    let mut s = session();
    let r = run(&mut s, "cornerComponent.add", json!({"glyph": "T", "extraNodes": [[200, 400]], "corner": "_corner.one"}));
    assert_eq!(r["corners"], json!([0]));
    let mut want = CornerHint::new("_corner.one", 0, 0);
    // The bottom edge of path 0 ends at its model node 1 (file node 0); the left edge of path 1
    // ends at its model node 0 (file node 3).
    want.target = CornerTarget::Intersection { shape: 0, node: 0, other_shape: 1, other_node: 3 };
    assert_eq!(corners(&s, "T"), [want]);

    let e = run_err(&mut s, "cornerComponent.add", json!({"glyph": "T", "extraNodes": [[100, 100]], "corner": "_corner.one"}));
    assert!(e.contains("no two segments"), "{e}");
    // A regular node is not an extra node.
    let e = run_err(&mut s, "cornerComponent.add", json!({"glyph": "T", "extraNodes": [[0, 400]], "corner": "_corner.one"}));
    assert!(e.contains("no two segments"), "{e}");
}

/// Replacing the glyph keeps the node, the scale and the alignment, and every other stored key.
#[test]
fn corner_component_set_glyph_keeps_node_scale_and_alignment() {
    let mut s = session();
    run(&mut s, "cornerComponent.add", json!({"glyph": "I", "points": [[0, 1]], "corner": "_corner.one"}));
    run(&mut s, "cornerComponent.setScale", json!({"glyph": "I", "corners": [0], "scaleX": 150, "scaleY": 80}));
    run(&mut s, "cornerComponent.setAlignment", json!({"glyph": "I", "corners": [0], "mode": "both"}));
    let r = run(&mut s, "cornerComponent.setGlyph", json!({"glyph": "I", "corners": [0], "corner": "_corner.two"}));
    assert_eq!(r["selection"], json!({"glyph": "I", "layer": "m01", "corners": [0]}));
    let mut want = CornerHint::new("_corner.two", 0, 0);
    want.scale = Vec2::new(1.5, 0.8);
    want.alignment = CornerAlignment::Both;
    assert_eq!(corners(&s, "I"), [want]);
    assert_eq!(keys(&s, "I", 0), ["name", "options", "origin", "scale", "type"]);

    for (params, why) in [
        (json!({"glyph": "I", "corners": [0], "corner": "notACorner"}), "not a corner glyph"),
        (json!({"glyph": "I", "corners": [3], "corner": "_corner.one"}), "does not resolve"),
        (json!({"glyph": "I", "corners": [], "corner": "_corner.one"}), "must not be empty"),
    ] {
        let e = run_err(&mut s, "cornerComponent.setGlyph", params.clone());
        assert!(e.contains(why), "{params}: {e}");
    }
}

/// A negative scale mirrors the corner; `flip`, the Palette's mirror buttons, negates the scale
/// of each corner on its axis, so mirroring twice gives the stored hint back.
#[test]
fn corner_component_set_scale_and_mirroring() {
    let mut s = session();
    run(&mut s, "cornerComponent.add", json!({"glyph": "I", "points": [[0, 0], [0, 1]], "corner": "_corner.one"}));
    run(&mut s, "cornerComponent.setScale", json!({"glyph": "I", "corners": [0], "scaleX": -100}));
    let c = corners(&s, "I");
    assert_eq!((c[0].scale, c[1].scale), (Vec2::new(-1.0, 1.0), Vec2::new(1.0, 1.0)));
    assert!(c[0].is_flipped());

    run(&mut s, "cornerComponent.setScale", json!({"glyph": "I", "corners": [0, 1], "flip": "horizontal"}));
    let c = corners(&s, "I");
    assert_eq!((c[0].scale, c[1].scale), (Vec2::new(1.0, 1.0), Vec2::new(-1.0, 1.0)));
    // An unscaled corner is written without `scale`.
    assert_eq!(keys(&s, "I", 0), ["name", "origin", "type"]);
    run(&mut s, "cornerComponent.setScale", json!({"glyph": "I", "corners": [1], "flip": "vertical"}));
    assert_eq!(corners(&s, "I")[1].scale, Vec2::new(-1.0, -1.0));
    // The Palette's mirror commands flip selected corners the same way.
    run(&mut s, "transform.mirrorHorizontal", json!({"glyph": "I", "corners": [1]}));
    assert_eq!(&*s.doc().unwrap().history.undo.last().unwrap().label, "Mirror Horizontally");
    run(&mut s, "transform.mirrorVertical", json!({"glyph": "I", "corners": [1]}));
    assert_eq!(&*s.doc().unwrap().history.undo.last().unwrap().label, "Mirror Vertically");
    assert_eq!(corners(&s, "I")[1].scale, Vec2::new(1.0, 1.0));
    assert_eq!(undo_steps(&s), 6);
    // Corners flip in place: other targets and an origin are refused, not dropped.
    for extra in [json!({"side": "background"}), json!({"shapes": [0]}), json!({"origin": [0, 0]})] {
        let mut params = json!({"glyph": "I", "corners": [1]});
        params.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let e = run_err(&mut s, "transform.mirrorHorizontal", params.clone());
        assert!(e.contains("is not taken"), "{params}: {e}");
    }
    assert_eq!(undo_steps(&s), 6);

    for (params, why) in [
        (json!({"glyph": "I", "corners": [0], "scaleX": 0}), "must not be 0"),
        (json!({"glyph": "I", "corners": [0], "scaleY": 20000}), "must not be 0"),
        (json!({"glyph": "I", "corners": [0]}), "give `scaleX`"),
        (json!({"glyph": "I", "corners": [0], "flip": "diagonal"}), "`flip`"),
    ] {
        let e = run_err(&mut s, "cornerComponent.setScale", params.clone());
        assert!(e.contains(why), "{params}: {e}");
    }
}

#[test]
fn corner_component_set_alignment_writes_the_stored_options() {
    let mut s = session();
    run(&mut s, "cornerComponent.add", json!({"glyph": "I", "points": [[0, 0]], "corner": "_corner.one"}));
    for (mode, want) in
        [("right", CornerAlignment::Right), ("none", CornerAlignment::Unaligned), ("both", CornerAlignment::Both), ("left", CornerAlignment::Left)]
    {
        run(&mut s, "cornerComponent.setAlignment", json!({"glyph": "I", "corners": [0], "mode": mode}));
        assert_eq!(corners(&s, "I")[0].alignment, want, "{mode}");
    }
    // Left is the default and is written without `options`.
    assert_eq!(keys(&s, "I", 0), ["name", "origin", "type"]);
    let e = run_err(&mut s, "cornerComponent.setAlignment", json!({"glyph": "I", "corners": [0], "mode": "unused"}));
    assert!(e.contains("`mode`"), "{e}");
}

/// A copied corner pastes onto every selected node with its scale and alignment; several copied
/// corners paste onto as many nodes; any other selection is refused and changes nothing.
#[test]
fn corner_copy_paste_onto_nodes() {
    let mut s = session();
    run(&mut s, "cornerComponent.add", json!({"glyph": "I", "points": [[0, 0]], "corner": "_corner.one"}));
    run(&mut s, "cornerComponent.setScale", json!({"glyph": "I", "corners": [0], "scaleX": -100}));
    run(&mut s, "cornerComponent.setAlignment", json!({"glyph": "I", "corners": [0], "mode": "right"}));
    let copied = run(&mut s, "edit.copy", json!({"glyph": "I", "corners": [0]}));
    assert_eq!(copied["corners"], 1);
    let text = copied["json"].as_str().unwrap().to_string();

    let r = run(&mut s, "edit.paste", json!({"glyph": "I", "json": text, "points": [[0, 1], [0, 2]]}));
    assert_eq!((r["pasted"].clone(), r["flavour"].clone(), r["corners"].clone()), (json!(2), json!("corners"), json!([1, 2])));
    let c = corners(&s, "I");
    assert_eq!(c.len(), 3);
    for (k, node) in [(1, 0), (2, 1)] {
        assert_eq!(c[k].target, CornerTarget::Node { shape: 0, node });
        assert_eq!((c[k].name.as_str(), c[k].scale, c[k].alignment), ("_corner.one", Vec2::new(-1.0, 1.0), CornerAlignment::Right));
    }

    // Into another glyph through `text`, two corners onto two nodes in order.
    run(&mut s, "cornerComponent.setGlyph", json!({"glyph": "I", "corners": [1], "corner": "_corner.two"}));
    let two = run(&mut s, "edit.copy", json!({"glyph": "I", "corners": [0, 1]}))["json"].as_str().unwrap().to_string();
    run(&mut s, "edit.paste", json!({"glyph": "T", "text": two, "points": [[1, 0], [1, 1]]}));
    let names: Vec<String> = corners(&s, "T").into_iter().map(|c| c.name).collect();
    assert_eq!(names, ["_corner.one", "_corner.two"]);

    for (params, why) in [
        (json!({"glyph": "I", "json": text}), "select the nodes"),
        (json!({"glyph": "I", "json": text, "shapes": [0]}), "paste onto nodes"),
        (json!({"glyph": "I", "json": text, "points": [[1, 0]]}), "ends an open path"),
        (json!({"glyph": "I", "json": two, "points": [[0, 3]]}), "select one node for each"),
        (json!({"glyph": "_corner.one", "json": text, "points": [[0, 1]]}), "itself"),
    ] {
        let e = run_err(&mut s, "edit.paste", params.clone());
        assert!(e.contains(why), "{params}: {e}");
    }
    let e = run_err(&mut s, "edit.copy", json!({"glyph": "T", "corners": [5]}));
    assert!(e.contains("does not resolve"), "{e}");
    // A cut of corners is refused rather than taking the whole layer.
    let e = run_err(&mut s, "edit.cut", json!({"glyph": "T", "corners": [0]}));
    assert!(e.contains("cannot be cut"), "{e}");
}
