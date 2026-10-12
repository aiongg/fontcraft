//! Transformations (M3.5): the Palette's mirror, scale, rotate, slant and align commands,
//! Path › Align Selection, and Path › Transformations with its export string.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::model::{Anchor, Component, Guide, LayerSide, Path, Shape};
use fontcraft_testkit::rect;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// The nodes of each path of a glyph's first master layer, as (x, y).
fn nodes(s: &mut Session, glyph: &str) -> Vec<Vec<(f64, f64)>> {
    let v = run(s, "layer.inspect", json!({"glyph": glyph}));
    v["shapes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|shape| {
            shape["nodes"].as_array().map(|n| n.iter().map(|n| (n["x"].as_f64().unwrap(), n["y"].as_f64().unwrap())).collect()).unwrap_or_default()
        })
        .collect()
}

/// The active font, for comparing before and after (undo steps and the journal left out).
fn font(s: &Session) -> Value {
    serde_json::to_value(s.font().unwrap()).unwrap()
}

fn anchor(s: &mut Session, glyph: &str, name: &str) -> (f64, f64) {
    let v = run(s, "layer.inspect", json!({"glyph": glyph}));
    let a = v["anchors"].as_array().unwrap().iter().find(|a| a["name"] == name).unwrap().clone();
    (a["x"].as_f64().unwrap(), a["y"].as_f64().unwrap())
}

/// `A` holds one closed path with a curve; every coordinate and the box centre (150, 100) are
/// whole numbers, so exact transformations give whole numbers back.
fn curvy() -> Session {
    let mut s = Session::with_document();
    let line = |x: f64, y: f64| json!({"x": x, "y": y});
    let off = |x: f64, y: f64| json!({"x": x, "y": y, "type": "offcurve"});
    let nodes = json!([line(0.0, 0.0), line(300.0, 0.0), off(300.0, 100.0), off(250.0, 200.0), {"x": 100, "y": 200, "type": "curve", "smooth": true}, line(0.0, 120.0)]);
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": nodes}));
    s
}

fn all_points(count: usize) -> Value {
    json!((0..count).map(|i| json!([0, i])).collect::<Vec<_>>())
}

#[test]
fn mirror_twice_and_four_quarter_turns_give_the_start_back() {
    let mut s = curvy();
    let start = nodes(&mut s, "A");
    let sel = json!({"glyph": "A", "points": all_points(6)});

    let v = run(&mut s, "transform.mirrorHorizontal", sel.clone());
    assert_eq!(v["origin"], json!([150.0, 100.0]), "the default origin is the centre of the selection's box");
    assert_eq!(nodes(&mut s, "A")[0][0], (300.0, 0.0));
    run(&mut s, "transform.mirrorHorizontal", sel.clone());
    assert_eq!(nodes(&mut s, "A"), start);

    run(&mut s, "transform.mirrorVertical", sel.clone());
    assert_eq!(nodes(&mut s, "A")[0][0], (0.0, 200.0));
    run(&mut s, "transform.mirrorVertical", sel.clone());
    assert_eq!(nodes(&mut s, "A"), start);

    let mut quarter = sel.clone();
    quarter["angle"] = json!(90);
    run(&mut s, "transform.rotateCounterclockwise", quarter.clone());
    // (0, 0) turns a quarter counterclockwise about (150, 100).
    assert_eq!(nodes(&mut s, "A")[0][0], (250.0, -50.0));
    for _ in 0..3 {
        run(&mut s, "transform.rotateCounterclockwise", quarter.clone());
    }
    assert_eq!(nodes(&mut s, "A"), start);

    // About a fixed point; a box point would move, since the box of a turned shape is another box.
    let mut angle = sel.clone();
    angle["angle"] = json!(30);
    angle["origin"] = json!([150, 100]);
    run(&mut s, "transform.rotateClockwise", angle.clone());
    assert_ne!(nodes(&mut s, "A"), start);
    run(&mut s, "transform.rotateCounterclockwise", angle);
    let back = nodes(&mut s, "A");
    for (a, b) in back[0].iter().zip(&start[0]) {
        assert!((a.0 - b.0).abs() <= 1.0 && (a.1 - b.1).abs() <= 1.0, "{a:?} vs {b:?}: opposite buttons agree up to the grid");
    }
}

