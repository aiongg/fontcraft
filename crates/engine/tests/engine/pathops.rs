//! Path operations (M3.6): Remove Overlap and the booleans, Correct Path Direction, extremes,
//! Tidy up Paths, Open Corner and Reconnect Nodes, Cut with Line, curve conversion and Round
//! Coordinates, in their one-glyph, Font View and all-masters forms. Fixtures are built in code.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::model::{Anchor, Axis, Component, Font, Glyph, GlyphTable, Instance, Layer, Master, Node, NodeKind, Path, Shape};
use fontcraft_engine::prelude::*;
use fontcraft_engine::{EngineError, Session};
use fontcraft_geom::Shape as _;
use fontcraft_testkit::{oval, rect, snapshot};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// Adds a path to `glyph` on `layer` (the first master's when `None`).
fn add_to(s: &mut Session, glyph: &str, layer: Option<&str>, nodes: Value, closed: bool) {
    let mut p = json!({"glyph": glyph, "nodes": nodes, "closed": closed});
    if let Some(layer) = layer {
        p["layer"] = json!(layer);
    }
    run(s, "path.addPath", p);
}

fn add(s: &mut Session, glyph: &str, nodes: Value) {
    add_to(s, glyph, None, nodes, true);
}

fn default_layer(s: &Session) -> String {
    s.font().unwrap().default_master().unwrap().id.clone()
}

/// The shapes of a glyph's layer (the first master's when `None`).
fn shapes(s: &Session, glyph: &str, layer: Option<&str>) -> Vec<Shape> {
    let id = layer.map_or_else(|| default_layer(s), str::to_string);
    let font = s.font().unwrap();
    let g = font.glyph(glyph).unwrap();
    g.layers.iter().find(|l| l.id == id).map(|l| l.shapes.clone()).unwrap_or_default()
}

fn paths(s: &Session, glyph: &str, layer: Option<&str>) -> Vec<Path> {
    shapes(s, glyph, layer).into_iter().filter_map(|s| if let Shape::Path(p) = s { Some(p) } else { None }).collect()
}

fn area(p: &Path) -> f64 {
    p.to_bez().unwrap().area()
}

fn total_area(ps: &[Path]) -> f64 {
    ps.iter().map(area).sum()
}

fn xy(p: &Path) -> Vec<(f64, f64)> {
    p.nodes.iter().map(|n| (n.x, n.y)).collect()
}

fn points(list: &[(f64, f64)]) -> Value {
    Value::Array(list.iter().map(|(x, y)| json!({"x": x, "y": y})).collect())
}

/// A second master `m02`.
fn second_master(s: &mut Session) {
    s.edit("setup", |font| {
        let mut m = font.default_master()?.clone();
        m.id = "m02".into();
        m.name = "Bold".into();
        Arc::make_mut(&mut font.masters).push(m);
        Ok(())
    })
    .unwrap();
}

fn overlapping_squares(s: &mut Session, glyph: &str, layer: Option<&str>) {
    add_to(s, glyph, layer, rect(0.0, 0.0, 100.0, 100.0), true);
    add_to(s, glyph, layer, rect(50.0, 50.0, 150.0, 150.0), true);
}

#[test]
fn remove_overlap_merges_the_selected_paths_and_keeps_node_data() {
    let mut s = Session::with_document();
    overlapping_squares(&mut s, "A", None);
    add(&mut s, "A", rect(300.0, 0.0, 400.0, 100.0));
    s.edit("name a node", |font| {
        let mut g = font.glyph_mut("A")?;
        if let Some(Shape::Path(p)) = g.layers[0].shapes.first_mut() {
            p.nodes[0].info.name = Some("corner".into());
        }
        Ok(())
    })
    .unwrap();

    let r = run(&mut s, "path.removeOverlap", json!({"glyph": "A", "shapes": [0, 1]}));
    let ps = paths(&s, "A", None);
    assert_eq!(ps.len(), 2, "the two squares merge; the third is not selected");
    assert!((area(&ps[0]) - 17_500.0).abs() < 1e-6, "outer contour counter-clockwise: {}", area(&ps[0]));
    assert_eq!(xy(&ps[1]), [(300.0, 0.0), (400.0, 0.0), (400.0, 100.0), (300.0, 100.0)]);
    assert_eq!(r["selection"]["shapes"], json!([0]));
    let named: Vec<&Node> = ps[0].nodes.iter().filter(|n| n.info.name.is_some()).collect();
    assert_eq!(named.len(), 1);
    assert_eq!((named[0].x, named[0].y), (0.0, 0.0), "the named node survives with its name");

    run(&mut s, "edit.undo", json!({}));
    assert_eq!(paths(&s, "A", None).len(), 3, "one undo step");
}

