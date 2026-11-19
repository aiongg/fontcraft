//! Match Masters and Relocate Start Points (M9.34) on a three-master glyph built in code, and the
//! corpus measure: one master of every compatible glyph scrambled with a fixed seed, then
//! matched back.
//!
//! Measure: `cargo test --release -p fontcraft-engine --test engine match_masters:: -- --ignored --nocapture`.
//! `FONTCRAFT_CORPUS` names the corpus folder (default: the repository's `corpus/`).
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_model::{Component, Font, Glyph, GlyphTable, Layer, Master, Node, NodeKind, Path, Shape};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

/// A closed cubic ring of radius `r` around `(cx, cy)`, starting at its bottom node.
fn ring(cx: f64, cy: f64, r: f64) -> Path {
    let k = 0.55 * r;
    let on = |x, y| Node::new(x, y, NodeKind::Curve);
    let off = |x, y| Node::new(x, y, NodeKind::OffCurve);
    Path::new(
        true,
        vec![
            on(cx, cy - r),
            off(cx + k, cy - r),
            off(cx + r, cy - k),
            on(cx + r, cy),
            off(cx + r, cy + k),
            off(cx + k, cy + r),
            on(cx, cy + r),
            off(cx - k, cy + r),
            off(cx - r, cy + k),
            on(cx - r, cy),
            off(cx - r, cy - k),
            off(cx - k, cy - r),
        ],
    )
}

/// One master's drawing: an outline `width` wide with a counter `stem` in from each side (drawn
/// clockwise), an acute, a ring above and a grave.
fn drawing(width: f64, stem: f64, radius: f64) -> Vec<Shape> {
    let mut counter = Path::rect(stem, stem, width - stem, 700.0 - stem);
    counter.reverse();
    vec![
        Shape::Path(Path::rect(0.0, 0.0, width, 700.0)),
        Shape::Path(counter),
        Shape::Component(Component::new("acute", width / 2.0, 750.0)),
        Shape::Path(ring(width / 2.0, 850.0, radius)),
        Shape::Component(Component::new("grave", width / 4.0, 750.0)),
    ]
}

fn master_layer(id: &str, width: f64, shapes: Vec<Shape>) -> Layer {
    let mut l = Layer::new(id, width);
    l.shapes = shapes;
    l
}

/// Light, Regular and Bold masters (`l`, `r`, `b`) and the glyph `o` drawn compatibly in each.
fn session() -> Session {
    let mut f = Font::new("Match Test");
    let template = f.masters[0].clone();
    f.masters = Arc::new(
        [("l", "Light"), ("r", "Regular"), ("b", "Bold")]
            .iter()
            .map(|(id, name)| Master { id: (*id).into(), name: (*name).into(), ..template.clone() })
            .collect(),
    );
    let mut o = Glyph::new("o");
    o.layers = vec![
        master_layer("l", 400.0, drawing(400.0, 40.0, 30.0)),
        master_layer("r", 500.0, drawing(500.0, 90.0, 45.0)),
        master_layer("b", 600.0, drawing(600.0, 160.0, 70.0)),
    ];
    let marks = ["acute", "grave"].map(|n| {
        let mut g = Glyph::new(n);
        g.layers = ["l", "r", "b"].iter().map(|m| master_layer(m, 0.0, Vec::new())).collect();
        g
    });
    f.glyphs = GlyphTable::from_glyphs([o].into_iter().chain(marks).map(Arc::new).collect());
    let mut s = Session::new();
    s.add_document(f, None).unwrap();
    s
}

fn shapes(s: &Session, glyph: &str, layer: &str) -> Vec<Shape> {
    s.font().unwrap().glyph(glyph).unwrap().layers.iter().find(|l| l.id == layer).unwrap().shapes.clone()
}

fn set_shapes(s: &mut Session, glyph: &str, layer: &str, shapes: Vec<Shape>) {
    s.edit("setup", |font| {
        let mut g = font.glyph_mut(glyph)?;
        g.layers.iter_mut().find(|l| l.id == layer).unwrap().shapes = shapes;
        Ok(())
    })
    .unwrap();
}

fn path_of(shape: &Shape) -> Path {
    match shape {
        Shape::Path(p) => p.clone(),
        other => panic!("not a path: {other:?}"),
    }
}

