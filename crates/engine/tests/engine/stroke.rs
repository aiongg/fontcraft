//! Strokes, masks and shape order (M3.11): Set Stroke, Expand Outline (one layer and all
//! masters), Mask, Shape Order, and the layer composite the canvas and export use.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::cmd::stroke::{CAP_AXIS_KEY, layer_outline};
use fontcraft_engine::io::{SaveFormat, open, save};
use fontcraft_engine::model::corner::{self, CornerHint, CornerTarget};
use fontcraft_engine::model::{Component, Dimension, Family, Layer, Node, NodeKind, Path, Shape, ShapeAttrs};
use fontcraft_geom::{BezPath, Point, Shape as _};
use fontcraft_hinting::{Edge, NodeRef, PsHint, ps_hints, set_ps_hints};
use fontcraft_testkit::temp_dir;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn layer(s: &Session, glyph: &str, id: &str) -> Layer {
    s.font().unwrap().glyph(glyph).unwrap().layer(id).unwrap().clone()
}

fn attrs(s: &Session, glyph: &str, shape: usize) -> ShapeAttrs {
    match &layer(s, glyph, "m01").shapes[shape] {
        Shape::Path(p) => p.info.attrs.clone(),
        other => panic!("not a path: {other:?}"),
    }
}

fn outline(s: &Session, glyph: &str) -> Vec<BezPath> {
    layer_outline(s.font().unwrap(), &layer(s, glyph, "m01")).unwrap()
}

fn area(paths: &[BezPath]) -> f64 {
    paths.iter().map(|p| p.area()).sum()
}

fn covers(paths: &[BezPath], x: f64, y: f64) -> bool {
    paths.iter().map(|p| p.winding(Point::new(x, y))).sum::<i32>() != 0
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.5
}

/// An open path from (0, 0) to (100, 0) in `glyph`'s first master layer.
fn add_line(s: &mut Session, glyph: &str) {
    run(s, "path.addPath", json!({"glyph": glyph, "closed": false, "nodes": [{"x": 0, "y": 0}, {"x": 100, "y": 0}]}));
}

fn lock(s: &mut Session, glyph: &str) {
    s.edit("Lock", |font| {
        font.glyph_mut(glyph)?.info.locked = true;
        Ok(())
    })
    .unwrap();
}

#[test]
fn set_stroke_writes_the_attributes_and_width_null_removes_them() {
    let mut s = Session::with_document();
    add_line(&mut s, "A");
    let r = run(
        &mut s,
        "path.setStroke",
        json!({"glyph": "A", "shapes": [0], "width": 20, "position": 50, "join": "round", "capStart": "round", "capEnd": "axisAligned", "capAxis": "y", "fill": true}),
    );
    let st = &r["strokes"][0];
    assert_eq!((st["width"].as_f64(), st["position"].as_f64(), st["join"].as_str()), (Some(20.0), Some(50.0), Some("round")));
    assert_eq!((st["capStart"].as_str(), st["capEnd"].as_str(), st["capAxis"].as_str()), (Some("round"), Some("axisAligned"), Some("y")));
    let a = attrs(&s, "A", 0);
    assert_eq!(a.stroke_width, Some(Dimension::Number(20.0)));
    // The model stores the position from −1 (left) to 1 (right), whatever the source format.
    assert_eq!(a.stroke_pos, Some(Dimension::Number(0.5)));
    assert_eq!((a.line_join, a.line_cap_start, a.line_cap_end, a.fill), (Some(1), Some(1), Some(4), true));
    assert_eq!(a.extra.get(Family::Native, CAP_AXIS_KEY).and_then(|v| v.as_str()), Some("y"));

    // Keys not given keep their values.
    run(&mut s, "path.setStroke", json!({"glyph": "A", "shapes": [0], "height": 10, "capAxis": "auto"}));
    let a = attrs(&s, "A", 0);
    assert_eq!((a.stroke_width, a.stroke_height), (Some(Dimension::Number(20.0)), Some(Dimension::Number(10.0))));
    assert!(a.extra.get(Family::Native, CAP_AXIS_KEY).is_none());

    // Refusals change nothing.
    let before = attrs(&s, "A", 0);
    for bad in [
        json!({"glyph": "A", "shapes": [0], "height": 0.1}),
        json!({"glyph": "A", "shapes": [0], "width": 0}),
        json!({"glyph": "A", "shapes": [0], "width": -5}),
        json!({"glyph": "A", "shapes": [0], "width": 1e9}),
        json!({"glyph": "A", "shapes": [0], "position": 150}),
        json!({"glyph": "A", "shapes": [0], "join": "sharp"}),
        json!({"glyph": "A", "shapes": [0], "capStart": "pointy"}),
        json!({"glyph": "A", "shapes": [0]}),
        json!({"glyph": "A", "shapes": [0], "width": null, "join": "round"}),
        json!({"glyph": "A", "width": 10}),
        json!({"glyph": "A", "shapes": [3], "width": 10}),
    ] {
        assert!(s.execute("path.setStroke", &bad).is_err(), "{bad}");
    }
    assert_eq!(attrs(&s, "A", 0), before);

    run(&mut s, "path.setStroke", json!({"glyph": "A", "shapes": [0], "width": null}));
    assert!(attrs(&s, "A", 0).is_empty(), "every stroke attribute is gone");

    lock(&mut s, "A");
    assert!(s.execute("path.setStroke", &json!({"glyph": "A", "shapes": [0], "width": 10})).is_err());
}