#[test]
fn remove_overlap_deletes_open_paths_unless_asked_to_keep_them() {
    for (keep, count) in [(false, 1), (true, 2)] {
        let mut s = Session::with_document();
        overlapping_squares(&mut s, "B", None);
        add_to(&mut s, "B", None, points(&[(0.0, 300.0), (100.0, 300.0)]), false);
        run(&mut s, "path.removeOverlap", json!({"glyph": "B", "keepOpenPaths": keep}));
        let ps = paths(&s, "B", None);
        assert_eq!(ps.len(), count, "keepOpenPaths: {keep}");
        assert!(ps[0].closed);
    }
}

#[test]
fn batch_and_all_masters_forms_skip_locked_glyphs() {
    let mut s = Session::with_document();
    second_master(&mut s);
    for glyph in ["A", "B"] {
        overlapping_squares(&mut s, glyph, None);
        overlapping_squares(&mut s, glyph, Some("m02"));
    }
    run(&mut s, "glyph.toggleLocked", json!({"glyphs": ["B"], "locked": true}));

    let r = run(&mut s, "path.removeOverlapForAllMasters", json!({"glyphs": ["A", "B"]}));
    assert_eq!(r["glyphs"], json!(["A"]));
    assert_eq!(r["skipped"][0]["glyph"], json!("B"));
    for layer in [None, Some("m02")] {
        assert_eq!(paths(&s, "A", layer).len(), 1);
        assert_eq!(paths(&s, "B", layer).len(), 2, "the locked glyph is left as it was");
    }

    // One glyph: a lock is an error, and nothing changes.
    let before = snapshot(&s);
    assert!(matches!(s.execute("path.removeOverlap", &json!({"glyph": "B"})), Err(EngineError::Locked(_))));
    assert!(matches!(s.execute("path.removeOverlapForAllMasters", &json!({"glyph": "B"})), Err(EngineError::Locked(_))));
    assert_eq!(snapshot(&s), before);

    // Font View form with several layers.
    run(&mut s, "glyph.toggleLocked", json!({"glyphs": ["B"], "locked": false}));
    let first = default_layer(&s);
    let r = run(&mut s, "path.removeOverlap", json!({"glyphs": ["B"], "layers": [first, "m02"]}));
    assert_eq!(r["skipped"], json!([]));
    assert_eq!(paths(&s, "B", Some("m02")).len(), 1);
}

