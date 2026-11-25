//! `matchLayers` of path.insertNode, edit.delete, path.makeNodeFirst and path.reverseContours (M9.35) and the node
//! commands "in All Masters" (M3.52) on a glyph built in code, and the corpus measures: a node added and deleted again
//! on the default master of every compatible glyph, and nodes added at random places in all masters.
//!
//! Measure: `cargo test --release -p fontcraft-engine --test engine match_layers:: -- --ignored --nocapture`.
//! `FONTCRAFT_CORPUS` names the corpus folder (default: the repository's `corpus/`).
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_model::{Axis, Font, Glyph, GlyphTable, Layer, Master, Node, NodeKind, Path, Shape};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn refused(s: &mut Session, id: &str, p: Value) -> String {
    let before = s.documents()[0].font.clone();
    let message = s.execute(id, &p).expect_err("a refusal").to_string();
    assert!(Arc::ptr_eq(&before, &s.documents()[0].font), "{id}: a refused command changed the font");
    message
}

/// A closed path with one curve from (0, 0) over the top to (w, 0), and a line back.
fn arch(w: f64) -> Path {
    let off = |x, y| Node::new(x, y, NodeKind::OffCurve);
    Path::new(true, vec![Node::new(0.0, 0.0, NodeKind::Line), off(0.0, 400.0), off(w, 400.0), Node::new(w, 0.0, NodeKind::Curve)])
}

fn layer(master: &str, id: &str, shapes: Vec<Path>) -> Layer {
    let mut l = Layer::new(master, 500.0);
    l.id = id.into();
    l.shapes = shapes.into_iter().map(Shape::Path).collect();
    l
}

/// Thin, Regular and Black; `a` has an arch and a box on Thin, Black and an intermediate layer, and a box alone on
/// Regular; `r` is a backup layer that could match.
fn session() -> Session {
    let drawing = |w: f64| vec![arch(w), Path::rect(0.0, 500.0, w, 600.0)];
    let mut f = Font::new("Match");
    f.axes = Arc::new(vec![Axis { name: "Weight".into(), tag: "wght".into(), ..Axis::default() }]);
    let master = |id: &str, w: f64| Master { id: id.into(), name: id.to_uppercase(), location: vec![w], ..Master::default() };
    f.masters = Arc::new(vec![master("thin", 100.0), master("regular", 400.0), master("black", 900.0)]);
    let mut brace = layer("thin", "brace", drawing(240.0));
    brace.info.attrs.coordinates = vec![Some(600.0)];
    let mut a = Glyph::new("a");
    a.layers = vec![
        layer("thin", "thin", drawing(100.0)),
        layer("regular", "regular", vec![Path::rect(0.0, 500.0, 200.0, 600.0)]),
        layer("black", "black", drawing(300.0)),
        brace,
        layer("thin", "r", drawing(100.0)),
    ];
    f.glyphs = GlyphTable::from_glyphs(vec![Arc::new(a)]);
    let mut s = Session::new();
    s.add_document(f, None).unwrap();
    s
}

fn path(s: &Session, id: &str, shape: usize) -> Path {
    let l = s.font().unwrap().glyph("a").unwrap().layers.iter().find(|l| l.id == id).unwrap();
    match &l.shapes[shape] {
        Shape::Path(p) => p.clone(),
        other => panic!("{other:?}"),
    }
}

fn kinds(s: &Session, id: &str) -> Vec<NodeKind> {
    path(s, id, 0).nodes.iter().map(|n| n.kind).collect()
}

