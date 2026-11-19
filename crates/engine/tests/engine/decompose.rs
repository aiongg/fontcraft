//! Decomposition and Component from Selection (M6.6): `glyph.decomposeGlyph`,
//! `glyph.decomposeGlyphForAllMasters`, `component.decompose`, `cornerComponent.decompose`,
//! `glyph.componentFromSelection`.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::model::corner::{self, CornerHint, CornerTarget};
use fontcraft_engine::model::{
    Anchor, Component, ComponentInfo, ComponentInfoBox, Font, Glyph, GlyphInfo, GlyphInfoBox, Layer, LayerAttrs, LayerInfo, LayerInfoBox, Node,
    NodeKind, PartProperty, Path, Shape, SmartPole, SmartValue,
};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn layer<'f>(f: &'f Font, glyph: &str, id: &str) -> &'f Layer {
    f.glyph(glyph).unwrap().layers.iter().find(|l| l.id == id).unwrap()
}

/// The bounding box of a path.
fn bbox(shape: &Shape) -> (f64, f64, f64, f64) {
    let Shape::Path(p) = shape else { panic!("not a path: {shape:?}") };
    p.nodes.iter().fold((f64::MAX, f64::MAX, f64::MIN, f64::MIN), |(x0, y0, x1, y1), n| (x0.min(n.x), y0.min(n.y), x1.max(n.x), y1.max(n.y)))
}

/// The shape each corner hint of `l` sits on.
fn corner_shapes(l: &Layer) -> Vec<usize> {
    l.info
        .hints
        .iter()
        .map(|h| match corner::corner_hint(h).unwrap().unwrap().target {
            CornerTarget::Node { shape, .. } => shape,
            other => panic!("{other:?}"),
        })
        .collect()
}

fn master_layer(width: f64, shapes: Vec<Shape>) -> Layer {
    let mut l = Layer::new("m01", width);
    l.shapes = shapes;
    l
}

fn new_glyph(f: &mut Font, name: &str, l: Layer) {
    let mut g = Glyph::new(name);
    g.layers.push(l);
    f.glyphs.push(g);
}

/// `A` (box 20–580 × 0–700, `top` (300, 700), `bottom` (300, 0)), `acutecomb` (`_top` (0, 500),
/// `top` (0, 720)), `Aacute` = `A` + `acutecomb` at (300, 200), and `Aacute.alt` = `Aacute` at
/// (100, 0) followed by a small square that carries a corner hint.
fn composites() -> Session {
    let mut s = Session::with_document();
    s.edit("fixture", |f| {
        {
            let mut a = f.glyph_mut("A").unwrap();
            let l = &mut a.layers[0];
            l.shapes.push(Shape::Path(Path::rect(20.0, 0.0, 580.0, 700.0)));
            l.anchors = vec![Anchor::new("top", 300.0, 700.0), Anchor::new("bottom", 300.0, 0.0)];
        }
        let mut acute = master_layer(0.0, vec![Shape::Path(Path::rect(-50.0, 520.0, 50.0, 620.0))]);
        acute.anchors = vec![Anchor::new("_top", 0.0, 500.0), Anchor::new("top", 0.0, 720.0)];
        new_glyph(f, "acutecomb", acute);
        let aacute =
            master_layer(600.0, vec![Shape::Component(Component::new("A", 0.0, 0.0)), Shape::Component(Component::new("acutecomb", 300.0, 200.0))]);
        new_glyph(f, "Aacute", aacute);
        let mut alt =
            master_layer(800.0, vec![Shape::Component(Component::new("Aacute", 100.0, 0.0)), Shape::Path(Path::rect(0.0, -100.0, 10.0, -90.0))]);
        alt.info.hints = vec![CornerHint::new("_corner.one", 1, 0).to_hint()];
        new_glyph(f, "Aacute.alt", alt);
        Ok(())
    })
    .unwrap();
    s
}

