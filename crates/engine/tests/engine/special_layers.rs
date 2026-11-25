//! Special layer commands: layers.setIntermediate, layers.clearIntermediate,
//! layers.setAlternate, layers.clearAlternate, layers.reInterpolate, glyph.setShapeOrder and
//! glyph.swapShapes, on fonts built in code.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::interp::instance::interpolate_layer;
use fontcraft_engine::interp::{AlternateSet, Designspace, GlyphSources, InterpOptions, Location, RangeSet};
use fontcraft_engine::model::corner::{CornerHint, corner_hint};
use fontcraft_engine::model::{Axis, Font, Glyph, GlyphTable, Layer, Master, Path, Shape};
use fontcraft_engine::{EngineError, Session};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// The refusal message; panics when the command succeeds or the font changed.
fn refused(s: &mut Session, id: &str, p: Value) -> String {
    let before = s.documents()[0].font.clone();
    let message = match s.execute(id, &p) {
        Err(EngineError::BadParams { message, .. }) => message,
        other => panic!("{id} {p}: expected a refusal, got {other:?}"),
    };
    assert!(Arc::ptr_eq(&before, &s.documents()[0].font), "{id}: a refused command changed the font");
    message
}

fn rect(x0: f64, w: f64) -> Shape {
    Shape::Path(Path::rect(x0, 0.0, x0 + w, 700.0))
}

fn layer(master: &str, id: &str, shapes: Vec<Shape>) -> Layer {
    let mut l = Layer::new(master, 500.0);
    l.id = id.into();
    l.shapes = shapes;
    l
}

/// A weight axis with `masters` (id, weight) and the glyph `e`, whose layers are `layers`.
fn session(masters: &[(&str, f64)], layers: Vec<Layer>) -> Session {
    let mut f = Font::new("Special");
    f.axes = Arc::new(vec![Axis { name: "Weight".into(), tag: "wght".into(), ..Axis::default() }]);
    f.masters =
        Arc::new(masters.iter().map(|(id, w)| Master { id: (*id).into(), name: (*id).into(), location: vec![*w], ..Master::default() }).collect());
    let mut e = Glyph::new("e");
    e.layers = layers;
    f.glyphs = GlyphTable::from_glyphs(vec![Arc::new(e)]);
    let mut s = Session::new();
    s.add_document(f, None).unwrap();
    s
}

/// Thin (100) and Black (900), `e` a box 100 wide on Thin and 300 on Black, and a backup layer
/// `b` under Thin.
fn two_masters() -> Session {
    session(
        &[("thin", 100.0), ("black", 900.0)],
        vec![
            layer("thin", "thin", vec![rect(0.0, 100.0)]),
            layer("black", "black", vec![rect(0.0, 300.0)]),
            layer("thin", "b", vec![rect(0.0, 10.0)]),
        ],
    )
}

fn e_layer(s: &Session, id: &str) -> Layer {
    s.font().unwrap().glyph("e").unwrap().layers.iter().find(|l| l.id == id).unwrap().clone()
}

fn right_edge(l: &Layer) -> f64 {
    match l.shapes.first() {
        Some(Shape::Path(p)) => p.nodes.iter().map(|n| n.x).fold(f64::NEG_INFINITY, f64::max),
        other => panic!("expected a path, got {other:?}"),
    }
}

#[test]
fn layers_set_intermediate_and_clear_intermediate() {
    let mut s = two_masters();
    let out = run(&mut s, "layers.setIntermediate", json!({"glyph": "e", "layer": "b", "coordinates": {"wght": 400}}));
    assert_eq!(out["label"], "{400}");
    assert_eq!(out["coordinates"], json!({"wght": 400.0}));
    assert_eq!(e_layer(&s, "b").info.attrs.coordinates, vec![Some(400.0)]);

    let m = refused(&mut s, "layers.setIntermediate", json!({"glyph": "e", "layer": "b", "coordinates": {"wght": 2000}}));
    assert!(m.contains("outside the range"), "{m}");
    let m = refused(&mut s, "layers.setIntermediate", json!({"glyph": "e", "layer": "b", "coordinates": {"wght": null}}));
    assert!(m.contains("same location"), "an empty coordinate lands on Thin's own layer: {m}");
    refused(&mut s, "layers.setIntermediate", json!({"glyph": "e", "layer": "b", "coordinates": {"wdth": 50}}));
    refused(&mut s, "layers.setIntermediate", json!({"glyph": "e", "layer": "thin", "coordinates": {"wght": 400}}));

    let out = run(&mut s, "layers.clearIntermediate", json!({"glyph": "e", "layer": "b"}));
    assert_eq!(out["label"], Value::Null, "a backup layer again");
    assert!(e_layer(&s, "b").info.attrs.coordinates.is_empty());
    refused(&mut s, "layers.clearIntermediate", json!({"glyph": "e", "layer": "b"}));
}