#[test]
fn stroke_attributes_survive_saving() {
    let dir = temp_dir("stroke-save");
    let mut s = Session::with_document();
    add_line(&mut s, "A");
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": fontcraft_testkit::rect(0.0, 0.0, 50.0, 50.0)}));
    run(
        &mut s,
        "path.setStroke",
        json!({"glyph": "A", "shapes": [0], "width": 30, "height": 20, "position": -100, "join": "bevel", "capEnd": "roundInset"}),
    );
    run(&mut s, "path.setMask", json!({"glyph": "A", "shapes": [1], "mask": true}));
    let want = (attrs(&s, "A", 0), attrs(&s, "A", 1));
    let file = dir.join("strokes.glyphs");
    save(s.font().unwrap(), &file, SaveFormat::Glyphs).unwrap();
    let (back, _) = open(&file, None).unwrap();
    let got = |i: usize| match &back.glyph("A").unwrap().layer("m01").unwrap().shapes[i] {
        Shape::Path(p) => p.info.attrs.clone(),
        other => panic!("not a path: {other:?}"),
    };
    assert_eq!((got(0), got(1)), want);

    // The forced cap axis has no key in that format; the native file keeps it.
    run(&mut s, "path.setStroke", json!({"glyph": "A", "shapes": [0], "capEnd": "axisAligned", "capAxis": "x"}));
    let native = dir.join("strokes.fontcraft");
    save(s.font().unwrap(), &native, SaveFormat::Native).unwrap();
    let (back, _) = open(&native, None).unwrap();
    let Shape::Path(p) = &back.glyph("A").unwrap().layer("m01").unwrap().shapes[0] else { panic!("not a path") };
    assert_eq!(p.info.attrs, attrs(&s, "A", 0));
}

