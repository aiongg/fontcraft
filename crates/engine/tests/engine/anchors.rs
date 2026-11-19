//! Anchor commands (M6.7): adding with unique names, renaming, placing and removing anchors, Set
//! Anchors and Reset Anchors on one master and on all, and the anchor and component cases of
//! Path › Align Selection. Fixtures are built in code.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::model::{Anchor, Component, Font, Layer, Path, Shape, metric};
use fontcraft_testkit::rect;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// Runs a command that must be refused, and checks that the font is the very same afterwards.
fn refused(s: &mut Session, id: &str, p: Value, needle: &str) {
    let before = s.doc().unwrap().font.clone();
    let err = s.execute(id, &p).expect_err(id).to_string();
    assert!(err.contains(needle), "{id}: expected an error naming {needle:?}, got {err:?}");
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "{id}: a refused command changes nothing");
}

/// The anchors of a glyph's layer as (name, x, y), in layer order.
fn anchors(f: &Font, glyph: &str, layer: &str) -> Vec<(String, f64, f64)> {
    let g = f.glyph(glyph).unwrap();
    let l = g.layers.iter().find(|l| l.id == layer).unwrap();
    l.anchors.iter().map(|a| (a.name.clone(), a.x, a.y)).collect()
}

fn names(f: &Font, glyph: &str, layer: &str) -> Vec<String> {
    anchors(f, glyph, layer).into_iter().map(|a| a.0).collect()
}

fn anchor(f: &Font, glyph: &str, name: &str) -> (f64, f64) {
    anchors(f, glyph, "m01").into_iter().find(|a| a.0 == name).map(|a| (a.1, a.2)).unwrap_or_else(|| panic!("{glyph} has no anchor {name}"))
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn anchor_add_gives_new_anchor_and_keeps_names_unique() {
    let mut s = Session::with_document();
    let before = s.doc().unwrap().font.clone();
    let r = run(&mut s, "anchor.add", json!({"glyph": "A", "x": 10, "y": 20}));
    assert_eq!(r["anchor"], json!("new anchor"));
    assert_eq!(r["selection"]["anchors"], json!(["new anchor"]));
    let r = run(&mut s, "anchor.add", json!({"glyph": "A", "x": 30, "y": 20}));
    assert_eq!(r["anchor"], json!("new anchor_"), "a taken name gets an underscore");
    run(&mut s, "anchor.add", json!({"glyph": "A", "name": "top", "x": 300.4, "y": 700}));
    let r = run(&mut s, "anchor.add", json!({"glyph": "A", "name": "top", "x": 0, "y": 0}));
    assert_eq!(r["anchor"], json!("top_"), "anchor names are unique on a layer");
    let f = s.font().unwrap();
    assert_eq!(names(f, "A", "m01"), ["new anchor", "new anchor_", "top", "top_"]);
    assert_eq!(anchor(f, "A", "top"), (300.0, 700.0), "the position is rounded to the grid");

    refused(&mut s, "anchor.add", json!({"glyph": "A", "name": "", "x": 0, "y": 0}), "empty");
    refused(&mut s, "anchor.add", json!({"glyph": "A", "x": 2_000_000, "y": 0}), "beyond");
    for _ in 0..4 {
        run(&mut s, "edit.undo", json!({}));
    }
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "each add is one undo step");
}

#[test]
fn anchor_rename_refuses_a_name_the_layer_has() {
    let mut s = Session::with_document();
    run(&mut s, "anchor.add", json!({"glyph": "A", "name": "top", "x": 300, "y": 700}));
    run(&mut s, "anchor.add", json!({"glyph": "A", "name": "bottom", "x": 300, "y": 0}));
    refused(&mut s, "anchor.rename", json!({"glyph": "A", "anchor": "top", "name": "bottom"}), "already has an anchor `bottom`");
    refused(&mut s, "anchor.rename", json!({"glyph": "A", "anchor": "nothing", "name": "x"}), "no anchor `nothing`");
    let r = run(&mut s, "anchor.rename", json!({"glyph": "A", "anchor": "top", "name": "top_viet"}));
    assert_eq!(r["selection"]["anchors"], json!(["top_viet"]));
    let f = s.font().unwrap();
    assert_eq!(names(f, "A", "m01"), ["top_viet", "bottom"]);
    assert_eq!(anchor(f, "A", "top_viet"), (300.0, 700.0), "renaming keeps the position");
}

