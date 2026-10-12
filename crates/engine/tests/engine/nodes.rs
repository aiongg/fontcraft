//! Node and segment commands (M3.4): moving, node types, converting, deleting with refit,
//! delete and break, inserting, connecting and closing, start nodes, direction, locks, the
//! primitives, the Info box queries and Fit Curve. Fixtures are built in code.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::model::plist::{Str, Value as Plist};
use fontcraft_engine::model::{Layer, Node, NodeKind, Path, Shape};
use fontcraft_engine::{EngineError, Session};
use fontcraft_testkit::{oval, rect, snapshot};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// One node as `layer.inspect` gives it: x, y, type, smooth.
#[derive(Clone, Debug, PartialEq)]
struct N {
    x: f64,
    y: f64,
    kind: String,
    smooth: bool,
}

fn layer(s: &mut Session, glyph: &str) -> Value {
    run(s, "layer.inspect", json!({ "glyph": glyph }))
}

fn nodes(s: &mut Session, glyph: &str, shape: usize) -> Vec<N> {
    let l = layer(s, glyph);
    l["shapes"][shape]["nodes"]
        .as_array()
        .unwrap_or_else(|| panic!("shape {shape} of {glyph} is not a path: {l}"))
        .iter()
        .map(|n| N {
            x: n["x"].as_f64().unwrap(),
            y: n["y"].as_f64().unwrap(),
            kind: n["type"].as_str().unwrap().into(),
            smooth: n["smooth"].as_bool().unwrap(),
        })
        .collect()
}

fn xy(n: &[N]) -> Vec<(f64, f64)> {
    n.iter().map(|n| (n.x, n.y)).collect()
}

fn shape_count(s: &mut Session, glyph: &str) -> usize {
    layer(s, glyph)["shapes"].as_array().unwrap().len()
}

fn closed(s: &mut Session, glyph: &str, shape: usize) -> bool {
    layer(s, glyph)["shapes"][shape]["closed"].as_bool().unwrap()
}

fn add(s: &mut Session, glyph: &str, nodes: Value, closed: bool) -> usize {
    run(s, "path.addPath", json!({ "glyph": glyph, "nodes": nodes, "closed": closed }))["shape"].as_u64().unwrap() as usize
}

fn line(points: &[(f64, f64)]) -> Value {
    Value::Array(points.iter().map(|(x, y)| json!({ "x": x, "y": y })).collect())
}

/// No rounding: spacing 0.
fn without_grid(s: &mut Session) {
    s.edit("grid", |font| {
        Arc::make_mut(&mut font.info).settings.grid_length = Some(0.0);
        Ok(())
    })
    .unwrap();
}

/// The unit cross product of the directions from `p` to `a` and to `b`: 0 when in line.
fn bend(p: &N, a: &N, b: &N) -> f64 {
    let (ax, ay, bx, by) = (a.x - p.x, a.y - p.y, b.x - p.x, b.y - p.y);
    (ax * by - ay * bx) / (ax.hypot(ay) * bx.hypot(by))
}

fn cubic(p: [(f64, f64); 4], t: f64) -> (f64, f64) {
    let s = 1.0 - t;
    let w = [s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t];
    (p.iter().zip(w).map(|(q, w)| q.0 * w).sum(), p.iter().zip(w).map(|(q, w)| q.1 * w).sum())
}