#[test]
fn expand_outline_replaces_a_stroke_by_its_outline() {
    let mut s = Session::with_document();
    add_line(&mut s, "A");
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": fontcraft_testkit::rect(200.0, 0.0, 300.0, 100.0)}));
    run(&mut s, "path.setStroke", json!({"glyph": "A", "shapes": [0], "width": 20}));
    assert!(near(area(&outline(&s, "A")), 2000.0 + 10_000.0), "the composite draws the stroke");

    let r = run(&mut s, "path.expandOutline", json!({"glyph": "A"}));
    assert_eq!(r["paths"], 1);
    assert_eq!(r["selection"]["shapes"], json!([0]));
    let l = layer(&s, "A", "m01");
    assert_eq!(l.shapes.len(), 2);
    let Shape::Path(p) = &l.shapes[0] else { panic!("not a path") };
    assert!(p.closed && p.info.attrs.is_empty());
    let mut points: Vec<(f64, f64)> = p.nodes.iter().map(|n| (n.x, n.y)).collect();
    points.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(points, [(0.0, -10.0), (0.0, 10.0), (100.0, -10.0), (100.0, 10.0)]);
    assert!(near(p.to_bez().unwrap().area(), 2000.0), "counter-clockwise");
    assert!(matches!(&l.shapes[1], Shape::Path(q) if q.nodes.len() == 4), "the unstroked path stays");

    assert!(s.execute("path.expandOutline", &json!({"glyph": "A"})).is_err(), "nothing is stroked any more");
    run(&mut s, "edit.undo", json!({}));
    assert!(attrs(&s, "A", 0).stroke_width.is_some());

    // A stroked closed path becomes a ring: two paths, one each way.
    run(&mut s, "path.setStroke", json!({"glyph": "A", "shapes": [1], "width": 20}));
    assert!(s.execute("path.expandOutline", &json!({"glyph": "A", "shapes": [5]})).is_err());
    let r = run(&mut s, "path.expandOutline", json!({"glyph": "A", "shapes": [1]}));
    assert_eq!(r["selection"]["shapes"], json!([1, 2]));
    let l = layer(&s, "A", "m01");
    assert_eq!(l.shapes.len(), 3);
    let ring: f64 = l.shapes[1..].iter().map(|sh| if let Shape::Path(p) = sh { p.to_bez().unwrap().area() } else { 0.0 }).sum();
    assert!(near(ring, 120.0 * 120.0 - 80.0 * 80.0), "{ring}");
    assert!(attrs(&s, "A", 0).stroke_width.is_some(), "only the listed path is expanded");

    // A locked glyph refuses it; in the Font View form it is skipped, like a glyph without a
    // stroked path.
    add_line(&mut s, "C");
    run(&mut s, "path.setStroke", json!({"glyph": "C", "shapes": [0], "width": 5}));
    lock(&mut s, "A");
    assert!(s.execute("path.expandOutline", &json!({"glyph": "A"})).is_err());
    let r = run(&mut s, "path.expandOutline", json!({"glyphs": ["A", "B", "C"]}));
    assert_eq!(r["glyphs"], json!(["C"]));
    let skipped: Vec<&str> = r["skipped"].as_array().unwrap().iter().map(|v| v["glyph"].as_str().unwrap()).collect();
    assert_eq!(skipped, ["A", "B"]);
}

/// Glyph `I`: a stroked square (path 0), a component and a stem (path 1, shape 2). A stem hint
/// and a corner component sit on the stem, another stem hint on the stroked square.
fn hinted() -> Session {
    let mut s = Session::with_document();
    run(&mut s, "glyph.newGlyph", json!({"name": "_corner.one"}));
    s.edit("setup", |font| {
        let mut g = font.glyph_mut("I")?;
        let layer = g.layer_mut("m01").unwrap();
        let mut square = Path::rect(0.0, 0.0, 100.0, 100.0);
        square.info.attrs.stroke_width = Some(Dimension::Number(20.0));
        layer.shapes = vec![Shape::Path(square), Shape::Component(Component::new("B", 0.0, 0.0)), Shape::Path(Path::rect(400.0, 0.0, 500.0, 700.0))];
        let node = |path, node| Edge::Node(NodeRef::new(path, node));
        set_ps_hints(layer, &[PsHint::stem(true, node(1, 0), node(1, 3)), PsHint::stem(false, node(0, 0), node(0, 1))]).unwrap();
        layer.info.hints.push(CornerHint::new("_corner.one", 2, 1).to_hint());
        Ok(())
    })
    .unwrap();
    s
}