#[test]
fn a_matched_edit_keeps_the_compatible_layers_compatible_in_one_undo_step() {
    let mut s = session();
    let before = s.documents()[0].font.clone();
    let r = run(&mut s, "path.insertNode", json!({"glyph": "a", "layer": "thin", "segment": [0, 3], "t": 0.5, "matchLayers": true}));
    assert_eq!(r["matched"], json!([{"glyph": "a", "layer": "black"}, {"glyph": "a", "layer": "brace"}]));
    assert_eq!(r["skipped"].as_array().unwrap().len(), 1, "{r}");
    assert_eq!((&r["skipped"][0]["layer"], &r["skipped"][0]["glyph"]), (&json!("regular"), &json!("a")));
    let node = r["node"][1].as_u64().unwrap() as usize;
    assert_eq!(kinds(&s, "thin").len(), 7);
    for id in ["black", "brace"] {
        assert_eq!(kinds(&s, id), kinds(&s, "thin"), "{id}");
    }
    assert_eq!((path(&s, "thin", 0).nodes[node].x, path(&s, "black", 0).nodes[node].x), (50.0, 150.0), "the same place along the segment");
    let g = s.font().unwrap().glyph("a").unwrap().clone();
    let old = before.glyph("a").unwrap();
    for id in ["regular", "r"] {
        let find = |g: &Glyph| g.layers.iter().find(|l| l.id == id).unwrap().clone();
        assert_eq!(find(&g), find(old), "{id} is left alone");
    }

    run(&mut s, "path.makeNodeFirst", json!({"glyph": "a", "layer": "black", "point": [0, node], "matchLayers": true}));
    assert_eq!(path(&s, "thin", 0).nodes[0].x, 50.0, "the same node starts every layer");
    run(&mut s, "path.reverseContours", json!({"glyph": "a", "layer": "black", "shapes": [1], "matchLayers": true}));
    assert_eq!(path(&s, "thin", 1).nodes[1].x, path(&s, "brace", 1).nodes[1].x, "both turned round");
    run(&mut s, "edit.delete", json!({"glyph": "a", "layer": "thin", "points": [[0, 0]], "matchLayers": true}));
    for id in ["black", "brace"] {
        assert_eq!(kinds(&s, id), kinds(&s, "thin"), "{id} after deleting");
    }
    run(&mut s, "edit.delete", json!({"glyph": "a", "layer": "thin", "shapes": [1], "matchLayers": true}));
    assert!(s.font().unwrap().glyph("a").unwrap().layers.iter().filter(|l| l.id != "regular" && l.id != "r").all(|l| l.shapes.len() == 1));
    for _ in 0..5 {
        run(&mut s, "edit.undo", json!({}));
    }
    assert_eq!(s.font().unwrap().glyph("a").unwrap().layers, old.layers, "one undo step per edit");
}

#[test]
fn a_refused_or_unmatched_edit_changes_no_other_layer() {
    let mut s = session();
    let message = refused(&mut s, "path.makeNodeFirst", json!({"glyph": "a", "layer": "thin", "point": [0, 1], "matchLayers": true}));
    assert!(message.contains("handle"), "{message}");
    refused(&mut s, "path.insertNode", json!({"glyph": "a", "layer": "thin", "segment": [0, 3], "t": 2, "matchLayers": true}));

    // Without the parameter, and on a backup layer, only the edited layer changes.
    let r = run(&mut s, "path.insertNode", json!({"glyph": "a", "layer": "thin", "segment": [0, 3]}));
    assert!(r.get("matched").is_none() && kinds(&s, "black").len() == 4);
    run(&mut s, "edit.undo", json!({}));
    let r = run(&mut s, "path.insertNode", json!({"glyph": "a", "layer": "r", "segment": [0, 3], "matchLayers": true}));
    assert_eq!((&r["matched"], &r["skipped"]), (&json!([]), &json!([])));
    assert_eq!((kinds(&s, "r").len(), kinds(&s, "thin").len()), (7, 4));
}

fn all_kinds(s: &Session, id: &str) -> Vec<(bool, Vec<NodeKind>)> {
    let l = s.font().unwrap().glyph("a").unwrap().layers.iter().find(|l| l.id == id).unwrap();
    l.shapes
        .iter()
        .map(|shape| match shape {
            Shape::Path(p) => (p.closed, p.nodes.iter().map(|n| n.kind).collect()),
            other => panic!("{other:?}"),
        })
        .collect()
}

