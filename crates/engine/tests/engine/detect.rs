//! Detect Composites and Detect Component (M13.15): `font.detectComposites`,
//! `path.detectComponent`.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::model::{Alignment, Anchor, Component, Font, Glyph, Layer, Node, NodeKind, Path, Shape, Transform};
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

/// A detected component placed at (`x`, `y`) with automatic alignment `alignment`.
fn placed_as(glyph: &str, x: f64, y: f64, alignment: Alignment) -> Component {
    let mut c = Component::new(glyph, x, y);
    c.info.alignment = alignment;
    c
}

/// A detected component: placed at (`x`, `y`) with automatic alignment off.
fn placed(glyph: &str, x: f64, y: f64) -> Component {
    placed_as(glyph, x, y, Alignment::Disabled)
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

/// The fixture with a `top` anchor on `A` and `_top` on `acutecomb` that put the acute
/// (`acute_x`, 0) away from where `Aacute` draws it.
fn anchored(acute_x: f64) -> Session {
    let mut s = flattened();
    s.edit("anchors", |f| {
        f.glyph_mut("A").unwrap().layers[0].anchors.push(Anchor::new("top", 300.0 - acute_x, 700.0));
        f.glyph_mut("acutecomb").unwrap().layers[0].anchors.push(Anchor::new("_top", 0.0, 500.0));
        Ok(())
    })
    .unwrap();
    s
}

/// Without anchors the acute would sit after the `A` and widen the glyph, so both new components
/// are left unaligned (the test above). With anchors that reproduce the drawing, both stay
/// aligned; with anchors that would move the acute, only the acute is left unaligned.
#[test]
fn detect_composites_keeps_alignment_only_where_it_reproduces_the_outline() {
    let mut s = anchored(0.0);
    run(&mut s, "font.detectComposites", json!({"glyphs": ["Aacute"]}));
    let aligned = |name: &str, x: f64, y: f64| Shape::Component(placed_as(name, x, y, Alignment::Default));
    assert_eq!(layer(s.font().unwrap(), "Aacute").shapes, vec![aligned("A", 0.0, 0.0), aligned("acutecomb", 300.0, 200.0)]);
    assert_eq!(layer(s.font().unwrap(), "Aacute").width, 600.0);

    let mut s = anchored(40.0);
    run(&mut s, "font.detectComposites", json!({"glyphs": ["Aacute"]}));
    assert_eq!(layer(s.font().unwrap(), "Aacute").shapes, vec![aligned("A", 0.0, 0.0), Shape::Component(placed("acutecomb", 300.0, 200.0))]);
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
    // Alignment stays at its default: a layer that keeps a path does not align its components.
    assert_eq!(shapes[0], Shape::Component(placed_as("acutecomb", 100.0, 0.0, Alignment::Default)));
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

/// A session whose font holds only `glyphs`, in this order.
fn only(glyphs: Vec<(&str, Vec<Shape>)>) -> Session {
    let mut s = Session::with_document();
    s.edit("fixture", |f| {
        f.glyphs.retain(|_| false);
        for (name, shapes) in glyphs {
            add_glyph(f, name, shapes);
        }
        Ok(())
    })
    .unwrap();
    s
}

/// The components of `glyph`'s layer as (base, x, y), in shape order; panics on any other shape.
fn parts(s: &Session, glyph: &str) -> Vec<(String, f64, f64)> {
    layer(s.font().unwrap(), glyph)
        .shapes
        .iter()
        .map(|sh| match sh {
            Shape::Component(c) => {
                let [.., x, y] = c.affine().as_coeffs();
                (c.glyph.clone(), x, y)
            }
            other => panic!("{glyph}: {other:?} is not a component"),
        })
        .collect()
}

fn part(glyph: &str, x: f64, y: f64) -> (String, f64, f64) {
    (glyph.to_string(), x, y)
}

const BREVE: [(f64, f64); 5] = [(-60.0, 520.0), (60.0, 520.0), (80.0, 600.0), (0.0, 560.0), (-80.0, 600.0)];
const DOT: [(f64, f64); 4] = [(-30.0, -120.0), (30.0, -120.0), (30.0, -60.0), (-30.0, -60.0)];
const TWO: [(f64, f64); 6] = [(0.0, 0.0), (200.0, 0.0), (200.0, 40.0), (70.0, 40.0), (200.0, 300.0), (0.0, 300.0)];
const SLASH: [(f64, f64); 4] = [(250.0, 0.0), (290.0, 0.0), (450.0, 700.0), (410.0, 700.0)];

/// Of two glyphs with the same outline, the combining mark is the base: `acute` comes first in
/// the font and is drawn exactly where `Aacute` has its accent, yet `Aacute` and `acute` both
/// get `acutecomb`.
#[test]
fn detect_composites_prefers_the_combining_mark_among_identical_outlines() {
    let mut s = only(vec![
        ("A", vec![poly(&A_OUTER, 0.0, 0.0), poly(&A_COUNTER, 0.0, 0.0)]),
        ("acute", vec![poly(&ACUTE, 300.0, 200.0)]),
        ("acutecomb", vec![poly(&ACUTE, 0.0, 0.0)]),
        ("Aacute", vec![poly(&A_OUTER, 0.0, 0.0), poly(&A_COUNTER, 0.0, 0.0), poly(&ACUTE, 300.0, 200.0)]),
    ]);
    let r = run(&mut s, "font.detectComposites", json!({}));
    assert_eq!(r["glyphs"], json!(["acute", "Aacute"]));
    assert_eq!(parts(&s, "Aacute"), vec![part("A", 0.0, 0.0), part("acutecomb", 300.0, 200.0)]);
    assert_eq!(parts(&s, "acute"), vec![part("acutecomb", 300.0, 200.0)]);
    assert!(matches!(layer(s.font().unwrap(), "acutecomb").shapes.as_slice(), [Shape::Path(_)]), "the mark keeps its outline");
}

/// A glyph detected as a composite of two glyphs (`Abreve`) is not used for part of a larger
/// glyph: `Abrevedotbelow` is built from `A`, `brevecomb` and `dotbelowcomb`. It still serves a
/// glyph whose whole outline it is (`Abrevecyrillic`), and a stacked combining mark made of
/// two marks (`brevedotcomb`) is still a base.
#[test]
fn detect_composites_builds_from_the_parts_not_from_a_detected_composite() {
    let mut s = only(vec![
        ("A", vec![poly(&A_OUTER, 0.0, 0.0), poly(&A_COUNTER, 0.0, 0.0)]),
        ("brevecomb", vec![poly(&BREVE, 0.0, 0.0)]),
        ("dotbelowcomb", vec![poly(&DOT, 0.0, 0.0)]),
        ("acutecomb", vec![poly(&ACUTE, 0.0, 0.0)]),
        ("breveacutecomb", vec![poly(&BREVE, 0.0, 0.0), poly(&ACUTE, 0.0, 120.0)]),
        ("Abreve", vec![poly(&A_OUTER, 0.0, 0.0), poly(&A_COUNTER, 0.0, 0.0), poly(&BREVE, 300.0, 200.0)]),
        ("Abrevecyrillic", vec![poly(&A_OUTER, 0.0, 0.0), poly(&A_COUNTER, 0.0, 0.0), poly(&BREVE, 300.0, 200.0)]),
        ("Abrevedotbelow", vec![poly(&A_OUTER, 0.0, 0.0), poly(&A_COUNTER, 0.0, 0.0), poly(&BREVE, 300.0, 200.0), poly(&DOT, 300.0, 0.0)]),
        ("Abreveacute", vec![poly(&A_OUTER, 0.0, 0.0), poly(&A_COUNTER, 0.0, 0.0), poly(&BREVE, 300.0, 200.0), poly(&ACUTE, 300.0, 320.0)]),
    ]);
    run(&mut s, "font.detectComposites", json!({}));
    assert_eq!(parts(&s, "Abreve"), vec![part("A", 0.0, 0.0), part("brevecomb", 300.0, 200.0)]);
    assert_eq!(parts(&s, "Abrevecyrillic"), vec![part("Abreve", 0.0, 0.0)], "a glyph with the same whole outline uses it");
    assert_eq!(parts(&s, "Abrevedotbelow"), vec![part("A", 0.0, 0.0), part("brevecomb", 300.0, 200.0), part("dotbelowcomb", 300.0, 0.0)]);
    assert_eq!(parts(&s, "breveacutecomb"), vec![part("brevecomb", 0.0, 0.0), part("acutecomb", 0.0, 120.0)]);
    assert_eq!(parts(&s, "Abreveacute"), vec![part("A", 0.0, 0.0), part("breveacutecomb", 300.0, 200.0)], "a stacked mark stays a base");
}

/// Of bases with the same outline that match the same paths, the one drawn where the glyph
/// has it wins: `twofifths` takes `two.numr` (no shift), not `two.dnom`, which comes first.
#[test]
fn detect_composites_takes_the_identical_base_drawn_where_the_glyph_has_it() {
    let mut s = only(vec![
        ("two.dnom", vec![poly(&TWO, 0.0, -100.0)]),
        ("two.numr", vec![poly(&TWO, 0.0, 400.0)]),
        ("fraction", vec![poly(&SLASH, 0.0, 0.0)]),
        ("twofifths", vec![poly(&TWO, 0.0, 400.0), poly(&SLASH, 0.0, 0.0)]),
    ]);
    run(&mut s, "font.detectComposites", json!({"glyphs": ["twofifths"]}));
    assert_eq!(parts(&s, "twofifths"), vec![part("two.numr", 0.0, 0.0), part("fraction", 0.0, 0.0)]);
}

/// Between glyphs with the same outline, the name without a suffix is the base: `colon` is made
/// of two `period`s and `colon.tf` of `colon`, though `colon.tf` comes first in the font.
#[test]
fn detect_composites_makes_the_suffixed_copy_the_composite() {
    let dot = |y: f64| Shape::Path(Path::rect(100.0, y, 160.0, y + 60.0));
    let mut s = only(vec![("period", vec![dot(0.0)]), ("colon.tf", vec![dot(0.0), dot(400.0)]), ("colon", vec![dot(0.0), dot(400.0)])]);
    run(&mut s, "font.detectComposites", json!({}));
    assert_eq!(parts(&s, "colon"), vec![part("period", 0.0, 0.0), part("period", 0.0, 400.0)]);
    assert_eq!(parts(&s, "colon.tf"), vec![part("colon", 0.0, 0.0)]);
}

/// Gives glyph `name` the code point `cp`.
fn code(s: &mut Session, name: &str, cp: u32) {
    s.edit("fixture", |f| {
        f.glyph_mut(name)?.unicodes = vec![cp];
        Ok(())
    })
    .unwrap();
}

/// A large closed outline, for enclosing shapes.
const RING: [(f64, f64); 8] =
    [(100.0, -50.0), (500.0, -50.0), (650.0, 100.0), (650.0, 600.0), (500.0, 750.0), (100.0, 750.0), (-50.0, 600.0), (-50.0, 100.0)];

/// Of bases with the same outline, the one whose code point is in the glyph's canonical
/// decomposition wins over the one drawn where the glyph has it: `Ddotbelow` (D + U+0323) takes
/// `dotlow` (U+0323), not `period`.
#[test]
fn detect_composites_takes_the_base_the_glyph_database_names() {
    let stem = || Shape::Path(Path::rect(0.0, 0.0, 200.0, 700.0));
    let mut s = only(vec![
        ("D", vec![stem()]),
        ("period", vec![poly(&DOT, 250.0, 0.0)]),
        ("dotlow", vec![poly(&DOT, 0.0, 700.0)]),
        ("Ddotbelow", vec![stem(), poly(&DOT, 250.0, 0.0)]),
    ]);
    code(&mut s, "dotlow", 0x0323);
    run(&mut s, "font.detectComposites", json!({"glyphs": ["Ddotbelow"]}));
    assert_eq!(parts(&s, "Ddotbelow"), vec![part("D", 0.0, 0.0), part("dotlow", 250.0, -700.0)]);
}

/// The base whose name spells more of the glyph's name wins, with `sub` read as `below`:
/// `dcircumflexbelow` takes `circumflexsubcomb`, though `circumflexcomb` is drawn in place.
#[test]
fn detect_composites_takes_the_base_named_longest_in_the_glyph_name() {
    let stem = || Shape::Path(Path::rect(0.0, 0.0, 200.0, 700.0));
    let mut s = only(vec![
        ("d", vec![stem()]),
        ("circumflexcomb", vec![poly(&BREVE, 300.0, -700.0)]),
        ("circumflexsubcomb", vec![poly(&BREVE, 0.0, 0.0)]),
        ("dcircumflexbelow", vec![stem(), poly(&BREVE, 300.0, -700.0)]),
    ]);
    run(&mut s, "font.detectComposites", json!({"glyphs": ["dcircumflexbelow"]}));
    assert_eq!(parts(&s, "dcircumflexbelow"), vec![part("d", 0.0, 0.0), part("circumflexsubcomb", 300.0, -700.0)]);
}

/// A base of another case than the glyph loses to one of its case: lowercase `amacron` takes
/// `macron.lc`, not the combining `macroncomb.uc` drawn in place.
#[test]
fn detect_composites_takes_the_base_of_the_glyph_case() {
    let stem = || Shape::Path(Path::rect(0.0, 0.0, 200.0, 500.0));
    let mut s = only(vec![
        ("a", vec![stem()]),
        ("macroncomb.uc", vec![poly(&BREVE, 100.0, 0.0)]),
        ("macron.lc", vec![poly(&BREVE, 0.0, 0.0)]),
        ("amacron", vec![stem(), poly(&BREVE, 100.0, 0.0)]),
    ]);
    run(&mut s, "font.detectComposites", json!({"glyphs": ["amacron"]}));
    assert_eq!(parts(&s, "amacron"), vec![part("a", 0.0, 0.0), part("macron.lc", 100.0, 0.0)]);
}

/// A base with suffixes the glyph lacks loses to one without: `one.circled` takes `one`, not
/// `one.tf` drawn in place.
#[test]
fn detect_composites_takes_the_base_without_suffixes_the_glyph_lacks() {
    let mut s = only(vec![
        ("ring", vec![poly(&RING, 0.0, 0.0)]),
        ("one", vec![poly(&TWO, 0.0, 0.0)]),
        ("one.tf", vec![poly(&TWO, 200.0, 200.0)]),
        ("one.circled", vec![poly(&RING, 0.0, 0.0), poly(&TWO, 200.0, 200.0)]),
    ]);
    run(&mut s, "font.detectComposites", json!({"glyphs": ["one.circled"]}));
    assert_eq!(parts(&s, "one.circled"), vec![part("ring", 0.0, 0.0), part("one", 200.0, 200.0)]);
}

/// An enclosing mark does not count as a combining mark: `enclosingcirclecomb` is a copy of
/// `largeCircle`, and `A.circled` takes `largeCircle`, which is drawn in place.
#[test]
fn detect_composites_builds_from_the_symbol_rather_than_the_enclosing_mark() {
    let mut s = only(vec![
        ("A", vec![poly(&A_OUTER, 0.0, 0.0), poly(&A_COUNTER, 0.0, 0.0)]),
        ("largeCircle", vec![poly(&RING, 0.0, 0.0)]),
        ("enclosingcirclecomb", vec![poly(&RING, -700.0, 0.0)]),
        ("A.circled", vec![poly(&A_OUTER, 0.0, 0.0), poly(&A_COUNTER, 0.0, 0.0), poly(&RING, 0.0, 0.0)]),
    ]);
    run(&mut s, "font.detectComposites", json!({}));
    assert_eq!(parts(&s, "enclosingcirclecomb"), vec![part("largeCircle", -700.0, 0.0)]);
    assert_eq!(parts(&s, "A.circled"), vec![part("A", 0.0, 0.0), part("largeCircle", 0.0, 0.0)]);
}

/// A copy of a basic Latin letter loses even where the glyph database's recipe and the glyph's
/// name point at it: `Alpha` is a copy of `A`, and `Alphatonos` is built from `A`.
#[test]
fn detect_composites_builds_from_the_latin_letter_a_look_alike_copies() {
    let mut s = only(vec![
        ("A", vec![poly(&A_OUTER, 0.0, 0.0), poly(&A_COUNTER, 0.0, 0.0)]),
        ("Alpha", vec![poly(&A_OUTER, 0.0, 0.0), poly(&A_COUNTER, 0.0, 0.0)]),
        ("tonos", vec![poly(&ACUTE, 0.0, 0.0)]),
        ("Alphatonos", vec![poly(&A_OUTER, 0.0, 0.0), poly(&A_COUNTER, 0.0, 0.0), poly(&ACUTE, 0.0, 200.0)]),
    ]);
    code(&mut s, "Alpha", 0x0391);
    run(&mut s, "font.detectComposites", json!({}));
    assert_eq!(parts(&s, "Alpha"), vec![part("A", 0.0, 0.0)]);
    assert_eq!(parts(&s, "Alphatonos"), vec![part("A", 0.0, 0.0), part("tonos", 0.0, 200.0)]);

    // With `Alpha` first in the font, `A` becomes the copy, and `Alphatonos` still uses `A`.
    let mut s = only(vec![
        ("Alpha", vec![poly(&A_OUTER, 0.0, 0.0), poly(&A_COUNTER, 0.0, 0.0)]),
        ("A", vec![poly(&A_OUTER, 0.0, 0.0), poly(&A_COUNTER, 0.0, 0.0)]),
        ("tonos", vec![poly(&ACUTE, 0.0, 0.0)]),
        ("Alphatonos", vec![poly(&A_OUTER, 0.0, 0.0), poly(&A_COUNTER, 0.0, 0.0), poly(&ACUTE, 0.0, 200.0)]),
    ]);
    code(&mut s, "Alpha", 0x0391);
    run(&mut s, "font.detectComposites", json!({}));
    assert_eq!(parts(&s, "A"), vec![part("Alpha", 0.0, 0.0)]);
    assert_eq!(parts(&s, "Alphatonos"), vec![part("A", 0.0, 0.0), part("tonos", 0.0, 200.0)]);
}

/// Hundreds of identical dots and hundreds of glyphs that copy one, all named with long runs
/// of suffixes the others lack: choosing among the dots looks each dot's suffixes up among the
/// glyph's, not compares every pair, so the run ends quickly.
#[test]
fn detect_composites_on_identical_bases_with_long_suffix_runs_finishes_quickly() {
    let mut s = only(Vec::new());
    s.edit("fixture", |f| {
        for i in 0..300u32 {
            add_glyph(f, &format!("d{i}{}", ".a".repeat(120)), vec![poly(&DOT, 0.0, 0.0)]);
        }
        for i in 0..300u32 {
            add_glyph(f, &format!("t{i}{}", ".b".repeat(120)), vec![poly(&DOT, 50.0, 700.0)]);
        }
        Ok(())
    })
    .unwrap();
    let start = std::time::Instant::now();
    let out = run(&mut s, "font.detectComposites", json!({}));
    // Comparing every pair of suffixes took about ten times as long.
    assert!(start.elapsed() < std::time::Duration::from_secs(15), "{:?}", start.elapsed());
    assert_eq!(out["components"], 599, "every glyph but the first dot copies a dot: {out}");
}

/// A suffix that gives a base the glyph's case wins over the copy drawn in place: capital
/// `uni1F09` (Alpha with dasia) takes `uni1FFE.tonos`, lowercase `uni1F01` (alpha with dasia)
/// takes `dasia`.
#[test]
fn detect_composites_takes_the_mark_suffixed_for_the_glyph_case() {
    let stem = || Shape::Path(Path::rect(0.0, 0.0, 200.0, 500.0));
    let mut s = only(vec![
        ("A", vec![poly(&A_OUTER, 0.0, 0.0), poly(&A_COUNTER, 0.0, 0.0)]),
        ("alpha", vec![stem()]),
        ("dasia", vec![poly(&BREVE, 0.0, 700.0)]),
        ("uni1FFE.tonos", vec![poly(&BREVE, 0.0, 0.0)]),
        ("uni1F09", vec![poly(&A_OUTER, 0.0, 0.0), poly(&A_COUNTER, 0.0, 0.0), poly(&BREVE, 0.0, 700.0)]),
        ("uni1F01", vec![stem(), poly(&BREVE, 0.0, 0.0)]),
    ]);
    run(&mut s, "font.detectComposites", json!({"glyphs": ["uni1F09", "uni1F01"]}));
    assert_eq!(parts(&s, "uni1F09"), vec![part("A", 0.0, 0.0), part("uni1FFE.tonos", 0.0, 700.0)]);
    assert_eq!(parts(&s, "uni1F01"), vec![part("alpha", 0.0, 0.0), part("dasia", 0.0, -700.0)]);
}

/// Glyphs made of several glyphs that still serve as parts: a letter the glyph database does
/// not decompose (`i`, an undotted stem and a dot) and a mark by the glyph database
/// (`psilivaria`, made of `psili` and `varia`).
#[test]
fn detect_composites_keeps_undecomposed_letters_and_stacked_marks_as_parts() {
    let stem = |x: f64| Shape::Path(Path::rect(x, 0.0, x + 80.0, 500.0));
    let mut s = only(vec![
        ("idotless", vec![stem(0.0)]),
        ("dotaccentcomb", vec![poly(&DOT, 40.0, 700.0)]),
        ("psili", vec![poly(&BREVE, 0.0, 0.0)]),
        ("varia", vec![poly(&ACUTE, 0.0, 0.0)]),
        ("i", vec![stem(0.0), poly(&DOT, 40.0, 700.0)]),
        ("psilivaria", vec![poly(&BREVE, 0.0, 0.0), poly(&ACUTE, 200.0, 0.0)]),
        ("tworoman", vec![stem(0.0), poly(&DOT, 40.0, 700.0), stem(300.0), poly(&DOT, 340.0, 700.0)]),
        ("iotapsilivaria", vec![stem(0.0), poly(&BREVE, 40.0, 200.0), poly(&ACUTE, 240.0, 200.0)]),
    ]);
    run(&mut s, "font.detectComposites", json!({}));
    assert_eq!(parts(&s, "i"), vec![part("idotless", 0.0, 0.0), part("dotaccentcomb", 0.0, 0.0)]);
    assert_eq!(parts(&s, "tworoman"), vec![part("i", 0.0, 0.0), part("i", 300.0, 0.0)]);
    assert_eq!(parts(&s, "iotapsilivaria"), vec![part("idotless", 0.0, 0.0), part("psilivaria", 40.0, 200.0)]);
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
/// `cargo test --release -p fontcraft-engine --test engine detect:: -- --ignored --nocapture`.
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
    s.add_document(source, Some(dir.clone())).unwrap();
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
        // Rebuilt from other bases: fewer components (a composite base), more (finer parts), or
        // as many (mostly glyphs with the same outline under another name).
        let (mut coarser, mut finer, mut same_count) = (0, 0, 0);
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
                    match b.len().cmp(&bases.len()) {
                        std::cmp::Ordering::Less => coarser += 1,
                        std::cmp::Ordering::Greater => finer += 1,
                        std::cmp::Ordering::Equal => same_count += 1,
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
        println!("  other bases: {coarser} with fewer components, {finer} with more, {same_count} with as many");
        println!("  missed (first 30): {:?}", missed.iter().take(30).collect::<Vec<_>>());
    }
    let _ = std::fs::remove_dir_all(&out);
}
