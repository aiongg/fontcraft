//! Background and layers (M3.10): Selection to Background and its reverse, swap, clear, assign,
//! Interpolate with Background, and the Layers palette commands.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use fontcraft_engine::Session;
use fontcraft_engine::io::{SaveFormat, save};
use fontcraft_engine::model::{Anchor, Component, Layer, LayerSide, Path, Shape};
use fontcraft_testkit::{snapshot, temp_dir};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// The master layer of `glyph` in the new font's only master.
fn layer(s: &Session, glyph: &str) -> Layer {
    s.font().unwrap().glyph(glyph).unwrap().layer("m01").unwrap().clone()
}

/// `A`: a 100-unit square (shape 0), a component of `B` (1) and an anchor `top` in the
/// foreground; a 200-unit square and an anchor `top` in the background.
fn session() -> Session {
    let mut s = Session::with_document();
    s.edit("setup", |font| {
        let mut g = font.glyph_mut("A")?;
        let l = g.layer_mut("m01").unwrap();
        l.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
        l.shapes.push(Shape::Component(Component::new("B", 10.0, 0.0)));
        l.anchors.push(Anchor::new("top", 50.0, 100.0));
        let b = l.parts_mut(LayerSide::Background);
        b.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 200.0, 200.0)));
        b.anchors.push(Anchor::new("top", 100.0, 200.0));
        Ok(())
    })
    .unwrap();
    s
}

fn lock(s: &mut Session, glyph: &str) {
    s.edit("Lock", |font| {
        font.glyph_mut(glyph)?.info.locked = true;
        Ok(())
    })
    .unwrap();
}

fn points(shape: &Shape) -> Vec<(f64, f64)> {
    match shape {
        Shape::Path(p) => p.nodes.iter().map(|n| (n.x, n.y)).collect(),
        other => panic!("not a path: {other:?}"),
    }
}

#[test]
fn selection_to_background_replaces_the_other_side_and_works_in_reverse() {
    let mut s = session();
    let r = run(&mut s, "path.selectionToBackground", json!({"glyph": "A", "points": [[0, 2]], "anchors": ["top"]}));
    assert_eq!(r["to"], "background");
    assert_eq!((r["shapes"].as_u64(), r["anchors"].as_u64()), (Some(1), Some(1)));
    let l = layer(&s, "A");
    let bg = l.parts(LayerSide::Background);
    assert_eq!(bg.shapes, &l.shapes[..1], "the touched path is copied whole and replaces the old background");
    assert_eq!(bg.anchors, &l.anchors[..]);
    assert_eq!(l.shapes.len(), 2, "the foreground is unchanged");

    // While the background is edited, the command writes the foreground.
    let r = run(&mut s, "path.selectionToBackground", json!({"glyph": "A", "side": "background"}));
    assert_eq!(r["to"], "foreground");
    let l = layer(&s, "A");
    assert_eq!(l.shapes.len(), 1);
    assert_eq!(points(&l.shapes[0]), points(&Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0))));

    // Font View form: each glyph's foreground into its background, one undo step.
    run(&mut s, "path.addPath", json!({"glyph": "C", "nodes": fontcraft_testkit::rect(0.0, 0.0, 50.0, 50.0)}));
    run(&mut s, "path.selectionToBackground", json!({"glyphs": ["A", "C"]}));
    assert_eq!(layer(&s, "C").parts(LayerSide::Background).shapes.len(), 1);
    run(&mut s, "edit.undo", json!({}));
    assert!(layer(&s, "C").info.background.is_none(), "undo removes the background it added");
}

#[test]
fn add_selection_to_background_keeps_what_is_there() {
    let mut s = session();
    run(&mut s, "path.addSelectionToBackground", json!({"glyph": "A", "shapes": [1], "anchors": ["top"]}));
    let l = layer(&s, "A");
    let bg = l.parts(LayerSide::Background);
    assert_eq!(bg.shapes.len(), 2);
    assert!(matches!(&bg.shapes[1], Shape::Component(c) if c.glyph == "B"));
    assert_eq!(bg.anchors.len(), 1, "an anchor of the same name is replaced");
    assert_eq!((bg.anchors[0].x, bg.anchors[0].y), (50.0, 100.0));
}