/// Edits glyph `a` of `s` with path `shape` replaced on every layer but Regular by `f` of the layer's width.
fn redraw(s: &mut Session, shape: usize, f: impl Fn(f64) -> Path) {
    s.edit("Redraw", |font| {
        let mut g = font.glyph_mut("a")?;
        for (id, w) in [("thin", 100.0), ("black", 300.0), ("brace", 240.0), ("r", 100.0)] {
            let l = g.layers.iter_mut().find(|l| l.id == id).unwrap();
            l.shapes[shape] = Shape::Path(f(w));
        }
        Ok(())
    })
    .unwrap();
}

/// Runs an "in All Masters" command on Thin: Black and the intermediate layer stay compatible with it, Regular is left
/// alone and named, the backup layer is left alone, and one undo takes it all back (then redone).
fn in_all_masters(s: &mut Session, id: &str, mut params: Value) -> Value {
    let before = s.font().unwrap().glyph("a").unwrap().layers.clone();
    params["glyph"] = json!("a");
    params["layer"] = json!("thin");
    let r = run(s, id, params);
    assert_eq!(r["matched"], json!([{"glyph": "a", "layer": "black"}, {"glyph": "a", "layer": "brace"}]), "{id}");
    let skipped = r["skipped"].as_array().unwrap();
    assert!(skipped.len() == 1 && skipped[0]["layer"] == "regular" && skipped[0]["reason"].as_str().unwrap().starts_with("REGULAR: "), "{id}: {r}");
    let after = s.font().unwrap().glyph("a").unwrap().layers.clone();
    for other in ["black", "brace"] {
        assert_eq!(all_kinds(s, other), all_kinds(s, "thin"), "{id}: {other}");
    }
    for (old, new) in before.iter().zip(&after) {
        assert_eq!(old == new, ["regular", "r"].contains(&old.id.as_str()), "{id}: {}", old.id);
    }
    run(s, "edit.undo", json!({}));
    assert_eq!(s.font().unwrap().glyph("a").unwrap().layers, before, "{id}: one undo step");
    run(s, "edit.redo", json!({}));
    r
}

#[test]
fn node_commands_in_all_masters_keep_the_layers_compatible_in_one_undo_step() {
    for (id, params) in [
        ("edit.deleteInAllMasters", json!({"points": [[1, 1]]})),
        ("path.deleteAndBreakInAllMasters", json!({"points": [[1, 1]]})),
        ("path.toggleNodeTypeInAllMasters", json!({"points": [[1, 1]]})),
        ("path.convertSegmentToCurveInAllMasters", json!({"segments": [[1, 1]]})),
        ("path.insertNodeInAllMasters", json!({"segment": [0, 3], "t": 0.25})),
        ("path.makeNodeFirstInAllMasters", json!({"point": [0, 3]})),
        ("path.alignSelectionInAllMasters", json!({"points": [[0, 3], [1, 0]]})),
        // The line ends inside Black's box, which is cut at the places where Thin's is.
        ("path.cutWithLineInAllMasters", json!({"from": [-20, 550], "to": [120, 550]})),
    ] {
        in_all_masters(&mut session(), id, params);
    }
    let mut s = session();
    in_all_masters(&mut s, "path.cutWithLineInAllMasters", json!({"from": [-20, 550], "to": [120, 550]}));
    assert_eq!(all_kinds(&s, "black").len(), 3, "the box is in two");

    let mut s = session();
    let opened = in_all_masters(&mut s, "path.openCornerInAllMasters", json!({"points": [[1, 1]]}));
    assert_eq!(all_kinds(&s, "black")[1].1.len(), 5);
    in_all_masters(&mut s, "path.reconnectNodesInAllMasters", json!({"points": opened["selection"]["points"]}));
    assert_eq!(path(&s, "black", 1), path(&session(), "black", 1), "the corner is back");
}

