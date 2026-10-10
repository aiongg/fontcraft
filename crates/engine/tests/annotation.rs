//! Annotations (M3.16): `annotation.add`, `update`, `move`, `delete` and `list`, their refusals,
//! a drag as one undo step, and annotations surviving a save in both `.glyphs` versions.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use fontcraft_engine::Session;
use fontcraft_testkit::snapshot;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn run_err(s: &mut Session, id: &str, p: Value) -> String {
    let before = snapshot(s);
    let e = s.execute(id, &p).err().unwrap_or_else(|| panic!("{id} {p} succeeded")).to_string();
    assert_eq!(snapshot(s), before, "{id} {p} failed but changed the session");
    e
}

fn notes(s: &mut Session) -> Vec<Value> {
    run(s, "annotation.list", json!({"glyph": "A"}))["annotations"].as_array().unwrap().clone()
}

fn undo_steps(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

/// `A` with one annotation of each type, added with the defaults.
fn session() -> Session {
    let mut s = Session::with_document();
    for (i, kind) in ["text", "arrow", "circle", "plus", "minus"].into_iter().enumerate() {
        let r = run(&mut s, "annotation.add", json!({"glyph": "A", "type": kind, "position": [10.0 * i as f64, 20.5]}));
        assert_eq!(r["annotation"], i);
        assert_eq!(r["selection"], json!({"glyph": "A", "layer": "m01", "annotations": [i]}));
    }
    s
}

#[test]
fn each_type_is_added_with_its_defaults_and_listed() {
    let mut s = session();
    assert_eq!(undo_steps(&s), 5, "one step per command");
    assert_eq!(
        notes(&mut s),
        [
            json!({"index": 0, "type": "text", "x": 0.0, "y": 20.5, "width": 200.0}),
            json!({"index": 1, "type": "arrow", "x": 10.0, "y": 20.5, "angle": 45.0}),
            json!({"index": 2, "type": "circle", "x": 20.0, "y": 20.5, "diameter": 100.0}),
            json!({"index": 3, "type": "plus", "x": 30.0, "y": 20.5}),
            json!({"index": 4, "type": "minus", "x": 40.0, "y": 20.5}),
        ]
    );
    // Stored as the file format names them, a circle's diameter as its width.
    let layer = s.font().unwrap().glyph("A").unwrap().layer("m01").unwrap();
    let a = &layer.info.annotations;
    assert_eq!((a[0].kind.as_str(), a[2].kind.as_str(), a[2].width), ("Text", "Circle", 100.0));
    // Positions are not rounded to the grid.
    assert_eq!(layer.info.annotations[0].y, 20.5);

    // Values given replace the defaults; the type is read without regard to case.
    run(&mut s, "annotation.add", json!({"glyph": "A", "type": "Text", "position": [1, 2], "text": "too narrow", "width": 120}));
    run(&mut s, "annotation.add", json!({"glyph": "A", "type": "arrow", "position": [1, 2], "angle": -90}));
    run(&mut s, "annotation.add", json!({"glyph": "A", "type": "circle", "position": [1, 2], "diameter": 40}));
    let n = notes(&mut s);
    assert_eq!(n[5], json!({"index": 5, "type": "text", "x": 1.0, "y": 2.0, "text": "too narrow", "width": 120.0}));
    assert_eq!(n[6]["angle"], 270.0, "angles are kept within 0 to 360");
    assert_eq!(n[7]["diameter"], 40.0);

    // The background has its own list.
    let r = run(&mut s, "annotation.add", json!({"glyph": "A", "side": "background", "type": "plus", "position": [5, 5]}));
    assert_eq!(r["selection"], json!({"glyph": "A", "layer": "m01", "side": "background", "annotations": [0]}));
    let bg = run(&mut s, "annotation.list", json!({"glyph": "A", "side": "background"}));
    assert_eq!((bg["side"].as_str(), bg["annotations"].as_array().unwrap().len()), (Some("background"), 1));
    assert_eq!(notes(&mut s).len(), 8);
}

#[test]
fn update_move_and_delete_change_what_they_name() {
    let mut s = session();
    let r =
        run(&mut s, "annotation.update", json!({"glyph": "A", "annotation": 0, "text": "a note\non two lines", "width": 300, "position": [5, 6]}));
    assert_eq!(r["selection"], json!({"glyph": "A", "layer": "m01", "annotations": [0]}));
    run(&mut s, "annotation.update", json!({"glyph": "A", "annotation": 1, "angle": 400}));
    run(&mut s, "annotation.update", json!({"glyph": "A", "annotation": 2, "diameter": 250}));
    let n = notes(&mut s);
    assert_eq!(n[0], json!({"index": 0, "type": "text", "x": 5.0, "y": 6.0, "text": "a note\non two lines", "width": 300.0}));
    assert_eq!((n[1]["angle"].as_f64(), n[2]["diameter"].as_f64()), (Some(40.0), Some(250.0)));
    // An empty text clears it.
    run(&mut s, "annotation.update", json!({"glyph": "A", "annotation": 0, "text": ""}));
    assert!(notes(&mut s)[0].get("text").is_none());

    let r = run(&mut s, "annotation.move", json!({"glyph": "A", "annotations": [1, 3, 1], "dx": 10, "dy": -100}));
    assert_eq!((r["moved"].as_u64(), &r["selection"]["annotations"]), (Some(2), &json!([1, 3])));
    let n = notes(&mut s);
    assert_eq!((n[1]["x"].as_f64(), n[1]["y"].as_f64()), (Some(20.0), Some(-79.5)));
    assert_eq!((n[3]["x"].as_f64(), n[3]["y"].as_f64()), (Some(40.0), Some(-79.5)));
    assert_eq!(n[2]["x"], 20.0, "the others stay");

    let steps = undo_steps(&s);
    let r = run(&mut s, "annotation.delete", json!({"glyph": "A", "annotations": [3, 0, 4]}));
    assert_eq!((r["removed"].as_u64(), &r["selection"]), (Some(3), &json!({"glyph": "A", "layer": "m01"})));
    assert_eq!(notes(&mut s).iter().map(|n| n["type"].as_str().unwrap()).collect::<Vec<_>>(), ["arrow", "circle"]);
    assert_eq!(undo_steps(&s), steps + 1);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(notes(&mut s).len(), 5);
}

#[test]
fn bad_values_wrong_types_and_bad_addresses_change_nothing() {
    let mut s = session();
    let cases = [
        ("annotation.add", json!({"glyph": "A", "type": "star", "position": [0, 0]}), "`type` must be"),
        ("annotation.add", json!({"glyph": "A", "type": "text"}), "`position` is required"),
        ("annotation.add", json!({"glyph": "A", "type": "text", "position": [0]}), "`position` must be"),
        ("annotation.add", json!({"glyph": "A", "type": "text", "position": [1e7, 0]}), "`position` must be"),
        ("annotation.add", json!({"glyph": "A", "type": "text", "position": ["a", 0]}), "`position` must be"),
        ("annotation.add", json!({"glyph": "A", "type": "arrow", "position": [0, 0], "width": 10}), "`width` applies to text"),
        ("annotation.add", json!({"glyph": "A", "type": "text", "position": [0, 0], "angle": 10}), "`angle` applies to arrow"),
        ("annotation.add", json!({"glyph": "A", "type": "plus", "position": [0, 0], "text": "x"}), "`text` applies to text"),
        ("annotation.add", json!({"glyph": "A", "type": "text", "position": [0, 0], "diameter": 10}), "`diameter` applies to circle"),
        ("annotation.add", json!({"glyph": "A", "type": "circle", "position": [0, 0], "diameter": 0}), "more than 0"),
        ("annotation.add", json!({"glyph": "A", "type": "text", "position": [0, 0], "width": -5}), "more than 0"),
        ("annotation.add", json!({"glyph": "A", "type": "text", "position": [0, 0], "text": "x".repeat(70_000)}), "bytes"),
        ("annotation.add", json!({"glyph": "A", "type": "text", "position": [0, 0], "shapes": [0]}), "takes no target lists"),
        ("annotation.add", json!({"glyph": "nope", "type": "text", "position": [0, 0]}), "no glyph `nope`"),
        ("annotation.add", json!({"glyph": "A", "layer": "nope", "type": "text", "position": [0, 0]}), "no layer `nope`"),
        ("annotation.update", json!({"glyph": "A", "annotation": 0}), "at least one"),
        ("annotation.update", json!({"glyph": "A", "annotation": 5, "position": [0, 0]}), "5 does not resolve"),
        ("annotation.update", json!({"glyph": "A", "annotation": -1, "position": [0, 0]}), "`annotation` must be"),
        ("annotation.update", json!({"glyph": "A", "annotation": 1, "text": "x"}), "`text` applies to text"),
        ("annotation.update", json!({"glyph": "A", "annotation": 0, "angle": 5}), "`angle` applies to arrow"),
        ("annotation.update", json!({"glyph": "A", "annotations": [0], "position": [0, 0]}), "`annotation` is required"),
        ("annotation.move", json!({"glyph": "A", "annotations": [], "dx": 1, "dy": 0}), "must not be empty"),
        ("annotation.move", json!({"glyph": "A", "annotations": [0, 9], "dx": 1, "dy": 0}), "9 does not resolve"),
        ("annotation.move", json!({"glyph": "A", "annotations": [0], "dx": 2e6, "dy": 0}), "beyond"),
        ("annotation.move", json!({"glyph": "A", "annotations": [0], "dx": 1}), "`dy` is required"),
        ("annotation.move", json!({"glyph": "A", "annotations": [0], "points": [[0, 0]], "dx": 1, "dy": 1}), "annotations only"),
        ("annotation.delete", json!({"glyph": "A"}), "must not be empty"),
        ("annotation.delete", json!({"glyph": "A", "annotations": [0, 5]}), "5 does not resolve"),
        ("annotation.list", json!({"glyph": "A", "annotations": [0]}), "takes no target lists"),
    ];
    for (id, params, message) in cases {
        let e = run_err(&mut s, id, params.clone());
        assert!(e.contains(message), "{id} {params}: {e}");
    }
}

#[test]
fn a_locked_glyph_refuses_annotation_edits() {
    let mut s = session();
    s.edit("Lock", |font| {
        font.glyph_mut("A")?.info.locked = true;
        Ok(())
    })
    .unwrap();
    for (id, params) in [
        ("annotation.add", json!({"glyph": "A", "type": "plus", "position": [0, 0]})),
        ("annotation.update", json!({"glyph": "A", "annotation": 0, "position": [0, 0]})),
        ("annotation.move", json!({"glyph": "A", "annotations": [0], "dx": 1, "dy": 1})),
        ("annotation.delete", json!({"glyph": "A", "annotations": [0]})),
    ] {
        let e = run_err(&mut s, id, params);
        assert!(e.contains("glyph `A` is locked"), "{id}: {e}");
    }
    assert_eq!(notes(&mut s).len(), 5, "listing still works");
}

#[test]
fn rotating_an_arrow_by_drag_is_one_undo_step() {
    let mut s = session();
    let steps = undo_steps(&s);
    s.begin("Rotate Annotation").unwrap();
    for angle in 0..90 {
        s.preview("annotation.update", &json!({"glyph": "A", "annotation": 1, "angle": angle})).unwrap();
    }
    assert!(s.commit().unwrap());
    assert_eq!(undo_steps(&s), steps + 1);
    assert_eq!(notes(&mut s)[1]["angle"], 89.0);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(notes(&mut s)[1]["angle"], 45.0);
}

#[test]
fn annotations_survive_saving_in_both_glyphs_versions() {
    let dir = std::env::temp_dir().join(format!("fontcraft-annotations-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = session();
    run(&mut s, "annotation.update", json!({"glyph": "A", "annotation": 0, "text": "the bar \"is\" short", "width": 150.25}));
    run(&mut s, "annotation.update", json!({"glyph": "A", "annotation": 1, "angle": 312.5}));
    run(&mut s, "annotation.add", json!({"glyph": "A", "side": "background", "type": "circle", "position": [-3.5, 700]}));
    let list = |s: &mut Session, side: &str| run(s, "annotation.list", json!({"glyph": "A", "side": side}))["annotations"].clone();
    let (fg, bg) = (list(&mut s, "foreground"), list(&mut s, "background"));
    for version in [2, 3] {
        let path = dir.join(format!("Notes{version}.glyphs"));
        run(&mut s, "file.saveAs", json!({"path": path, "version": version}));
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": path}));
        assert_eq!(list(&mut back, "foreground"), fg, "version {version}");
        assert_eq!(list(&mut back, "background"), bg, "version {version}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
