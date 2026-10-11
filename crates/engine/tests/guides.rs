//! Guides, measurement and the Dimensions notepad (M3.9): the guide commands on local and global
//! guides, the ruler across contours and overlaps, point coordinates with italic deltas, the lines
//! Edit View draws from font info, and Dimensions values per master.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::model::plist::Value as PValue;
use fontcraft_engine::model::{Shape, metric};
use fontcraft_engine::{EngineError, Session};
use fontcraft_testkit::{rect, snapshot};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// Runs a command that must fail, and checks that it changed nothing.
fn refused(s: &mut Session, id: &str, p: Value) -> EngineError {
    let before = snapshot(s);
    let e = s.execute(id, &p).err().unwrap_or_else(|| panic!("{id} {p} should fail"));
    assert_eq!(snapshot(s), before, "{id} failed but changed the session");
    e
}

fn close(a: &Value, b: f64) -> bool {
    a.as_f64().is_some_and(|a| (a - b).abs() < 1e-6)
}

fn guides(s: &mut Session, glyph: &str) -> Vec<Value> {
    run(s, "guide.list", json!({"glyph": glyph}))["guides"].as_array().unwrap().clone()
}

fn count(list: &[Value], scope: &str) -> usize {
    list.iter().filter(|g| g["scope"] == scope).count()
}

/// `A` holds the rectangle 0,0–100,100 (shape 0).
fn session() -> Session {
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(0.0, 0.0, 100.0, 100.0)}));
    s
}