#[test]
fn a_scrambled_master_is_matched_back_to_its_original_order() {
    let mut s = session();
    let original = shapes(&s, "o", "b");
    // Bold scrambled: shapes reordered, the outline turned clockwise and restarted, the counter
    // restarted, the ring turned and restarted.
    let mut outline = path_of(&original[0]);
    outline.reverse();
    outline.nodes.rotate_left(1);
    let mut counter = path_of(&original[1]);
    counter.nodes.rotate_left(2);
    let mut ring = path_of(&original[3]);
    ring.reverse();
    let last_on = ring.nodes.iter().rposition(Node::on_curve).unwrap();
    ring.nodes.rotate_left(last_on);
    let scrambled = vec![original[4].clone(), Shape::Path(ring), Shape::Path(outline), original[2].clone(), Shape::Path(counter)];
    set_shapes(&mut s, "o", "b", scrambled);
    let regular = shapes(&s, "o", "r");

    let r = run(&mut s, "glyph.matchMasters", json!({"glyph": "o"}));
    assert_eq!(r["glyphs"], json!(["o"]), "{r}");
    assert_eq!(r["skipped"], json!([]), "{r}");
    assert_eq!(shapes(&s, "o", "b"), original, "Bold is back in its original order, directions and starts");
    assert_eq!(shapes(&s, "o", "r"), regular, "a compatible master is left alone");

    // A start moved on a rectangle keeps the masters compatible: only strong matching fixes it.
    let mut moved = regular.clone();
    let mut outer = path_of(&moved[0]);
    outer.nodes.rotate_left(1);
    moved[0] = Shape::Path(outer);
    set_shapes(&mut s, "o", "r", moved.clone());
    let r = run(&mut s, "glyph.matchMasters", json!({"glyph": "o"}));
    assert_eq!(r["glyphs"], json!([]), "{r}");
    assert_eq!(shapes(&s, "o", "r"), moved);
    let r = run(&mut s, "glyph.matchMasters", json!({"glyphs": ["o"], "masters": ["r"], "strong": true}));
    assert_eq!(r["glyphs"], json!(["o"]), "{r}");
    assert_eq!(shapes(&s, "o", "r"), regular);
}

#[test]
fn a_layer_with_a_different_node_count_is_named_and_left_alone() {
    let mut s = session();
    let mut bold = shapes(&s, "o", "b");
    let mut outline = path_of(&bold[0]);
    outline.nodes.insert(1, Node::new(300.0, 0.0, NodeKind::Line));
    bold[0] = bold[1].clone();
    bold[1] = Shape::Path(outline);
    set_shapes(&mut s, "o", "b", bold.clone());
    let before = s.font().unwrap().glyph("o").unwrap().clone();

    let r = run(&mut s, "glyph.matchMasters", json!({"glyph": "o", "strong": true}));
    assert_eq!(r["glyphs"], json!([]), "{r}");
    let skipped = r["skipped"].as_array().unwrap();
    assert_eq!(skipped.len(), 1, "{r}");
    assert_eq!(skipped[0]["glyph"], "o");
    assert_eq!(skipped[0]["layer"], "b");
    assert!(skipped[0]["reason"].as_str().unwrap().contains("path"), "{r}");
    assert_eq!(**s.font().unwrap().glyph("o").unwrap(), *before, "nothing changed");
}

#[test]
fn relocate_start_points_starts_closed_paths_nearest_the_bottom_left() {
    let mut s = session();
    let mut light = shapes(&s, "o", "l");
    let mut outline = path_of(&light[0]);
    outline.nodes.rotate_left(2);
    light[0] = Shape::Path(outline);
    let open = Path::new(false, vec![Node::new(300.0, 300.0, NodeKind::Line), Node::new(100.0, 100.0, NodeKind::Line)]);
    light.push(Shape::Path(open.clone()));
    set_shapes(&mut s, "o", "l", light);

    let r = run(&mut s, "glyph.relocateStartPoints", json!({"glyph": "o", "masters": ["l"]}));
    assert_eq!(r["glyphs"], json!(["o"]), "{r}");
    let after = shapes(&s, "o", "l");
    let first = |i: usize| {
        let n = &path_of(&after[i]).nodes[0];
        (n.x, n.y)
    };
    assert_eq!(first(0), (0.0, 0.0), "the outline starts at the corner");
    assert_eq!(first(1), (40.0, 40.0), "the counter starts at its node nearest the corner");
    assert_eq!(path_of(&after[5]), open, "an open path keeps its start");
    let r = run(&mut s, "glyph.relocateStartPoints", json!({"glyph": "o", "masters": ["l"]}));
    assert_eq!(r["glyphs"], json!([]), "a second run changes nothing");
}

// ── Corpus measure ──────────────────────────────────────────────────────────────────────────────