#[test]
fn decompose_glyph_replaces_nested_components_by_paths_and_brings_their_anchors() {
    let mut s = composites();
    let r = run(&mut s, "glyph.decomposeGlyph", json!({"glyph": "Aacute.alt"}));
    assert_eq!(r["glyphs"], json!(["Aacute.alt"]));
    assert_eq!(r["anchorsAdded"], json!(2));
    let l = layer(s.font().unwrap(), "Aacute.alt", "m01");
    let boxes: Vec<_> = l.shapes.iter().map(bbox).collect();
    assert_eq!(boxes, [(120.0, 0.0, 680.0, 700.0), (350.0, 720.0, 450.0, 820.0), (0.0, -100.0, 10.0, -90.0)], "the paths of A and the acute, moved");
    let mut anchors: Vec<(String, f64, f64)> = l.anchors.iter().map(|a| (a.name.clone(), a.x, a.y)).collect();
    anchors.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(anchors, [("bottom".to_string(), 400.0, 0.0), ("top".to_string(), 400.0, 920.0)], "the propagated anchors");
    assert_eq!(corner_shapes(l), [2], "the hint follows its square to its new index");
}

#[test]
fn decompose_keeps_nested_components_and_adds_no_anchors() {
    let mut s = composites();
    let r = run(&mut s, "component.decompose", json!({"glyph": "Aacute.alt", "components": [0]}));
    assert_eq!(r["selection"]["shapes"], json!([0, 1]));
    let l = layer(s.font().unwrap(), "Aacute.alt", "m01");
    let placed: Vec<(String, [f64; 6])> = l.shapes[..2]
        .iter()
        .map(|sh| match sh {
            Shape::Component(c) => (c.glyph.clone(), c.affine().as_coeffs()),
            other => panic!("not a component: {other:?}"),
        })
        .collect();
    assert_eq!(placed, [("A".to_string(), [1.0, 0.0, 0.0, 1.0, 100.0, 0.0]), ("acutecomb".to_string(), [1.0, 0.0, 0.0, 1.0, 400.0, 200.0])]);
    assert_eq!(bbox(&l.shapes[2]), (0.0, -100.0, 10.0, -90.0));
    assert!(l.anchors.is_empty(), "one level adds no anchors");
    assert_eq!(corner_shapes(l), [2]);
}

#[test]
fn decompose_refuses_what_it_cannot_do_and_changes_nothing() {
    let mut s = composites();
    let before = s.doc().unwrap().font.clone();
    for (id, p, says) in [
        ("component.decompose", json!({"glyph": "Aacute.alt", "components": [1]}), "not a component"),
        ("component.decompose", json!({"glyph": "Aacute.alt", "components": [9]}), "does not resolve"),
        ("glyph.decomposeGlyph", json!({"glyph": "A"}), "no components"),
        ("cornerComponent.decompose", json!({"glyph": "A"}), "no corner components"),
        ("cornerComponent.decompose", json!({"glyph": "Aacute.alt"}), "no corner glyph"),
    ] {
        let e = s.execute(id, &p).unwrap_err().to_string();
        assert!(e.contains(says), "{id} {p}: {e}");
        assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "{id} {p} changed the font");
    }
}

