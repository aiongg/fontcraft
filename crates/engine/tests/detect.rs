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

/// A detected component: placed at (`x`, `y`) with automatic alignment off.
fn placed(glyph: &str, x: f64, y: f64) -> Component {
    let mut c = Component::new(glyph, x, y);
    c.info.alignment = fontcraft_engine::model::Alignment::Disabled;
    c
}

fn add_glyph(f: &mut Font, name: &str, shapes: Vec<Shape>) {
    let mut l = Layer::new("m01", 600.0);
    l.shapes = shapes;
    let mut g = Glyph::new(name);
    g.layers.push(l);
    // A new document seeds `B` and other basic glyphs; the fixture replaces them.
    f.glyphs.remove(name);
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
    assert_eq!(layer(f, "Aacute").shapes, vec![Shape::Component(placed("A", 0.0, 0.0)), Shape::Component(placed("acutecomb", 300.0, 200.0))]);
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
    assert_eq!(shapes[0], Shape::Component(placed("acutecomb", 100.0, 0.0)));
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

fn read_tree(dir: &std::path::Path, root: &std::path::Path, out: &mut fontcraft_io_glyphs::Files) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            read_tree(&p, root, out);
        } else {
            out.insert(p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"), std::fs::read(&p).unwrap());
        }
    }
}

/// `docs/plans/M13.md` § 5: Inter Roman's Regular exported as OTF (which flattens every
/// composite) and opened again; Detect Composites at tolerance 0.5 and 1 rebuilds the source's
/// composites made only of unscaled, unrotated components, with the same base glyphs.
/// `cargo test --release -p fontcraft-engine --test detect -- --ignored --nocapture`.
#[test]
#[ignore = "corpus measure"]
fn composite_recovery_on_inter() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/inter/src/Inter-Roman.glyphspackage");
    if !dir.is_dir() {
        return;
    }
    let mut files = fontcraft_io_glyphs::Files::new();
    read_tree(&dir, &dir, &mut files);
    let source = fontcraft_io_glyphs::read_package(&files).unwrap();
    let master = source.masters.iter().find(|m| m.name == "Regular").unwrap().id.clone();
    // Each composite of the source to look for: its name and its base glyphs, sorted.
    let mut wanted: Vec<(String, Vec<String>)> = Vec::new();
    for g in source.glyphs.iter().filter(|g| g.export) {
        let Some(l) = g.layers.iter().find(|l| l.id == master) else { continue };
        let mut bases = Vec::new();
        let all_plain = !l.shapes.is_empty()
            && l.shapes.iter().all(|s| match s {
                Shape::Component(c) => {
                    let [a, b, c2, d, _, _] = c.affine().as_coeffs();
                    bases.push(c.glyph.clone());
                    (a, b, c2, d) == (1.0, 0.0, 0.0, 1.0)
                }
                _ => false,
            });
        if all_plain {
            bases.sort();
            wanted.push((g.name.clone(), bases));
        }
    }
    let out = std::env::temp_dir().join(format!("fontcraft-recovery-{}", std::process::id()));
    std::fs::create_dir_all(&out).unwrap();
    let otf = out.join("Inter-Regular.otf");
    let mut s = Session::new();
    s.add_document(source, None).unwrap();
    run(&mut s, "font.customParameters.add", json!({"scope": "font", "name": "Don't use Production Names", "value": true}));
    run(&mut s, "file.exportStatic", json!({"instances": ["Regular"], "outlines": "cff", "destination": otf, "timestamp": 0}));
    for tolerance in [0.5, 1.0] {
        let mut flat = Session::new();
        run(&mut flat, "file.open", json!({"path": otf}));
        let t = std::time::Instant::now();
        let r = run(&mut flat, "font.detectComposites", json!({"tolerance": tolerance}));
        let took = t.elapsed();
        let font = flat.font().unwrap();
        let (mut found, mut other_bases, mut missed) = (0, 0, Vec::new());
        for (name, bases) in &wanted {
            let got = font.glyph(name).and_then(|g| g.layers.first()).map(|l| {
                let mut b: Vec<String> = l
                    .shapes
                    .iter()
                    .filter_map(|s| match s {
                        Shape::Component(c) => Some(c.glyph.clone()),
                        _ => None,
                    })
                    .collect();
                b.sort();
                (b, l.shapes.iter().all(|s| matches!(s, Shape::Component(_))))
            });
            match got {
                Some((b, true)) if &b == bases => found += 1,
                Some((b, true)) => {
                    if other_bases < 25 {
                        println!("  {name}: source {bases:?}, rebuilt {b:?}");
                    }
                    other_bases += 1
                }
                _ => missed.push(name.as_str()),
            }
        }
        let share = 100.0 * f64::from(found) / wanted.len() as f64;
        println!(
            "tolerance {tolerance}: {found} of {} composites rebuilt with their bases ({share:.1}%), {other_bases} with other bases, {} missed; {} glyphs changed in {took:?}",
            wanted.len(),
            missed.len(),
            r["glyphs"].as_array().map_or(0, Vec::len)
        );
        println!("  missed (first 30): {:?}", missed.iter().take(30).collect::<Vec<_>>());
    }
    let _ = std::fs::remove_dir_all(&out);
}
