//! Stored selections (M13.32): `edit.storeSelection`, `edit.removeStoredSelection` and
//! `edit.getStoredSelections`, kept in the glyph's user data, mapped onto other masters by index,
//! and kept by `.glyphs` files.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::model::fc_keys::STORED_SELECTIONS;
use fontcraft_engine::model::{Layer, Path, Shape};
use fontcraft_engine::{EngineError, Session};
use fontcraft_testkit::{snapshot, temp_dir};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn refused(s: &mut Session, id: &str, p: Value) -> EngineError {
    let before = snapshot(s);
    let e = s.execute(id, &p).err().unwrap_or_else(|| panic!("{id} {p} should fail"));
    assert_eq!(snapshot(s), before, "{id} failed but changed the session");
    e
}

/// Glyph A: a square (four nodes) and an anchor `top` in `m01`; in `m02` a rectangle with the same
/// structure and no anchor.
fn two_masters() -> Session {
    let mut s = Session::with_document();
    s.edit("setup", |font| {
        let first = font.default_master()?.id.clone();
        let mut m = font.default_master()?.clone();
        m.id = "m02".into();
        m.name = "Bold".into();
        Arc::make_mut(&mut font.masters).push(m);
        let mut g = font.glyph_mut("A")?;
        let l = g.layers.iter_mut().find(|l| l.id == first).unwrap();
        l.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
        let mut wide = Layer::new("m02", 600.0);
        wide.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 300.0, 100.0)));
        g.layers.push(wide);
        Ok(())
    })
    .unwrap();
    run(&mut s, "anchor.add", json!({"glyph": "A", "name": "top", "x": 50, "y": 100}));
    s
}

fn get(s: &mut Session, layer: &str) -> Vec<Value> {
    run(s, "edit.getStoredSelections", json!({"glyph": "A", "layer": layer}))["selections"].as_array().unwrap().clone()
}

#[test]
fn a_stored_selection_maps_onto_another_master_by_index() {
    let mut s = two_masters();
    let r = run(&mut s, "edit.storeSelection", json!({"glyph": "A", "name": "stem", "points": [[0, 0], [0, 1]], "anchors": ["top"]}));
    assert_eq!((r["replaced"].as_bool(), r["count"].as_u64()), (Some(false), Some(1)), "{r}");

    let here = &get(&mut s, "m01")[0];
    assert_eq!((&here["points"], &here["anchors"], &here["missing"]), (&json!([[0, 0], [0, 1]]), &json!(["top"]), &json!(0)));
    // The second master has the same nodes but no anchor: the nodes map, the anchor is missing.
    let there = &get(&mut s, "m02")[0];
    assert_eq!((&there["points"], &there["anchors"], &there["missing"]), (&json!([[0, 0], [0, 1]]), &json!([]), &json!(1)));

    // Storing under the same name replaces the entry; one undo step brings the old one back.
    let r = run(&mut s, "edit.storeSelection", json!({"glyph": "A", "name": "stem", "points": [[0, 3]]}));
    assert_eq!((r["replaced"].as_bool(), r["count"].as_u64()), (Some(true), Some(1)));
    assert_eq!(get(&mut s, "m01")[0]["points"], json!([[0, 3]]));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(get(&mut s, "m01")[0]["points"], json!([[0, 0], [0, 1]]));

    // Refusals change nothing.
    refused(&mut s, "edit.storeSelection", json!({"glyph": "A", "name": "x"}));
    refused(&mut s, "edit.storeSelection", json!({"glyph": "A", "name": "x", "points": [[0, 9]]}));
    refused(&mut s, "edit.storeSelection", json!({"glyph": "A", "name": "", "points": [[0, 0]]}));
    refused(&mut s, "edit.storeSelection", json!({"glyph": "A", "name": "x", "anchors": ["nope"]}));
    refused(&mut s, "edit.storeSelection", json!({"glyph": "nope", "name": "x", "points": [[0, 0]]}));
    refused(&mut s, "edit.removeStoredSelection", json!({"glyph": "A", "name": "nope"}));
    refused(&mut s, "edit.getStoredSelections", json!({"glyph": "A", "name": "nope"}));

    // Removing the last entry removes the user-data key.
    run(&mut s, "edit.removeStoredSelection", json!({"glyph": "A", "name": "stem"}));
    assert!(s.font().unwrap().glyph("A").unwrap().info.user_data.get(STORED_SELECTIONS).is_none());
}

#[test]
fn a_glyph_keeps_at_most_256_stored_selections() {
    let mut s = two_masters();
    for i in 0..256 {
        run(&mut s, "edit.storeSelection", json!({"glyph": "A", "name": format!("s{i}"), "points": [[0, 0]]}));
    }
    refused(&mut s, "edit.storeSelection", json!({"glyph": "A", "name": "one more", "points": [[0, 0]]}));
    // Replacing an existing entry still works at the limit.
    run(&mut s, "edit.storeSelection", json!({"glyph": "A", "name": "s0", "points": [[0, 1]]}));
}

#[test]
fn stored_selections_survive_saving_to_glyphs() {
    let mut s = two_masters();
    run(&mut s, "edit.storeSelection", json!({"glyph": "A", "name": "corner", "points": [[0, 2]], "shapes": [0], "anchors": ["top"]}));
    run(&mut s, "edit.storeSelection", json!({"glyph": "A", "name": "base", "points": [[0, 0], [0, 1]]}));
    let dir = temp_dir("stored-selections");
    let path = dir.join("Stored.glyphs");
    run(&mut s, "file.saveAs", json!({ "path": path }));

    let mut opened = Session::new();
    run(&mut opened, "file.open", json!({ "path": path }));
    let first = opened.font().unwrap().default_master().unwrap().id.clone();
    let list = get(&mut opened, &first);
    let names: Vec<&str> = list.iter().map(|e| e["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["corner", "base"], "both entries, in stored order");
    assert_eq!((&list[0]["points"], &list[0]["shapes"], &list[0]["anchors"]), (&json!([[0, 2]]), &json!([0]), &json!(["top"])));
    assert_eq!(list[1]["points"], json!([[0, 0], [0, 1]]));
    let _ = std::fs::remove_dir_all(&dir);
}