#[test]
fn bad_targets_change_nothing() {
    let mut s = session();
    let before = snapshot(&s);
    for p in [
        json!({"glyph": "A", "shapes": [9]}),
        json!({"glyph": "A", "guides": [{"scope": "local", "index": 0}]}),
        json!({"glyphs": ["A"], "shapes": [0]}),
        json!({"glyph": "A", "glyphs": ["A"]}),
        json!({"glyph": "nope"}),
    ] {
        assert!(s.execute("path.selectionToBackground", &p).is_err(), "{p}");
    }
    assert!(s.execute("path.interpolateWithBackground", &json!({"glyph": "A", "amount": 1e9})).is_err());
    assert!(s.execute("path.swapWithBackground", &json!({"glyph": "A", "layer": "nope"})).is_err());
    assert_eq!(snapshot(&s), before);
}

#[test]
fn swapping_twice_restores_the_layer() {
    let mut s = session();
    run(&mut s, "path.addPath", json!({"glyph": "C", "nodes": fontcraft_testkit::rect(0.0, 0.0, 50.0, 50.0)}));
    let (a, c) = (layer(&s, "A"), layer(&s, "C"));
    let r = run(&mut s, "path.swapWithBackground", json!({"glyph": "A"}));
    assert_eq!(r["selection"], json!({"glyph": "A", "layer": "m01"}));
    let swapped = layer(&s, "A");
    assert_eq!(swapped.shapes, a.parts(LayerSide::Background).shapes);
    assert_eq!(swapped.parts(LayerSide::Background).shapes, &a.shapes[..]);
    run(&mut s, "path.swapWithBackground", json!({"glyph": "A"}));
    assert_eq!(layer(&s, "A"), a);
    // A layer without a background gets none back.
    run(&mut s, "path.swapWithBackground", json!({"glyphs": ["C"]}));
    assert!(layer(&s, "C").shapes.is_empty());
    run(&mut s, "path.swapWithBackground", json!({"glyphs": ["C"]}));
    assert_eq!(layer(&s, "C"), c);
}

#[test]
fn clear_background_skips_locked_glyphs_in_a_list_and_refuses_one() {
    let mut s = session();
    run(&mut s, "path.addPath", json!({"glyph": "B", "side": "background", "nodes": fontcraft_testkit::rect(0.0, 0.0, 5.0, 5.0)}));
    lock(&mut s, "B");
    let r = run(&mut s, "path.clearBackground", json!({"glyphs": ["A", "B"]}));
    assert_eq!(r["glyphs"], json!(["A"]));
    assert_eq!(r["skipped"][0]["glyph"], "B");
    assert!(layer(&s, "A").info.background.is_none());
    assert_eq!(layer(&s, "B").parts(LayerSide::Background).shapes.len(), 1);
    let before = snapshot(&s);
    assert!(s.execute("path.clearBackground", &json!({"glyph": "B"})).is_err());
    assert_eq!(snapshot(&s), before);
}

#[test]
fn interpolate_blends_toward_the_background_and_skips_incompatible_layers() {
    let mut s = Session::with_document();
    s.edit("setup", |font| {
        for (name, extra) in [("A", false), ("C", true)] {
            let mut g = font.glyph_mut(name)?;
            let l = g.layer_mut("m01").unwrap();
            l.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
            l.anchors.push(Anchor::new("top", 50.0, 100.0));
            let b = l.parts_mut(LayerSide::Background);
            b.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 200.0, 200.0)));
            b.anchors.push(Anchor::new("top", 100.0, 200.0));
            if extra {
                b.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 1.0, 1.0)));
            }
        }
        Ok(())
    })
    .unwrap();
    let c = layer(&s, "C");
    let r = run(&mut s, "path.interpolateWithBackground", json!({"glyphs": ["A", "C"], "amount": 50}));
    assert_eq!(r["glyphs"], json!(["A"]));
    assert_eq!(r["skipped"][0]["glyph"], "C");
    assert_eq!(r["skipped"][0]["layer"], "m01");
    assert_eq!(layer(&s, "C"), c, "an incompatible layer is left as it is");
    let a = layer(&s, "A");
    assert_eq!(points(&a.shapes[0]), points(&Shape::Path(Path::rect(0.0, 0.0, 150.0, 150.0))));
    assert_eq!((a.anchors[0].x, a.anchors[0].y), (75.0, 150.0));

    // Beyond 100 % it extrapolates (from 150 toward 200 and past it).
    run(&mut s, "path.interpolateWithBackground", json!({"glyph": "A", "amount": 200}));
    assert_eq!(points(&layer(&s, "A").shapes[0]), points(&Shape::Path(Path::rect(0.0, 0.0, 250.0, 250.0))));
}