#[test]
fn moving_carries_handles_keeps_smooth_nodes_in_line_and_nudges() {
    let mut s = Session::with_document();
    add(&mut s, "O", oval(350.0, 350.0, 300.0, 360.0), true);
    let before = nodes(&mut s, "O", 0);
    // A node carries its two handles; moved points are rounded to the unit grid.
    let r = run(&mut s, "selection.move", json!({"glyph": "O", "points": [[0, 0]], "dx": 10, "dy": 0}));
    assert_eq!(r["selection"]["points"], json!([[0, 0]]));
    let after = nodes(&mut s, "O", 0);
    assert_eq!((after[0].x, after[1].x, after[11].x), (660.0, 660.0, 660.0));
    assert_eq!(after[1].y, before[1].y.round());
    assert_eq!(xy(&after[2..11]), xy(&before[2..11]), "points that did not move keep their coordinates");

    // A handle of a smooth node alone: the opposite handle turns to stay in line.
    run(&mut s, "selection.move", json!({"glyph": "O", "points": [[0, 1]], "dx": -40, "dy": 0}));
    let n = nodes(&mut s, "O", 0);
    assert_eq!((n[0].x, n[0].y), (660.0, 350.0), "the node stays");
    assert!(bend(&n[0], &n[1], &n[11]).abs() < 0.01, "handles in line: {n:?}");
    assert_ne!(n[11].x, 660.0);

    // pointsOnly: a handle alone keeps its angle and only changes length.
    let h = (n[1].x - n[0].x, n[1].y - n[0].y);
    run(&mut s, "selection.move", json!({"glyph": "O", "points": [[0, 1]], "dx": 30, "dy": 30, "mode": "pointsOnly"}));
    let m = nodes(&mut s, "O", 0);
    let g = (m[1].x - m[0].x, m[1].y - m[0].y);
    assert!((h.0 * g.1 - h.1 * g.0).abs() / (h.0.hypot(h.1) * g.0.hypot(g.1)) < 0.01, "same angle: {h:?} {g:?}");
    assert!(g.0.hypot(g.1) > h.0.hypot(h.1));

    // Nudge scales the handles of a segment with one moved end between its ends.
    let mut s = Session::with_document();
    let curve = json!([
        {"x": 0, "y": 0}, {"x": 100, "y": 0},
        {"x": 100, "y": 50, "type": "offcurve"}, {"x": 50, "y": 100, "type": "offcurve"}, {"x": 0, "y": 100, "type": "curve"},
    ]);
    add(&mut s, "A", curve.clone(), true);
    add(&mut s, "B", curve, true);
    run(&mut s, "selection.move", json!({"glyph": "A", "points": [[0, 1]], "dx": 0, "dy": 20, "mode": "nudge"}));
    assert_eq!(xy(&nodes(&mut s, "A", 0)), [(0.0, 0.0), (100.0, 20.0), (100.0, 60.0), (50.0, 100.0), (0.0, 100.0)]);
    run(&mut s, "selection.move", json!({"glyph": "B", "points": [[0, 1]], "dx": 0, "dy": 20}));
    assert_eq!(xy(&nodes(&mut s, "B", 0)), [(0.0, 0.0), (100.0, 20.0), (100.0, 70.0), (50.0, 100.0), (0.0, 100.0)]);
}

#[test]
fn moving_shapes_anchors_and_guides_and_duplicating() {
    let mut s = Session::with_document();
    add(&mut s, "A", rect(0.0, 0.0, 100.0, 100.0), true);
    s.edit("setup", |font| {
        let mut g = font.glyph_mut("A")?;
        let l = g.layers.first_mut().unwrap();
        l.anchors.push(fontcraft_engine::model::Anchor::new("top", 50.0, 700.0));
        l.info.guides.push(fontcraft_engine::model::Guide { x: 20.0, ..Default::default() });
        Ok(())
    })
    .unwrap();
    run(
        &mut s,
        "selection.move",
        json!({"glyph": "A", "shapes": [0], "anchors": ["top"], "guides": [{"scope": "local", "index": 0}], "dx": 5.4, "dy": -5}),
    );
    let l = layer(&mut s, "A");
    assert_eq!(xy(&nodes(&mut s, "A", 0)), [(5.0, -5.0), (105.0, -5.0), (105.0, 95.0), (5.0, 95.0)]);
    assert_eq!((l["anchors"][0]["x"].as_f64(), l["anchors"][0]["y"].as_f64()), (Some(55.0), Some(695.0)));
    assert_eq!(l["guides"][0]["x"].as_f64(), Some(25.0));

    // Duplicate the whole path: the original stays, the copy moves and is selected.
    let r = run(&mut s, "selection.move", json!({"glyph": "A", "points": [[0, 0], [0, 1], [0, 2], [0, 3]], "dx": 200, "dy": 0, "duplicate": true}));
    assert_eq!(r["selection"]["points"], json!([[1, 0], [1, 1], [1, 2], [1, 3]]));
    assert_eq!(xy(&nodes(&mut s, "A", 0)), [(5.0, -5.0), (105.0, -5.0), (105.0, 95.0), (5.0, 95.0)]);
    assert_eq!(xy(&nodes(&mut s, "A", 1)), [(205.0, -5.0), (305.0, -5.0), (305.0, 95.0), (205.0, 95.0)]);
    assert!(closed(&mut s, "A", 1));
    // Part of a path: its selected segment is copied as an open path.
    run(&mut s, "selection.move", json!({"glyph": "A", "points": [[0, 0], [0, 1]], "dx": 0, "dy": 300, "duplicate": true}));
    assert_eq!(xy(&nodes(&mut s, "A", 2)), [(5.0, 295.0), (105.0, 295.0)]);
    assert!(!closed(&mut s, "A", 2));
    // Duplicating an anchor gives the copy a free name.
    let r = run(&mut s, "selection.move", json!({"glyph": "A", "anchors": ["top"], "dx": 10, "dy": 0, "duplicate": true}));
    assert_eq!(r["selection"]["anchors"], json!(["top_"]));

    // Bad input changes nothing.
    let before = snapshot(&s);
    for bad in [
        json!({"glyph": "A", "points": [[0, 0]], "dx": 1}),
        json!({"glyph": "A", "dx": 1, "dy": 1}),
        json!({"glyph": "A", "points": [[0, 9]], "dx": 1, "dy": 1}),
        json!({"glyph": "A", "points": [[0, 0]], "dx": 1e300, "dy": 1}),
        json!({"glyph": "A", "points": [[0, 0]], "dx": 1, "dy": 1, "mode": "sideways"}),
        json!({"glyph": "A", "points": [[0, 0]], "dx": 1_500_000, "dy": 0}),
    ] {
        assert!(s.execute("selection.move", &bad).is_err(), "{bad}");
    }
    assert_eq!(snapshot(&s), before);
}

