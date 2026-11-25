//! Editing quadratic outlines (M3.54): smooth ends of one-handle segments, deleting, copying, Fit Curve's refusal, and
//! the corpus measure.
//!
//! `cargo test --release -p fontcraft-engine --test engine quadratic_edit:: -- --ignored --nocapture`.
//! `FONTCRAFT_CORPUS` names the corpus folder (default: the repository's `corpus/`).
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path as FilePath, PathBuf};

use fontcraft_engine::Session;
use fontcraft_model::{NodeKind, Path, Shape};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn on(x: i32, y: i32, kind: &str, smooth: bool) -> Value {
    json!({"x": x, "y": y, "type": kind, "smooth": smooth})
}

fn off(x: i32, y: i32) -> Value {
    json!({"x": x, "y": y, "type": "offcurve"})
}

fn session(nodes: Value, closed: bool) -> Session {
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": nodes, "closed": closed}));
    s
}

fn path(s: &Session) -> Path {
    let font = s.font().unwrap();
    let master = font.default_master().unwrap().id.clone();
    match font.glyph("A").unwrap().layer(&master).unwrap().shapes.last() {
        Some(Shape::Path(p)) => p.clone(),
        other => panic!("not a path: {other:?}"),
    }
}

fn xy(s: &Session) -> Vec<(f64, f64)> {
    path(s).nodes.iter().map(|n| (n.x, n.y)).collect()
}

/// A circle as TrueType tools draw it: four smooth nodes, one handle between each two.
fn ring() -> Session {
    let q = |x, y| on(x, y, "qcurve", true);
    session(json!([q(0, -100), off(100, -100), q(100, 0), off(100, 100), q(0, 100), off(-100, 100), q(-100, 0), off(-100, -100)]), true)
}

#[test]
fn the_only_handle_of_a_segment_keeps_both_smooth_ends_smooth() {
    // Round a ring the smooth nodes beside the dragged handle slide onto the lines to the next handles.
    let mut s = ring();
    run(&mut s, "selection.move", json!({"glyph": "A", "points": [[0, 1]], "dx": 20, "dy": -20}));
    assert_eq!(
        xy(&s),
        [(10.0, -110.0), (120.0, -120.0), (110.0, -10.0), (100.0, 100.0), (0.0, 100.0), (-100.0, 100.0), (-100.0, 0.0), (-100.0, -100.0)]
    );

    // Between two lines the handle is where the lines cross, when a line's far end moves and when a node moves.
    let corner = json!([on(0, 0, "line", false), on(100, 0, "line", true), off(200, 0), on(200, 100, "qcurve", true), on(200, 200, "line", false)]);
    let mut s = session(corner.clone(), false);
    run(&mut s, "selection.move", json!({"glyph": "A", "points": [[0, 4]], "dx": 20, "dy": 0}));
    assert_eq!(xy(&s)[2], (180.0, 0.0));
    let mut s = session(corner.clone(), false);
    run(&mut s, "selection.move", json!({"glyph": "A", "points": [[0, 1], [0, 0]], "dx": 0, "dy": 30}));
    assert_eq!(xy(&s)[2], (200.0, 30.0), "up the other line");
    // Dragged itself, it stays there.
    let mut s = session(corner, false);
    run(&mut s, "selection.move", json!({"glyph": "A", "points": [[0, 2]], "dx": 13, "dy": -8}));
    assert_eq!(xy(&s)[2], (200.0, 0.0));

    // Beyond a smooth end with a curve, that curve's handle turns.
    let mut s = session(
        json!([on(0, 0, "line", false), off(100, 100), on(200, 100, "qcurve", true), off(250, 100), off(350, 0), on(400, 0, "curve", false)]),
        false,
    );
    run(&mut s, "selection.move", json!({"glyph": "A", "points": [[0, 1]], "dx": 0, "dy": 100}));
    let p = xy(&s);
    let (a, b) = ((p[2].0 - p[1].0, p[2].1 - p[1].1), (p[3].0 - p[2].0, p[3].1 - p[2].1));
    assert!((a.0 * b.1 - a.1 * b.0).abs() / a.0.hypot(a.1) < 1.0 && b.1 < 0.0, "{p:?}");
}

