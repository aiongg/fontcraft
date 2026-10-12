//! Per-node stroke widths (M13.38): `path.setNodeWidths`, and the stroke outline and Expand
//! Outline reading the widths.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use fontcraft_engine::Session;
use fontcraft_engine::cmd::stroke::layer_outline;
use fontcraft_engine::io::{SaveFormat, open, save};
use fontcraft_engine::model::{Layer, Shape, node_width};
use fontcraft_geom::{BezPath, Shape as _};
use fontcraft_testkit::temp_dir;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn layer(s: &Session) -> Layer {
    s.font().unwrap().glyph("A").unwrap().layer("m01").unwrap().clone()
}

fn widths(s: &Session, node: usize) -> Option<[f64; 2]> {
    match &layer(s).shapes[0] {
        Shape::Path(p) => node_width::get(&p.nodes[node]),
        other => panic!("not a path: {other:?}"),
    }
}

fn bounds(paths: &[BezPath]) -> (f64, f64) {
    let b = paths.iter().map(|p| p.bounding_box()).reduce(|a, b| a.union(b)).unwrap();
    (b.y0, b.y1)
}

/// A stroked open path from (0, 0) to (100, 0), 20 units wide and centred.
fn stroked_line() -> Session {
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "A", "closed": false, "nodes": [{"x": 0, "y": 0}, {"x": 100, "y": 0}]}));
    run(&mut s, "path.setStroke", json!({"glyph": "A", "shapes": [0], "width": 20}));
    s
}

#[test]
fn set_node_widths_shapes_the_stroke_and_clear_restores_it() {
    let mut s = stroked_line();
    let plain = bounds(&layer_outline(s.font().unwrap(), &layer(&s)).unwrap());
    assert!((plain.0 + 10.0).abs() < 1e-6 && (plain.1 - 10.0).abs() < 1e-6, "{plain:?}");

    // Only `left` given: the right side keeps the path's 10 units.
    let r = run(&mut s, "path.setNodeWidths", json!({"glyph": "A", "points": [[0, 1]], "left": 30}));
    assert_eq!(r["widths"], json!([[30.0, 10.0]]));
    assert_eq!(widths(&s, 1), Some([30.0, 10.0]));
    assert_eq!(widths(&s, 0), None);
    let (y0, y1) = bounds(&layer_outline(s.font().unwrap(), &layer(&s)).unwrap());
    assert!((y0 + 10.0).abs() < 1e-6 && (y1 - 30.0).abs() < 1e-6, "the stroke widens towards node 1 on the left: {y0} {y1}");

    // One value per point.
    run(&mut s, "path.setNodeWidths", json!({"glyph": "A", "points": [[0, 0], [0, 1]], "left": [5, 6], "right": [7, 8]}));
    assert_eq!((widths(&s, 0), widths(&s, 1)), (Some([5.0, 7.0]), Some([6.0, 8.0])));

    let r = run(&mut s, "path.setNodeWidths", json!({"glyph": "A", "points": [[0, 0], [0, 1]], "clear": true}));
    assert_eq!(r["widths"], json!([null, null]));
    assert_eq!(bounds(&layer_outline(s.font().unwrap(), &layer(&s)).unwrap()), plain);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(widths(&s, 1), Some([6.0, 8.0]), "one undo step brings the widths back");
}

#[test]
fn set_node_widths_refusals_change_nothing() {
    let mut s = stroked_line();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": fontcraft_testkit::rect(200.0, 0.0, 300.0, 100.0)}));
    let before = layer(&s);
    for bad in [
        json!({"glyph": "A", "points": [[0, 1]]}),
        json!({"glyph": "A", "points": [[0, 1]], "left": -1}),
        json!({"glyph": "A", "points": [[0, 1]], "left": 1e9}),
        json!({"glyph": "A", "points": [[0, 1]], "left": [1, 2]}),
        json!({"glyph": "A", "points": [[0, 1]], "left": "wide"}),
        json!({"glyph": "A", "points": [[0, 1]], "left": 5, "clear": true}),
        json!({"glyph": "A", "points": [[0, 7]], "left": 5}),
        json!({"glyph": "A", "points": [[1, 0]], "left": 5}),
        json!({"glyph": "A", "shapes": [0], "left": 5}),
        json!({"glyph": "A", "left": 5}),
    ] {
        assert!(s.execute("path.setNodeWidths", &bad).is_err(), "{bad}");
    }
    assert_eq!(layer(&s), before);

    s.edit("Lock", |font| {
        font.glyph_mut("A")?.info.locked = true;
        Ok(())
    })
    .unwrap();
    assert!(s.execute("path.setNodeWidths", &json!({"glyph": "A", "points": [[0, 1]], "left": 5})).is_err());
}

#[test]
fn expand_outline_and_saving_keep_node_widths() {
    let dir = temp_dir("thickness-save");
    let mut s = stroked_line();
    run(&mut s, "path.setNodeWidths", json!({"glyph": "A", "points": [[0, 1]], "left": 30, "right": 0}));
    for (format, name) in [(SaveFormat::Glyphs, "widths.glyphs"), (SaveFormat::Native, "widths.fontcraft")] {
        let file = dir.join(name);
        save(s.font().unwrap(), &file, format).unwrap();
        let (back, _) = open(&file, None).unwrap();
        let Shape::Path(p) = &back.glyph("A").unwrap().layer("m01").unwrap().shapes[0] else { panic!("not a path") };
        assert_eq!(node_width::get(&p.nodes[1]), Some([30.0, 0.0]), "{name}");
    }

    run(&mut s, "path.expandOutline", json!({"glyph": "A"}));
    let shapes = layer(&s).shapes;
    let outline: Vec<BezPath> = shapes.iter().map(|sh| if let Shape::Path(p) = sh { p.to_bez().unwrap() } else { panic!("not a path") }).collect();
    let (y0, y1) = bounds(&outline);
    assert!((y0 + 10.0).abs() < 0.5 && (y1 - 30.0).abs() < 0.5, "the expanded outline follows the node widths: {y0} {y1}");
}