#[test]
fn toggling_node_types_and_converting_segments() {
    let mut s = Session::with_document();
    add(&mut s, "A", rect(0.0, 0.0, 100.0, 100.0), true);
    add(&mut s, "B", rect(0.0, 0.0, 100.0, 100.0), true);
    add(&mut s, "C", rect(0.0, 0.0, 100.0, 100.0), true);

    // Keeping the outline: only the type changes.
    let r = run(&mut s, "path.toggleNodeTypeKeepingOutline", json!({"glyph": "A", "points": [[0, 1]]}));
    assert_eq!(r["selection"]["points"], json!([[0, 1]]));
    let n = nodes(&mut s, "A", 0);
    assert_eq!(n.len(), 4);
    assert!(n[1].smooth);
    run(&mut s, "path.toggleNodeType", json!({"glyph": "A", "points": [[0, 1]]}));
    assert!(!nodes(&mut s, "A", 0)[1].smooth, "toggling again makes it a corner");

    // A corner between two lines turning smooth gets handles on both sides, lined up.
    let r = run(&mut s, "path.toggleNodeType", json!({"glyph": "B", "points": [[0, 1]]}));
    assert_eq!(r["selection"]["points"], json!([[0, 3]]));
    let n = nodes(&mut s, "B", 0);
    assert_eq!(
        n.iter().map(|n| n.kind.as_str()).collect::<Vec<_>>(),
        ["line", "offcurve", "offcurve", "curve", "offcurve", "offcurve", "curve", "line"]
    );
    assert!(n[3].smooth && (n[3].x, n[3].y) == (100.0, 0.0));
    assert!(bend(&n[3], &n[2], &n[4]).abs() < 0.02, "handles in line: {n:?}");

    // Straight segments become curves with handles on the line; curves are left alone.
    let r = run(&mut s, "path.convertSegmentToCurve", json!({"glyph": "C", "segments": [[0, 1], [0, 0]]}));
    assert_eq!(r["selection"]["segments"], json!([[0, 0], [0, 3]]));
    let n = nodes(&mut s, "C", 0);
    assert_eq!(
        xy(&n),
        [(0.0, 0.0), (33.0, 0.0), (67.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0), (0.0, 67.0), (0.0, 33.0)],
        "the segment that ends at the first node takes its handles from the end of the list"
    );
    assert_eq!((n[0].kind.as_str(), n[3].kind.as_str()), ("curve", "curve"));
    let before = snapshot(&s);
    assert!(s.execute("path.convertSegmentToCurve", &json!({"glyph": "C", "segment": [0, 3]})).is_err(), "already a curve");
    assert!(s.execute("path.toggleNodeType", &json!({"glyph": "C", "points": [[0, 1]]})).is_err(), "a handle has no type");
    assert_eq!(snapshot(&s), before);
}