#[test]
fn expand_outline_keeps_hints_on_their_nodes() {
    let mut s = hinted();
    let r = run(&mut s, "path.expandOutline", json!({"glyph": "I"}));
    assert_eq!(r["selection"]["shapes"], json!([0, 1]), "the ring is two paths");
    let l = layer(&s, "I", "m01");
    assert_eq!(l.shapes.len(), 4);
    let ps: Vec<PsHint> = ps_hints(&l).unwrap().into_iter().map(|(_, h)| h).collect();
    assert_eq!(ps.len(), 1, "the hint on the expanded square is gone: {ps:?}");
    let node = |path, node| Edge::Node(NodeRef::new(path, node));
    assert_eq!((ps[0].origin, ps[0].other), (node(2, 0), Some(node(2, 3))), "the stem is the third path now");
    assert_eq!(ps[0].edges(&l).unwrap(), (0.0, 700.0));
    let corners: Vec<CornerTarget> = l.info.hints.iter().filter_map(|h| corner::corner_hint(h).unwrap()).map(|c| c.target).collect();
    assert_eq!(corners, [CornerTarget::Node { shape: 3, node: 1 }], "a corner counts shapes, the component included");

    // Undo puts the hints back with the stroke.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(ps_hints(&layer(&s, "I", "m01")).unwrap().len(), 2);
}

/// `A` with a stroked open line in both masters; a second master `m02`.
fn two_masters() -> Session {
    let mut s = Session::with_document();
    s.edit("setup", |font| {
        let mut m = font.default_master()?.clone();
        m.id = "m02".into();
        m.name = "Bold".into();
        Arc::make_mut(&mut font.masters).push(m);
        let mut g = font.glyph_mut("A")?;
        let mut line = Path::new(false, vec![Node::new(0.0, 0.0, NodeKind::Line), Node::new(100.0, 0.0, NodeKind::Line)]);
        line.info.attrs.stroke_width = Some(Dimension::Number(20.0));
        g.layers.first_mut().unwrap().shapes.push(Shape::Path(line.clone()));
        let mut l = Layer::new("m02", 600.0);
        line.info.attrs.stroke_width = Some(Dimension::Number(40.0));
        l.shapes.push(Shape::Path(line));
        g.layers.push(l);
        Ok(())
    })
    .unwrap();
    s
}

#[test]
fn expand_outline_for_all_masters() {
    let mut s = two_masters();
    let r = run(&mut s, "path.expandOutlineForAllMasters", json!({"glyph": "A", "shapes": [0]}));
    assert_eq!((r["glyphs"].clone(), r["skipped"].clone()), (json!(["A"]), json!([])));
    for (id, thick) in [("m01", 20.0), ("m02", 40.0)] {
        let l = layer(&s, "A", id);
        let Shape::Path(p) = &l.shapes[0] else { panic!("not a path") };
        assert!(p.closed && p.info.attrs.is_empty());
        assert!(near(p.to_bez().unwrap().area(), 100.0 * thick), "{id}");
    }
    assert!(s.execute("path.expandOutlineForAllMasters", &json!({"glyph": "A"})).is_err(), "no stroke is left");
    assert!(s.execute("path.expandOutlineForAllMasters", &json!({"glyph": "A", "layer": "m01"})).is_err());
    run(&mut s, "edit.undo", json!({}));
    let r = run(&mut s, "path.expandOutlineForAllMasters", json!({"glyphs": ["A", "B"]}));
    assert_eq!(r["glyphs"], json!(["A"]));
    assert!(r["skipped"].as_array().unwrap().iter().all(|v| v["glyph"] == "B"));
}

