//! Revert Glyph, Copy Layer to Layer, Paste After and Compare Layers (M13.44).
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use fontcraft_engine::Session;
use fontcraft_engine::model::{Component, Layer, Path, Shape};
use fontcraft_testkit::snapshot;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn layer(s: &Session, glyph: &str, id: &str) -> Layer {
    s.font().unwrap().glyph(glyph).unwrap().layers.iter().find(|l| l.id == id).unwrap().clone()
}

fn xs(shape: &Shape) -> Vec<f64> {
    match shape {
        Shape::Path(p) => p.nodes.iter().map(|n| n.x).collect(),
        other => panic!("not a path: {other:?}"),
    }
}

/// Adds a non-master layer `bk` of master `m01` to `glyph`, holding `shapes`.
fn add_backup(s: &mut Session, glyph: &str, shapes: Vec<Shape>) {
    s.edit("setup", |font| {
        let mut l = Layer::new("m01", 600.0);
        l.id = "bk".into();
        l.shapes = shapes;
        font.glyph_mut(glyph)?.layers.push(l);
        Ok(())
    })
    .unwrap();
}

#[test]
fn revert_glyph_restores_one_glyph_and_keeps_other_unsaved_edits() {
    let mut s = Session::with_document();
    let saved_a = layer(&s, "A", "m01");
    s.edit("draw", |font| {
        for name in ["A", "B"] {
            let mut g = font.glyph_mut(name)?;
            let l = g.layer_mut("m01").unwrap();
            l.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
            l.width = 321.0;
        }
        Ok(())
    })
    .unwrap();

    let r = run(&mut s, "file.revertGlyph", json!({"glyphs": ["A"]}));
    assert_eq!(r["reverted"], json!(["A"]));
    assert_eq!(layer(&s, "A", "m01"), saved_a, "A is as it was saved");
    assert_eq!(layer(&s, "B", "m01").width, 321.0, "B keeps its unsaved edit");
    assert!(s.doc().unwrap().modified());

    run(&mut s, "edit.undo", json!({}));
    assert_eq!(layer(&s, "A", "m01").width, 321.0, "the revert is one undo step");

    // With `layers`, only those layers come back; a layer added since the save is removed.
    add_backup(&mut s, "B", vec![]);
    run(&mut s, "file.revertGlyph", json!({"glyphs": ["B"], "layers": ["bk"]}));
    let b = s.font().unwrap().glyph("B").unwrap().clone();
    assert_eq!(b.layers.len(), 1, "the backup layer added since the save is gone");
    assert_eq!(b.layers[0].width, 321.0, "the master layer was not named, so it keeps its edit");

    // Without `glyphs`: every glyph that differs from the saved file.
    let r = run(&mut s, "file.revertGlyph", json!({}));
    assert_eq!(r["reverted"], json!(["A", "B"]));
    assert_eq!(layer(&s, "B", "m01").width, 600.0);

    // A glyph the saved file does not have is refused, and nothing changes.
    run(&mut s, "glyph.newGlyph", json!({"name": "A.alt"}));
    let before = snapshot(&s);
    assert!(s.execute("file.revertGlyph", &json!({"glyphs": ["A.alt"]})).is_err());
    assert_eq!(snapshot(&s), before);
}

#[test]
fn copy_layer_to_layer_replaces_or_appends() {
    let mut s = Session::with_document();
    s.edit("draw", |font| {
        let mut g = font.glyph_mut("A")?;
        let l = g.layer_mut("m01").unwrap();
        l.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
        l.shapes.push(Shape::Component(Component::new("B", 200.0, 0.0)));
        Ok(())
    })
    .unwrap();
    add_backup(&mut s, "A", vec![Shape::Path(Path::rect(0.0, 0.0, 50.0, 50.0))]);

    let r = run(&mut s, "edit.copyLayerToLayer", json!({"glyphs": ["A", "B"], "from": "m01", "to": "bk"}));
    assert_eq!(r["copied"], json!(["A"]));
    assert_eq!(r["skipped"][0]["glyph"], "B", "B has no layer bk");
    assert_eq!(layer(&s, "A", "bk").shapes, layer(&s, "A", "m01").shapes, "replace drops what bk had");
    assert_eq!(layer(&s, "A", "bk").width, 600.0, "the width is not copied");

    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.copyLayerToLayer", json!({"glyphs": ["A"], "from": "m01", "to": "bk", "mode": "append"}));
    let bk = layer(&s, "A", "bk");
    assert_eq!(bk.shapes.len(), 3, "append keeps the old square and adds the path and the component");
    assert_eq!(xs(&bk.shapes[0]).iter().cloned().fold(0.0, f64::max), 50.0);
    assert!(matches!(&bk.shapes[2], Shape::Component(c) if c.glyph == "B"));

    let before = snapshot(&s);
    assert!(s.execute("edit.copyLayerToLayer", &json!({"glyphs": ["B"], "from": "m01", "to": "bk"})).is_err(), "no glyph has both layers");
    assert!(s.execute("edit.copyLayerToLayer", &json!({"from": "m01", "to": "bk", "mode": "merge"})).is_err());
    assert_eq!(snapshot(&s), before);
}