/// The ends of a path meet on Thin only, so every layer gets a closing segment; a node that is smooth on one layer
/// already ends up smooth on all.
#[test]
fn what_differs_between_the_drawings_is_settled_for_all_of_them() {
    let open = |w: f64| {
        let line = |x, y| Node::new(x, y, NodeKind::Line);
        Path::new(false, vec![line(0.0, 0.0), line(w, 0.0), line(w, 100.0), line(w - 100.0, 0.0)])
    };
    for (id, params) in
        [("path.closeOpenPathsInAllMasters", json!({"shapes": [1]})), ("path.connectNodesInAllMasters", json!({"points": [[1, 0], [1, 3]]}))]
    {
        let mut s = session();
        redraw(&mut s, 1, open);
        in_all_masters(&mut s, id, params);
        assert_eq!(all_kinds(&s, "thin")[1], (true, vec![NodeKind::Line; 4]), "{id}");
    }

    let mut s = session();
    redraw(&mut s, 0, |w| {
        let mut p = arch(w);
        p.nodes[3].smooth = w > 250.0;
        p
    });
    in_all_masters(&mut s, "path.toggleNodeTypeInAllMasters", json!({"points": [[0, 3]]}));
    assert!(["thin", "black", "brace"].iter().all(|id| path(&s, id, 0).nodes.iter().filter(|n| n.smooth).count() == 1));
}

#[test]
fn an_edit_one_master_cannot_take_is_refused_whole() {
    let mut s = session();
    redraw(&mut s, 1, |w| {
        let mut p = Path::rect(0.0, 500.0, w, 600.0);
        p.nodes[1].smooth = w > 250.0;
        p
    });
    let message = refused(&mut s, "path.openCornerInAllMasters", json!({"glyph": "a", "layer": "thin", "points": [[1, 1]]}));
    assert!(message.contains("BLACK"), "{message}");
    refused(&mut s, "path.reconnectNodesInAllMasters", json!({"glyph": "a", "layer": "thin", "points": [[1, 0]]}));
    // Each layer lines up on its own coordinate; anchors are matched by name.
    s.edit("Anchors", |font| {
        let mut g = font.glyph_mut("a")?;
        for l in g.layers.iter_mut().filter(|l| l.id != "brace") {
            l.anchors.push(fontcraft_model::Anchor::new("top", 10.0, 700.0));
        }
        Ok(())
    })
    .unwrap();
    let r = run(&mut s, "path.alignSelectionInAllMasters", json!({"glyph": "a", "layer": "thin", "points": [[1, 1]], "anchors": ["top"]}));
    assert_eq!((r["matched"].as_array().unwrap().len(), r["skipped"].as_array().unwrap().len()), (1, 2), "{r}");
    let top = |id: &str| s.font().unwrap().glyph("a").unwrap().layers.iter().find(|l| l.id == id).unwrap().anchors[0].x;
    assert_eq!((top("thin"), top("black"), top("regular")), (path(&s, "thin", 1).nodes[1].x, path(&s, "black", 1).nodes[1].x, 10.0));
}

// ── Corpus measure ──────────────────────────────────────────────────────────────────────────────