/// `A`: a component of `B` (a 100-unit square), then a square over its right half, then a
/// square above both. `B` is filled by `path.addPath`.
fn masked() -> Session {
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "B", "nodes": fontcraft_testkit::rect(0.0, 0.0, 100.0, 100.0)}));
    s.edit("setup", |font| {
        let mut g = font.glyph_mut("A")?;
        let l = g.layer_mut("m01").unwrap();
        l.shapes.push(Shape::Component(Component::new("B", 0.0, 0.0)));
        l.shapes.push(Shape::Path(Path::rect(50.0, 0.0, 150.0, 100.0)));
        l.shapes.push(Shape::Path(Path::rect(0.0, 200.0, 100.0, 300.0)));
        Ok(())
    })
    .unwrap();
    s
}

#[test]
fn a_mask_cuts_the_shapes_below_it_components_included() {
    let mut s = masked();
    run(&mut s, "path.setMask", json!({"glyph": "A", "shapes": [1], "mask": true}));
    assert_eq!(attrs(&s, "A", 1).mask, Some(1));
    let out = outline(&s, "A");
    assert!(near(area(&out), 5000.0 + 10_000.0), "{}", area(&out));
    assert!(covers(&out, 25.0, 50.0), "the component's left half stays");
    assert!(!covers(&out, 75.0, 50.0) && !covers(&out, 125.0, 50.0), "the mask cuts and is not drawn");
    assert!(covers(&out, 50.0, 250.0), "the square above the mask is whole");

    // A stroked mask cuts with its stroke: a line across the component's middle, 20 wide.
    run(&mut s, "path.setMask", json!({"glyph": "A", "shapes": [1], "mask": false}));
    add_line(&mut s, "A");
    run(&mut s, "path.setStroke", json!({"glyph": "A", "shapes": [3], "width": 20}));
    run(&mut s, "path.setMask", json!({"glyph": "A", "shapes": [3], "mask": true}));
    let out = outline(&s, "A");
    assert!(!covers(&out, 25.0, 5.0) && !covers(&out, 75.0, 5.0) && covers(&out, 25.0, 50.0));
    assert!(covers(&out, 125.0, 5.0), "the band ends where the line ends");

    assert!(s.execute("path.setMask", &json!({"glyph": "A", "shapes": [0], "mask": true})).is_err(), "a component is not a path");
    assert!(s.execute("path.setMask", &json!({"glyph": "A", "shapes": [1]})).is_err(), "`mask` is required");
    lock(&mut s, "A");
    assert!(s.execute("path.setMask", &json!({"glyph": "A", "shapes": [1], "mask": true})).is_err());
}

#[test]
fn shape_order_decides_what_a_mask_cuts() {
    let mut s = masked();
    run(&mut s, "path.setMask", json!({"glyph": "A", "shapes": [1], "mask": true}));
    // The mask first: nothing is below it, so it cuts nothing.
    let r = run(&mut s, "filter.shapeOrder", json!({"glyph": "A", "order": [1, 0, 2]}));
    assert_eq!(r["order"], json!([1, 0, 2]));
    let l = layer(&s, "A", "m01");
    assert!(matches!(&l.shapes[1], Shape::Component(c) if c.glyph == "B"));
    let out = outline(&s, "A");
    assert!(near(area(&out), 20_000.0) && covers(&out, 75.0, 50.0));

    for bad in [
        json!({"glyph": "A"}),
        json!({"glyph": "A", "order": [0, 0, 1]}),
        json!({"glyph": "A", "order": [0, 1]}),
        json!({"glyph": "A", "order": [0, 1, 5]}),
        json!({"glyph": "A", "order": [0, 1, -2]}),
    ] {
        assert!(s.execute("filter.shapeOrder", &bad).is_err(), "{bad}");
    }
    run(&mut s, "edit.undo", json!({}));
    assert!(near(area(&outline(&s, "A")), 15_000.0));
    lock(&mut s, "A");
    assert!(s.execute("filter.shapeOrder", &json!({"glyph": "A", "order": [2, 1, 0]})).is_err());
}