#[test]
fn a_whole_layer_of_several_glyphs_is_one_undo_step() {
    let mut s = Session::with_document();
    for g in ["A", "B"] {
        run(&mut s, "path.addPath", json!({"glyph": g, "nodes": rect(100.0, 0.0, 300.0, 700.0)}));
    }
    s.edit("anchor", |font| {
        let master = font.default_master()?.id.clone();
        font.glyph_mut("A")?.layer_mut(&master).unwrap().anchors.push(Anchor::new("top", 200.0, 700.0));
        Ok(())
    })
    .unwrap();
    let before = font(&s);

    // A metrics origin sits at half the advance width (600 / 2).
    let v = run(&mut s, "transform.mirrorHorizontal", json!({"glyphs": ["A", "B"], "origin": "baseline"}));
    assert_eq!((v["glyphs"].clone(), v["layers"].clone()), (json!(["A", "B"]), json!(2)));
    for g in ["A", "B"] {
        assert_eq!(nodes(&mut s, g)[0], [(500.0, 0.0), (300.0, 0.0), (300.0, 700.0), (500.0, 700.0)], "{g}");
    }
    assert_eq!(anchor(&mut s, "A", "top"), (400.0, 700.0), "a whole layer takes its anchors along");

    run(&mut s, "edit.undo", json!({}));
    assert_eq!(font(&s), before, "one undo step restores both glyphs");

    // Target lists name things in one glyph.
    assert!(s.execute("transform.scale", &json!({"glyphs": ["A"], "points": [[0, 0]], "x": 50})).is_err());
    assert!(s.execute("transform.scale", &json!({"glyph": "A", "glyphs": ["B"], "x": 50})).is_err());
    assert_eq!(font(&s), before);
}

#[test]
fn scale_and_reverse_scale_undo_each_other() {
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(100.0, 0.0, 300.0, 700.0)}));
    let start = nodes(&mut s, "A");
    let p = json!({"glyph": "A", "x": 120, "origin": {"mode": "box", "boxPoint": "bottomLeft"}});
    run(&mut s, "transform.scale", p.clone());
    assert_eq!(nodes(&mut s, "A")[0][2], (340.0, 840.0), "without y both axes take x");
    run(&mut s, "transform.reverseScale", p);
    assert_eq!(nodes(&mut s, "A"), start, "reverse scale divides by 1.2, it does not multiply by 0.8");

    run(&mut s, "transform.scale", json!({"glyph": "A", "x": 50, "y": 200, "origin": [100, 0]}));
    assert_eq!(nodes(&mut s, "A")[0][2], (200.0, 1400.0));

    let before = font(&s);
    for bad in [
        json!({"glyph": "A", "x": 0}),
        json!({"glyph": "A", "x": 1e300}),
        json!({"glyph": "A"}),
        json!({"glyph": "A", "x": 50, "origin": {"mode": "reference"}}),
    ] {
        assert!(s.execute("transform.reverseScale", &bad).is_err(), "{bad}");
    }
    // A scale that throws points beyond the coordinate limit fails and changes nothing.
    assert!(s.execute("transform.scale", &json!({"glyph": "A", "x": 1_000_000})).is_err());
    assert_eq!(font(&s), before);
}