#[test]
fn anchor_set_position_and_remove_respect_locks() {
    let mut s = Session::with_document();
    run(&mut s, "anchor.add", json!({"glyph": "A", "name": "top", "x": 300, "y": 700}));
    run(&mut s, "anchor.add", json!({"glyph": "A", "name": "bottom", "x": 300, "y": 0}));
    run(&mut s, "anchor.add", json!({"glyph": "A", "name": "ogonek", "x": 500, "y": 0}));
    let r = run(&mut s, "anchor.setPosition", json!({"glyph": "A", "anchor": "top", "x": 320.6}));
    assert_eq!((r["x"].clone(), r["y"].clone()), (json!(321.0), json!(700.0)), "an omitted coordinate keeps its value");
    refused(&mut s, "anchor.setPosition", json!({"glyph": "A", "anchor": "top"}), "give `x`, `y` or both");

    s.edit("lock", |f| {
        let mut g = f.glyph_mut("A")?;
        if let Some(a) = g.layers[0].anchors.iter_mut().find(|a| a.name == "ogonek") {
            a.info.locked = true;
        }
        Ok(())
    })
    .unwrap();
    refused(&mut s, "anchor.setPosition", json!({"glyph": "A", "anchor": "ogonek", "y": 10}), "locked");
    refused(&mut s, "anchor.remove", json!({"glyph": "A", "anchors": ["top", "ogonek"]}), "locked");
    refused(&mut s, "anchor.remove", json!({"glyph": "A", "anchors": ["nothing"]}), "nothing");

    let r = run(&mut s, "anchor.remove", json!({"glyph": "A", "anchors": ["top", "bottom"]}));
    assert_eq!(r["removed"], json!(2));
    assert_eq!(names(s.font().unwrap(), "A", "m01"), ["ogonek"]);
}

/// `A` with an outline from (100, 0) to (500, 700) on every master and a `top` anchor in a place
/// of its own on the first.
fn set_anchors_fixture(two_masters: bool) -> Session {
    let mut s = Session::with_document();
    s.edit("fixture", |f| {
        if two_masters {
            let mut m = f.default_master()?.clone();
            m.id = "m02".into();
            m.name = "Bold".into();
            Arc::make_mut(&mut f.masters).push(m);
        }
        let mut g = f.glyph_mut("A")?;
        g.layers[0].shapes.push(Shape::Path(Path::rect(100.0, 0.0, 500.0, 700.0)));
        g.layers[0].anchors.push(Anchor::new("top", 10.0, 10.0));
        if two_masters {
            let mut l = Layer::new("m02", 600.0);
            l.shapes.push(Shape::Path(Path::rect(100.0, 0.0, 500.0, 700.0)));
            g.layers.push(l);
        }
        Ok(())
    })
    .unwrap();
    s
}

#[test]
fn set_anchors_adds_the_missing_defaults_and_reset_anchors_rebuilds_them() {
    let mut s = set_anchors_fixture(false);
    let r = run(&mut s, "glyph.setAnchors", json!({"glyph": "A"}));
    assert_eq!(r["glyphs"], json!(["A"]));
    let f = s.font().unwrap();
    assert!(names(f, "A", "m01").contains(&"bottom".to_string()), "an uppercase letter gets bottom: {:?}", anchors(f, "A", "m01"));
    assert_eq!(anchor(f, "A", "top"), (10.0, 10.0), "an anchor already there keeps its place");
    assert_eq!(anchor(f, "A", "bottom"), (300.0, 0.0), "bottom sits on the baseline, centred on the outline");

    // Running it again adds nothing and makes no undo step.
    let before = s.doc().unwrap().font.clone();
    let r = run(&mut s, "glyph.setAnchors", json!({"glyphs": ["A"]}));
    assert_eq!(r["added"], json!(0));
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before));

    run(&mut s, "anchor.add", json!({"glyph": "A", "name": "mine", "x": 0, "y": 0}));
    let r = run(&mut s, "glyph.resetAnchors", json!({"glyph": "A"}));
    assert_eq!(r["glyphs"], json!(["A"]));
    let f = s.font().unwrap();
    assert!(!names(f, "A", "m01").contains(&"mine".to_string()), "reset removes anchors that are not defaults");
    assert_eq!(anchor(f, "A", "top"), (300.0, 700.0), "reset puts top back at the cap height");

    // A locked anchor stays where it is, and the default of its name is not added beside it.
    run(&mut s, "anchor.add", json!({"glyph": "A", "name": "mine", "x": 0, "y": 0}));
    run(&mut s, "anchor.setPosition", json!({"glyph": "A", "anchor": "top", "x": 5, "y": 5}));
    s.edit("lock", |f| {
        let mut g = f.glyph_mut("A")?;
        for a in g.layers[0].anchors.iter_mut().filter(|a| a.name == "top" || a.name == "mine") {
            a.info.locked = true;
        }
        Ok(())
    })
    .unwrap();
    run(&mut s, "glyph.resetAnchors", json!({"glyph": "A"}));
    let f = s.font().unwrap();
    assert_eq!(anchor(f, "A", "top"), (5.0, 5.0), "the locked top keeps its place");
    assert_eq!(names(f, "A", "m01").iter().filter(|n| *n == "top").count(), 1);
    assert!(names(f, "A", "m01").contains(&"mine".to_string()), "a locked anchor that is not a default stays too");
}