fn corpus() -> PathBuf {
    std::env::var_os("FONTCRAFT_CORPUS").map_or_else(|| FsPath::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus"), PathBuf::from)
}

/// xorshift64*: the same scramble on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Same shapes in the same order, node kind for node kind.
fn compatible(a: &[Shape], b: &[Shape]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| match (x, y) {
            (Shape::Path(p), Shape::Path(q)) => {
                p.closed == q.closed && p.nodes.len() == q.nodes.len() && p.nodes.iter().zip(&q.nodes).all(|(m, n)| m.kind == n.kind)
            }
            (Shape::Component(c), Shape::Component(d)) => c.glyph == d.glyph,
            _ => false,
        })
}

/// Shuffles the shapes, turns about half the closed paths round and restarts every closed path
/// at a random on-curve node.
fn scramble(shapes: &mut [Shape], rng: &mut Rng) {
    for i in (1..shapes.len()).rev() {
        let j = rng.below(i + 1);
        shapes.swap(i, j);
    }
    for shape in shapes.iter_mut() {
        let Shape::Path(p) = shape else { continue };
        if !p.closed {
            continue;
        }
        if rng.below(2) == 1 {
            p.reverse();
        }
        let on: Vec<usize> = p.nodes.iter().enumerate().filter(|(_, n)| n.on_curve()).map(|(i, _)| i).collect();
        if !on.is_empty() {
            let k = on[rng.below(on.len())];
            p.nodes.rotate_left(k);
        }
    }
}

/// Scrambles the first non-default master of every compatible glyph and matches it back.
/// Returns (restored, scrambled).
fn measure(path: &FsPath) -> (usize, usize) {
    let mut s = Session::new();
    s.execute("file.open", &json!({"path": path})).unwrap();
    let font = s.font().unwrap();
    let reference = font.default_master().unwrap().id.clone();
    let Some(target) = font.masters.iter().find(|m| m.id != reference).map(|m| m.id.clone()) else { return (0, 0) };
    let mut originals: Vec<(String, Vec<Shape>)> = Vec::new();
    for g in font.glyphs.iter() {
        if g.info.locked {
            continue;
        }
        let layer = |id: &str| g.layers.iter().find(|l| l.id == id);
        let (Some(a), Some(b)) = (layer(&reference), layer(&target)) else { continue };
        let usable = b.shapes.iter().all(|s| matches!(s, Shape::Path(_) | Shape::Component(_)));
        if usable && b.shapes.iter().any(|s| matches!(s, Shape::Path(_))) && compatible(&a.shapes, &b.shapes) {
            originals.push((g.name.clone(), b.shapes.clone()));
        }
    }
    let mut rng = Rng(0x5eed_0f3a_7c01);
    let scrambled: Vec<(String, Vec<Shape>)> = originals
        .iter()
        .map(|(name, shapes)| {
            let mut shapes = shapes.clone();
            scramble(&mut shapes, &mut rng);
            (name.clone(), shapes)
        })
        .collect();
    s.edit("scramble", |font| {
        for (name, shapes) in &scrambled {
            let mut g = font.glyph_mut(name)?;
            g.layers.iter_mut().find(|l| l.id == target).unwrap().shapes = shapes.clone();
        }
        Ok(())
    })
    .unwrap();
    let names: Vec<&str> = originals.iter().map(|(n, _)| n.as_str()).collect();
    let r = s.execute("glyph.matchMasters", &json!({"glyphs": names, "masters": [target], "strong": true})).unwrap();
    let font = s.font().unwrap();
    let mut restored = 0;
    let mut failed = Vec::new();
    for (name, shapes) in &originals {
        let now = &font.glyph(name).unwrap().layers.iter().find(|l| l.id == target).unwrap().shapes;
        if now == shapes {
            restored += 1;
        } else if failed.len() < 20 {
            failed.push(name.clone());
        }
    }
    eprintln!(
        "{}: {restored} of {} restored; skipped {}; first failures {failed:?}",
        path.display(),
        originals.len(),
        r["skipped"].as_array().map_or(0, Vec::len)
    );
    (restored, originals.len())
}

#[test]
#[ignore = "corpus measure"]
fn scrambled_corpus_masters_are_matched_back() {
    let root = corpus();
    let sources = [root.join("oswald/sources/Oswald.glyphs"), root.join("inter/src/Inter-Roman.glyphspackage")];
    let mut failures = Vec::new();
    for path in sources.iter().filter(|p| p.exists()) {
        let (restored, total) = measure(path);
        if total > 0 && (restored as f64) < 0.95 * total as f64 {
            failures.push(format!("{}: {restored} of {total}", path.display()));
        }
    }
    assert!(failures.is_empty(), "fewer than 95% restored: {failures:?}");
}