#[test]
fn slants_shear_by_the_tangent_about_the_origin() {
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(100.0, 0.0, 300.0, 700.0)}));
    let start = nodes(&mut s, "A");
    run(&mut s, "transform.slantRight", json!({"glyph": "A", "angle": 45, "origin": "baseline"}));
    assert_eq!(nodes(&mut s, "A")[0], [(100.0, 0.0), (300.0, 0.0), (1000.0, 700.0), (800.0, 700.0)]);
    run(&mut s, "transform.slantLeft", json!({"glyph": "A", "angle": 45, "origin": "baseline"}));
    assert_eq!(nodes(&mut s, "A"), start);

    run(&mut s, "transform.slantUp", json!({"glyph": "A", "angle": 45, "origin": [0, 0]}));
    assert_eq!(nodes(&mut s, "A")[0][1], (300.0, 300.0), "right of the origin moves up");
    run(&mut s, "transform.slantDown", json!({"glyph": "A", "angle": 45, "origin": [0, 0]}));
    assert_eq!(nodes(&mut s, "A"), start);

    assert!(s.execute("transform.slantRight", &json!({"glyph": "A", "angle": 90})).is_err());
}

#[test]
fn handles_anchors_guides_and_components_move_with_the_selection() {
    let mut s = curvy();
    s.edit("setup", |font| {
        let master = font.default_master()?.id.clone();
        if let Some(m) = Arc::make_mut(&mut font.masters).first_mut() {
            m.guides.push(Guide { x: 0.0, y: 500.0, ..Guide::default() });
        }
        let mut g = font.glyph_mut("A")?;
        let l = g.layer_mut(&master).unwrap();
        l.shapes.push(Shape::Component(Component::new("B", 10.0, 0.0)));
        l.anchors.push(Anchor::new("top", 50.0, 700.0));
        l.info.guides.push(Guide { x: 20.0, angle: 90.0, ..Guide::default() });
        l.parts_mut(LayerSide::Background).shapes.push(Shape::Path(Path::rect(0.0, 0.0, 10.0, 10.0)));
        Ok(())
    })
    .unwrap();

    // The curve node (index 4) carries its handle (3); the other handle (2) belongs to node 1.
    run(&mut s, "transform.mirrorHorizontal", json!({"glyph": "A", "points": [[0, 4]], "origin": [100, 0]}));
    let n = nodes(&mut s, "A");
    assert_eq!((n[0][4], n[0][3], n[0][2]), ((100.0, 200.0), (-50.0, 200.0), (300.0, 100.0)));

    // A component keeps a decomposed placement under a mirror.
    run(&mut s, "transform.mirrorHorizontal", json!({"glyph": "A", "shapes": [1], "origin": [0, 0]}));
    let v = run(&mut s, "layer.inspect", json!({"glyph": "A"}));
    assert_eq!(v["shapes"][1]["transform"], json!([-1.0, 0.0, 0.0, 1.0, -10.0, 0.0]));

    // A quarter turn about the origin: a vertical guide at x = 20 becomes a horizontal one.
    run(
        &mut s,
        "transform.rotateCounterclockwise",
        json!({"glyph": "A", "anchors": ["top"], "guides": [{"scope": "local", "index": 0}, {"scope": "global", "index": 0}], "angle": 90, "origin": [0, 0]}),
    );
    let v = run(&mut s, "layer.inspect", json!({"glyph": "A"}));
    assert_eq!((v["anchors"][0]["x"].clone(), v["anchors"][0]["y"].clone()), (json!(-700.0), json!(50.0)));
    assert_eq!((v["guides"][0]["x"].clone(), v["guides"][0]["y"].clone(), v["guides"][0]["angle"].clone()), (json!(0.0), json!(20.0), json!(180.0)));
    assert_eq!((v["guides"][1]["x"].clone(), v["guides"][1]["y"].clone(), v["guides"][1]["angle"].clone()), (json!(-500.0), json!(0.0), json!(90.0)));

    // The background is addressed with `side`.
    run(&mut s, "transform.scale", json!({"glyph": "A", "side": "background", "x": 200, "origin": [0, 0]}));
    let v = run(&mut s, "layer.inspect", json!({"glyph": "A", "side": "background"}));
    assert_eq!(v["shapes"][0]["nodes"][2]["x"], json!(20.0));

    // A locked glyph refuses, and nothing changes.
    s.edit("lock", |font| {
        font.glyph_mut("A")?.info.locked = true;
        Ok(())
    })
    .unwrap();
    let before = font(&s);
    let e = s.execute("transform.mirrorVertical", &json!({"glyph": "A"})).unwrap_err().to_string();
    assert!(e.contains("locked"), "{e}");
    assert_eq!(font(&s), before);
}