#[test]
fn guides_are_added_changed_moved_and_removed() {
    let mut s = session();
    // Local: the position is rounded to the grid (spacing 1), the angle kept from 0 to 360.
    let out = run(&mut s, "guide.add", json!({"glyph": "A", "x": 10.4, "y": 500.6, "angle": -90}));
    assert_eq!(out["guide"], json!({"scope": "local", "index": 0}));
    assert_eq!(out["selection"]["guides"], json!([{"scope": "local", "index": 0}]));
    let g = guides(&mut s, "A")[0].clone();
    assert_eq!((g["x"].as_f64(), g["y"].as_f64(), g["angle"].as_f64()), (Some(10.0), Some(501.0), Some(270.0)));
    // Through two nodes: the diagonal of the rectangle.
    run(&mut s, "guide.add", json!({"glyph": "A", "throughPoints": [[0, 0], [0, 2]]}));
    let g = guides(&mut s, "A")[1].clone();
    assert!(close(&g["angle"], 45.0) && close(&g["x"], 0.0), "{g}");
    refused(&mut s, "guide.add", json!({"glyph": "A", "throughPoints": [[0, 0], [0, 0]]}));
    refused(&mut s, "guide.add", json!({"glyph": "A", "throughPoints": [[0, 0], [0, 2]], "y": 3}));
    refused(&mut s, "guide.add", json!({"glyph": "A", "throughPoints": [[0, 0], [5, 0]]}));

    // Global: on the master, seen from every glyph of it; undoable like any edit.
    let out = run(&mut s, "guide.addGlobal", json!({"y": 700}));
    assert_eq!((out["master"].as_str(), &out["guide"]), (Some("m01"), &json!({"scope": "global", "index": 0})));
    assert_eq!(count(&guides(&mut s, "B"), "global"), 1);
    let out = run(&mut s, "guide.addGlobal", json!({"glyph": "A", "x": 5}));
    assert_eq!(out["selection"]["guides"], json!([{"scope": "global", "index": 1}]));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(count(&guides(&mut s, "A"), "global"), 1);

    // Local → global → local, keeping the properties.
    run(&mut s, "guide.setProperties", json!({"glyph": "A", "guide": {"scope": "local", "index": 0}, "name": "stem", "relativeTo": "both"}));
    let out = run(&mut s, "guide.makeGlobal", json!({"glyph": "A", "guides": [{"scope": "local", "index": 0}]}));
    assert_eq!(out["selection"]["guides"], json!([{"scope": "global", "index": 1}]));
    let list = guides(&mut s, "A");
    assert_eq!((count(&list, "local"), count(&list, "global")), (1, 2));
    let moved = list.iter().find(|g| g["scope"] == "global" && g["index"] == 1).unwrap();
    assert_eq!((moved["name"].as_str(), moved["relativeTo"].as_str()), (Some("stem"), Some("both")));
    assert_eq!(count(&guides(&mut s, "B"), "global"), 2, "a global guide shows in every glyph");
    refused(&mut s, "guide.makeGlobal", json!({"glyph": "A", "guides": [{"scope": "global", "index": 0}]}));
    let out = run(&mut s, "guide.makeLocal", json!({"glyph": "A", "guides": [{"scope": "global", "index": 1}]}));
    assert_eq!(out["selection"]["guides"], json!([{"scope": "local", "index": 1}]));

    // Properties: grid rounding, clearing with an empty string, a filter only on a global guide.
    let local0 = json!({"scope": "local", "index": 0});
    run(&mut s, "guide.setProperties", json!({"glyph": "A", "guide": local0, "x": 33.3, "measurement": true, "color": "1,0,0,1"}));
    let g = guides(&mut s, "A")[0].clone();
    assert_eq!((g["x"].as_f64(), g["measurement"].as_bool(), g["color"].as_str()), (Some(33.0), Some(true), Some("1,0,0,1")));
    run(&mut s, "guide.setProperties", json!({"glyph": "A", "guide": {"scope": "local", "index": 1}, "name": ""}));
    assert!(guides(&mut s, "A")[1].get("name").is_none());
    refused(&mut s, "guide.setProperties", json!({"glyph": "A", "guide": local0, "filter": "name == 'A'"}));
    refused(&mut s, "guide.setProperties", json!({"glyph": "A", "guide": local0, "color": "red"}));
    refused(&mut s, "guide.setProperties", json!({"glyph": "A", "guide": local0, "relativeTo": "middle"}));
    run(&mut s, "guide.setProperties", json!({"glyph": "A", "guide": {"scope": "global", "index": 0}, "filter": "case == upper"}));
    assert_eq!(guides(&mut s, "A").iter().find(|g| g["scope"] == "global").unwrap()["filter"], "case == upper");

    // The angle lock refuses a new angle unless the same call opens it.
    run(&mut s, "guide.setProperties", json!({"glyph": "A", "guide": local0, "angleLocked": true}));
    refused(&mut s, "guide.setProperties", json!({"glyph": "A", "guide": local0, "angle": 10}));
    run(&mut s, "guide.setProperties", json!({"glyph": "A", "guide": local0, "angle": 10, "angleLocked": false}));
    assert!(close(&guides(&mut s, "A")[0]["angle"], 10.0));

    // Measurement toggles without `on`.
    assert_eq!(run(&mut s, "guide.toggleMeasurement", json!({"glyph": "A", "guide": local0}))["measurement"], false);
    assert_eq!(run(&mut s, "guide.toggleMeasurement", json!({"glyph": "A", "guide": local0}))["measurement"], true);

    // A locked guide refuses changes until it is unlocked.
    let both = json!([{"scope": "local", "index": 0}, {"scope": "global", "index": 0}]);
    assert_eq!(run(&mut s, "guide.lock", json!({"glyph": "A", "guides": both}))["guides"], 2);
    assert!(guides(&mut s, "A").iter().all(|g| g["index"] != 0 || g["locked"] == true));
    refused(&mut s, "guide.remove", json!({"glyph": "A", "guides": both}));
    refused(&mut s, "guide.setProperties", json!({"glyph": "A", "guide": local0, "x": 1}));
    refused(&mut s, "guide.toggleMeasurement", json!({"glyph": "A", "guide": local0}));
    refused(&mut s, "guide.makeGlobal", json!({"glyph": "A", "guides": [local0]}));
    run(&mut s, "guide.unlock", json!({"glyph": "A", "guides": both}));

    // Remove local and global guides in one step.
    let before = guides(&mut s, "A").len();
    let out = run(&mut s, "guide.remove", json!({"glyph": "A", "guides": both}));
    assert_eq!(out["removed"], 2);
    assert_eq!(guides(&mut s, "A").len(), before - 2);
    refused(&mut s, "guide.remove", json!({"glyph": "A", "guides": [{"scope": "local", "index": 99}]}));
}

