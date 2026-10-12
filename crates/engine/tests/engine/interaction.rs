//! Engine foundations for editing (M3.1): target objects, interactions (begin, preview, commit,
//! cancel), `layer.inspect` and the glyph lock.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::model::{Anchor, Annotation, Component, Guide, LayerSide, Path, Shape};
use fontcraft_engine::params::{GuideScope, Params, Targets};
use fontcraft_engine::{EngineError, Session};
use fontcraft_testkit::{rect, snapshot};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// `A` holds a closed rectangle (shape 0), a component (1) and an open path (2); an anchor, a
/// local guide, an annotation and a background path; its master has one global guide.
fn session() -> Session {
    let mut s = Session::with_document();
    s.edit("setup", |font| {
        let master = font.default_master()?.id.clone();
        if let Some(m) = Arc::make_mut(&mut font.masters).first_mut() {
            m.guides.push(Guide { y: 500.0, ..Guide::default() });
        }
        let mut g = font.glyph_mut("A")?;
        let l = g.layer_mut(&master).unwrap();
        l.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
        l.shapes.push(Shape::Component(Component::new("B", 10.0, 0.0)));
        let mut open = Path::rect(200.0, 0.0, 300.0, 100.0);
        open.closed = false;
        l.shapes.push(Shape::Path(open));
        l.anchors.push(Anchor::new("top", 50.0, 700.0));
        l.info.guides.push(Guide { x: 20.0, angle: 90.0, ..Guide::default() });
        l.info.annotations.push(Annotation {
            kind: "Text".into(),
            x: 1.0,
            y: 2.0,
            angle: 0.0,
            text: Some("note".into()),
            width: 0.0,
            extra: Default::default(),
        });
        l.parts_mut(LayerSide::Background).shapes.push(Shape::Path(Path::rect(0.0, 0.0, 10.0, 10.0)));
        Ok(())
    })
    .unwrap();
    s
}

fn targets(v: Value) -> fontcraft_engine::Result<Targets> {
    Targets::parse(&Params::new("test.cmd", &v)?)
}

#[test]
fn every_target_kind_resolves() {
    let s = session();
    let font = s.font().unwrap();
    let t = targets(json!({
        "glyph": "A",
        "points": [[0, 0], [0, 3], [2, 1], [0, 0]],
        "segments": [[0, 0], [2, 3]],
        "segment": [0, 2],
        "shapes": [1, 2],
        "anchors": ["top"],
        "guides": [{"scope": "local", "index": 0}, {"scope": "global", "index": 0}],
        "annotations": [0],
    }))
    .unwrap();
    assert_eq!(t.points, [(0, 0), (0, 3), (2, 1)], "repeats are dropped");
    assert_eq!(t.segments, [(0, 0), (2, 3), (0, 2)]);
    assert_eq!(t.guides.iter().map(|g| g.scope).collect::<Vec<_>>(), [GuideScope::Local, GuideScope::Global]);
    let r = t.resolve(font).unwrap();
    assert_eq!((r.glyph.name.as_str(), r.layer.id.as_str(), r.parts.shapes.len()), ("A", "m01", 3));
    // The target object goes back out in the same shape (a command's `selection`).
    let back = targets(t.to_json()).unwrap();
    assert_eq!(back, t);
    // The layer by id, and the background side.
    let bg = targets(json!({"glyph": "A", "layer": "m01", "side": "background", "points": [[0, 3]]})).unwrap();
    assert_eq!(bg.resolve(font).unwrap().parts.shapes.len(), 1);

    // layer.inspect lists everything with the addresses the targets use.
    let mut s = s;
    let v = run(&mut s, "layer.inspect", json!({"glyph": "A"}));
    assert_eq!(v["layer"], "m01");
    assert_eq!(v["shapes"][0]["nodes"][3], json!({"index": 3, "x": 0.0, "y": 100.0, "type": "line", "smooth": false}));
    assert_eq!(v["shapes"][1]["kind"], "component");
    assert_eq!(v["shapes"][1]["transform"], json!([1.0, 0.0, 0.0, 1.0, 10.0, 0.0]));
    assert_eq!(v["shapes"][2]["closed"], false);
    assert_eq!(v["anchors"], json!([{"name": "top", "x": 50.0, "y": 700.0}]));
    assert_eq!(v["guides"][0]["scope"], "local");
    assert_eq!(v["guides"][1], json!({"scope": "global", "index": 0, "x": 0.0, "y": 500.0, "angle": 0.0, "locked": false}));
    assert_eq!(v["annotations"][0]["text"], "note");
    let bg = run(&mut s, "layer.inspect", json!({"glyph": "A", "side": "background"}));
    assert_eq!(bg["shapes"].as_array().unwrap().len(), 1);
    assert_eq!(bg["side"], "background");
}