#[test]
fn layers_set_alternate_and_clear_alternate() {
    let mut s = two_masters();
    let out = run(&mut s, "layers.setAlternate", json!({"glyph": "e", "layer": "b", "ranges": {"wght": {"min": 600}}}));
    assert_eq!(out["label"], "[600‹wg]");
    let rule = e_layer(&s, "b").info.attrs.axis_rules.clone();
    assert_eq!((rule.len(), rule[0].min, rule[0].max), (1, Some(600.0), None));

    let m = refused(&mut s, "layers.setAlternate", json!({"glyph": "e", "layer": "b", "ranges": {"wght": {"min": 700, "max": 300}}}));
    assert!(m.contains("minimum"), "{m}");
    refused(&mut s, "layers.setAlternate", json!({"glyph": "e", "layer": "b", "ranges": {"wght": {"low": 1}}}));
    refused(&mut s, "layers.setAlternate", json!({"glyph": "e", "layer": "thin", "ranges": {}}));
    // A master layer may carry a range; an intermediate layer keeps its coordinates.
    assert_eq!(run(&mut s, "layers.setAlternate", json!({"glyph": "e", "layer": "black", "ranges": {"wght": {"max": 800}}}))["label"], "[wg‹800]");
    run(&mut s, "layers.setIntermediate", json!({"glyph": "e", "layer": "b", "coordinates": {"wght": 400}}));
    assert_eq!(run(&mut s, "layers.clearAlternate", json!({"glyph": "e", "layer": "b"}))["label"], "{400}");
    assert!(e_layer(&s, "b").info.attrs.axis_rules.is_empty());
    refused(&mut s, "layers.clearAlternate", json!({"glyph": "e", "layer": "b"}));
}

/// Six masters on two axes and the glyph `e`, a box on each, with a backup layer `b` under the first; the masters' ids.
fn six_masters() -> (Session, Vec<String>) {
    let places: Vec<(String, f64, f64)> =
        [75.0, 100.0].into_iter().flat_map(|wd| [100.0, 400.0, 900.0].map(|wg| (format!("m{wg}-{wd}"), wg, wd))).collect();
    let mut f = Font::new("Special");
    let axis = |name: &str, tag: &str| Axis { name: name.into(), tag: tag.into(), ..Axis::default() };
    f.axes = Arc::new(vec![axis("Weight", "wght"), axis("Width", "wdth")]);
    let master = |(id, wg, wd): &(String, f64, f64)| Master { id: id.clone(), name: id.clone(), location: vec![*wg, *wd], ..Master::default() };
    f.masters = Arc::new(places.iter().map(master).collect());
    let mut e = Glyph::new("e");
    e.layers = places.iter().map(|(id, wg, wd)| layer(id, id, vec![rect(0.0, wg / 4.0 + wd)])).collect();
    e.layers.push(layer(&places[0].0, "b", vec![rect(0.0, 10.0)]));
    let mut spare = Glyph::new("o");
    spare.layers = places.iter().map(|(id, ..)| layer(id, id, vec![rect(0.0, 50.0)])).collect();
    f.glyphs = GlyphTable::from_glyphs(vec![Arc::new(e), Arc::new(spare)]);
    let mut s = Session::new();
    s.add_document(f, None).unwrap();
    (s, places.into_iter().map(|p| p.0).collect())
}

/// The layers of `glyph` without their ids: what interpolates and what is exported.
fn drawings(s: &Session, glyph: &str) -> Vec<(String, Vec<Shape>, Value)> {
    let font = s.font().unwrap();
    let layers = &font.glyph(glyph).unwrap().layers;
    layers.iter().map(|l| (l.master.clone(), l.shapes.clone(), serde_json::to_value(&l.info.attrs).unwrap())).collect()
}

