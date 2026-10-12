//! Corner and point helpers (M13.26): Make Ink Trap, Loop Corner, Replace with Corner, Align
//! Points to Background, Collapse Points and Same Width / Same Height, each with its refusals.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_model::plist::{Number, Value as Plist};
use fontcraft_model::{Component, Font, LayerSide, Node, NodeKind, Path, Shape};
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

/// A session whose glyph `g` (advance 600) holds `shapes`, with an empty glyph `b` for
/// components.
fn session(shapes: Vec<Shape>) -> Session {
    session_with(shapes, |_| {})
}

fn session_with(shapes: Vec<Shape>, change: impl FnOnce(&mut Font)) -> Session {
    let mut font = Font::new("Helpers");
    for name in ["g", "b"] {
        if font.glyph(name).is_none() {
            font.add_glyph(name, Vec::new(), 600.0).unwrap();
        }
    }
    font.glyph_mut("g").unwrap().layer_mut("m01").unwrap().shapes = shapes;
    change(&mut font);
    let mut s = Session::new();
    s.add_document(font, None).unwrap();
    s
}

fn nodes(s: &Session, shape: usize) -> Vec<(f64, f64)> {
    let layer = s.font().unwrap().glyph("g").unwrap().layer("m01").unwrap();
    match &layer.shapes[shape] {
        Shape::Path(p) => p.nodes.iter().map(|n| (n.x, n.y)).collect(),
        other => panic!("shape {shape} is {other:?}"),
    }
}

fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// An L whose inside corner is node 3 at (100, 100).
fn ell() -> Shape {
    lines(true, &[(0.0, 0.0), (300.0, 0.0), (300.0, 100.0), (100.0, 100.0), (100.0, 400.0), (0.0, 400.0)])
}

#[test]
fn make_ink_trap_splits_a_corner_by_the_given_or_the_masters_width() {
    let mut s = session(vec![ell()]);
    let out = run(&mut s, "path.makeInkTrap", json!({"glyph": "g", "points": [[0, 3]], "width": 20}));
    assert_eq!(out["selection"]["points"], json!([[0, 3], [0, 4]]));
    let n = nodes(&s, 0);
    assert_eq!(n.len(), 7);
    assert!((distance(n[3], n[4]) - 20.0).abs() <= 1.0, "{n:?}");
    // The trap cuts into the ink: the copy ending the bottom arm moves into the arm.
    assert!(n[3].0 > 100.0 && n[3].1 < 100.0, "{n:?}");

    let mut s = session_with(vec![ell()], |f| Arc::make_mut(&mut f.masters)[0].params.set("inkTrapWidth", Plist::Number(Number::int(40))));
    run(&mut s, "path.makeInkTrap", json!({"glyph": "g", "points": [[0, 3]]}));
    let n = nodes(&s, 0);
    assert!((distance(n[3], n[4]) - 40.0).abs() <= 1.0, "{n:?}");
}

#[test]
fn make_ink_trap_refuses_what_is_not_a_corner() {
    let mut s = session(vec![ell(), lines(false, &[(500.0, 0.0), (600.0, 0.0)])]);
    // The ends of an open path have one segment each.
    let e = run_err(&mut s, "path.makeInkTrap", json!({"glyph": "g", "points": [[1, 0], [1, 1]]}));
    assert!(e.contains("corner"), "{e}");
    run_err(&mut s, "path.makeInkTrap", json!({"glyph": "g", "points": []}));
    run_err(&mut s, "path.makeInkTrap", json!({"glyph": "g", "points": [[0, 3]], "width": -5}));
    run_err(&mut s, "path.makeInkTrap", json!({"glyph": "g", "shapes": [0]}));
}

#[test]
fn loop_corner_crosses_the_segments_past_the_corner() {
    let mut s = session(vec![lines(true, &[(100.0, 0.0), (400.0, 0.0), (400.0, 500.0), (100.0, 500.0)])]);
    let out = run(&mut s, "path.loopCorner", json!({"glyph": "g", "points": [[0, 1]], "size": 10}));
    assert_eq!(out["selection"]["points"], json!([[0, 1], [0, 2]]));
    assert_eq!(nodes(&s, 0), vec![(100.0, 0.0), (410.0, 0.0), (400.0, -10.0), (400.0, 500.0), (100.0, 500.0)]);
    run_err(&mut s, "path.loopCorner", json!({"glyph": "g", "points": [[0, 0]], "size": 0}));
    run_err(&mut s, "path.loopCorner", json!({"glyph": "g"}));
}