#[test]
fn deleting_a_node_refits_the_curve() {
    let mut s = Session::with_document();
    without_grid(&mut s);
    let c = [(0.0, 0.0), (0.0, 120.0), (180.0, 200.0), (300.0, 200.0)];
    let nodes_of = |c: &[(f64, f64); 4]| {
        json!([
            {"x": c[0].0, "y": c[0].1},
            {"x": c[1].0, "y": c[1].1, "type": "offcurve"},
            {"x": c[2].0, "y": c[2].1, "type": "offcurve"},
            {"x": c[3].0, "y": c[3].1, "type": "curve"},
        ])
    };
    add(&mut s, "A", nodes_of(&c), false);
    let r = run(&mut s, "path.insertNode", json!({"glyph": "A", "segment": [0, 3], "t": 0.37}));
    assert_eq!(r["node"], json!([0, 3]));
    assert_eq!(nodes(&mut s, "A", 0).len(), 7);
    run(&mut s, "edit.delete", json!({"glyph": "A", "points": [[0, 3]]}));
    let n = nodes(&mut s, "A", 0);
    assert_eq!(n.len(), 4);
    let fitted = [(n[0].x, n[0].y), (n[1].x, n[1].y), (n[2].x, n[2].y), (n[3].x, n[3].y)];
    // Every one of 100 points of the old curve lies within 0.5 unit of the new one.
    let dense: Vec<(f64, f64)> = (0..=2000).map(|k| cubic(fitted, k as f64 / 2000.0)).collect();
    for k in 0..100 {
        let p = cubic(c, k as f64 / 99.0);
        let d = dense.iter().map(|q| (q.0 - p.0).hypot(q.1 - p.1)).fold(f64::INFINITY, f64::min);
        assert!(d < 0.5, "sample {k} is {d} away");
    }

    // On a closed oval: the path stays closed with three nodes. The new handles are rounded
    // when the grid is on; the points the deletion does not touch keep their coordinates.
    let mut s = Session::with_document();
    add(&mut s, "O", oval(350.0, 350.0, 300.0, 360.0), true);
    let before = nodes(&mut s, "O", 0);
    run(&mut s, "edit.delete", json!({"glyph": "O", "points": [[0, 3]]}));
    let n = nodes(&mut s, "O", 0);
    assert_eq!(n.len(), 9);
    assert!(closed(&mut s, "O", 0));
    assert!(n[1..3].iter().all(|n| n.x.fract() == 0.0 && n.y.fract() == 0.0), "{n:?}");
    assert_eq!(xy(&n[3..]), xy(&before[6..]));
    assert!(n[1].x == 650.0 && n[2].x == 50.0, "the handles keep the vertical tangents: {n:?}");
}

#[test]
fn deleting_handles_shapes_and_other_things() {
    let mut s = Session::with_document();
    add(&mut s, "O", oval(350.0, 350.0, 300.0, 360.0), true);
    add(&mut s, "O", rect(0.0, 0.0, 10.0, 10.0), true);
    add(&mut s, "O", rect(20.0, 0.0, 30.0, 10.0), true);
    // A curve segment that loses a handle becomes a line.
    run(&mut s, "edit.delete", json!({"glyph": "O", "points": [[0, 1]]}));
    let n = nodes(&mut s, "O", 0);
    assert_eq!(n.len(), 10);
    assert_eq!((n[1].kind.as_str(), n[1].x, n[1].y), ("line", 350.0, 710.0));
    // Every point of a path, and a shape by index: both go.
    let r = run(&mut s, "edit.delete", json!({"glyph": "O", "points": [[1, 0], [1, 1], [1, 2], [1, 3]], "shapes": [2]}));
    assert_eq!(r["selection"], json!({"glyph": "O"}));
    assert_eq!(shape_count(&mut s, "O"), 1);
    // Two nodes left of an open path: deleting one removes the path.
    add(&mut s, "O", line(&[(0.0, 0.0), (10.0, 0.0)]), false);
    run(&mut s, "edit.delete", json!({"glyph": "O", "points": [[1, 0]]}));
    assert_eq!(shape_count(&mut s, "O"), 1);
    // Nothing named, or a segment, is an error.
    assert!(s.execute("edit.delete", &json!({"glyph": "O"})).is_err());
    assert!(s.execute("edit.delete", &json!({"glyph": "O", "segments": [[0, 0]]})).is_err());
}