/// For All Masters sets up an alternate shape over six masters in one undo step, as adding and setting a layer under
/// each master by hand does: same layers, same sources. The layers of one list take a range together and lose it
/// together, and a second run finds nothing to add.
#[test]
fn for_all_masters_sets_up_what_layers_added_by_hand_do() {
    let ranges = json!({"wght": {"min": 600}});
    let (mut s, masters) = six_masters();
    let before = s.font().unwrap().clone();
    let steps = s.doc().unwrap().history.undo.len();
    let out = run(&mut s, "layers.setAlternate", json!({"glyph": "e", "layer": "b", "allMasters": true, "ranges": ranges}));
    assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1, "one undo step");
    let rows = out["layers"].as_array().unwrap();
    assert_eq!((rows.len(), &rows[0]["layer"], &rows[0]["added"]), (6, &json!("b"), &json!(false)), "{out}");
    assert!(rows.iter().all(|r| r["label"] == "[600‹wg]") && rows[1..].iter().all(|r| r["added"] == true), "{out}");

    let (mut hand, _) = six_masters();
    run(&mut hand, "layers.setAlternate", json!({"glyph": "e", "layer": "b", "ranges": ranges}));
    for m in &masters[1..] {
        let id = run(&mut hand, "layers.addBackupLayer", json!({"glyphs": ["e"], "master": m, "name": "x"}))["layer"].clone();
        run(&mut hand, "layers.setAlternate", json!({"glyph": "e", "layer": id, "ranges": ranges}));
    }
    assert_eq!(drawings(&s, "e"), drawings(&hand, "e"));
    let sources = |s: &Session| -> Vec<(RangeSet, Vec<(String, Location)>)> {
        let font = s.font().unwrap();
        let sources = GlyphSources::of(font, &Designspace::new(font).unwrap(), "e").unwrap();
        let places = |a: &AlternateSet| a.sources.iter().map(|x| (x.master.clone(), x.location.clone())).collect();
        sources.alternates.iter().map(|a| (a.ranges.clone(), places(a))).collect()
    };
    let (one, by_hand) = (sources(&s), sources(&hand));
    assert_eq!(one, by_hand);
    assert_eq!((one.len(), one[0].1.len()), (1, 6));
    let m = refused(&mut s, "layers.setAlternate", json!({"glyphs": ["e"], "allMasters": true, "ranges": ranges}));
    assert!(m.contains("every master"), "{m}");

    // The layers of a list change together.
    let ids: Vec<Value> = rows.iter().map(|r| r["layer"].clone()).collect();
    let out = run(&mut s, "layers.setAlternate", json!({"glyph": "e", "layers": ids, "ranges": {"wght": {"min": 700}}}));
    assert!(out["layers"].as_array().unwrap().iter().all(|r| r["label"] == "[700‹wg]"), "{out}");
    let out = run(&mut s, "layers.clearAlternate", json!({"glyph": "e", "layers": ids}));
    assert_eq!(out["layers"].as_array().unwrap().len(), 6);
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.font().unwrap().glyphs, before.glyphs, "undone");

    // Other glyphs get a layer under each master; a layer that is not the glyph's, or no layer, is refused.
    let out = run(&mut s, "layers.setAlternate", json!({"glyph": "e", "layer": "b", "allMasters": true, "glyphs": ["e", "o"], "ranges": ranges}));
    assert_eq!(out["layers"].as_array().unwrap().iter().filter(|r| r["glyph"] == "o").count(), 6, "{out}");
    refused(&mut s, "layers.setAlternate", json!({"glyph": "e", "layers": ["b", "none"], "ranges": ranges}));
    refused(&mut s, "layers.setAlternate", json!({"glyph": "e", "ranges": ranges}));
}

/// Intermediate layers for all masters: one per place the coordinates and the masters' own give, none where the glyph
/// has a source already. Two layers of a list that land in one place are refused.
#[test]
fn for_all_masters_adds_one_intermediate_layer_per_place() {
    let (mut s, masters) = six_masters();
    let at = json!({"wght": 650, "wdth": null});
    let out = run(&mut s, "layers.setIntermediate", json!({"glyph": "e", "layer": "b", "allMasters": true, "coordinates": at}));
    let rows = out["layers"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "one at each width: {out}");
    assert_eq!((&rows[0]["layer"], &rows[0]["label"]), (&json!("b"), &json!("{650, 75}")));
    assert_eq!((&rows[1]["added"], &rows[1]["label"]), (&json!(true), &json!("{650, 100}")));
    let added = e_layer(&s, rows[1]["layer"].as_str().unwrap());
    assert_eq!((added.master.as_str(), &added.shapes), (masters[3].as_str(), &e_layer(&s, &masters[3]).shapes), "a copy of its master layer");
    refused(&mut s, "layers.setIntermediate", json!({"glyphs": ["e"], "allMasters": true, "coordinates": at}));

    let other = run(&mut s, "layers.addBackupLayer", json!({"glyphs": ["e"], "master": masters[1], "name": "x"}))["layer"].clone();
    let m = refused(&mut s, "layers.setIntermediate", json!({"glyph": "e", "layers": ["b", other], "coordinates": {"wght": 500}}));
    assert!(m.contains("same location"), "{m}");
    refused(&mut s, "layers.setIntermediate", json!({"glyph": "e", "layers": [masters[0], "b"], "coordinates": {"wght": 500}}));
}