/// `_part.bar`, a smart glyph whose Width runs from 100 (its master layer, a 100 × 100 square)
/// to 300 (a pole layer, 300 × 100), and `user` with a component of it at (50, 0), Width 200.
fn smart_session() -> Session {
    let mut s = Session::with_document();
    s.edit("fixture", |f| {
        let mut part = Glyph::new("_part.bar");
        part.info = GlyphInfoBox::new(GlyphInfo {
            parts: vec![PartProperty {
                name: "Width".into(),
                bottom_value: 100.0,
                top_value: 300.0,
                bottom_name: None,
                top_name: None,
                extra: Default::default(),
            }],
            ..Default::default()
        });
        for (id, top, width) in [("m01", false, 100.0), ("wide", true, 300.0)] {
            let mut l = master_layer(600.0, vec![Shape::Path(Path::rect(0.0, 0.0, width, 100.0))]);
            l.id = id.into();
            l.info = LayerInfoBox::new(LayerInfo {
                attrs: LayerAttrs { smart_poles: vec![SmartPole { property: "Width".into(), top }], ..Default::default() },
                ..Default::default()
            });
            part.layers.push(l);
        }
        f.glyphs.push(part);
        let mut c = Component::new("_part.bar", 50.0, 0.0);
        c.info =
            ComponentInfoBox::new(ComponentInfo { smart_values: vec![SmartValue { property: "Width".into(), value: 200.0 }], ..Default::default() });
        new_glyph(f, "user", master_layer(600.0, vec![Shape::Component(c)]));
        Ok(())
    })
    .unwrap();
    s
}

#[test]
fn a_smart_component_decomposes_to_its_outline_at_its_values() {
    for id in ["component.decompose", "glyph.decomposeGlyph"] {
        let mut s = smart_session();
        run(&mut s, id, json!({"glyph": "user"}));
        let l = layer(s.font().unwrap(), "user", "m01");
        assert_eq!(l.shapes.iter().map(bbox).collect::<Vec<_>>(), [(50.0, 0.0, 250.0, 100.0)], "{id}: the outline at Width 200, moved");
    }
}

/// `_corner.one` (an open path), `L` (a triangle with that corner on its node 0) and `Lcomp` (a
/// small square, then a component of `L`).
fn corner_session() -> Session {
    let mut s = Session::with_document();
    s.edit("fixture", |f| {
        let line = |pts: &[(f64, f64)]| pts.iter().map(|&(x, y)| Node::new(x, y, NodeKind::Line)).collect::<Vec<_>>();
        let corner = Path::new(false, line(&[(0.0, 50.0), (-50.0, 50.0), (-50.0, 0.0), (25.0, 0.0)]));
        new_glyph(f, "_corner.one", master_layer(0.0, vec![Shape::Path(corner)]));
        {
            let mut g = f.glyph_mut("L").unwrap();
            let l = &mut g.layers[0];
            l.shapes = vec![Shape::Path(Path::new(true, line(&[(100.0, 300.0), (200.0, 100.0), (414.0, 100.0)])))];
            l.info.hints = vec![CornerHint::new("_corner.one", 0, 0).to_hint()];
        }
        let comp = master_layer(600.0, vec![Shape::Path(Path::rect(0.0, -100.0, 10.0, -90.0)), Shape::Component(Component::new("L", 0.0, 0.0))]);
        new_glyph(f, "Lcomp", comp);
        Ok(())
    })
    .unwrap();
    s
}

#[test]
fn decompose_corner_fits_the_corner_into_its_path() {
    let mut s = corner_session();
    let before = s.doc().unwrap().font.clone();
    let fitted = corner::insert_corners(&before, layer(&before, "L", "m01")).unwrap().unwrap();
    assert_ne!(fitted, layer(&before, "L", "m01").shapes, "the fixture's corner changes the path");
    let r = run(&mut s, "cornerComponent.decompose", json!({"glyph": "L"}));
    assert_eq!(r["decomposed"], json!(1));
    let l = layer(s.font().unwrap(), "L", "m01");
    assert_eq!(l.shapes, fitted);
    assert!(l.info.hints.is_empty(), "the corner component is gone");
}

#[test]
fn decomposing_a_composite_fits_the_corners_of_its_base() {
    for id in ["glyph.decomposeGlyph", "component.decompose"] {
        let mut s = corner_session();
        let font = s.doc().unwrap().font.clone();
        let fitted = corner::insert_corners(&font, layer(&font, "L", "m01")).unwrap().unwrap();
        run(&mut s, id, json!({"glyph": "Lcomp"}));
        let l = layer(s.font().unwrap(), "Lcomp", "m01");
        assert_eq!(l.shapes[1..], fitted[..], "{id}");
    }
}