#[test]
fn replace_with_corner_restores_a_cut_corner_and_closes_a_gap() {
    let chamfer = lines(true, &[(0.0, 0.0), (100.0, 0.0), (100.0, 80.0), (80.0, 100.0), (0.0, 100.0)]);
    let open = lines(false, &[(200.0, 60.0), (200.0, 0.0), (300.0, 0.0), (260.0, 40.0)]);
    let mut s = session(vec![chamfer, open]);
    let out = run(&mut s, "path.replaceWithCorner", json!({"glyph": "g", "segments": [[0, 3]], "shapes": [1]}));
    assert_eq!(out["selection"]["points"], json!([[0, 2], [1, 2]]));
    assert_eq!(nodes(&s, 0), vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
    assert_eq!(nodes(&s, 1), vec![(200.0, 0.0), (300.0, 0.0), (200.0, 100.0)]);
    let layer = s.font().unwrap().glyph("g").unwrap().layer("m01").unwrap();
    assert!(matches!(&layer.shapes[1], Shape::Path(p) if p.closed));
}

#[test]
fn replace_with_corner_refuses_parallel_neighbours_and_wrong_targets() {
    let slot = lines(true, &[(0.0, 0.0), (100.0, 0.0), (100.0, 10.0), (0.0, 10.0)]);
    let mut s = session(vec![slot]);
    let e = run_err(&mut s, "path.replaceWithCorner", json!({"glyph": "g", "segments": [[0, 2]]}));
    assert!(e.contains("parallel"), "{e}");
    let e = run_err(&mut s, "path.replaceWithCorner", json!({"glyph": "g", "shapes": [0]}));
    assert!(e.contains("closed"), "{e}");
    run_err(&mut s, "path.replaceWithCorner", json!({"glyph": "g", "segments": [[0, 1], [0, 2]]}));
    run_err(&mut s, "path.replaceWithCorner", json!({"glyph": "g"}));
}

#[test]
fn align_to_mask_snaps_points_within_the_tolerance_to_background_nodes() {
    let mut s = session_with(vec![lines(true, &[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)])], |f| {
        let mut g = f.glyph_mut("g").unwrap();
        let layer = g.layer_mut("m01").unwrap();
        layer.parts_mut(LayerSide::Background).shapes.push(lines(true, &[(10.0, -20.0), (300.0, 0.0), (300.0, 300.0)]));
    });
    let out = run(&mut s, "path.alignToMask", json!({"glyph": "g", "points": [[0, 0], [0, 2]]}));
    assert_eq!(out["moved"], json!(1));
    assert_eq!(nodes(&s, 0), vec![(10.0, -20.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
    // A smaller tolerance leaves the node where it is.
    let out = run(&mut s, "path.alignToMask", json!({"glyph": "g", "points": [[0, 1]], "tolerance": 5}));
    assert_eq!(out["moved"], json!(0));
    run_err(&mut s, "path.alignToMask", json!({"glyph": "g", "points": [[0, 1]], "tolerance": 0}));
    run_err(&mut s, "path.alignToMask", json!({"glyph": "g", "points": [[0, 1]], "side": "background"}));

    let mut s = session(vec![lines(true, &[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)])]);
    let e = run_err(&mut s, "path.alignToMask", json!({"glyph": "g", "points": [[0, 0]]}));
    assert!(e.contains("background"), "{e}");
}

#[test]
fn collapse_points_meets_at_the_centre_with_retracted_handles() {
    let curve = Shape::Path(Path::new(
        false,
        vec![
            Node::new(0.0, 0.0, NodeKind::Line),
            Node::new(10.0, 40.0, NodeKind::OffCurve),
            Node::new(90.0, 40.0, NodeKind::OffCurve),
            Node::new(100.0, 0.0, NodeKind::Curve),
        ],
    ));
    let mut s = session(vec![curve, lines(false, &[(0.0, 200.0), (50.0, 300.0)])]);
    let out = run(&mut s, "path.collapsePoints", json!({"glyph": "g", "points": [[0, 0], [0, 3], [1, 1]]}));
    assert_eq!((out["x"].clone(), out["y"].clone()), (json!(50.0), json!(150.0)));
    assert!(nodes(&s, 0).iter().all(|&p| p == (50.0, 150.0)), "{:?}", nodes(&s, 0));
    assert_eq!(nodes(&s, 1), vec![(0.0, 200.0), (50.0, 150.0)]);
    let e = run_err(&mut s, "path.collapsePoints", json!({"glyph": "g", "points": [[1, 0]]}));
    assert!(e.contains("two"), "{e}");
}

#[test]
fn scale_to_same_size_matches_the_widest_path_or_the_advance() {
    let small = lines(true, &[(0.0, 0.0), (100.0, 0.0), (100.0, 50.0), (0.0, 50.0)]);
    let wide = lines(true, &[(300.0, 0.0), (500.0, 0.0), (500.0, 50.0), (300.0, 50.0)]);
    let mut s = session(vec![small.clone(), wide]);
    run(&mut s, "path.scaleToSameSize", json!({"glyph": "g", "shapes": [0, 1], "dimension": "width", "proportional": false}));
    assert_eq!(nodes(&s, 0), vec![(-50.0, 0.0), (150.0, 0.0), (150.0, 50.0), (-50.0, 50.0)]);
    assert_eq!(nodes(&s, 1), vec![(300.0, 0.0), (500.0, 0.0), (500.0, 50.0), (300.0, 50.0)]);

    let mut s = session(vec![small]);
    run(&mut s, "path.scaleToSameSize", json!({"glyph": "g", "shapes": [0], "dimension": "width"}));
    assert_eq!(nodes(&s, 0), vec![(-250.0, -125.0), (350.0, -125.0), (350.0, 175.0), (-250.0, 175.0)]);
}

#[test]
fn scale_to_same_size_refuses_components_and_unknown_dimensions() {
    let mut s = session(vec![lines(true, &[(0.0, 0.0), (100.0, 0.0), (100.0, 50.0)]), Shape::Component(Component::new("b", 0.0, 0.0))]);
    run_err(&mut s, "path.scaleToSameSize", json!({"glyph": "g", "shapes": [0], "dimension": "depth"}));
    let e = run_err(&mut s, "path.scaleToSameSize", json!({"glyph": "g", "shapes": [0, 1], "dimension": "width"}));
    assert!(e.contains("not a path"), "{e}");
    run_err(&mut s, "path.scaleToSameSize", json!({"glyph": "g", "dimension": "height"}));
}