/// `A`: shape 0 is rect(0, 0, 100, 100), shape 1 rect(200, 50, 250, 300); the background holds
/// rect(10, 10, 90, 90).
fn two_rects() -> Session {
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(0.0, 0.0, 100.0, 100.0)}));
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(200.0, 50.0, 250.0, 300.0)}));
    run(&mut s, "path.addPath", json!({"glyph": "A", "side": "background", "nodes": rect(10.0, 10.0, 90.0, 90.0)}));
    s
}

#[test]
fn align_buttons_line_objects_up_on_the_selection_box() {
    let mut s = two_rects();
    let shapes = json!({"glyph": "A", "shapes": [0, 1]});
    let before = font(&s);
    run(&mut s, "transform.alignLeft", shapes.clone());
    assert_eq!(nodes(&mut s, "A")[1][0], (0.0, 50.0));
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "transform.alignTop", shapes.clone());
    assert_eq!(nodes(&mut s, "A")[0][0], (0.0, 200.0));
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "transform.alignCenterHorizontal", shapes.clone());
    let n = nodes(&mut s, "A");
    assert_eq!((n[0][0].0, n[0][1].0, n[1][0].0, n[1][1].0), (75.0, 175.0, 100.0, 150.0), "both centred on x = 125");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(font(&s), before);

    // Two nodes of different paths: each node is one object; (250, 300) goes to x = 0.
    run(&mut s, "transform.alignLeft", json!({"glyph": "A", "points": [[0, 0], [1, 2]]}));
    let n = nodes(&mut s, "A");
    assert_eq!((n[0][0], n[1][2], n[1][1]), ((0.0, 0.0), (0.0, 300.0), (250.0, 50.0)));
    run(&mut s, "edit.undo", json!({}));

    // One object: nothing to align with; the result says so and nothing changes.
    let v = run(&mut s, "transform.alignBottom", json!({"glyph": "A", "shapes": [1]}));
    assert_eq!(v["moved"], json!(0));
    assert!(v["message"].is_string());
    assert_eq!(font(&s), before);
}

#[test]
fn align_selection_puts_points_on_one_coordinate() {
    let mut s = two_rects();
    let before = font(&s);
    // The box of (0, 0), (100, 0) and (200, 50) is wider than tall: they share one y, the
    // box centre's by default.
    let pts = json!({"glyph": "A", "points": [[0, 0], [0, 1], [1, 0]]});
    let v = run(&mut s, "path.alignSelection", pts.clone());
    assert_eq!((v["axis"].clone(), v["coordinate"].clone(), v["moved"].clone()), (json!("y"), json!(25.0), json!(3)));
    let n = nodes(&mut s, "A");
    assert_eq!((n[0][0], n[0][1], n[1][0]), ((0.0, 25.0), (100.0, 25.0), (200.0, 25.0)));
    run(&mut s, "edit.undo", json!({}));

    let mut low = pts.clone();
    low["origin"] = json!({"mode": "box", "boxPoint": "bottomLeft"});
    run(&mut s, "path.alignSelection", low);
    assert_eq!(nodes(&mut s, "A")[1][0], (200.0, 0.0));
    run(&mut s, "edit.undo", json!({}));

    let mut vertical = pts;
    vertical["axis"] = json!("vertical");
    let v = run(&mut s, "path.alignSelection", vertical);
    assert_eq!((v["axis"].clone(), v["coordinate"].clone()), (json!("x"), json!(100.0)));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(font(&s), before);

    // One node goes onto the nearest node of the background.
    let v = run(&mut s, "path.alignSelection", json!({"glyph": "A", "points": [[0, 2]]}));
    assert_eq!(v["to"], json!([90.0, 90.0]));
    assert_eq!(nodes(&mut s, "A")[0][2], (90.0, 90.0));
}