/// A second master `m02`; `A` on both; `B` on both: a square, a component of `A`, a square.
/// `B`'s first master has corner hints on both squares.
fn parts_session() -> Session {
    let mut s = Session::with_document();
    s.edit("fixture", |f| {
        let mut m = f.default_master()?.clone();
        m.id = "m02".into();
        m.name = "Bold".into();
        Arc::make_mut(&mut f.masters).push(m);
        {
            let mut a = f.glyph_mut("A")?;
            a.layers[0].shapes.push(Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
            let mut l = Layer::new("m02", 650.0);
            l.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 120.0, 100.0)));
            a.layers.push(l);
        }
        let mut b = f.glyph_mut("B")?;
        let l = &mut b.layers[0];
        l.shapes = vec![
            Shape::Path(Path::rect(0.0, 0.0, 50.0, 50.0)),
            Shape::Component(Component::new("A", 200.0, 0.0)),
            Shape::Path(Path::rect(400.0, 0.0, 450.0, 50.0)),
        ];
        l.info.hints = vec![CornerHint::new("_corner.x", 0, 0).to_hint(), CornerHint::new("_corner.x", 2, 0).to_hint()];
        let mut l = Layer::new("m02", 700.0);
        l.shapes = vec![
            Shape::Path(Path::rect(0.0, 0.0, 60.0, 60.0)),
            Shape::Component(Component::new("A", 210.0, 0.0)),
            Shape::Path(Path::rect(400.0, 0.0, 460.0, 60.0)),
        ];
        b.layers.push(l);
        Ok(())
    })
    .unwrap();
    s
}

#[test]
fn component_from_selection_moves_mixed_shapes_on_every_master() {
    let mut s = parts_session();
    let r = run(&mut s, "glyph.componentFromSelection", json!({"glyph": "B", "shapes": [0, 1]}));
    assert_eq!(r["name"], json!("_part.B"));
    assert_eq!(r["skipped"], json!([]));
    assert_eq!(r["selection"], json!({"glyph": "B", "layer": "m01", "shapes": [0]}));
    let f = s.font().unwrap();
    for (m, square) in [("m01", (0.0, 0.0, 50.0, 50.0)), ("m02", (0.0, 0.0, 60.0, 60.0))] {
        let b = layer(f, "B", m);
        assert_eq!(b.shapes.len(), 2, "{m}");
        assert!(matches!(&b.shapes[0], Shape::Component(c) if c.glyph == "_part.B" && c.transform.is_identity()), "{m}");
        let part = layer(f, "_part.B", m);
        assert_eq!(part.width, b.width, "{m}");
        assert_eq!(bbox(&part.shapes[0]), square, "{m}: the square keeps its coordinates");
        assert!(matches!(&part.shapes[1], Shape::Component(c) if c.glyph == "A"), "{m}");
    }
    assert_eq!(corner_shapes(layer(f, "B", "m01")), [1], "the kept square's hint follows it");
    assert_eq!(corner_shapes(layer(f, "_part.B", "m01")), [0], "the moved square's hint moves with it");
}

#[test]
fn a_part_with_an_underscore_name_does_not_export() {
    let mut s = parts_session();
    run(&mut s, "glyph.componentFromSelection", json!({"glyph": "B", "shapes": [0]}));
    assert!(!s.font().unwrap().glyph("_part.B").unwrap().export, "the suggested `_part.` name makes a part");
    run(&mut s, "glyph.componentFromSelection", json!({"glyph": "B", "shapes": [1], "name": "piece"}));
    assert!(s.font().unwrap().glyph("piece").unwrap().export, "any other name exports");
    run(&mut s, "glyph.newGlyph", json!({"name": "_connect"}));
    assert!(!s.font().unwrap().glyph("_connect").unwrap().export, "New Glyph follows the same rule");
}