#[test]
fn stale_and_hostile_addresses_are_errors_that_change_nothing() {
    let mut s = session();
    let bad = [
        json!({"glyph": "A", "points": [[9, 0]]}),
        json!({"glyph": "A", "points": [[1, 0]]}),
        json!({"glyph": "A", "points": [[0, 4]]}),
        json!({"glyph": "A", "segments": [[2, 0]]}),
        json!({"glyph": "A", "shapes": [3]}),
        json!({"glyph": "A", "anchors": ["bottom"]}),
        json!({"glyph": "A", "guides": [{"scope": "global", "index": 1}]}),
        json!({"glyph": "A", "annotations": [1]}),
        json!({"glyph": "A", "layer": "nope", "points": [[0, 0]]}),
        json!({"glyph": "nope"}),
        json!({"glyph": "A", "side": "background", "points": [[0, 4]]}),
        // Hostile forms.
        json!({"glyph": "A", "points": [[-1, 0]]}),
        json!({"glyph": "A", "points": [[0.5, 0]]}),
        json!({"glyph": "A", "points": [[1e300, 0]]}),
        json!({"glyph": "A", "points": [[0, 0, 0]]}),
        json!({"glyph": "A", "points": "all"}),
        json!({"glyph": "A", "guides": [{"scope": "both", "index": 0}]}),
        json!({"glyph": "A", "guides": [0]}),
        json!({"glyph": "A", "anchors": [1]}),
        json!({"glyph": "A", "side": "middle"}),
        json!({"glyph": "A", "glyphs": ["A", "B"], "shapes": [0]}),
        json!({"glyph": "A", "shapes": vec![0; fontcraft_engine::params::MAX_TARGETS + 1]}),
    ];
    for params in bad {
        let before = snapshot(&s);
        let font = s.doc().unwrap().font.clone();
        // An edit that addresses the targets fails as a whole and leaves the font as it was.
        let r = targets(params.clone()).and_then(|t| {
            s.edit("Move", |font| {
                t.edit_layer(font, |l| {
                    let mut parts = l.parts_mut(t.side);
                    for &(sh, n) in &t.points {
                        if let Some(node) = parts.node_mut(sh, n) {
                            node.x += 1.0;
                        }
                    }
                    Ok(())
                })
            })
        });
        assert!(r.is_err(), "{params} resolved");
        assert!(Arc::ptr_eq(&s.doc().unwrap().font, &font), "{params} changed the font");
        assert_eq!(snapshot(&s), before);
    }
    // A stale address: valid before the shape was removed, an error after.
    let t = targets(json!({"glyph": "A", "points": [[2, 1]]})).unwrap();
    assert!(t.resolve(s.font().unwrap()).is_ok());
    s.edit("Remove", |font| {
        font.glyph_mut("A")?.layer_mut("m01").unwrap().shapes.pop();
        Ok(())
    })
    .unwrap();
    let e = t.resolve(s.font().unwrap()).unwrap_err().to_string();
    assert!(e.contains("[2, 1] does not resolve: the layer has 2 shapes"), "{e}");
    let e = run_err(&mut s, "layer.inspect", json!({"glyph": "A", "points": [[0, 0]]}));
    assert!(e.contains("takes no target lists"), "{e}");
}

fn run_err(s: &mut Session, id: &str, p: Value) -> String {
    let before = snapshot(s);
    let e = s.execute(id, &p).err().unwrap_or_else(|| panic!("{id} {p} succeeded")).to_string();
    assert_eq!(snapshot(s), before, "{id} {p} failed but changed the session");
    e
}