/// A source added at a location is the interpolation there, so it changes no instance at that location; it is one undo
/// step and cannot be added twice.
#[test]
fn layers_add_intermediate_draws_the_interpolation_and_changes_nothing_there() {
    let mut s = two_masters();
    let at = |s: &Session, w: f64| {
        let font = s.font().unwrap();
        let sources = GlyphSources::of(font, &Designspace::new(font).unwrap(), "e").unwrap();
        interpolate_layer(font, &sources, &Location(vec![w]), &InterpOptions::default()).unwrap()
    };
    let (before, between) = (at(&s, 300.0), at(&s, 200.0));
    let out = run(&mut s, "layers.addIntermediate", json!({"glyph": "e", "location": {"wght": 300}}));
    assert_eq!((out["master"].as_str(), out["label"].as_str()), (Some("thin"), Some("{300}")), "the nearest master");
    let added = e_layer(&s, out["layer"].as_str().unwrap());
    assert_eq!((right_edge(&added), added.width), (right_edge(&before), before.width));
    assert_eq!(added.info.attrs.coordinates, [Some(300.0)]);
    assert!((right_edge(&at(&s, 300.0)) - right_edge(&before)).abs() < 1e-6, "nothing moves at the location");
    assert!((right_edge(&at(&s, 200.0)) - right_edge(&between)).abs() < 1.0, "nor between it and the master");

    let m = refused(&mut s, "layers.addIntermediate", json!({"glyph": "e", "location": {"wght": 300}}));
    assert!(m.contains("already has a source there"), "{m}");
    refused(&mut s, "layers.addIntermediate", json!({"glyph": "e", "location": {"wght": 100}}));
    refused(&mut s, "layers.addIntermediate", json!({"glyph": "e", "location": {"wght": 2000}}));
    refused(&mut s, "layers.addIntermediate", json!({"glyph": "e", "location": {"opsz": 12}}));
    refused(&mut s, "layers.addIntermediate", json!({"glyph": "e", "location": {}}));
    let count = s.font().unwrap().glyph("e").unwrap().layers.len();
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.font().unwrap().glyph("e").unwrap().layers.len(), count - 1, "one undo step");
}

#[test]
fn layers_re_interpolate_equals_the_interpolation_at_the_layer_location() {
    // A backup layer at weight 500: what the glyph interpolates to there.
    let mut s = two_masters();
    let expected = {
        let font = s.font().unwrap();
        let sources = GlyphSources::of(font, &Designspace::new(font).unwrap(), "e").unwrap();
        interpolate_layer(font, &sources, &Location(vec![500.0]), &InterpOptions::default()).unwrap()
    };
    let m = refused(&mut s, "layers.reInterpolate", json!({"glyph": "e", "layer": "b"}));
    assert!(m.contains("location"), "a backup layer needs a location: {m}");
    let out = run(&mut s, "layers.reInterpolate", json!({"glyph": "e", "layer": "b", "location": {"wght": 500}}));
    assert_eq!(out, json!({"glyphs": ["e"], "skipped": []}));
    let b = e_layer(&s, "b");
    assert_eq!((b.shapes.clone(), b.width), (expected.shapes, expected.width));
    assert_eq!(right_edge(&b), 200.0);
    assert_eq!(b.info.name, None, "the layer keeps its own id and attributes");

    // A master layer is interpolated from the other masters, leaving its own drawing out, in one
    // undo step.
    let mut s = session(
        &[("thin", 100.0), ("regular", 500.0), ("black", 900.0)],
        vec![
            layer("thin", "thin", vec![rect(0.0, 100.0)]),
            layer("regular", "regular", vec![rect(0.0, 999.0)]),
            layer("black", "black", vec![rect(0.0, 300.0)]),
        ],
    );
    run(&mut s, "layers.reInterpolate", json!({"glyphs": ["e"], "layer": "regular"}));
    assert_eq!(right_edge(&e_layer(&s, "regular")), 200.0);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(right_edge(&e_layer(&s, "regular")), 999.0);

    // An intermediate layer is interpolated at its coordinates.
    let mut s = two_masters();
    run(&mut s, "layers.setIntermediate", json!({"glyph": "e", "layer": "b", "coordinates": {"wght": 300}}));
    run(&mut s, "layers.reInterpolate", json!({"glyph": "e", "layer": "b"}));
    assert_eq!(right_edge(&e_layer(&s, "b")), 150.0);
}