#[test]
fn a_locked_glyph_keeps_its_local_guides_but_not_the_masters() {
    let mut s = session();
    run(&mut s, "guide.add", json!({"glyph": "A", "y": 10}));
    s.edit("lock", |font| {
        font.glyph_mut("A")?.info.locked = true;
        Ok(())
    })
    .unwrap();
    assert!(matches!(refused(&mut s, "guide.add", json!({"glyph": "A", "y": 20})), EngineError::Locked(_)));
    refused(&mut s, "guide.remove", json!({"glyph": "A", "guides": [{"scope": "local", "index": 0}]}));
    refused(&mut s, "guide.makeGlobal", json!({"glyph": "A", "guides": [{"scope": "local", "index": 0}]}));
    // Global guides belong to the master.
    run(&mut s, "guide.addGlobal", json!({"glyph": "A", "y": 300}));
    run(&mut s, "guide.setProperties", json!({"glyph": "A", "guide": {"scope": "global", "index": 0}, "y": 310}));
    refused(&mut s, "guide.makeLocal", json!({"glyph": "A", "guides": [{"scope": "global", "index": 0}]}));
}

#[test]
fn the_ruler_crosses_two_contours_and_merges_overlaps() {
    let mut s = Session::with_document();
    // `.notdef` is an outer rectangle 50,0–450,700 with a counter 100,50–400,650.
    let out = run(&mut s, "measure.ruler", json!({"glyph": ".notdef", "from": [0, 350], "to": [500, 350]}));
    let xs: Vec<f64> = out["crossings"].as_array().unwrap().iter().map(|c| c["x"].as_f64().unwrap()).collect();
    assert_eq!(xs.len(), 4, "{out}");
    for (x, want) in xs.iter().zip([50.0, 100.0, 400.0, 450.0]) {
        assert!((x - want).abs() < 1e-6, "{out}");
    }
    let gaps = out["gaps"].as_array().unwrap();
    let distances: Vec<f64> = gaps.iter().map(|g| g["distance"].as_f64().unwrap()).collect();
    assert!(distances.iter().zip([50.0, 300.0, 50.0]).all(|(d, w)| (d - w).abs() < 1e-6), "{out}");
    assert_eq!(gaps.iter().map(|g| g["inside"].as_bool().unwrap()).collect::<Vec<_>>(), [true, false, true]);
    assert!(close(&out["length"], 500.0) && close(&out["angle"], 270.0), "pointing right is 270: {out}");
    let up = run(&mut s, "measure.ruler", json!({"glyph": ".notdef", "from": [250, -100], "to": [250, 800]}));
    assert!(close(&up["angle"], 0.0) && up["crossings"].as_array().unwrap().len() == 4, "{up}");
    // A ruler that ends inside the outline sees only the crossings on it.
    let short = run(&mut s, "measure.ruler", json!({"glyph": ".notdef", "from": [75, 350], "to": [500, 350]}));
    assert_eq!(short["crossings"].as_array().unwrap().len(), 3, "{short}");

    // Two overlapping rectangles measure as one; a line through a corner only touches it.
    run(&mut s, "path.addPath", json!({"glyph": "B", "nodes": rect(0.0, 0.0, 100.0, 100.0)}));
    run(&mut s, "path.addPath", json!({"glyph": "B", "nodes": rect(50.0, 0.0, 150.0, 100.0)}));
    run(&mut s, "path.addPath", json!({"glyph": "B", "nodes": [{"x": 300, "y": 0}, {"x": 400, "y": 0}, {"x": 350, "y": 50}]}));
    let out = run(&mut s, "measure.ruler", json!({"glyph": "B", "from": [-10, 50], "to": [500, 50]}));
    let xs: Vec<f64> = out["crossings"].as_array().unwrap().iter().map(|c| c["x"].as_f64().unwrap()).collect();
    assert!(xs.len() == 2 && (xs[0]).abs() < 1e-6 && (xs[1] - 150.0).abs() < 1e-6, "{out}");

    // A measurement guide measures along its whole line; components are measured like paths.
    run(&mut s, "path.addPath", json!({"glyph": "C", "nodes": rect(0.0, 0.0, 100.0, 100.0)}));
    s.edit("component", |font| {
        let mut g = font.glyph_mut("D")?;
        // Alignment off keeps the component where it is drawn after the next command.
        let mut c = fontcraft_engine::model::Component::new("C", 200.0, 0.0);
        c.info = fontcraft_engine::model::ComponentInfoBox::new(fontcraft_engine::model::ComponentInfo {
            alignment: fontcraft_engine::model::Alignment::Disabled,
            ..Default::default()
        });
        g.layers[0].shapes.push(Shape::Component(c));
        Ok(())
    })
    .unwrap();
    run(&mut s, "guide.add", json!({"glyph": "D", "y": 50}));
    let out = run(&mut s, "measure.ruler", json!({"glyph": "D", "guide": {"scope": "local", "index": 0}}));
    let xs: Vec<f64> = out["crossings"].as_array().unwrap().iter().map(|c| c["x"].as_f64().unwrap()).collect();
    assert!(xs.len() == 2 && (xs[0] - 200.0).abs() < 1e-6 && (xs[1] - 300.0).abs() < 1e-6, "{out}");
    assert!(out.get("angle").is_none());
    refused(&mut s, "measure.ruler", json!({"glyph": "D", "from": [1, 1], "to": [1, 1]}));
    refused(&mut s, "measure.ruler", json!({"glyph": "D", "from": [1, 1]}));
}

