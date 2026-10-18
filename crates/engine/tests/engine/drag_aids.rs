//! Servant nodes and the Power Nudge switch (M13.33): `path.setServantNode` and
//! `path.togglePowerNudge`. The drags that use them are tested in the tools crate and the shell.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use fontcraft_engine::model::fc_keys::SERVANT;
use fontcraft_engine::model::{Node, Shape};
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

/// Glyph A with one open curve: (0,0), handles (0,100) (50,200), (100,200) smooth, handles
/// (150,200) (200,250), (200,300).
fn curve() -> Session {
    let mut s = Session::with_document();
    let nodes = json!([
        {"x": 0, "y": 0},
        {"x": 0, "y": 100, "type": "offcurve"}, {"x": 50, "y": 200, "type": "offcurve"}, {"x": 100, "y": 200, "type": "curve", "smooth": true},
        {"x": 150, "y": 200, "type": "offcurve"}, {"x": 200, "y": 250, "type": "offcurve"}, {"x": 200, "y": 300, "type": "curve"},
    ]);
    run(&mut s, "path.addPath", json!({"glyph": "A", "closed": false, "nodes": nodes}));
    s
}

fn node(s: &Session, index: usize) -> Node {
    let font = s.font().unwrap();
    let master = font.default_master().unwrap().id.clone();
    match &font.glyph("A").unwrap().layer(&master).unwrap().shapes[0] {
        Shape::Path(p) => p.nodes[index].clone(),
        other => panic!("not a path: {other:?}"),
    }
}

fn axis(n: &Node) -> Option<String> {
    n.info.user_data.get(SERVANT)?.as_dict()?.get("axis")?.as_str().map(str::to_string)
}

#[test]
fn servant_nodes_are_declared_cleared_and_kept_by_glyphs_files() {
    let mut s = curve();
    let r = run(&mut s, "path.setServantNode", json!({"glyph": "A", "points": [[0, 3], [0, 6]], "axis": "x"}));
    assert_eq!(r["servants"], json!(2));
    assert_eq!(r["selection"]["points"], json!([[0, 3], [0, 6]]));
    assert_eq!((axis(&node(&s, 3)).as_deref(), axis(&node(&s, 6)).as_deref()), (Some("x"), Some("x")));
    assert_eq!(axis(&node(&s, 0)), None);

    // The same declaration again changes nothing; another axis replaces it.
    assert_eq!(run(&mut s, "path.setServantNode", json!({"glyph": "A", "points": [[0, 3]], "axis": "x"}))["servants"], json!(0));
    run(&mut s, "path.setServantNode", json!({"glyph": "A", "points": [[0, 3]], "axis": "y"}));
    assert_eq!(axis(&node(&s, 3)).as_deref(), Some("y"));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(axis(&node(&s, 3)).as_deref(), Some("x"), "one undo step per declaration");

    // A .glyphs file keeps the link in the node's user data.
    let dir = temp_dir("servant-nodes");
    let path = dir.join("Servants.glyphs");
    run(&mut s, "file.saveAs", json!({ "path": path }));
    let mut opened = Session::new();
    run(&mut opened, "file.open", json!({ "path": path }));
    assert_eq!(axis(&node(&opened, 3)).as_deref(), Some("x"));
    let _ = std::fs::remove_dir_all(&dir);

    // none clears it, and leaves no empty user data behind.
    run(&mut s, "path.setServantNode", json!({"glyph": "A", "points": [[0, 3], [0, 6]], "axis": "none"}));
    assert!(node(&s, 3).info.user_data.is_empty() && node(&s, 6).info.user_data.is_empty());
}

#[test]
fn set_servant_node_refuses_bad_targets_without_changing_anything() {
    let mut s = curve();
    for (params, needle) in [
        (json!({"glyph": "A", "points": [[0, 1]], "axis": "x"}), "handle"),
        (json!({"glyph": "A", "points": [[0, 3], [0, 99]], "axis": "x"}), "99"),
        (json!({"glyph": "A", "points": [[0, 3]], "axis": "diagonal"}), "axis"),
        (json!({"glyph": "A", "points": [], "axis": "x"}), "points"),
        (json!({"glyph": "A", "shapes": [0], "axis": "x"}), "points"),
    ] {
        let e = refused(&mut s, "path.setServantNode", params.clone());
        assert!(e.to_string().contains(needle), "{params}: {e}");
    }
    run(&mut s, "glyph.toggleLocked", json!({"glyphs": ["A"], "locked": true}));
    refused(&mut s, "path.setServantNode", json!({"glyph": "A", "points": [[0, 3]], "axis": "x"}));
}

#[test]
fn power_nudge_switches_and_lasts_in_the_settings() {
    let mut s = Session::new();
    assert_eq!(run(&mut s, "settings.get", json!({"id": "editing.powerNudge"}))["value"], json!(false));
    assert_eq!(run(&mut s, "path.togglePowerNudge", json!({}))["on"], json!(true));
    assert_eq!(s.settings().get_bool("editing.powerNudge"), Some(true));
    assert_eq!(run(&mut s, "path.togglePowerNudge", json!({}))["on"], json!(false));
    assert_eq!(run(&mut s, "path.togglePowerNudge", json!({"on": true}))["on"], json!(true));
    assert_eq!(run(&mut s, "path.togglePowerNudge", json!({"on": true}))["on"], json!(true), "on: true keeps it on");
    refused(&mut s, "path.togglePowerNudge", json!({"on": "yes"}));
    assert_eq!(run(&mut s, "settings.get", json!({"id": "editing.leverRatio"}))["value"], json!(6.0));
    run(&mut s, "settings.set", json!({"id": "editing.leverRatio", "value": 4}));
    assert!(s.execute("settings.set", &json!({"id": "editing.leverRatio", "value": 0.5})).is_err(), "the ratio is at least 1");
}