#[test]
fn delete_and_break_opens_paths() {
    let mut s = Session::with_document();
    add(&mut s, "A", rect(0.0, 0.0, 100.0, 100.0), true);
    add(&mut s, "A", rect(200.0, 0.0, 300.0, 100.0), true);
    // One node: it goes with both its segments.
    run(&mut s, "path.deleteAndBreak", json!({"glyph": "A", "points": [[0, 1]]}));
    assert!(!closed(&mut s, "A", 0));
    assert_eq!(xy(&nodes(&mut s, "A", 0)), [(100.0, 100.0), (0.0, 100.0), (0.0, 0.0)]);
    // Two neighbouring nodes: only the segment between them goes.
    run(&mut s, "path.deleteAndBreak", json!({"glyph": "A", "points": [[1, 0], [1, 1]]}));
    assert_eq!(xy(&nodes(&mut s, "A", 1)), [(300.0, 0.0), (300.0, 100.0), (200.0, 100.0), (200.0, 0.0)]);
    assert!(!closed(&mut s, "A", 1));
    // The middle segment of an open path: two open paths, in the place of the one.
    run(&mut s, "path.deleteAndBreak", json!({"glyph": "A", "segments": [[1, 2]]}));
    assert_eq!(shape_count(&mut s, "A"), 3);
    assert_eq!(xy(&nodes(&mut s, "A", 1)), [(300.0, 0.0), (300.0, 100.0)]);
    assert_eq!(xy(&nodes(&mut s, "A", 2)), [(200.0, 100.0), (200.0, 0.0)]);
    assert!(s.execute("path.deleteAndBreak", &json!({"glyph": "A"})).is_err());
}