#[test]
fn coordinates_give_deltas_relative_to_the_italic_angle() {
    let mut s = Session::with_document();
    s.edit("italic", |font| {
        let master = font.default_master()?.id.clone();
        font.set_metric_pos(&master, metric::ITALIC_ANGLE, 10.0)?;
        // No grid, so the slanted point keeps its exact position.
        Arc::make_mut(&mut font.info).settings.grid_length = Some(0.0);
        Ok(())
    })
    .unwrap();
    let lean = 100.0 * 10f64.to_radians().tan();
    run(&mut s, "path.addPath", json!({"glyph": "A", "closed": false, "nodes": [{"x": 0, "y": 0}, {"x": lean, "y": 100}]}));
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(200.0, 0.0, 300.0, 100.0)}));
    let out = run(&mut s, "measure.coordinates", json!({"glyph": "A"}));
    assert_eq!(out["points"].as_array().unwrap().len(), 6);
    let deltas = out["deltas"].as_array().unwrap();
    assert_eq!(deltas.len(), 5, "one for the open path, four for the closed one: {out}");
    assert!(close(&deltas[0]["italicDx"], 0.0) && close(&deltas[0]["dx"], lean), "a step along the italic angle: {out}");
    assert_eq!((&deltas[4]["from"], &deltas[4]["to"]), (&json!(3), &json!(0)), "the closing step");
}