#[test]
fn component_from_selection_refuses_a_taken_name() {
    let mut s = parts_session();
    let before = s.doc().unwrap().font.clone();
    let e = s.execute("glyph.componentFromSelection", &json!({"glyph": "B", "shapes": [0], "name": "A"})).unwrap_err().to_string();
    assert!(e.contains("already a glyph"), "{e}");
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before));
}

#[test]
fn a_master_without_matching_shapes_gets_an_empty_part_layer_and_undo_restores_the_font() {
    let mut s = parts_session();
    s.edit("setup", |f| {
        f.add_glyph("_part.B", Vec::new(), 0.0)?;
        let mut b = f.glyph_mut("B")?;
        b.layers[1].shapes[1] = Shape::Path(Path::rect(200.0, 0.0, 300.0, 60.0));
        Ok(())
    })
    .unwrap();
    let before = s.doc().unwrap().font.clone();
    let r = run(&mut s, "glyph.componentFromSelection", json!({"glyph": "B", "shapes": [0, 1]}));
    assert_eq!(r["name"], json!("_part.B.001"), "the suggested name skips the taken one");
    assert_eq!(r["skipped"][0]["layer"], json!("m02"));
    let f = s.font().unwrap();
    assert!(layer(f, "_part.B.001", "m02").shapes.is_empty());
    assert_eq!(layer(f, "B", "m02").shapes.len(), 3, "the master without matching shapes keeps them");
    assert_eq!(layer(f, "_part.B.001", "m01").shapes.len(), 2);
    run(&mut s, "edit.undo", json!({}));
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "one undo gives back the very font");
}

#[test]
fn decompose_for_all_masters_decomposes_every_master_layer() {
    let mut s = parts_session();
    let r = run(&mut s, "glyph.decomposeGlyphForAllMasters", json!({"glyphs": ["B", "C"]}));
    assert_eq!(r["glyphs"], json!(["B"]));
    let skipped: Vec<&str> = r["skipped"].as_array().unwrap().iter().map(|v| v["glyph"].as_str().unwrap()).collect();
    assert_eq!(skipped, ["C", "C"], "C has no components on m01 and no layer on m02");
    let f = s.font().unwrap();
    for (m, a) in [("m01", (200.0, 0.0, 300.0, 100.0)), ("m02", (210.0, 0.0, 330.0, 100.0))] {
        let b = layer(f, "B", m);
        assert!(b.shapes.iter().all(|sh| matches!(sh, Shape::Path(_))), "{m}");
        assert_eq!(bbox(&b.shapes[1]), a, "{m}: A's path of that master, moved");
    }
    assert_eq!(corner_shapes(layer(f, "B", "m01")), [0, 2], "one path per component: no hint moves");
}

/// A fan-out of components that draw nothing: `top` uses `mid` 1,000 times and `mid` uses the
/// empty `leaf` 1,000 times, a million visits for no shape. Each visit counts, so the command is
/// refused at once and the font is left as it was.
#[test]
fn nested_components_that_draw_nothing_still_count_and_are_refused_quickly() {
    let mut s = Session::with_document();
    s.edit("fan-out", |f| {
        new_glyph(f, "leaf", master_layer(0.0, vec![]));
        new_glyph(f, "mid", master_layer(0.0, (0..1_000).map(|_| Shape::Component(Component::new("leaf", 0.0, 0.0))).collect()));
        new_glyph(f, "top", master_layer(0.0, (0..1_000).map(|_| Shape::Component(Component::new("mid", 0.0, 0.0))).collect()));
        Ok(())
    })
    .unwrap();
    let before = s.doc().unwrap().font.clone();
    let started = std::time::Instant::now();
    let e = s.execute("glyph.decomposeGlyph", &json!({"glyph": "top"})).unwrap_err().to_string();
    assert!(started.elapsed() < std::time::Duration::from_secs(20), "took {:?}", started.elapsed());
    assert!(e.contains("visits more than"), "{e}");
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before));
}