/// `A` and `B` with a rectangle in both masters; `B`'s second master has a triangle.
fn two_masters() -> Session {
    let mut s = Session::with_document();
    s.edit("setup", |font| {
        let mut m = font.default_master()?.clone();
        m.id = "m02".into();
        m.name = "Bold".into();
        Arc::make_mut(&mut font.masters).push(m);
        for (glyph, second) in [
            ("A", Path::rect(0.0, 0.0, 120.0, 100.0)),
            (
                "B",
                Path::new(true, vec![Node::new(0.0, 0.0, NodeKind::Line), Node::new(9.0, 0.0, NodeKind::Line), Node::new(0.0, 9.0, NodeKind::Line)]),
            ),
        ] {
            let mut g = font.glyph_mut(glyph)?;
            g.layers.first_mut().unwrap().shapes.push(Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
            let mut l = Layer::new("m02", 600.0);
            l.shapes.push(Shape::Path(second));
            g.layers.push(l);
        }
        Ok(())
    })
    .unwrap();
    s
}

fn layer_nodes(s: &mut Session, glyph: &str, layer: &str) -> usize {
    run(s, "layer.inspect", json!({"glyph": glyph, "layer": layer}))["shapes"][0]["nodes"].as_array().unwrap().len()
}

#[test]
fn all_masters_forms_match_by_address_and_skip_what_differs() {
    let mut s = two_masters();
    let r = run(&mut s, "edit.deleteInAllMasters", json!({"glyph": "A", "points": [[0, 1]]}));
    assert_eq!(r["skipped"], json!([]));
    assert_eq!((layer_nodes(&mut s, "A", "m01"), layer_nodes(&mut s, "A", "m02")), (3, 3));
    let r = run(&mut s, "edit.deleteInAllMasters", json!({"glyph": "B", "points": [[0, 1]]}));
    assert_eq!(r["skipped"][0]["layer"], json!("m02"));
    assert_eq!((layer_nodes(&mut s, "B", "m01"), layer_nodes(&mut s, "B", "m02")), (3, 3), "the triangle is left alone");

    let mut s = two_masters();
    run(&mut s, "path.deleteAndBreakInAllMasters", json!({"glyph": "A", "points": [[0, 1]], "layer": "m02"}));
    for id in ["m01", "m02"] {
        assert!(!run(&mut s, "layer.inspect", json!({"glyph": "A", "layer": id}))["shapes"][0]["closed"].as_bool().unwrap());
    }

    let mut s = two_masters();
    let r = run(&mut s, "path.reverseContoursForAllMasters", json!({"glyphs": ["A", "B"]}));
    assert_eq!(r["reversed"], json!(4));
    let second = run(&mut s, "layer.inspect", json!({"glyph": "A", "layer": "m02"}));
    assert_eq!(second["shapes"][0]["nodes"][1]["y"], json!(100.0), "reversed: up the left side first");
}

#[test]
fn connecting_closing_inserting_and_start_nodes() {
    let mut s = Session::with_document();
    add(&mut s, "A", line(&[(0.0, 0.0), (100.0, 0.0)]), false);
    add(&mut s, "A", line(&[(300.0, 0.0), (200.0, 0.0)]), false);
    // Two paths: one, at the lower index, with a straight segment between the ends.
    let r = run(&mut s, "path.connectNodes", json!({"glyph": "A", "points": [[1, 1], [0, 1]]}));
    assert_eq!(r["selection"]["points"], json!([[0, 1], [0, 2]]));
    assert_eq!(shape_count(&mut s, "A"), 1);
    assert_eq!(xy(&nodes(&mut s, "A", 0)), [(0.0, 0.0), (100.0, 0.0), (200.0, 0.0), (300.0, 0.0)]);
    // The two ends of one path: it closes.
    run(&mut s, "path.connectNodes", json!({"glyph": "A", "points": [[0, 0], [0, 3]]}));
    assert!(closed(&mut s, "A", 0));
    assert!(s.execute("path.connectNodes", &json!({"glyph": "A", "points": [[0, 0], [0, 1]]})).is_err(), "not open ends");

    // Ends that coincide become one node; the drag that drops one end on another is a batch.
    add(&mut s, "B", line(&[(0.0, 0.0), (100.0, 0.0)]), false);
    add(&mut s, "B", line(&[(110.0, 0.0), (110.0, 100.0)]), false);
    run(
        &mut s,
        "command.batch",
        json!({"label": "Connect Nodes", "steps": [
            {"command": "selection.move", "params": {"glyph": "B", "points": [[0, 1]], "dx": 10, "dy": 0}},
            {"command": "path.connectNodes", "params": {"glyph": "B", "points": [[0, 1], [1, 0]]}},
        ]}),
    );
    assert_eq!(xy(&nodes(&mut s, "B", 0)), [(0.0, 0.0), (110.0, 0.0), (110.0, 100.0)]);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(shape_count(&mut s, "B"), 2, "one undo step");

    // Close Open Paths, by shape or by a point on it.
    add(&mut s, "C", line(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)]), false);
    run(&mut s, "path.closeOpenPaths", json!({"glyph": "C", "points": [[0, 1]]}));
    assert!(closed(&mut s, "C", 0));
    assert!(s.execute("path.closeOpenPaths", &json!({"glyph": "C", "shapes": [0]})).is_err(), "already closed");

    // Insert a node on a line, and at the extreme of a curve.
    run(&mut s, "path.insertNode", json!({"glyph": "C", "segment": [0, 1]}));
    assert_eq!(xy(&nodes(&mut s, "C", 0))[1], (50.0, 0.0));
    add(
        &mut s,
        "D",
        json!([{"x": 0, "y": 0}, {"x": 0, "y": 100, "type": "offcurve"}, {"x": 300, "y": 100, "type": "offcurve"}, {"x": 300, "y": 0, "type": "curve"}]),
        false,
    );
    let r = run(&mut s, "path.insertNode", json!({"glyph": "D", "segment": [0, 3], "near": [140, 80], "snap": "extremum"}));
    assert_eq!(r["node"], json!([0, 3]));
    let n = nodes(&mut s, "D", 0);
    assert_eq!((n[3].x, n[3].y, n[3].kind.as_str(), n[3].smooth), (150.0, 75.0, "curve", true));
    assert!(s.execute("path.insertNode", &json!({"glyph": "C", "segment": [0, 1], "t": 1.0})).is_err());

    // Make Node First rotates the closed path; Reverse Contours keeps the start node.
    let mut s = Session::with_document();
    add(&mut s, "A", rect(0.0, 0.0, 100.0, 100.0), true);
    let r = run(&mut s, "path.makeNodeFirst", json!({"glyph": "A", "point": [0, 2]}));
    assert_eq!(r["selection"]["points"], json!([[0, 0]]));
    assert_eq!(xy(&nodes(&mut s, "A", 0)), [(100.0, 100.0), (0.0, 100.0), (0.0, 0.0), (100.0, 0.0)]);
    run(&mut s, "path.reverseContours", json!({"glyphs": ["A"]}));
    assert_eq!(xy(&nodes(&mut s, "A", 0)), [(100.0, 100.0), (100.0, 0.0), (0.0, 0.0), (0.0, 100.0)]);
    run(&mut s, "path.reverseContours", json!({"glyph": "A", "shapes": [0]}));
    assert_eq!(xy(&nodes(&mut s, "A", 0)), [(100.0, 100.0), (0.0, 100.0), (0.0, 0.0), (100.0, 0.0)]);
}