#[test]
fn display_lines_follow_font_info_and_parameters() {
    let mut s = Session::with_document();
    s.edit("setup", |font| {
        let info = Arc::make_mut(&mut font.info);
        info.settings.grid_length = Some(10.0);
        info.settings.grid_sub_division = Some(2.0);
        let m = Arc::make_mut(&mut font.masters).first_mut().unwrap();
        m.params.set("ascender", PValue::String("greek: 705".into()));
        m.params.set("CJK Grid", PValue::Number(fontcraft_engine::model::plist::Number::int(4)));
        m.params.set("CJK Guide", PValue::String("han:10".into()));
        font.glyph_mut("A")?.info.script = Some("greek".into());
        font.glyph_mut("B")?.info.script = Some("han".into());
        Ok(())
    })
    .unwrap();
    let a = run(&mut s, "guide.displayLines", json!({"glyph": "A"}));
    assert_eq!(a["grid"]["step"], 5.0);
    let asc = a["metrics"].as_array().unwrap().iter().find(|m| m["kind"] == metric::ASCENDER).unwrap();
    assert_eq!((asc["y"].as_f64(), asc["source"].as_str()), (Some(705.0), Some("parameter")));
    assert_eq!(a["cjk"], json!({"box": [0.0, -200.0, 600.0, 705.0], "rows": 4.0, "columns": 4.0}));
    let b = run(&mut s, "guide.displayLines", json!({"glyph": "B"}));
    assert_eq!(b["cjk"]["guide"], json!([60.0, -100.0, 540.0, 700.0]));
    assert!(b["metrics"].as_array().unwrap().iter().all(|m| m["source"] == "master"));
    // The new guide snaps to the 5-unit grid.
    run(&mut s, "guide.add", json!({"glyph": "A", "x": 12, "y": 13}));
    let g = guides(&mut s, "A")[0].clone();
    assert_eq!((g["x"].as_f64(), g["y"].as_f64()), (Some(10.0), Some(15.0)));
}

#[test]
fn dimensions_are_noted_per_master() {
    let mut s = Session::with_document();
    run(&mut s, "dimensions.setValue", json!({"key": "nV", "value": 90}));
    run(&mut s, "dimensions.setValue", json!({"key": "arBar", "value": 50.5}));
    let all = run(&mut s, "dimensions.getValues", json!({}));
    assert_eq!(all["values"], json!({"nV": 90.0, "arBar": 50.5}));
    let latin = run(&mut s, "dimensions.getValues", json!({"script": "latin"}));
    assert_eq!(latin["values"], json!({"nV": 90.0}));
    assert!(latin["fields"].as_array().unwrap().contains(&json!("HH")));
    refused(&mut s, "dimensions.getValues", json!({"script": "nowhere"}));
    refused(&mut s, "dimensions.setValue", json!({"master": "nope", "key": "nV", "value": 1}));
    refused(&mut s, "dimensions.setValue", json!({"key": "", "value": 1}));

    // A field kept as text stays text; clearing every field removes the user-data key.
    s.edit("text", |font| {
        let mut d = fontcraft_engine::model::plist::Dict::new();
        let mut m = fontcraft_engine::model::plist::Dict::new();
        m.insert("HH", PValue::String("80".into()));
        d.insert("m01", PValue::Dict(m));
        Arc::make_mut(&mut font.info).user_data.0.insert("GSDimensionPlugin.Dimensions", PValue::Dict(d));
        Ok(())
    })
    .unwrap();
    run(&mut s, "dimensions.setValue", json!({"key": "HH", "value": 85}));
    let stored = |s: &Session| s.font().unwrap().info.user_data.get("GSDimensionPlugin.Dimensions").cloned();
    let Some(PValue::Dict(d)) = stored(&s) else { panic!("no dimensions") };
    assert!(matches!(d.get("m01").and_then(|m| m.as_dict()).and_then(|m| m.get("HH")), Some(PValue::String(t)) if t.as_str() == "85"));
    run(&mut s, "dimensions.setValue", json!({"key": "HH"}));
    assert!(stored(&s).is_none());
    run(&mut s, "edit.undo", json!({}));
    assert!(stored(&s).is_some(), "undo brings the value back");
}

#[test]
fn background_guides_are_their_own() {
    let mut s = session();
    run(&mut s, "path.addPath", json!({"glyph": "A", "side": "background", "nodes": rect(0.0, 0.0, 10.0, 10.0)}));
    run(&mut s, "guide.add", json!({"glyph": "A", "side": "background", "y": 5}));
    assert_eq!(count(&guides(&mut s, "A"), "local"), 0);
    let bg = run(&mut s, "guide.list", json!({"glyph": "A", "side": "background"}));
    assert_eq!(bg["guides"].as_array().unwrap().len(), 1);
}