fn corpus() -> PathBuf {
    std::env::var_os("FONTCRAFT_CORPUS").map_or_else(|| FsPath::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus"), PathBuf::from)
}

fn incompatible(s: &mut Session, glyphs: &[String]) -> Vec<String> {
    let r = s.query("interpolation.checkCompatibility", &json!({"glyphs": glyphs, "enforceAll": true})).unwrap();
    r["incompatible"].as_array().unwrap().iter().map(|g| g.as_str().unwrap().to_string()).collect()
}

/// Adds a node on the first path of the default master's layer of every compatible glyph, checks, deletes it again and
/// checks. Returns the glyphs that did not stay compatible, and how many were edited.
fn measure(path: &FsPath) -> (Vec<String>, usize) {
    let mut s = Session::new();
    s.execute("file.open", &json!({"path": path})).unwrap();
    let font = s.font().unwrap();
    let master = font.default_master().unwrap().id.clone();
    let mut edits: Vec<(String, usize, usize)> = Vec::new();
    for g in font.glyphs.iter().filter(|g| !g.info.locked) {
        let Some(l) = g.layer(&master) else { continue };
        let segment = l.shapes.iter().enumerate().find_map(|(i, shape)| match shape {
            Shape::Path(p) => p.segments().last().map(|seg| (i, seg.end)),
            _ => None,
        });
        if let Some((shape, end)) = segment {
            edits.push((g.name.clone(), shape, end));
        }
    }
    let names: Vec<String> = edits.iter().map(|e| e.0.clone()).collect();
    let before = incompatible(&mut s, &names);
    edits.retain(|e| !before.contains(&e.0));
    let names: Vec<String> = edits.iter().map(|e| e.0.clone()).collect();
    let mut broken = Vec::new();
    let mut added: Vec<(String, Value)> = Vec::new();
    for (glyph, shape, end) in &edits {
        let params = json!({"glyph": glyph, "layer": master, "segment": [shape, end], "matchLayers": true});
        match s.execute("path.insertNode", &params) {
            Ok(r) => added.push((glyph.clone(), r["node"].clone())),
            Err(e) => broken.push(format!("{glyph}: {e}")),
        }
    }
    broken.extend(incompatible(&mut s, &names).into_iter().map(|g| format!("{g} after adding")));
    for (glyph, node) in &added {
        if let Err(e) = s.execute("edit.delete", &json!({"glyph": glyph, "layer": master, "points": [node], "matchLayers": true})) {
            broken.push(format!("{glyph}: {e}"));
        }
    }
    broken.extend(incompatible(&mut s, &names).into_iter().map(|g| format!("{g} after deleting")));
    eprintln!("{}: {} glyphs edited, {} not compatible or refused", path.display(), edits.len(), broken.len());
    (broken, edits.len())
}

#[test]
#[ignore = "corpus measure"]
fn corpus_glyphs_stay_compatible_after_a_matched_add_and_delete() {
    let root = corpus();
    let sources = [root.join("oswald/sources/Oswald.glyphs"), root.join("inter/src/Inter-Roman.glyphspackage")];
    for path in sources.iter().filter(|p| p.exists()) {
        let (broken, _) = measure(path);
        assert!(broken.is_empty(), "{}: {:?}", path.display(), &broken[..broken.len().min(20)]);
    }
}

/// `path.insertNodeInAllMasters` at 100 random places of compatible glyphs: every glyph stays compatible.
#[test]
#[ignore = "corpus measure"]
fn corpus_glyphs_stay_compatible_after_random_insertions_in_all_masters() {
    let root = corpus();
    let sources = [root.join("oswald/sources/Oswald.glyphs"), root.join("inter/src/Inter-Roman.glyphspackage")];
    for path in sources.iter().filter(|p| p.exists()) {
        let mut s = Session::new();
        s.execute("file.open", &json!({"path": path})).unwrap();
        let font = s.font().unwrap();
        let master = font.default_master().unwrap().id.clone();
        let mut segments: Vec<(String, usize, usize)> = Vec::new();
        for g in font.glyphs.iter().filter(|g| !g.info.locked) {
            for (i, shape) in g.layer(&master).iter().flat_map(|l| l.shapes.iter().enumerate()) {
                if let Shape::Path(p) = shape {
                    segments.extend(p.segments().iter().map(|seg| (g.name.clone(), i, seg.end)));
                }
            }
        }
        let names: Vec<String> = segments.iter().map(|e| e.0.clone()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
        let before = incompatible(&mut s, &names);
        segments.retain(|e| !before.contains(&e.0));
        let mut seed = 0x9e37_79b9_u64;
        let mut next = || {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) as usize
        };
        let start = std::time::Instant::now();
        for _ in 0..100 {
            let (glyph, shape, end) = segments[next() % segments.len()].clone();
            let t = 0.05 + (next() % 900) as f64 / 1000.0;
            // Earlier insertions move the addresses of later nodes: one that ends no segment any more is refused.
            let _ = s.execute("path.insertNodeInAllMasters", &json!({"glyph": glyph, "layer": master, "segment": [shape, end], "t": t}));
        }
        eprintln!("{}: 100 insertions in {:?}", path.display(), start.elapsed());
        let broken: Vec<String> = incompatible(&mut s, &names).into_iter().filter(|g| !before.contains(g)).collect();
        assert!(broken.is_empty(), "{}: {broken:?}", path.display());
    }
}