#[test]
fn an_interaction_is_one_undo_step_and_one_journal_entry() {
    let mut s = session();
    let (undo, journal) = (s.doc().unwrap().history.undo.len(), s.journal().len());
    let shapes = |s: &Session| s.font().unwrap().glyph("A").unwrap().layer("m01").unwrap().shapes.len();
    s.begin("Drag").unwrap();
    for i in 0..100 {
        let x = f64::from(i);
        s.preview("path.addPath", &json!({"glyph": "A", "nodes": rect(x, 0.0, x + 10.0, 10.0)})).unwrap();
        assert_eq!(shapes(&s), 4, "each preview starts from the snapshot");
    }
    // While the interaction runs, other edits wait; queries still run.
    assert!(matches!(s.execute("glyph.newGlyph", &json!({})), Err(EngineError::Disabled(..))));
    run(&mut s, "layer.inspect", json!({"glyph": "A"}));
    // A preview that fails keeps the previous one.
    let shown = s.doc().unwrap().font.clone();
    assert!(s.preview("path.addPath", &json!({"glyph": "A", "nodes": []})).is_err());
    assert!(s.preview("edit.undo", &json!({})).is_err());
    assert!(s.preview("layer.inspect", &json!({"glyph": "A"})).is_err());
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &shown));
    assert!(s.begin("Again").is_err());

    assert!(s.commit().unwrap());
    let doc = s.doc().unwrap();
    assert_eq!(doc.history.undo.len(), undo + 1);
    assert_eq!(doc.history.undo.last().unwrap().label.as_ref(), "Drag");
    assert_eq!(s.journal().len(), journal + 1);
    let (id, params) = s.journal().last().unwrap();
    assert_eq!((id.as_str(), &params["nodes"][0]["x"]), ("path.addPath", &json!(99.0)));
    assert_eq!(shapes(&s), 4);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(shapes(&s), 3);
    assert!(s.commit().is_err() && s.cancel().is_err(), "no interaction runs");

    // A command.batch can be previewed (the tools preview a move and a connect as one).
    s.begin("Batch").unwrap();
    let step = json!({"command": "path.addPath", "params": {"glyph": "A", "nodes": rect(0.0, 0.0, 5.0, 5.0)}});
    s.preview("command.batch", &json!({"steps": [step, step]})).unwrap();
    assert_eq!(shapes(&s), 5);
    assert!(s.commit().unwrap());
    assert_eq!(s.doc().unwrap().history.undo.last().unwrap().label.as_ref(), "Batch");

    // An interaction that changed nothing leaves no step.
    let undo = s.doc().unwrap().history.undo.len();
    s.begin("Nothing").unwrap();
    assert!(!s.commit().unwrap());
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
}

#[test]
fn cancel_restores_the_snapshot_itself() {
    let mut s = session();
    let before = s.doc().unwrap().font.clone();
    let (undo, journal, revision) = (s.doc().unwrap().history.undo.len(), s.journal().len(), s.doc().unwrap().revision);
    s.begin("Drag").unwrap();
    for i in 0..10 {
        let x = f64::from(i);
        s.preview("path.addPath", &json!({"glyph": "A", "nodes": rect(x, 0.0, x + 1.0, 1.0)})).unwrap();
    }
    assert!(s.doc().unwrap().revision > revision, "previews show as changes");
    s.cancel().unwrap();
    let doc = s.doc().unwrap();
    assert!(Arc::ptr_eq(&doc.font, &before));
    assert_eq!((doc.history.undo.len(), s.journal().len()), (undo, journal));
    run(&mut s, "glyph.newGlyph", json!({}));
}

#[test]
fn a_locked_glyph_refuses_edits() {
    let mut s = session();
    s.edit("Lock", |font| {
        font.glyph_mut("A")?.info.locked = true;
        Ok(())
    })
    .unwrap();
    let e = run_err(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(0.0, 0.0, 1.0, 1.0)}));
    assert!(e.contains("glyph `A` is locked"), "{e}");
    // The rule holds for any edit, whichever code makes it, and in a preview.
    let e = s.edit("Sneak", |font| {
        font.glyph_mut("A")?.layer_mut("m01").unwrap().width = 1.0;
        Ok(())
    });
    assert!(matches!(e, Err(EngineError::Locked(ref g)) if g == "A"), "{e:?}");
    s.begin("Drag").unwrap();
    assert!(matches!(s.preview("path.addPath", &json!({"glyph": "A", "nodes": rect(0.0, 0.0, 1.0, 1.0)})), Err(EngineError::Locked(_))));
    s.cancel().unwrap();
    // Other glyphs, and unlocking, are fine; after unlocking the glyph can be edited.
    run(&mut s, "path.addPath", json!({"glyph": "B", "nodes": rect(0.0, 0.0, 1.0, 1.0)}));
    run(&mut s, "glyph.setUnicodes", json!({"glyphs": ["A"], "unicodes": ["0041"]}));
    s.edit("Unlock", |font| {
        font.glyph_mut("A")?.info.locked = false;
        Ok(())
    })
    .unwrap();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(0.0, 0.0, 1.0, 1.0)}));
}