#[test]
fn booleans_subtract_intersect_and_unite() {
    let mut s = Session::with_document();
    // Without a selection the frontmost (last) path is cut out of the rest.
    add(&mut s, "A", rect(0.0, 0.0, 300.0, 300.0));
    add(&mut s, "A", rect(100.0, 100.0, 200.0, 200.0));
    run(&mut s, "transform.booleanSubtract", json!({"glyph": "A"}));
    let ps = paths(&s, "A", None);
    assert_eq!(ps.len(), 2);
    assert!((total_area(&ps) - 80_000.0).abs() < 1e-6, "a square with a counter: {}", total_area(&ps));

    overlapping_squares(&mut s, "B", None);
    run(&mut s, "transform.booleanIntersect", json!({"glyphs": ["B"]}));
    let ps = paths(&s, "B", None);
    assert_eq!(ps.len(), 1);
    assert!((area(&ps[0]) - 2_500.0).abs() < 1e-6);

    overlapping_squares(&mut s, "C", None);
    add(&mut s, "C", rect(300.0, 0.0, 400.0, 100.0));
    let r = run(&mut s, "transform.booleanUnion", json!({"glyph": "C", "shapes": [0, 1]}));
    assert_eq!(paths(&s, "C", None).len(), 2);
    assert_eq!(r["selection"]["shapes"], json!([0]));

    // One path: nothing to subtract it from.
    add(&mut s, "D", rect(0.0, 0.0, 100.0, 100.0));
    let before = snapshot(&s);
    assert!(s.execute("transform.booleanSubtract", &json!({"glyph": "D"})).is_err());
    assert_eq!(snapshot(&s), before);
    let r = run(&mut s, "transform.booleanSubtract", json!({"glyphs": ["D"]}));
    assert_eq!(r["skipped"][0]["glyph"], json!("D"));
    assert_eq!(xy(&paths(&s, "D", None)[0]), [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
}

#[test]
fn correct_path_direction_nests_restarts_and_sorts_components_among_paths() {
    let mut s = Session::with_document();
    add(&mut s, "B", rect(0.0, 0.0, 100.0, 100.0));
    // A counter drawn the same way as its outer contour, both starting away from the bottom left,
    // after a component that lies to the right of both.
    add(&mut s, "O", points(&[(300.0, 300.0), (0.0, 300.0), (0.0, 0.0), (300.0, 0.0)]));
    add(&mut s, "O", points(&[(200.0, 200.0), (100.0, 200.0), (100.0, 100.0), (200.0, 100.0)]));
    s.edit("component", |font| {
        let mut g = font.glyph_mut("O")?;
        g.layers[0].shapes.insert(0, Shape::Component(Component::new("B", 500.0, 0.0)));
        Ok(())
    })
    .unwrap();

    run(&mut s, "path.correctPathDirection", json!({"glyph": "O"}));
    let all = shapes(&s, "O", None);
    let kinds: Vec<&str> = all.iter().map(|s| if matches!(s, Shape::Path(_)) { "path" } else { "component" }).collect();
    assert_eq!(kinds, ["path", "path", "component"], "sorted by left edge");
    let ps = paths(&s, "O", None);
    assert!(area(&ps[0]) > 0.0, "outer counter-clockwise");
    assert!(area(&ps[1]) < 0.0, "counter clockwise");
    assert_eq!(xy(&ps[0])[0], (0.0, 0.0), "starts at the lowest, leftmost node");
    assert_eq!(xy(&ps[1])[0], (100.0, 100.0));
}

#[test]
fn correct_path_direction_for_all_masters_covers_every_master() {
    let mut s = Session::with_document();
    second_master(&mut s);
    for layer in [None, Some("m02")] {
        add_to(&mut s, "O", layer, rect(0.0, 0.0, 300.0, 300.0), true);
        add_to(&mut s, "O", layer, rect(100.0, 100.0, 200.0, 200.0), true);
    }
    let r = run(&mut s, "path.correctPathDirectionForAllMasters", json!({"glyph": "O"}));
    assert_eq!(r["glyphs"], json!(["O"]));
    for layer in [None, Some("m02")] {
        assert!(area(&paths(&s, "O", layer)[1]) < 0.0);
    }
}

#[test]
fn add_extremes_puts_a_smooth_node_at_the_top_of_a_hump() {
    let hump = json!([
        {"x": 0, "y": 0},
        {"x": 0, "y": 100, "type": "offcurve"}, {"x": 200, "y": 100, "type": "offcurve"}, {"x": 200, "y": 0, "type": "curve"},
    ]);
    for id in ["path.addExtremes", "path.forceExtremes"] {
        let mut s = Session::with_document();
        add(&mut s, "A", hump.clone());
        run(&mut s, id, json!({"glyphs": ["A"]}));
        let p = &paths(&s, "A", None)[0];
        assert_eq!(p.nodes.len(), 7, "{id}: {:?}", xy(p));
        let top = p.nodes.iter().find(|n| n.on_curve() && n.y == 75.0).unwrap_or_else(|| panic!("{id}: no node at the top: {:?}", xy(p)));
        assert_eq!((top.x, top.smooth), (100.0, true));
    }
}

#[test]
fn tidy_up_removes_a_node_on_a_straight_line_on_every_master() {
    let mut s = Session::with_document();
    second_master(&mut s);
    let extra = points(&[(0.0, 0.0), (50.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
    add(&mut s, "A", extra.clone());
    run(&mut s, "path.tidyUpPaths", json!({"glyph": "A"}));
    assert_eq!(xy(&paths(&s, "A", None)[0]), [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);

    add(&mut s, "B", extra.clone());
    add_to(&mut s, "B", Some("m02"), extra, true);
    run(&mut s, "path.tidyUpPathsForAllMasters", json!({"glyphs": ["B"]}));
    for layer in [None, Some("m02")] {
        assert_eq!(paths(&s, "B", layer)[0].nodes.len(), 4);
    }
}

#[test]
fn an_opened_corner_reconnects_to_the_same_corner() {
    let mut s = Session::with_document();
    add(&mut s, "A", rect(0.0, 0.0, 200.0, 200.0));
    let r = run(&mut s, "path.openCorner", json!({"glyph": "A", "points": [[0, 1]], "overlap": 20}));
    let opened = r["selection"]["points"].clone();
    assert_eq!(opened.as_array().map(Vec::len), Some(2));
    let p = &paths(&s, "A", None)[0];
    assert_eq!(p.nodes.len(), 5);
    assert!(xy(p).contains(&(220.0, 0.0)) && xy(p).contains(&(200.0, -20.0)), "{:?}", xy(p));

    run(&mut s, "path.reconnectNodes", json!({"glyph": "A", "points": opened}));
    let p = &paths(&s, "A", None)[0];
    assert_eq!(p.nodes.len(), 4, "{:?}", xy(p));
    assert!(xy(p).contains(&(200.0, 0.0)));
}

#[test]
fn a_cut_splits_a_square_into_two_closed_pieces() {
    let mut s = Session::with_document();
    add(&mut s, "A", rect(0.0, 0.0, 100.0, 100.0));
    let r = run(&mut s, "path.cutWithLine", json!({"glyph": "A", "from": [40, -10], "to": [40, 110]}));
    let ps = paths(&s, "A", None);
    assert_eq!(ps.len(), 2);
    let mut areas: Vec<f64> = ps.iter().map(area).collect();
    areas.sort_by(f64::total_cmp);
    assert_eq!(areas, [4_000.0, 6_000.0]);
    assert_eq!(r["selection"]["points"].as_array().map(Vec::len), Some(4));
}

#[test]
fn quadratic_and_cubic_conversions() {
    let mut s = Session::with_document();
    let spline = json!([
        {"x": 0, "y": 0},
        {"x": 0, "y": 100, "type": "offcurve"}, {"x": 200, "y": 100, "type": "offcurve"}, {"x": 200, "y": 0, "type": "qcurve"},
    ]);
    add(&mut s, "A", spline);
    let before = area(&paths(&s, "A", None)[0]);
    let r = run(&mut s, "path.convertToCubic", json!({"glyph": "A"}));
    assert_eq!(r["segments"], json!(1));
    let p = &paths(&s, "A", None)[0];
    assert!(p.nodes.iter().all(|n| n.kind != NodeKind::QCurve));
    assert!((area(p) - before).abs() < 1e-9, "exact");

    run(&mut s, "path.convertToQuadratic", json!({"glyph": "A"}));
    let p = &paths(&s, "A", None)[0];
    assert!(p.nodes.iter().all(|n| n.kind != NodeKind::Curve));
    assert!((area(p) - before).abs() < 0.01 * before.abs());

    // Two masters of the same structure stay compatible.
    second_master(&mut s);
    add(&mut s, "O", oval(350.0, 350.0, 300.0, 360.0));
    add_to(&mut s, "O", Some("m02"), oval(350.0, 350.0, 330.0, 380.0), true);
    run(&mut s, "path.convertToQuadratic", json!({"glyph": "O", "allMasters": true}));
    let (a, b) = (&paths(&s, "O", None)[0], &paths(&s, "O", Some("m02"))[0]);
    assert!(a.nodes.iter().all(|n| n.kind != NodeKind::Curve));
    let kinds = |p: &Path| p.nodes.iter().map(|n| n.kind).collect::<Vec<_>>();
    assert_eq!(kinds(a), kinds(b));
}

#[test]
fn round_coordinates_rounds_nodes_anchors_and_component_offsets() {
    let mut s = Session::with_document();
    add(&mut s, "A", rect(0.0, 0.0, 100.0, 100.0));
    s.edit("fractions", |font| {
        let mut g = font.glyph_mut("A")?;
        let l = &mut g.layers[0];
        if let Some(Shape::Path(p)) = l.shapes.first_mut() {
            (p.nodes[0].x, p.nodes[0].y) = (10.4, 20.6);
        }
        l.shapes.push(Shape::Component(Component::new("B", 3.3, 4.7)));
        l.anchors.push(Anchor::new("top", 1.6, 2.4));
        Ok(())
    })
    .unwrap();
    run(&mut s, "path.roundCoordinates", json!({"glyphs": ["A"]}));
    let all = shapes(&s, "A", None);
    let Shape::Path(p) = &all[0] else { panic!("not a path") };
    assert_eq!(xy(p)[0], (10.0, 21.0));
    let Shape::Component(c) = &all[1] else { panic!("not a component") };
    assert_eq!(c.affine().as_coeffs()[4..], [3.0, 5.0]);
    let a = &s.font().unwrap().glyph("A").unwrap().layers[0].anchors[0];
    assert_eq!((a.x, a.y), (2.0, 2.0));
}

// The Edit View passes its selection: only the selected nodes, the handles of the selected
// on-curve nodes and the nodes of the selected segments move; the anchor stays.
#[test]
fn round_coordinates_on_a_selection_leaves_the_rest() {
    let mut s = Session::with_document();
    add(&mut s, "A", rect(0.0, 0.0, 100.0, 100.0));
    // path.addPath rounds to the grid, so the fractions go in directly.
    let (on, off) = (NodeKind::Line, NodeKind::OffCurve);
    let nodes = [(0.4, 0.4, on), (100.4, 0.4, on), (100.4, 50.4, off), (80.4, 100.4, off), (50.4, 100.4, NodeKind::Curve), (0.4, 100.4, on)];
    s.edit("fractions", |font| {
        let mut g = font.glyph_mut("A")?;
        let l = &mut g.layers[0];
        l.shapes[0] = Shape::Path(Path::new(true, nodes.iter().map(|&(x, y, k)| Node::new(x, y, k)).collect()));
        l.anchors.push(Anchor::new("top", 1.6, 2.4));
        Ok(())
    })
    .unwrap();
    let r = run(&mut s, "path.roundCoordinates", json!({"glyph": "A", "points": [[0, 1], [0, 5]]}));
    assert_eq!(r["incompatible"], json!([]));
    assert_eq!(xy(&paths(&s, "A", None)[0]), [(0.4, 0.4), (100.0, 0.0), (100.0, 50.0), (80.4, 100.4), (50.4, 100.4), (0.0, 100.0)]);

    run(&mut s, "path.roundCoordinates", json!({"glyph": "A", "segments": [[0, 4]]}));
    assert_eq!(xy(&paths(&s, "A", None)[0]), [(0.4, 0.4), (100.0, 0.0), (100.0, 50.0), (80.0, 100.0), (50.0, 100.0), (0.0, 100.0)]);
    let a = &s.font().unwrap().glyph("A").unwrap().layers[0].anchors[0];
    assert_eq!((a.x, a.y), (1.6, 2.4));
}

/// Light (100) and Bold (900) with an instance between them, so their layers must stay
/// compatible. `A` has a node in the middle of its bottom edge on both.
fn two_master_session() -> Session {
    let mut f = Font::new("Compat");
    f.axes = Arc::new(vec![Axis { name: "Weight".into(), tag: "wght".into(), ..Axis::default() }]);
    f.masters = Arc::new(
        [("light", 100.0), ("bold", 900.0)]
            .iter()
            .map(|(id, w)| Master { id: (*id).into(), name: (*id).into(), location: vec![*w], ..Master::default() })
            .collect(),
    );
    f.instances = Arc::new(vec![Instance { name: "Regular".into(), location: vec![400.0], ..Instance::default() }]);
    let mut g = Glyph::new("A");
    for master in ["light", "bold"] {
        let mut l = Layer::new(master, 500.0);
        let nodes =
            [(0.0, 0.0), (50.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)].iter().map(|&(x, y)| Node::new(x, y, NodeKind::Line)).collect();
        l.shapes.push(Shape::Path(Path::new(true, nodes)));
        g.layers.push(l);
    }
    f.glyphs = GlyphTable::from_glyphs(vec![Arc::new(g)]);
    let mut s = Session::new();
    s.add_document(f, None).unwrap();
    s
}

// Tidying one master removes a node the other master still has; tidying both keeps them matched.
#[test]
fn tidy_up_lists_the_glyphs_it_makes_incompatible() {
    let mut s = two_master_session();
    assert!(s.glyph_compat("A").unwrap().ok);
    let r = run(&mut s, "path.tidyUpPaths", json!({"glyph": "A", "layer": "light"}));
    assert_eq!(r["incompatible"], json!(["A"]));

    let mut s = two_master_session();
    let r = run(&mut s, "path.tidyUpPathsForAllMasters", json!({"glyphs": ["A"]}));
    assert_eq!(r["incompatible"], json!([]));
    assert!(s.glyph_compat("A").unwrap().ok);
}

#[test]
fn bad_parameters_change_nothing() {
    let mut s = Session::with_document();
    add(&mut s, "A", rect(0.0, 0.0, 100.0, 100.0));
    let before = snapshot(&s);
    for (id, p) in [
        ("path.cutWithLine", json!({"glyph": "A", "from": [10, 10], "to": [10, 10]})),
        ("path.cutWithLine", json!({"glyph": "A", "from": [10, "x"], "to": [10, 10]})),
        ("path.cutWithLine", json!({"glyphs": ["A"], "from": [0, -10], "to": [0, 110]})),
        ("path.reconnectNodes", json!({"glyph": "A", "points": [[0, 1]]})),
        ("path.openCorner", json!({"glyph": "A"})),
        ("path.openCorner", json!({"glyph": "A", "points": [[0, 1]], "overlap": -5})),
        ("path.removeOverlap", json!({"glyph": "A", "shapes": [5]})),
        ("path.removeOverlap", json!({"glyphs": ["A"], "shapes": [0]})),
        ("path.convertToQuadratic", json!({"glyph": "A", "tolerance": 0})),
        ("path.roundCoordinatesForAllMasters", json!({"glyph": "A", "layer": "m01"})),
        ("path.roundCoordinates", json!({"glyphs": ["A"], "points": [[0, 1]]})),
        ("path.roundCoordinates", json!({"glyph": "A", "segments": [[0, 9]]})),
        ("path.roundCoordinates", json!({"glyph": "A", "allMasters": true, "points": [[0, 1]]})),
    ] {
        assert!(s.execute(id, &p).is_err(), "{id} {p}");
        assert_eq!(snapshot(&s), before, "{id} {p}");
    }
}

#[test]
fn remove_overlap_closes_the_open_corners_an_export_erases() {
    let ear =
        |ear: f64| json!([{"x": 0, "y": 0}, {"x": 100, "y": 0}, {"x": 100, "y": 100.0 + ear}, {"x": 100.0 + ear, "y": 100}, {"x": 0, "y": 100}]);
    let points = |s: &mut Session| -> Vec<(i64, i64)> {
        let v = s.execute("layer.inspect", &json!({"glyph": "A"})).unwrap();
        let nodes = v["shapes"].as_array().unwrap().iter().flat_map(|p| p["nodes"].as_array().unwrap().clone());
        let mut out: Vec<(i64, i64)> = nodes.map(|n| (n["x"].as_f64().unwrap() as i64, n["y"].as_f64().unwrap() as i64)).collect();
        out.sort_unstable();
        out
    };
    let mut s = Session::with_document();
    s.execute("path.addPath", &json!({"glyph": "A", "nodes": ear(40.0)})).unwrap();
    s.execute("path.removeOverlap", &json!({"glyph": "A"})).unwrap();
    assert_eq!(points(&mut s), [(0, 0), (0, 100), (100, 0), (100, 100)]);
    // A larger ear is part of the shape.
    let mut s = Session::with_document();
    s.execute("path.addPath", &json!({"glyph": "A", "nodes": ear(150.0)})).unwrap();
    s.execute("path.removeOverlap", &json!({"glyph": "A"})).unwrap();
    assert!(points(&mut s).contains(&(250, 100)));
}