#[test]
fn locks_refuse_or_mark() {
    let mut s = Session::with_document();
    add(&mut s, "A", rect(0.0, 0.0, 100.0, 100.0), true);
    run(&mut s, "path.lockPath", json!({"glyph": "A", "points": [[0, 2]]}));
    assert_eq!(layer(&mut s, "A")["shapes"][0]["locked"], json!(true));
    run(&mut s, "path.unlockPath", json!({"glyph": "A", "shapes": [0]}));
    assert_eq!(layer(&mut s, "A")["shapes"][0]["locked"], json!(false));

    let r = run(&mut s, "glyph.toggleLocked", json!({"glyphs": ["A"]}));
    assert_eq!(r["locked"], json!(true));
    let before = snapshot(&s);
    for (id, p) in [
        ("selection.move", json!({"glyph": "A", "points": [[0, 0]], "dx": 1, "dy": 0})),
        ("edit.delete", json!({"glyph": "A", "points": [[0, 0]]})),
        ("edit.deleteInAllMasters", json!({"glyph": "A", "points": [[0, 0]]})),
        ("path.toggleNodeType", json!({"glyph": "A", "points": [[0, 0]]})),
        ("path.reverseContoursForAllMasters", json!({"glyph": "A"})),
        ("path.addRectangle", json!({"glyph": "A", "x": 0, "y": 0, "width": 10, "height": 10})),
        ("fitCurve.setHandleLength", json!({"glyph": "A", "points": [[0, 0]], "percent": 50})),
    ] {
        assert!(matches!(s.execute(id, &p), Err(EngineError::Locked(_))), "{id}");
    }
    assert_eq!(snapshot(&s), before);
    let r = run(&mut s, "glyph.toggleLocked", json!({"glyph": "A"}));
    assert_eq!(r["locked"], json!(false));
}

#[test]
fn rectangles_and_ellipses() {
    let mut s = Session::with_document();
    let r = run(&mut s, "path.addRectangle", json!({"glyph": "A", "x": 10.4, "y": 0, "width": 100, "height": -50}));
    assert_eq!(r["selection"]["shapes"], json!([0]));
    assert_eq!(xy(&nodes(&mut s, "A", 0)), [(10.0, -50.0), (110.0, -50.0), (110.0, 0.0), (10.0, 0.0)]);
    run(&mut s, "path.addEllipse", json!({"glyph": "A", "x": 0, "y": 0, "width": 200, "height": 100, "fromCenter": true}));
    let n = nodes(&mut s, "A", 1);
    assert_eq!(n.len(), 12);
    let ons: Vec<(f64, f64)> = n.iter().filter(|n| n.kind == "curve").map(|n| (n.x, n.y)).collect();
    assert_eq!(ons, [(0.0, -50.0), (100.0, 0.0), (0.0, 50.0), (-100.0, 0.0)]);
    assert!(n.iter().filter(|n| n.kind == "curve").all(|n| n.smooth));
    assert!(s.execute("path.addEllipse", &json!({"glyph": "A", "x": 0, "y": 0, "width": 0, "height": 10})).is_err());
}

