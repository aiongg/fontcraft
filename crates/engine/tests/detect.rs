//! Detect Composites and Detect Component (M13.15): `font.detectComposites`,
//! `path.detectComponent`.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::model::{Component, Font, Glyph, Layer, Node, NodeKind, Path, Shape, Transform};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn layer<'f>(f: &'f Font, glyph: &str) -> &'f Layer {
    f.glyph(glyph).unwrap().layers.iter().find(|l| l.id == "m01").unwrap()
}

/// A closed polygon moved by `(dx, dy)`.
fn poly(points: &[(f64, f64)], dx: f64, dy: f64) -> Shape {
    Shape::Path(Path::new(true, points.iter().map(|&(x, y)| Node::new(x + dx, y + dy, NodeKind::Line)).collect()))
}

const A_OUTER: [(f64, f64); 4] = [(20.0, 0.0), (580.0, 0.0), (340.0, 700.0), (260.0, 700.0)];
const A_COUNTER: [(f64, f64); 3] = [(220.0, 200.0), (300.0, 500.0), (380.0, 200.0)];
const ACUTE: [(f64, f64); 4] = [(-50.0, 520.0), (50.0, 520.0), (90.0, 620.0), (10.0, 620.0)];
const PAREN: [(f64, f64); 6] = [(100.0, -100.0), (200.0, -100.0), (150.0, 300.0), (220.0, 700.0), (120.0, 700.0), (50.0, 300.0)];

fn add_glyph(f: &mut Font, name: &str, shapes: Vec<Shape>) {
    let mut l = Layer::new("m01", 600.0);
    l.shapes = shapes;
    let mut g = Glyph::new(name);
    g.layers.push(l);
    f.glyphs.push(g);
}

/// `A` drawn as paths, `acutecomb`, `Aacute` drawn as the paths of both (the acute moved by
/// (300, 200)), `parenleft`, `parenright` drawn as `parenleft` mirrored about x = 200, and `B`
/// holding the acute at (100, 0) and a square.
fn flattened() -> Session {
    let mut s = Session::with_document();
    s.edit("fixture", |f| {
        f.glyph_mut("A").unwrap().layers[0].shapes = vec![poly(&A_OUTER, 0.0, 0.0), poly(&A_COUNTER, 0.0, 0.0)];
        add_glyph(f, "acutecomb", vec![poly(&ACUTE, 0.0, 0.0)]);
        add_glyph(f, "Aacute", vec![poly(&A_OUTER, 0.0, 0.0), poly(&A_COUNTER, 0.0, 0.0), poly(&ACUTE, 300.0, 200.0)]);
        add_glyph(f, "parenleft", vec![poly(&PAREN, 0.0, 0.0)]);
        let mirrored: Vec<(f64, f64)> = PAREN.iter().map(|&(x, y)| (400.0 - x, y)).collect();
        let Shape::Path(mut right) = poly(&mirrored, 0.0, 0.0) else { panic!("a path") };
        right.reverse();
        add_glyph(f, "parenright", vec![Shape::Path(right)]);
        add_glyph(f, "B", vec![poly(&ACUTE, 100.0, 0.0), Shape::Path(Path::rect(0.0, 0.0, 30.0, 30.0))]);
        Ok(())
    })
    .unwrap();
    s
}

#[test]
fn detect_composites_rebuilds_a_flattened_accented_glyph_from_its_bases() {
    let mut s = flattened();
    let r = run(&mut s, "font.detectComposites", json!({"glyphs": ["A", "acutecomb", "Aacute"]}));
    assert_eq!(r["glyphs"], json!(["Aacute"]));
    assert_eq!(r["components"], json!(2));
    assert_eq!(r["stopped"], Value::Null);
    let f = s.font().unwrap();
    assert_eq!(
        layer(f, "Aacute").shapes,
        vec![Shape::Component(Component::new("A", 0.0, 0.0)), Shape::Component(Component::new("acutecomb", 300.0, 200.0))]
    );
    assert_eq!(layer(f, "A").shapes.len(), 2, "a base keeps its paths");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(layer(s.font().unwrap(), "Aacute").shapes.len(), 3, "one undo step gives the paths back");
}

#[test]
fn detect_composites_finds_a_mirrored_copy_only_with_flips() {
    let mut s = flattened();
    let before = s.doc().unwrap().font.clone();
    let r = run(&mut s, "font.detectComposites", json!({"glyphs": ["parenright"]}));
    assert_eq!(r["glyphs"], json!([]));
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "no match leaves the font as it was");
    let r = run(&mut s, "font.detectComposites", json!({"glyphs": ["parenright"], "flips": true}));
    assert_eq!(r["glyphs"], json!(["parenright"]));
    let shapes = &layer(s.font().unwrap(), "parenright").shapes;
    let [Shape::Component(c)] = shapes.as_slice() else { panic!("{shapes:?}") };
    assert_eq!(c.glyph, "parenleft");
    assert_eq!(c.transform, Transform::Decomposed { pos: (400.0, 0.0), scale: (-1.0, 1.0), angle: 0.0, slant: (0.0, 0.0) });
}

#[test]
fn detect_composites_refuses_a_bad_tolerance_and_changes_nothing() {
    let mut s = flattened();
    let before = s.doc().unwrap().font.clone();
    for bad in [json!({"tolerance": -1}), json!({"tolerance": 1000}), json!({"glyphs": ["nope"]}), json!({"flips": "yes"})] {
        assert!(s.execute("font.detectComposites", &bad).is_err(), "{bad}");
    }
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before));
}

#[test]
fn detect_component_replaces_the_chosen_paths_with_the_glyph_they_repeat() {
    let mut s = flattened();
    let r = run(&mut s, "path.detectComponent", json!({"glyph": "B", "shapes": [0]}));
    assert_eq!(r["component"], json!("acutecomb"));
    assert_eq!(r["selection"]["shapes"], json!([0]));
    let shapes = &layer(s.font().unwrap(), "B").shapes;
    assert_eq!(shapes[0], Shape::Component(Component::new("acutecomb", 100.0, 0.0)));
    assert!(matches!(shapes[1], Shape::Path(_)), "the other path stays");
    assert_eq!(shapes.len(), 2);
}

#[test]
fn detect_component_without_a_matching_glyph_is_an_error_that_changes_nothing() {
    let mut s = flattened();
    let before = s.doc().unwrap().font.clone();
    assert!(s.execute("path.detectComponent", &json!({"glyph": "B", "shapes": [1]})).is_err());
    assert!(s.execute("path.detectComponent", &json!({"glyph": "B"})).is_err(), "the whole layer is no glyph's outline");
    assert!(s.execute("path.detectComponent", &json!({"glyph": "B", "shapes": [5]})).is_err());
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before));
}

#[test]
fn detect_composites_on_many_glyphs_that_almost_match_stops_at_the_work_limit() {
    // 50,000 squares of different sizes: every glyph is a candidate base of every later one,
    // and no candidate fits.
    let mut s = Session::with_document();
    s.edit("fixture", |f| {
        for i in 0..50_000u32 {
            let side = 10.0 + 2.0 * f64::from(i);
            add_glyph(f, &format!("sq{i}"), vec![Shape::Path(Path::rect(0.0, 0.0, side, side))]);
        }
        Ok(())
    })
    .unwrap();
    let start = std::time::Instant::now();
    let out = run(&mut s, "font.detectComposites", json!({}));
    assert!(start.elapsed() < std::time::Duration::from_secs(60), "{:?}", start.elapsed());
    assert_eq!(out["components"], 0);
    assert!(out["stopped"].is_string(), "{out}");
}