/// The two handles of an implied node move as one, and a chain keeps its nodes.
#[test]
fn an_implied_node_moves_with_its_two_handles() {
    let mut s = session(json!([on(0, 0, "line", false), off(0, 100), off(100, 200), off(200, 100), on(200, 0, "qcurve", false)]), false);
    let implied = path(&s).implied_nodes();
    assert_eq!(implied.iter().map(|((a, b), p)| (*a, *b, p.x, p.y)).collect::<Vec<_>>(), [(1, 2, 50.0, 150.0), (2, 3, 150.0, 150.0)]);
    run(&mut s, "selection.move", json!({"glyph": "A", "points": [[0, 1], [0, 2]], "dx": 10, "dy": 5}));
    assert_eq!(path(&s).implied_nodes()[0].1, fontcraft_geom::Point::new(60.0, 155.0));
    assert_eq!((xy(&s)[0], xy(&s)[3], xy(&s)[4]), ((0.0, 0.0), (200.0, 100.0), (200.0, 0.0)));

    let blob = session(json!([off(0, 0), off(100, 0), off(100, 100), off(0, 100)]), true);
    assert_eq!(
        path(&blob).implied_nodes().iter().map(|(_, p)| (p.x, p.y)).collect::<Vec<_>>(),
        [(50.0, 0.0), (100.0, 50.0), (50.0, 100.0), (0.0, 50.0)]
    );
}