#[test]
fn the_string_form_and_the_parameter_form_give_the_same_font() {
    let setup = || {
        let mut s = Session::with_document();
        for g in ["A", "B"] {
            run(&mut s, "path.addPath", json!({"glyph": g, "nodes": rect(100.0, 0.0, 300.0, 700.0)}));
        }
        s
    };
    let settings = json!({"scale": {"x": 90}, "slant": 10, "translate": {"x": 20, "y": 0}, "origin": "xHeight"});
    let mut a = setup();
    let mut p = settings.clone();
    p["glyphs"] = json!(["A", "B"]);
    run(&mut a, "path.transformations", p);

    let copied = run(&mut a, "path.transformations.copyFilterParameter", settings);
    let text = copied["text"].as_str().unwrap().to_string();
    assert_eq!(text, "Transformations;ScaleX:90;ScaleY:90;Slant:10;SlantCorrection:0;OffsetX:20;Origin:2;");

    let mut b = setup();
    run(&mut b, "path.transformations", json!({"glyphs": ["A", "B"], "string": text}));
    for g in ["A", "B"] {
        assert_eq!(nodes(&mut a, g), nodes(&mut b, g), "{g}");
    }
    assert_ne!(nodes(&mut a, "A"), nodes(&mut setup(), "A"));

    // The example the reference's dialog copies.
    let v = run(&mut a, "path.transformations.copyFilterParameter", json!({"slant": 10, "origin": "baseline"}));
    assert_eq!(v["text"], json!("Transformations;Slant:10;SlantCorrection:0;Origin:4;"));
    let v = run(&mut a, "path.transformations.copyPreFilterParameter", json!({"slant": 10}));
    assert_eq!((v["text"].clone(), v["parameter"].clone()), (json!("Transformations;Slant:10;SlantCorrection:0;Origin:4;"), json!("PreFilter")));
}

#[test]
fn transformations_follow_their_targets_and_refuse_what_they_cannot_do() {
    let mut s = Session::with_document();
    for g in ["A", "B"] {
        run(&mut s, "path.addPath", json!({"glyph": g, "nodes": rect(100.0, 0.0, 300.0, 700.0)}));
    }
    // A string without glyph or glyphs runs on every glyph its scoping lets through.
    let v = run(&mut s, "path.transformations", json!({"string": "Transformations;OffsetX:10;Origin:4;include:B"}));
    assert_eq!(v["glyphs"], json!(["B"]));
    assert_eq!(nodes(&mut s, "B")[0][0], (110.0, 0.0));
    assert_eq!(nodes(&mut s, "A")[0][0], (100.0, 0.0));

    // Selected points only.
    run(&mut s, "path.transformations", json!({"glyph": "A", "points": [[0, 2]], "translate": {"x": 5, "y": 5}}));
    let n = nodes(&mut s, "A");
    assert_eq!((n[0][2], n[0][3]), ((305.0, 705.0), (100.0, 700.0)));

    let before = font(&s);
    for bad in [
        json!({"glyphs": ["A"], "slant": 10, "cursivy": true}),
        json!({"glyphs": ["A"], "string": "Transformations;LSB:10;Origin:4;"}),
        json!({"glyphs": ["A"], "string": "Transformations;Origin:4;", "slant": 5}),
        json!({"glyphs": ["A"], "slant": 95}),
        json!({"glyphs": ["A"], "layers": ["nope"], "slant": 5}),
        json!({"slant": 5}),
    ] {
        assert!(s.execute("path.transformations", &bad).is_err(), "{bad}");
    }
    let e = s.execute("path.transformations", &json!({"glyphs": ["A"], "slant": 10, "cursivy": true})).unwrap_err().to_string();
    assert!(e.contains("stem"), "{e}");
    assert_eq!(font(&s), before);
}