#[test]
fn glyph_set_shape_order_and_swap_shapes() {
    let mut s = session(&[("m", 400.0)], vec![layer("m", "m", vec![rect(0.0, 10.0), rect(100.0, 20.0), rect(200.0, 30.0)])]);
    let edges = |s: &Session| -> Vec<f64> {
        e_layer(s, "m")
            .shapes
            .iter()
            .map(|sh| match sh {
                Shape::Path(p) => p.nodes.iter().map(|n| n.x).fold(f64::NEG_INFINITY, f64::max),
                _ => f64::NAN,
            })
            .collect()
    };
    // A corner on the third path, which moves to the front.
    s.edit("corner", |font| {
        font.glyph_mut("e")?.layer_mut("m").unwrap().info.hints.push(CornerHint::new("_corner.a", 2, 1).to_hint());
        Ok(())
    })
    .unwrap();
    let out = run(&mut s, "glyph.setShapeOrder", json!({"glyph": "e", "layer": "m", "order": [2, 0, 1]}));
    assert_eq!(out["order"], json!([2, 0, 1]));
    assert_eq!(edges(&s), [230.0, 10.0, 120.0]);
    let hints: Vec<CornerHint> = e_layer(&s, "m").info.hints.iter().map(|h| corner_hint(h).unwrap().unwrap()).collect();
    assert_eq!(hints, [CornerHint::new("_corner.a", 0, 1)], "the hint follows its path");
    for bad in [json!([0, 0, 1]), json!([0, 1]), json!([0, 1, 3])] {
        let m = refused(&mut s, "glyph.setShapeOrder", json!({"glyph": "e", "layer": "m", "order": bad}));
        assert!(m.contains("not a permutation"), "{m}");
    }

    run(&mut s, "glyph.swapShapes", json!({"glyph": "e", "layer": "m", "a": 0, "b": 2}));
    assert_eq!(edges(&s), [120.0, 10.0, 230.0]);
    let hints: Vec<CornerHint> = e_layer(&s, "m").info.hints.iter().map(|h| corner_hint(h).unwrap().unwrap()).collect();
    assert_eq!(hints, [CornerHint::new("_corner.a", 2, 1)], "the hint follows its path");
    refused(&mut s, "glyph.swapShapes", json!({"glyph": "e", "layer": "m", "a": 1, "b": 1}));
    refused(&mut s, "glyph.swapShapes", json!({"glyph": "e", "layer": "m", "a": 0, "b": 3}));
}

/// PostScript hints count paths, not shapes: with a component between two paths, a stem hint on
/// the second path follows it to the front as path 0.
#[test]
fn glyph_set_shape_order_moves_stem_hints_by_path_index() {
    use fontcraft_engine::model::Component;
    use fontcraft_hinting::{Edge, NodeRef, PsHint, ps_hints, set_ps_hints};
    let mut s = session(&[("m", 400.0)], vec![layer("m", "m", vec![rect(0.0, 10.0), rect(200.0, 30.0)])]);
    run(&mut s, "glyph.newGlyph", json!({"name": "dot"}));
    let node = |path, n| Edge::Node(NodeRef::new(path, n));
    s.edit("component and stem", |font| {
        let mut g = font.glyph_mut("e")?;
        let l = g.layer_mut("m").unwrap();
        l.shapes.insert(1, Shape::Component(Component::new("dot", 0.0, 0.0)));
        set_ps_hints(l, &[PsHint::stem(false, node(1, 0), node(1, 1))]).unwrap();
        Ok(())
    })
    .unwrap();
    let out = run(&mut s, "glyph.setShapeOrder", json!({"glyph": "e", "layer": "m", "order": [2, 0, 1]}));
    let l = e_layer(&s, "m");
    assert!(matches!(l.shapes[2], Shape::Component(_)), "{:?}", l.shapes);
    let ps = ps_hints(&l).unwrap();
    assert_eq!(ps.len(), 1, "{:?}", l.info.hints);
    assert_eq!((ps[0].1.origin, ps[0].1.other), (node(0, 0), Some(node(0, 1))), "the stem's path is path 0 now");
    assert_eq!(ps[0].1.edges(&l).unwrap(), (200.0, 230.0));
    assert!(out.get("hintUpkeep").is_none(), "nothing is unlinked or removed: {out}");
}