#[test]
fn deleting_and_copying_keep_quadratic_curves_and_fit_curve_says_why_not() {
    let mut s = ring();
    run(&mut s, "edit.delete", json!({"glyph": "A", "points": [[0, 2]]}));
    let kinds: Vec<NodeKind> = path(&s).nodes.iter().map(|n| n.kind).collect();
    assert!(kinds.iter().all(|k| matches!(k, NodeKind::QCurve | NodeKind::OffCurve)), "{kinds:?}");
    assert_eq!(kinds.iter().filter(|k| **k == NodeKind::QCurve).count(), 3);
    let handles = kinds.iter().filter(|k| **k == NodeKind::OffCurve).count();
    assert!((3..=8).contains(&handles), "the new segment follows the old curve with one quadratic segment: {kinds:?}");

    let mut s = session(json!([off(0, 0), off(100, 0), off(100, 100), off(0, 100)]), true);
    run(&mut s, "edit.delete", json!({"glyph": "A", "points": [[0, 1]]}));
    assert_eq!(xy(&s), [(0.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
    let copied = run(&mut s, "edit.copy", json!({"glyph": "A", "points": [[0, 0], [0, 1], [0, 2]]}));
    run(&mut s, "edit.delete", json!({"glyph": "A", "shapes": [0]}));
    run(&mut s, "edit.paste", json!({"glyph": "A", "json": copied["json"]}));
    assert!(path(&s).closed && path(&s).nodes.len() == 3, "the whole path is copied");

    let mut s = ring();
    let e = s.execute("fitCurve.applyStep", &json!({"glyph": "A", "points": [[0, 1]], "step": 3})).expect_err("quadratic").to_string();
    assert!(e.contains("quadratic"), "{e}");
}

// ── Corpus measure ──────────────────────────────────────────────────────────────────────────────

fn sources(dir: &FilePath, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.file_name().is_some_and(|n| n == ".git" || n == "fontc") {
            continue;
        }
        if p.extension().is_some_and(|x| x == "ttf") {
            out.push(p);
        } else if p.is_dir() {
            sources(&p, out);
        }
    }
}

/// How far the neighbours of the smooth node `k` are from one line through it; `None` when it is no smooth node beside
/// a handle.
fn kink(path: &Path, k: usize) -> Option<f64> {
    let n = path.nodes.len();
    let node = &path.nodes[k];
    if n < 3 || !node.on_curve() || !node.smooth {
        return None;
    }
    let (a, b) = (&path.nodes[(k + n - 1) % n], &path.nodes[(k + 1) % n]);
    if a.on_curve() && b.on_curve() {
        return None;
    }
    let (u, v) = (node.point() - a.point(), b.point() - node.point());
    (u.hypot() > 1e-9 && v.hypot() > 1e-9).then(|| u.cross(v).abs() / u.hypot().max(v.hypot()))
}

/// 1,000 random moves of handles and nodes of the closed quadratic paths of corpus TrueType fonts: node types stay, and
/// a smooth node that was smooth before is within 1 unit of smooth after.
#[test]
#[ignore = "reads the corpus"]
fn random_moves_of_quadratic_outlines_keep_types_and_smooth_nodes() {
    let root =
        std::env::var_os("FONTCRAFT_CORPUS").map_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../corpus")), PathBuf::from);
    let mut list = Vec::new();
    sources(&root, &mut list);
    list.sort();
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut random = |below: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % below.max(1) as u64) as usize
    };
    let (mut moves, mut bad) = (0usize, Vec::new());
    for file in list.iter().take(10) {
        let mut s = Session::new();
        if s.execute("file.open", &json!({"path": file})).is_err() {
            continue;
        }
        let found: Vec<(String, String, usize)> = {
            let font = s.font().unwrap();
            let closed = |shape: &Shape| matches!(shape, Shape::Path(p) if p.closed && p.nodes.iter().any(|n| n.kind == NodeKind::QCurve));
            let layers = font.glyphs.iter().filter(|g| !g.info.locked).flat_map(|g| g.layers.iter().map(move |l| (g, l)));
            layers
                .filter(|(_, l)| l.is_master_layer())
                .flat_map(|(g, l)| {
                    l.shapes.iter().enumerate().filter(|(_, shape)| closed(shape)).map(move |(i, _)| (g.name.to_string(), l.id.to_string(), i))
                })
                .collect()
        };
        for _ in 0..100 {
            if found.is_empty() {
                break;
            }
            let (glyph, layer, shape) = found[random(found.len())].clone();
            let path = |s: &Session| match &s.font().unwrap().glyph(&glyph).unwrap().layers.iter().find(|l| l.id == layer).unwrap().shapes[shape] {
                Shape::Path(p) => p.clone(),
                _ => unreachable!("the shape was a path"),
            };
            let before = path(&s);
            let node = random(before.nodes.len());
            let (dx, dy) = (random(31) as f64 - 15.0, random(31) as f64 - 15.0);
            let p = json!({"glyph": glyph, "layer": layer, "points": [[shape, node]], "dx": dx, "dy": dy});
            if (dx, dy) == (0.0, 0.0) || s.execute("selection.move", &p).is_err() {
                continue;
            }
            moves += 1;
            let after = path(&s);
            assert!(before.nodes.iter().zip(&after.nodes).all(|(a, b)| a.kind == b.kind), "{}: {glyph}", file.display());
            for k in 0..before.nodes.len() {
                if kink(&before, k).is_some_and(|d| d <= 1.0) && kink(&after, k).is_some_and(|d| d > 1.0) {
                    bad.push(format!("{}: {glyph} path {shape} node {k} after [{node}] by ({dx}, {dy})", file.display()));
                }
            }
            run(&mut s, "edit.undo", json!({}));
        }
    }
    println!("{} fonts, {moves} moves, {} kinks", list.len().min(10), bad.len());
    for line in bad.iter().take(40) {
        println!("  {line}");
    }
    assert!(bad.is_empty(), "{} smooth nodes kinked", bad.len());
}