#[test]
fn set_and_reset_anchors_for_all_masters_change_every_master_layer() {
    let mut s = set_anchors_fixture(true);
    run(&mut s, "glyph.setAnchors", json!({"glyph": "A"}));
    assert!(names(s.font().unwrap(), "A", "m02").is_empty(), "the plain command changes one master");

    run(&mut s, "glyph.setAnchorsForAllMasters", json!({"glyphs": ["A"]}));
    let f = s.font().unwrap();
    assert!(names(f, "A", "m02").contains(&"top".to_string()), "the second master gets the defaults too");
    assert_eq!(anchor(f, "A", "top"), (10.0, 10.0), "the first master keeps its own top");

    let r = run(&mut s, "glyph.resetAnchorsForAllMasters", json!({"glyph": "A"}));
    assert!(r["removed"].as_u64().unwrap() > 0);
    let f = s.font().unwrap();
    assert_eq!(anchor(f, "A", "top"), (300.0, 700.0));
    assert_eq!(names(f, "A", "m01"), names(f, "A", "m02"), "both masters get the same set");
    refused(&mut s, "glyph.resetAnchorsForAllMasters", json!({"glyph": "A", "layer": "m01"}), "every master layer");
}

/// A path from (100, 0) to (300, 100) and an anchor `top` at (0, 500) on an italic master of
/// 10°, with no grid.
fn italic_fixture() -> Session {
    let mut s = Session::with_document();
    s.edit("italic", |f| {
        let master = f.default_master()?.id.clone();
        f.set_metric_pos(&master, metric::ITALIC_ANGLE, 10.0)?;
        Arc::make_mut(&mut f.info).settings.grid_length = Some(0.0);
        Ok(())
    })
    .unwrap();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(100.0, 0.0, 300.0, 100.0)}));
    run(&mut s, "anchor.add", json!({"glyph": "A", "name": "top", "x": 0, "y": 500}));
    s
}

fn node(f: &Font, glyph: &str, shape: usize, n: usize) -> (f64, f64) {
    let l = &f.glyph(glyph).unwrap().layers[0];
    let n = l.node(shape, n).unwrap();
    (n.x, n.y)
}

#[test]
fn align_selection_puts_an_anchor_over_one_node_or_between_two_along_the_italic_angle() {
    let mut s = italic_fixture();
    let lean = 10f64.to_radians().tan();
    let (n0, n1) = {
        let f = s.font().unwrap();
        (node(f, "A", 0, 0), node(f, "A", 0, 1))
    };
    run(&mut s, "path.alignSelection", json!({"glyph": "A", "anchors": ["top"], "points": [[0, 0]]}));
    let f = s.font().unwrap();
    let (x, y) = anchor(f, "A", "top");
    assert!(close(x, n0.0 + (500.0 - n0.1) * lean) && close(y, 500.0), "above the node along the italic angle: ({x}, {y})");
    assert_eq!(node(f, "A", 0, 0), n0, "the node stays");
    run(&mut s, "edit.undo", json!({}));

    run(&mut s, "path.alignSelection", json!({"glyph": "A", "anchors": ["top"], "points": [[0, 0], [0, 1]]}));
    let f = s.font().unwrap();
    let mid = ((n0.0 + n1.0) / 2.0, (n0.1 + n1.1) / 2.0);
    let (x, y) = anchor(f, "A", "top");
    assert!(close(x, mid.0 + (500.0 - mid.1) * lean) && close(y, 500.0), "centred between the nodes: ({x}, {y})");
    assert_eq!((node(f, "A", 0, 0), node(f, "A", 0, 1)), (n0, n1), "the nodes stay");
}