#[test]
fn assign_background_copies_from_another_file_by_glyph_name() {
    let dir = temp_dir("background-assign");
    let file = dir.join("source.fontcraft");
    let mut src = Session::with_document();
    run(&mut src, "path.addPath", json!({"glyph": "A", "nodes": fontcraft_testkit::rect(0.0, 0.0, 300.0, 300.0)}));
    save(src.font().unwrap(), &file, SaveFormat::Native).unwrap();

    let mut s = session();
    run(&mut s, "glyph.newGlyph", json!({"name": "only.here"}));
    lock(&mut s, "C");
    let r = run(&mut s, "path.assignBackground", json!({"file": file.to_string_lossy(), "glyphs": ["A", "only.here", "C"]}));
    assert_eq!(r["glyphs"], json!(["A"]));
    let reasons: Vec<&str> = r["skipped"].as_array().unwrap().iter().map(|v| v["glyph"].as_str().unwrap()).collect();
    assert_eq!(reasons, ["only.here", "C"]);
    let l = layer(&s, "A");
    let bg = l.parts(LayerSide::Background);
    assert_eq!(bg.shapes, &layer(&src, "A").shapes[..]);
    assert!(bg.anchors.is_empty(), "the source's anchors replace the old ones");

    // Without a file the font itself is the source: a snapshot of the current outlines.
    run(&mut s, "path.assignBackground", json!({"glyphs": ["A"]}));
    let l = layer(&s, "A");
    assert_eq!(l.parts(LayerSide::Background).shapes, &l.shapes[..]);

    assert!(s.execute("path.assignBackground", &json!({"file": dir.join("missing.glyphs").to_string_lossy()})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn backup_layers_are_added_listed_renamed_reordered_and_deleted() {
    let mut s = session();
    let r = run(&mut s, "layers.addBackupLayer", json!({"glyphs": ["A"], "name": "first"}));
    let first = r["layer"].as_str().unwrap().to_string();
    assert_eq!(r["selection"], json!({"glyph": "A", "layer": first}));
    let copy = s.font().unwrap().glyph("A").unwrap().layers.iter().find(|l| l.id == first).unwrap().clone();
    let master = layer(&s, "A");
    assert_eq!((copy.master.as_str(), copy.shapes.clone(), copy.info.name.as_deref()), ("m01", master.shapes.clone(), Some("first")));
    let second = run(&mut s, "layers.addBackupLayer", json!({"glyphs": ["A"]}))["layer"].as_str().unwrap().to_string();
    assert_ne!(first, second);

    let list = run(&mut s, "layers.list", json!({"glyph": "A"}));
    let rows = list["layers"].as_array().unwrap();
    let ids: Vec<&str> = rows.iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["m01", first.as_str(), second.as_str()]);
    assert_eq!((rows[0]["kind"].as_str(), rows[0]["name"].as_str()), (Some("master"), Some("Regular")));
    assert_eq!(rows[1]["kind"], "backup");
    assert!(rows[2]["name"].as_str().unwrap().len() >= 16, "the default name is a date and time");

    run(&mut s, "layers.rename", json!({"glyph": "A", "layer": second, "name": "older"}));
    run(&mut s, "layers.reorder", json!({"glyph": "A", "layer": second, "toIndex": 0}));
    let list = run(&mut s, "layers.list", json!({"glyph": "A"}));
    assert_eq!(list["layers"][1]["id"], second.as_str());
    assert_eq!(list["layers"][1]["name"], "older");

    run(&mut s, "layers.toggleVisibility", json!({"glyphs": ["A"], "layer": first}));
    assert_eq!(run(&mut s, "layers.list", json!({"glyph": "A"}))["layers"][2]["visible"], true);

    let before = snapshot(&s);
    for (id, p) in [
        ("layers.delete", json!({"glyphs": ["A"], "layers": ["m01"]})),
        ("layers.delete", json!({"glyphs": ["A"], "layers": ["nope"]})),
        ("layers.rename", json!({"glyph": "A", "layer": "m01", "name": "x"})),
        ("layers.rename", json!({"glyph": "A", "layer": first, "name": " "})),
        ("layers.reorder", json!({"glyph": "A", "layer": "m01", "toIndex": 0})),
        ("layers.reorder", json!({"glyph": "A", "layer": first, "toIndex": 2})),
        ("layers.useAsMaster", json!({"glyphs": ["A"], "layer": "m01"})),
    ] {
        assert!(s.execute(id, &p).is_err(), "{id} {p}");
    }
    assert_eq!(snapshot(&s), before);

    let r = run(&mut s, "layers.delete", json!({"glyphs": ["A"], "layers": [first, second]}));
    assert_eq!(r["deleted"], 2);
    assert_eq!(s.font().unwrap().glyph("A").unwrap().layers.len(), 1);
}

#[test]
fn reorder_moves_a_layer_among_its_own_masters_layers() {
    let mut s = session();
    s.edit("second master", |font| {
        let mut m = font.default_master()?.clone();
        m.id = "m02".into();
        m.name = "Bold".into();
        std::sync::Arc::make_mut(&mut font.masters).push(m);
        font.glyph_mut("A")?.layers.push(Layer::new("m02", 600.0));
        Ok(())
    })
    .unwrap();
    let mut add = |master: &str, name: &str| {
        run(&mut s, "layers.addBackupLayer", json!({"glyphs": ["A"], "master": master, "name": name}))["layer"].as_str().unwrap().to_string()
    };
    let (one, other, two, three) = (add("m01", "one"), add("m02", "other"), add("m01", "two"), add("m01", "three"));
    let list = |s: &mut Session| -> Vec<String> {
        run(s, "layers.list", json!({"glyph": "A"}))["layers"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap().to_string()).collect()
    };
    // The index counts the master's other layers; the second master's backup sits between them
    // in the glyph and stays where it is.
    for (layer, to, expected) in [(&three, 0, [&three, &one, &two]), (&three, 2, [&one, &two, &three]), (&one, 1, [&two, &one, &three])] {
        run(&mut s, "layers.reorder", json!({"glyph": "A", "layer": layer, "toIndex": to}));
        let ids = list(&mut s);
        assert_eq!(ids, ["m01", expected[0].as_str(), expected[1].as_str(), expected[2].as_str(), "m02", other.as_str()], "{layer} to {to}");
    }
    assert!(s.execute("layers.reorder", &json!({"glyph": "A", "layer": other, "toIndex": 1})).is_err(), "m02 has one backup");
}

#[test]
fn use_as_master_swaps_the_backup_in_and_keeps_the_old_master_content() {
    let mut s = session();
    let backup = run(&mut s, "layers.addBackupLayer", json!({"glyphs": ["A"]}))["layer"].as_str().unwrap().to_string();
    let old = layer(&s, "A");
    // Draw something new in the master layer, then restore the backup.
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": fontcraft_testkit::rect(0.0, 0.0, 9.0, 9.0)}));
    let edited = layer(&s, "A");
    let r = run(&mut s, "layers.useAsMaster", json!({"glyphs": ["A"], "layer": backup, "name": "before restore"}));
    let kept = r["backup"].as_str().unwrap().to_string();
    let g = s.font().unwrap().glyph("A").unwrap().clone();
    assert_eq!(g.layers.len(), 2);
    assert!(g.layers.iter().all(|l| l.id != backup), "the restored backup is removed");
    assert_eq!(layer(&s, "A").shapes, old.shapes);
    let k = g.layers.iter().find(|l| l.id == kept).unwrap();
    assert_eq!((k.shapes.clone(), k.info.name.as_deref()), (edited.shapes.clone(), Some("before restore")));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(layer(&s, "A"), edited, "one undo step");
}

#[test]
fn layer_commands_refuse_a_locked_glyph() {
    let mut s = session();
    lock(&mut s, "A");
    let before = snapshot(&s);
    for (id, p) in [
        ("layers.addBackupLayer", json!({"glyphs": ["A"]})),
        ("layers.toggleVisibility", json!({"glyphs": ["A"], "layer": "m01"})),
        ("path.selectionToBackground", json!({"glyph": "A"})),
        ("path.swapWithBackground", json!({"glyph": "A"})),
    ] {
        assert!(s.execute(id, &p).is_err(), "{id}");
    }
    assert_eq!(snapshot(&s), before);
}
