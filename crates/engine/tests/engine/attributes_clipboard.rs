//! Copy Attributes and Paste Attributes (M3.57).
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use fontcraft_engine::Session;
use fontcraft_model::{Path, Shape};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

/// Four rectangles in glyph A, the first with a masking stroke.
fn session() -> Session {
    let mut s = Session::with_document();
    for k in 0..4 {
        run(&mut s, "path.addRectangle", json!({"glyph": "A", "x": 200 * k, "y": 0, "width": 100, "height": 100}));
    }
    run(
        &mut s,
        "path.setStroke",
        json!({"glyph": "A", "shapes": [0], "width": 30, "height": 20, "join": "round", "capStart": "square", "fill": true}),
    );
    run(&mut s, "path.setMask", json!({"glyph": "A", "shapes": [0], "mask": true}));
    s
}

fn path(s: &Session, i: usize) -> Path {
    let font = s.font().unwrap();
    let master = font.default_master().unwrap().id.clone();
    match &font.glyph("A").unwrap().layer(&master).unwrap().shapes[i] {
        Shape::Path(p) => p.clone(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn one_stroke_goes_onto_three_paths_in_one_undo_step() {
    let mut s = session();
    let copied = run(&mut s, "edit.copyAttributes", json!({"glyph": "A", "points": [[0, 0], [0, 1], [0, 2], [0, 3]]}));
    assert_eq!(copied["kind"], "path");
    let before = s.font().unwrap().glyph("A").unwrap().clone();
    // Paths named whole, and by one of their points.
    let r = run(&mut s, "edit.pasteAttributes", json!({"glyph": "A", "shapes": [1, 2], "points": [[3, 2]], "text": copied["text"]}));
    assert_eq!((&r["kind"], &r["changed"]), (&json!("path"), &json!(3)));
    for i in 1..4 {
        assert_eq!(path(&s, i).info.attrs, path(&s, 0).info.attrs, "path {i}");
        assert_eq!(path(&s, i).nodes, session_path(i).nodes, "no point moves");
    }
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.font().unwrap().glyph("A").unwrap().layers, before.layers);

    // A path without a stroke takes one away.
    let copied = run(&mut s, "edit.copyAttributes", json!({"glyph": "A", "shapes": [1]}));
    run(&mut s, "edit.pasteAttributes", json!({"glyph": "A", "shapes": [0], "text": copied["text"]}));
    assert!(path(&s, 0).info.attrs.is_empty());
}

fn session_path(i: usize) -> Path {
    path(&session(), i)
}

#[test]
fn a_nodes_type_and_name_go_onto_nodes_and_bad_text_changes_nothing() {
    let mut s = session();
    s.edit("Name", |font| {
        let master = font.default_master()?.id.clone();
        let mut g = font.glyph_mut("A")?;
        for l in g.layers.iter_mut().filter(|l| l.id == master) {
            if let Some(Shape::Path(p)) = l.shapes.get_mut(0) {
                p.nodes[1].smooth = true;
                p.nodes[1].info.name = Some("tip".into());
            }
        }
        Ok(())
    })
    .unwrap();
    let copied = run(&mut s, "edit.copyAttributes", json!({"glyph": "A", "points": [[0, 1]]}));
    assert_eq!(copied["kind"], "node");
    run(&mut s, "edit.pasteAttributes", json!({"glyph": "A", "points": [[1, 0], [2, 3]], "text": copied["text"]}));
    for (i, k) in [(1, 0), (2, 3)] {
        let n = &path(&s, i).nodes[k];
        assert_eq!((n.smooth, n.info.name.as_deref()), (true, Some("tip")));
    }
    assert!(path(&s, 1).info.attrs.is_empty(), "the path's attributes stay");

    let before = s.font().unwrap().glyph("A").unwrap().clone();
    let wide = json!({"format": "fontcraft-attributes", "version": 1, "path": {"stroke_width": 1e300}}).to_string();
    let name = json!({"format": "fontcraft-attributes", "version": 1, "node": {"smooth": true, "name": "a".repeat(1000)}}).to_string();
    for text in ["", "{}", "words", &wide, &name] {
        assert!(s.execute("edit.pasteAttributes", &json!({"glyph": "A", "points": [[1, 1]], "text": text})).is_err(), "{text}");
    }
    assert!(s.execute("edit.pasteAttributes", &json!({"glyph": "A", "shapes": [1], "text": copied["text"]})).is_err(), "a node needs nodes");
    assert_eq!(s.font().unwrap().glyph("A").unwrap().layers, before.layers);
}