/// `A`: a path from (100, 0) to (300, 100) and a component of `B` at (0, 0). `B` has an outline.
fn component_fixture() -> Session {
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "B", "nodes": rect(50.0, 0.0, 150.0, 100.0)}));
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(100.0, 0.0, 300.0, 100.0)}));
    s.edit("component", |f| {
        f.glyph_mut("A")?.layers[0].shapes.push(Shape::Component(Component::new("B", 0.0, 0.0)));
        Ok(())
    })
    .unwrap();
    s
}

fn offset(f: &Font) -> (f64, f64) {
    let Shape::Component(c) = &f.glyph("A").unwrap().layers[0].shapes[1] else { panic!("shape 1 is not a component") };
    let k = c.affine().as_coeffs();
    (k[4], k[5])
}

#[test]
fn align_selection_puts_a_component_origin_on_a_node_with_and_without_an_origin_anchor() {
    let mut s = component_fixture();
    let target = node(s.font().unwrap(), "A", 0, 2);
    let r = run(&mut s, "path.alignSelection", json!({"glyph": "A", "shapes": [1], "points": [[0, 2]]}));
    assert_eq!(r["moved"], json!(1));
    let f = s.font().unwrap();
    assert_eq!(offset(f), target, "without an origin anchor the base glyph's (0, 0) lands on the node");
    assert_eq!(node(f, "A", 0, 2), target, "the node stays");
    run(&mut s, "edit.undo", json!({}));

    run(&mut s, "anchor.add", json!({"glyph": "B", "name": "origin", "x": 50, "y": 20}));
    run(&mut s, "path.alignSelection", json!({"glyph": "A", "shapes": [1], "points": [[0, 2]]}));
    assert_eq!(offset(s.font().unwrap()), (target.0 - 50.0, target.1 - 20.0), "the origin anchor lands on the node");
    run(&mut s, "edit.undo", json!({}));

    s.edit("lock", |f| {
        if let Shape::Component(c) = &mut f.glyph_mut("A")?.layers[0].shapes[1] {
            c.info.locked = true;
        }
        Ok(())
    })
    .unwrap();
    refused(&mut s, "path.alignSelection", json!({"glyph": "A", "shapes": [1], "points": [[0, 2]]}), "locked");
}

#[test]
fn on_a_slanted_master_the_component_origin_is_below_the_slanted_sidebearing_at_half_the_x_height() {
    let mut s = component_fixture();
    s.edit("italic", |f| {
        let master = f.default_master()?.id.clone();
        f.set_metric_pos(&master, metric::ITALIC_ANGLE, 10.0)?;
        f.set_metric_pos(&master, metric::X_HEIGHT, 500.0)?;
        Arc::make_mut(&mut f.info).settings.grid_length = Some(0.0);
        Ok(())
    })
    .unwrap();
    let target = node(s.font().unwrap(), "A", 0, 2);
    run(&mut s, "path.alignSelection", json!({"glyph": "A", "shapes": [1], "points": [[0, 2]]}));
    // The sidebearing line leaves the baseline at x = 0 and reaches half the x-height (250)
    // 250 · tan 10° to the right; the origin point is on the baseline below that.
    let origin_x = 250.0 * 10f64.to_radians().tan();
    let (x, y) = offset(s.font().unwrap());
    assert!((x - (target.0 - origin_x)).abs() < 1e-6 && (y - target.1).abs() < 1e-6, "({x}, {y}) for the node at {target:?}");

    // An origin anchor still wins.
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "anchor.add", json!({"glyph": "B", "name": "origin", "x": 50, "y": 20}));
    run(&mut s, "path.alignSelection", json!({"glyph": "A", "shapes": [1], "points": [[0, 2]]}));
    assert_eq!(offset(s.font().unwrap()), (target.0 - 50.0, target.1 - 20.0));
}