#[test]
fn info_box_queries_and_set_position() {
    let mut s = Session::with_document();
    add(&mut s, "A", rect(0.0, 0.0, 100.0, 50.0), true);
    add(&mut s, "O", oval(350.0, 350.0, 300.0, 360.0), true);
    let all = json!([[0, 0], [0, 1], [0, 2], [0, 3]]);
    let r = run(&mut s, "selection.getInfo", json!({"glyph": "A", "points": all}));
    assert_eq!((r["x"].as_f64(), r["y"].as_f64(), r["width"].as_f64(), r["height"].as_f64()), (Some(0.0), Some(0.0), Some(100.0), Some(50.0)));
    let r = run(&mut s, "selection.getInfo", json!({"glyph": "A", "points": [[0, 0], [0, 1]]}));
    assert_eq!(r["distance"]["length"], json!(100.0));
    let r = run(&mut s, "selection.getInfo", json!({"glyph": "O", "points": [[0, 1]]}));
    assert_eq!(r["handle"]["node"], json!([0, 0]));
    assert_eq!(r["handle"]["dx"], json!(0.0));

    let r = run(&mut s, "selection.setPosition", json!({"glyph": "A", "points": all, "x": 200, "y": 100, "referencePoint": "center"}));
    assert_eq!((r["dx"].as_f64(), r["dy"].as_f64()), (Some(150.0), Some(75.0)));
    assert_eq!(xy(&nodes(&mut s, "A", 0))[0], (150.0, 75.0));
    run(&mut s, "selection.setPosition", json!({"glyph": "A", "points": [[0, 2]], "x": 300}));
    assert_eq!(xy(&nodes(&mut s, "A", 0))[2], (300.0, 125.0));
    assert!(s.execute("selection.setPosition", &json!({"glyph": "A", "points": all})).is_err(), "no x or y");
    assert!(s.execute("selection.getInfo", &json!({"glyph": "A", "points": all, "referencePoint": "middle"})).is_err());
}

#[test]
fn fit_curve_sets_handle_percentages() {
    let mut s = Session::with_document();
    // A quarter circle with 55 % handles.
    let quarter = json!([{"x": 100, "y": 0}, {"x": 100, "y": 55, "type": "offcurve"}, {"x": 55, "y": 100, "type": "offcurve"}, {"x": 0, "y": 100, "type": "curve"}]);
    add(&mut s, "A", quarter, false);
    let handles = |s: &mut Session| {
        let n = nodes(s, "A", 0);
        (n[1].y, n[2].x)
    };
    let r = run(&mut s, "fitCurve.getHandleLength", json!({"glyph": "A", "points": [[0, 1]]}));
    assert_eq!(r["segments"][0]["percents"], json!([55.0, 55.0]));
    assert_eq!(r["segments"][0]["segment"], json!([0, 3]));
    // A handle names its segment; both handles change.
    run(&mut s, "fitCurve.applyStep", json!({"glyph": "A", "points": [[0, 2]], "step": 1}));
    assert_eq!(handles(&mut s), (48.0, 48.0));
    run(&mut s, "fitCurve.applyStep", json!({"glyph": "A", "segments": [[0, 3]], "step": 8}));
    assert_eq!(handles(&mut s), (74.0, 74.0));
    run(&mut s, "fitCurve.decrease", json!({"glyph": "A", "points": [[0, 0], [0, 3]]}));
    assert_eq!(handles(&mut s), (70.0, 70.0), "the step below 74: 70.29, rounded");
    run(&mut s, "fitCurve.setHandleLength", json!({"glyph": "A", "points": [[0, 1]], "percent": 140}));
    assert_eq!(handles(&mut s), (100.0, 100.0), "clamped to 100 %");
    run(&mut s, "selection.move", json!({"glyph": "A", "points": [[0, 1]], "dx": 0, "dy": -60, "mode": "pointsOnly"}));
    run(&mut s, "fitCurve.balanceHandles", json!({"glyph": "A", "points": [[0, 1]]}));
    assert_eq!(handles(&mut s), (70.0, 70.0), "the mean of 40 % and 100 %");
    // The font's range parameter.
    s.edit("range", |font| {
        Arc::make_mut(&mut font.info).params.set("Fit Curve Panel Settings", Plist::String(Str::from("50, 70".to_string())));
        Ok(())
    })
    .unwrap();
    run(&mut s, "fitCurve.applyStep", json!({"glyph": "A", "points": [[0, 1]], "step": 1}));
    assert_eq!(handles(&mut s), (50.0, 50.0));
    run(&mut s, "fitCurve.increase", json!({"glyph": "A", "points": [[0, 1]]}));
    assert_eq!(handles(&mut s), (53.0, 53.0), "52.86, rounded");
    assert!(s.execute("fitCurve.applyStep", &json!({"glyph": "A", "points": [[0, 1]], "step": 9})).is_err());
    assert!(s.execute("fitCurve.getHandleLength", &json!({"glyph": "A", "points": [[0, 0]]})).is_err(), "one end node names no segment");
}