#[test]
fn commands_list_reports_targets() {
    let mut s = session();
    let v = run(&mut s, "commands.list", json!({"pattern": "layer.inspect"}));
    assert_eq!(v["commands"][0]["targets"], json!(["layer"]));
    let v = run(&mut s, "commands.list", json!({"pattern": "edit.undo"}));
    assert!(v["commands"][0].get("targets").is_none());
}

/// One preview step of an edit to one glyph of a large font: the time a drag pays per pointer
/// move. Skipped when the corpus is absent.
#[test]
fn a_preview_step_on_a_corpus_font_is_fast() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/oswald/sources/Oswald.glyphs");
    if !path.is_file() {
        eprintln!("corpus/ absent: skipped (cargo xtask corpus --fetch)");
        return;
    }
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": path}));
    let glyphs = s.font().unwrap().glyphs.len();
    s.begin("Drag").unwrap();
    let steps = 50u32;
    let start = std::time::Instant::now();
    for i in 0..steps {
        let x = f64::from(i);
        s.preview("path.addPath", &json!({"glyph": "A", "nodes": rect(x, 0.0, x + 10.0, 10.0)})).unwrap();
    }
    let per_step = start.elapsed() / steps;
    assert!(s.commit().unwrap());
    // A node drag: each pointer move previews `selection.move` of one node from where it began.
    s.begin("Drag").unwrap();
    let start = std::time::Instant::now();
    for i in 0..steps {
        s.preview("selection.move", &json!({"glyph": "A", "points": [[0, 0]], "dx": f64::from(i), "dy": 0})).unwrap();
    }
    let per_drag_step = start.elapsed() / steps;
    assert!(s.commit().unwrap());
    eprintln!("preview step on Oswald ({glyphs} glyphs): add a path {per_step:?}, move a node {per_drag_step:?}");
    if !cfg!(debug_assertions) {
        for step in [per_step, per_drag_step] {
            assert!(step < std::time::Duration::from_millis(16), "a preview step took {step:?}");
        }
    }
}

/// Points a command creates or moves land on the font's grid (spacing ÷ subdivision); an unset
/// spacing counts as 1 and spacing 0 turns rounding off.
#[test]
fn created_and_moved_points_land_on_the_grid() {
    let cases = [
        (None, [(0.0, 0.0), (104.0, 0.0), (104.0, 96.0)], (12.0, -7.0)),
        (Some(10.0), [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)], (10.0, -10.0)),
        (Some(0.0), [(0.4, 0.4), (103.7, 0.4), (103.7, 96.2)], (12.7, -6.2)),
    ];
    for (spacing, created, moved) in cases {
        let mut s = Session::with_document();
        s.edit("grid", |font| {
            Arc::make_mut(&mut font.info).settings.grid_length = spacing;
            Ok(())
        })
        .unwrap();
        run(&mut s, "glyph.newGlyph", json!({"name": "gridTest"}));
        let nodes = json!([{"x": 0.4, "y": 0.4}, {"x": 103.7, "y": 0.4}, {"x": 103.7, "y": 96.2}]);
        run(&mut s, "path.addPath", json!({"glyph": "gridTest", "nodes": nodes}));
        let points = |s: &mut Session| -> Vec<(f64, f64)> {
            let v = run(s, "layer.inspect", json!({"glyph": "gridTest"}));
            v["shapes"][0]["nodes"].as_array().unwrap().iter().map(|n| (n["x"].as_f64().unwrap(), n["y"].as_f64().unwrap())).collect()
        };
        let close = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9;
        let at = points(&mut s);
        assert!(at.iter().zip(created).all(|(a, b)| close(*a, b)), "spacing {spacing:?}: created {at:?}");
        run(&mut s, "selection.move", json!({"glyph": "gridTest", "points": [[0, 0]], "dx": 12.3, "dy": -6.6}));
        let at = points(&mut s);
        assert!(close(at[0], moved), "spacing {spacing:?}: moved {at:?}");
    }
}
