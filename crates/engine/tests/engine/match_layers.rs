//! `matchLayers` of path.insertNode, edit.delete, path.makeNodeFirst and path.reverseContours (M9.35) on a glyph built
//! in code, and the corpus measure: a node added and deleted again on the default master of every compatible glyph.
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