#[test]
fn paste_after_widens_the_advance() {
    let mut s = Session::with_document();
    s.edit("draw", |font| {
        for (name, width, rect) in [("A", 200.0, (0.0, 100.0)), ("B", 300.0, (10.0, 50.0)), ("T", 100.0, (0.0, 0.0))] {
            let mut g = font.glyph_mut(name)?;
            let l = g.layer_mut("m01").unwrap();
            l.width = width;
            if rect.1 > 0.0 {
                l.shapes.push(Shape::Path(Path::rect(rect.0, 0.0, rect.1, 100.0)));
            }
        }
        Ok(())
    })
    .unwrap();
    let copied = run(&mut s, "edit.copy", json!({"glyphs": ["A", "B"]}));

    let r = run(&mut s, "edit.pasteAfter", json!({"glyph": "T", "json": copied["json"]}));
    assert_eq!(r["pasted"], json!(["A", "B"]));
    let t = layer(&s, "T", "m01");
    assert_eq!(t.width, 600.0, "100 plus the widths of A and B");
    assert_eq!(r["width"], 600.0);
    assert_eq!(t.shapes.len(), 2);
    let range = |sh: &Shape| {
        let x = xs(sh);
        (x.iter().cloned().fold(f64::MAX, f64::min), x.iter().cloned().fold(f64::MIN, f64::max))
    };
    assert_eq!(range(&t.shapes[0]), (100.0, 200.0), "A starts at T's old advance");
    assert_eq!(range(&t.shapes[1]), (310.0, 350.0), "B starts after A's advance");
    assert_eq!(r["selection"]["shapes"], json!([0, 1]));

    // Pasting again goes after what is there now: the word grows.
    run(&mut s, "edit.pasteAfter", json!({"glyph": "T", "json": copied["json"]}));
    assert_eq!(layer(&s, "T", "m01").width, 1100.0);

    // A copied glyph holding a component of the target is refused, and nothing changes.
    s.edit("component", |font| {
        font.glyph_mut("C")?.layer_mut("m01").unwrap().shapes.push(Shape::Component(Component::new("T", 0.0, 0.0)));
        Ok(())
    })
    .unwrap();
    let with_t = run(&mut s, "edit.copy", json!({"glyphs": ["C"]}));
    let before = snapshot(&s);
    assert!(s.execute("edit.pasteAfter", &json!({"glyph": "T", "json": with_t["json"]})).is_err());
    let fragment = run(&mut s, "edit.copy", json!({"glyph": "A"}));
    assert!(s.execute("edit.pasteAfter", &json!({"glyph": "T", "json": fragment["json"]})).is_err(), "a drawing fragment is refused");
    assert_eq!(snapshot(&s), before);
}

#[test]
fn compare_layers_with_tolerance() {
    let mut s = Session::with_document();
    let square = Path::rect(0.0, 0.0, 100.0, 100.0);
    let mut moved = square.clone();
    moved.nodes[1].x += 0.5;
    s.edit("draw", |font| {
        for name in ["A", "B"] {
            font.glyph_mut(name)?.layer_mut("m01").unwrap().shapes.push(Shape::Path(square.clone()));
        }
        Ok(())
    })
    .unwrap();
    add_backup(&mut s, "A", vec![Shape::Path(moved)]);
    add_backup(&mut s, "B", vec![Shape::Path(square.clone())]);

    let r = run(&mut s, "font.compareLayers", json!({"glyphs": ["A", "B", "C"], "layerA": "m01", "layerB": "bk"}));
    assert_eq!(r["different"], json!(["A"]));
    assert_eq!(r["first"], "A");
    assert_eq!(r["same"], 1);
    assert_eq!(r["missing"], json!(["C"]));
    assert!(r["details"][0]["reason"].as_str().unwrap().contains("node 1 of path 0"));

    let r = run(&mut s, "font.compareLayers", json!({"glyphs": ["A", "B"], "layerA": "m01", "layerB": "bk", "tolerance": 1}));
    assert_eq!(r["different"], json!([]));
    assert_eq!(r["same"], 2);

    assert!(s.execute("font.compareLayers", &json!({"layerA": "m01", "layerB": "nope"})).is_err(), "no glyph has both layers");
    assert!(s.execute("font.compareLayers", &json!({"layerA": "m01", "layerB": "bk", "tolerance": -1})).is_err());
}
