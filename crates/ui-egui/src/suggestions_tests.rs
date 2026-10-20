//! Snapping suggestions in the shell: the View › Suggest switches, the master values they read,
//! and a canvas drag that snaps an edge to a declared stem.

use std::sync::Arc;

use fontcraft_model::plist::{Array, Dict, Number, Value as Plist};
use fontcraft_model::{Shape, StemDef};
use fontcraft_tools::suggestions::Switches;
use serde_json::{Value, json};

use crate::canvas::suggestions::{DISTANCE_PARAMETER, distance, suggest};
use crate::{FontcraftApp, control};

fn call(app: &mut FontcraftApp, method: &str, params: Value) -> Value {
    control::handle(app, &json!({"id": 1, "method": method, "params": params}))
}

fn drag(app: &mut FontcraftApp, from: (f64, f64), to: (f64, f64)) {
    let mid = ((from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0);
    let events = json!([
        {"kind": "down", "x": from.0, "y": from.1},
        {"kind": "drag", "x": mid.0, "y": mid.1},
        {"kind": "drag", "x": to.0, "y": to.1},
        {"kind": "up", "x": to.0, "y": to.1},
    ]);
    let r = call(app, "ui.pointer", json!({ "events": events }));
    assert_eq!(r["ok"], true, "{r}");
}

/// The x of every node of glyph A's first path, sorted, without repeats.
fn xs(app: &FontcraftApp) -> Vec<f64> {
    let font = app.session.font().unwrap();
    let layer = font.glyph("A").unwrap().layer(&font.masters[0].id).unwrap();
    let Some(Shape::Path(path)) = layer.shapes.first() else { panic!("no path") };
    let mut xs: Vec<f64> = path.nodes.iter().map(|n| n.x).collect();
    xs.sort_by(f64::total_cmp);
    xs.dedup();
    xs
}

/// An app editing glyph A, a 100-unit square at the origin, in a font whose master declares a
/// vertical stem of 80 and a `suggestDistance` of 40 across and 25 up.
fn app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    let mut font = app.session.font().unwrap().clone();
    Arc::make_mut(&mut font.stems).push(StemDef { name: "lc".into(), ..Default::default() });
    let count = font.stems.len();
    let master = Arc::make_mut(&mut font.masters).first_mut().unwrap();
    master.stem_values.resize(count, 0.0);
    *master.stem_values.last_mut().unwrap() = 80.0;
    let mut d = Dict::new();
    d.insert("x", Plist::Number(Number::int(40)));
    d.insert("y", Plist::Number(Number::int(25)));
    master.params.set(DISTANCE_PARAMETER, Plist::Dict(d));
    app.session.add_document(font, None).unwrap();
    app.run("path.addRectangle", &json!({"glyph": "A", "x": 0, "y": 0, "width": 100, "height": 100})).unwrap();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
    app.run("view.setZoom", &json!({"size": 1000, "center": [50, 50]})).unwrap();
    app
}

#[test]
fn view_suggest_switches_toggle_and_a_dragged_edge_snaps_to_a_declared_stem() {
    let mut app = app();
    assert_eq!(app.state.panels.edit_aids.suggest, Switches::default(), "every suggestion starts off");
    assert_eq!(app.run("view.suggestStems", &json!({})).unwrap(), json!({"on": true}), "no `on`: toggle");
    assert_eq!(app.run("view.suggestStems", &json!({"on": true})).unwrap(), json!({"on": true}));
    assert!(app.run("view.suggestStems", &json!({"on": 1})).is_err());
    assert!(app.state.panels.edit_aids.suggest.stems);
    for id in ["view.suggestHandles", "view.suggestLines", "view.suggestDistance"] {
        assert_eq!(app.run(id, &json!({"on": true})).unwrap(), json!({"on": true}), "{id}");
    }
    assert_eq!(app.state.panels.edit_aids.suggest, Switches { handles: true, lines: true, distance: true, stems: true });

    // The master's values reach the tools.
    {
        let font = app.session.font().unwrap();
        let tab = app.state.tab().unwrap();
        let s = suggest(&app.session, font, tab, app.state.panels.edit_aids.suggest);
        assert_eq!(s.stems.len(), 1);
        assert_eq!(s.stems[0].width, 80.0);
        assert_eq!(s.distance, Some((40.0, 25.0)));
    }

    // The right edge dragged from x 100 to 82 lands at 80: the stem from the left edge.
    for id in ["view.suggestLines", "view.suggestDistance"] {
        app.run(id, &json!({"on": false})).unwrap();
    }
    drag(&mut app, (100.0, 50.0), (82.0, 50.0));
    assert_eq!(xs(&app), vec![0.0, 80.0]);
    // With Suggest Stems off, the edge goes where it is dragged.
    app.run("view.suggestStems", &json!({"on": false})).unwrap();
    drag(&mut app, (80.0, 50.0), (62.0, 50.0));
    assert_eq!(xs(&app), vec![0.0, 62.0]);
}

#[test]
fn suggest_distance_reads_a_dictionary_an_array_or_one_number() {
    let n = |v: i64| Plist::Number(Number::int(v));
    let mut d = Dict::new();
    d.insert("x", n(30));
    assert_eq!(distance(&Plist::Dict(d)), Some((30.0, 0.0)));
    assert_eq!(distance(&Plist::Array(Array { items: vec![n(10), n(20)], ..Default::default() })), Some((10.0, 20.0)));
    assert_eq!(distance(&n(12)), Some((12.0, 12.0)));
    assert_eq!(distance(&Plist::Dict(Dict::new())), None);
}

#[test]
fn view_suggest_menu_items_carry_a_check_mark_while_on() {
    let mut app = app();
    let checked = |app: &mut FontcraftApp, id: &str| {
        let tree = app.menu_cache.tree(&app.session, &app.state);
        tree.iter().flat_map(|m| m.items.iter()).find(|i| i.command == id).and_then(|i| i.checked)
    };
    assert_eq!(checked(&mut app, "view.suggestStems"), Some(false));
    app.run("view.suggestStems", &json!({"on": true})).unwrap();
    assert_eq!(checked(&mut app, "view.suggestStems"), Some(true), "the cached menu follows the switch");
    assert_eq!(checked(&mut app, "view.suggestLines"), Some(false));
    assert_eq!(checked(&mut app, "view.showTunniLines"), None, "not one of the switches with a check mark");
    let tree = call(&mut app, "ui.menu.list", json!({}));
    let items: Vec<&Value> = tree["result"].as_array().unwrap().iter().flat_map(|m| m["items"].as_array().unwrap()).collect();
    let stems = items.iter().find(|i| i["command"] == "view.suggestStems").unwrap();
    assert_eq!(stems["checked"], json!(true), "{stems}");
}
