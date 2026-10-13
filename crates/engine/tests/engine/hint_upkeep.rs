//! Node-reference upkeep (M10.3): after an outline edit, PostScript hints and TrueType
//! instructions stay on the same nodes, in the command's own undo step.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::model::{Layer, Node, NodeKind, Path, Shape};
use fontcraft_hinting::{
    Direction, Edge, NodeOrder, NodeRef, PsHint, StemRef, TtInstruction, TtKind, TtPoint, ZoneRef, ps_hints, set_ps_hints, set_tt_instructions,
    tt_instructions,
};
use serde_json::{Value, json};

fn line(x: f64, y: f64) -> Node {
    Node::new(x, y, NodeKind::Line)
}

fn node(path: usize, node: usize) -> Edge {
    Edge::Node(NodeRef::new(path, node))
}

fn tt_node(path: usize, node: usize) -> TtPoint {
    TtPoint::Node(NodeRef::new(path, node))
}

/// Glyph `I` with `paths` on master `m01`, the PostScript hints `ps` and the instructions `tt`.
fn session(paths: Vec<Path>, ps: &[PsHint], tt: &[TtInstruction]) -> Session {
    let mut s = Session::with_document();
    s.edit("setup", |font| {
        let mut g = font.glyph_mut("I")?;
        let layer = g.layer_mut("m01").unwrap();
        layer.shapes = paths.into_iter().map(Shape::Path).collect();
        set_ps_hints(layer, ps).unwrap();
        set_tt_instructions(layer, tt).unwrap();
        Ok(())
    })
    .unwrap();
    s
}

fn layer(s: &Session) -> Layer {
    s.font().unwrap().glyph("I").unwrap().layer("m01").unwrap().clone()
}

fn ps(s: &Session) -> Vec<PsHint> {
    ps_hints(&layer(s)).unwrap().into_iter().map(|(_, h)| h).collect()
}

fn tt(s: &Session) -> Vec<TtInstruction> {
    tt_instructions(&layer(s)).unwrap().into_iter().map(|(_, t)| t).collect()
}

/// Where a TrueType node point lies on the current outline.
fn tt_place(s: &Session, p: TtPoint) -> (f64, f64) {
    let TtPoint::Node(r) = p else { panic!("{p:?} is not a node") };
    let l = layer(s);
    let n = r.check(&l).unwrap();
    (n.x, n.y)
}

/// Runs `id`, checks that undo gives back the very font, runs it again and returns its result.
fn run_undoable(s: &mut Session, id: &str, p: Value) -> Value {
    let before = s.doc().unwrap().font.clone();
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    assert!(!Arc::ptr_eq(&s.doc().unwrap().font, &before), "{id} changed nothing");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "{id}: undo does not give back the very font");
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// A stem from 100 to 200 by 0 to 700, nodes (100, 0), (200, 0), (200, 700), (100, 700); a
/// horizontal stem hint from node 0 to node 3, a Snap on node 2.
fn stem() -> Session {
    session(
        vec![Path::rect(100.0, 0.0, 200.0, 700.0)],
        &[PsHint::stem(true, node(0, 0), node(0, 3))],
        &[TtInstruction::new(TtKind::Snap { zone: ZoneRef::Auto }, Direction::Vertical, tt_node(0, 2))],
    )
}

#[test]
fn inserting_a_node_before_a_linked_one_shifts_the_reference() {
    let mut s = stem();
    run_undoable(&mut s, "path.insertNode", json!({"glyph": "I", "segment": [0, 1]}));
    assert_eq!(layer(&s).shapes.len(), 1);
    let h = &ps(&s)[0];
    assert_eq!((h.origin, h.other), (node(0, 0), Some(node(0, 4))));
    assert_eq!(tt(&s)[0].target, tt_node(0, 3));
    assert_eq!(tt_place(&s, tt(&s)[0].target), (200.0, 700.0));
}

/// Stored values count a closed path's start node last, so a reference to it is stored anew when
/// the path gains a node, although it stays on the same node.
#[test]
fn a_reference_to_the_start_node_follows_the_node_count_of_its_closed_path() {
    let mut s = session(vec![Path::rect(100.0, 0.0, 200.0, 700.0)], &[PsHint::stem(false, node(0, 0), node(0, 1))], &[]);
    run_undoable(&mut s, "path.insertNode", json!({"glyph": "I", "segment": [0, 2]}));
    let h = &ps(&s)[0];
    assert_eq!((h.origin, h.other), (node(0, 0), Some(node(0, 1))));
    assert_eq!(h.edges(&layer(&s)).unwrap(), (100.0, 200.0));
    let stored = layer(&s).info.hints[0].value.as_dict().unwrap().get("origin").cloned().unwrap();
    assert_eq!(NodeRef::from_value(&stored, &NodeOrder::default()), Some(NodeRef::new(0, 4)), "the start node is stored last");
}

#[test]
fn deleting_a_linked_node_unlinks_postscript_ends_and_removes_instructions() {
    let mut s = stem();
    let out = run_undoable(&mut s, "edit.delete", json!({"glyph": "I", "points": [[0, 0], [0, 2]]}));
    let h = &ps(&s)[0];
    assert_eq!((h.origin, h.other), (Edge::Value(0.0), Some(node(0, 1))), "the deleted end keeps its last y");
    assert!(tt(&s).is_empty(), "the Snap on the deleted node is removed");
    let upkeep = &out["hintUpkeep"];
    assert_eq!(upkeep["unlinked"], json!(1), "{out}");
    let removed = upkeep["removed"].as_array().unwrap();
    assert_eq!(removed.len(), 1, "{out}");
    assert!(removed[0].as_str().unwrap().contains("Snap"), "{out}");
}

#[test]
fn reversing_a_contour_remaps_the_references() {
    let mut s = stem();
    run_undoable(&mut s, "path.reverseContours", json!({"glyph": "I", "shapes": [0]}));
    let h = &ps(&s)[0];
    let l = layer(&s);
    assert_eq!(h.edges(&l).unwrap(), (0.0, 700.0));
    assert_eq!(h.other, Some(node(0, 1)), "(100, 700) is the second node of the reversed path");
    assert_eq!(tt_place(&s, tt(&s)[0].target), (200.0, 700.0));
}

#[test]
fn changing_the_start_point_remaps_the_references() {
    let mut s = stem();
    run_undoable(&mut s, "path.makeNodeFirst", json!({"glyph": "I", "point": [0, 2]}));
    let h = &ps(&s)[0];
    assert_eq!((h.origin, h.other), (node(0, 2), Some(node(0, 1))));
    assert_eq!(tt(&s)[0].target, tt_node(0, 0));
}

#[test]
fn remove_overlap_remaps_by_place_and_moves_instructions_to_the_new_crossings() {
    // A stem and a bar from x 150 to 300 by y 300 to 400: the bar's left nodes lie inside the
    // stem and go; the crossings (200, 300) and (200, 400) are new nodes.
    let bar = Path::new(true, vec![line(150.0, 300.0), line(300.0, 300.0), line(300.0, 400.0), line(150.0, 400.0)]);
    let mut s = session(
        vec![Path::rect(100.0, 0.0, 200.0, 700.0), bar],
        &[PsHint::stem(true, node(0, 0), node(0, 3)), PsHint::stem(true, node(1, 1), node(1, 3))],
        &[
            TtInstruction::new(TtKind::Snap { zone: ZoneRef::Auto }, Direction::Vertical, tt_node(1, 0)),
            TtInstruction::new(
                TtKind::Snap { zone: ZoneRef::Auto },
                Direction::Vertical,
                TtPoint::Intersection { a: NodeRef::new(0, 2), b: NodeRef::new(1, 1) },
            ),
            TtInstruction::new(TtKind::Snap { zone: ZoneRef::Auto }, Direction::Vertical, tt_node(0, 2)),
        ],
    );
    let out = run_undoable(&mut s, "path.removeOverlap", json!({"glyph": "I"}));
    assert_eq!(layer(&s).shapes.len(), 1);
    let l = layer(&s);
    let hints = ps(&s);
    assert_eq!(hints[0].edges(&l).unwrap(), (0.0, 700.0), "linked ends follow their nodes into the merged path");
    assert!(matches!(hints[0].origin, Edge::Node(_)) && matches!(hints[0].other, Some(Edge::Node(_))));
    assert_eq!(hints[1].other, Some(Edge::Value(400.0)), "the removed node's end becomes its last y");
    assert_eq!(hints[1].edges(&l).unwrap(), (300.0, 400.0));
    let t = tt(&s);
    assert_eq!(t.len(), 3, "{out}");
    assert_eq!(tt_place(&s, t[0].target), (200.0, 300.0), "the removed node's instruction moves to the nearest crossing");
    assert_eq!(tt_place(&s, t[1].target), (200.0, 300.0), "the intersection becomes the crossing's node");
    assert_eq!(tt_place(&s, t[2].target), (200.0, 700.0));
    assert_eq!(out["hintUpkeep"]["moved"], json!(2), "{out}");
}

#[test]
fn expand_outline_unlinks_the_stroked_path_and_keeps_the_others() {
    // Path 0 is an open stroked line; path 1 a stem whose hints must follow it.
    let stroke = Path::new(false, vec![line(400.0, 100.0), line(400.0, 600.0)]);
    let mut s = session(
        vec![stroke, Path::rect(100.0, 0.0, 200.0, 700.0)],
        &[PsHint::stem(false, node(0, 0), node(1, 1)), PsHint::stem(true, node(1, 0), node(1, 3))],
        &[TtInstruction {
            origin: Some(tt_node(1, 0)),
            ..TtInstruction::new(TtKind::Stem { stem: StemRef::Auto, triple: None }, Direction::Horizontal, tt_node(0, 1))
        }],
    );
    s.execute("path.setStroke", &json!({"glyph": "I", "shapes": [0], "width": 40})).unwrap();
    let out = run_undoable(&mut s, "path.expandOutline", json!({"glyph": "I"}));
    let l = layer(&s);
    let hints = ps(&s);
    assert_eq!(hints[0].origin, Edge::Value(400.0), "the stroked line's node is gone: its x stays");
    assert_eq!(hints[0].edges(&l).unwrap(), (200.0, 400.0));
    assert_eq!(hints[1].edges(&l).unwrap(), (0.0, 700.0));
    assert!(tt(&s).is_empty(), "{out}");
    assert_eq!(out["hintUpkeep"]["removed"].as_array().map(Vec::len), Some(1), "{out}");
}

#[test]
fn moving_nodes_keeps_the_references_as_they_are() {
    // (200, 700) moves onto no other node's place; the structure is unchanged.
    let mut s = stem();
    let before = layer(&s).info.hints.clone();
    s.execute("selection.move", &json!({"glyph": "I", "points": [[0, 2], [0, 3]], "dx": 0, "dy": 50})).unwrap();
    assert_eq!(layer(&s).info.hints, before);
    assert_eq!(ps(&s)[0].edges(&layer(&s)).unwrap(), (0.0, 750.0));
}

/// Moving the stem up by its own height puts its new bottom nodes on the old top places; the
/// links stay on the nodes that moved.
#[test]
fn moving_a_contour_by_the_gap_between_its_nodes_keeps_every_link_on_its_node() {
    let mut s = stem();
    let before = layer(&s).info.hints.clone();
    s.execute("selection.move", &json!({"glyph": "I", "points": [[0, 0], [0, 1], [0, 2], [0, 3]], "dx": 0, "dy": 700})).unwrap();
    assert_eq!(layer(&s).info.hints, before);
    assert_eq!(ps(&s)[0].edges(&layer(&s)).unwrap(), (700.0, 1400.0));
    assert_eq!(tt_place(&s, tt(&s)[0].target), (200.0, 1400.0));
}

/// "=": two identical bars, moved up by the gap between them; the lower bar lands on the upper
/// bar's old places, and each bar keeps its own hint.
#[test]
fn identical_contours_moved_onto_each_others_places_keep_their_own_links() {
    let mut s = session(
        vec![Path::rect(100.0, 0.0, 200.0, 100.0), Path::rect(100.0, 300.0, 200.0, 400.0)],
        &[PsHint::stem(true, node(0, 0), node(0, 3)), PsHint::stem(true, node(1, 0), node(1, 3))],
        &[],
    );
    let before = layer(&s).info.hints.clone();
    let all: Vec<[usize; 2]> = (0..2).flat_map(|p| (0..4).map(move |n| [p, n])).collect();
    s.execute("selection.move", &json!({"glyph": "I", "points": all, "dx": 0, "dy": 300})).unwrap();
    assert_eq!(layer(&s).info.hints, before);
    let edges: Vec<(f64, f64)> = ps(&s).iter().map(|h| h.edges(&layer(&s)).unwrap()).collect();
    assert_eq!(edges, vec![(300.0, 400.0), (600.0, 700.0)]);
}

/// A drag previews each step from the interaction's snapshot; the upkeep runs on every preview
/// and the commit leaves one undo step with the links of the last preview.
#[test]
fn an_interaction_keeps_the_links_of_its_last_preview_in_one_undo_step() {
    let mut s = stem();
    let (undo, before) = (s.doc().unwrap().history.undo.len(), layer(&s).info.hints.clone());
    s.begin("Drag").unwrap();
    s.preview("selection.move", &json!({"glyph": "I", "points": [[0, 0], [0, 1], [0, 2], [0, 3]], "dx": 0, "dy": 700})).unwrap();
    assert_eq!(ps(&s)[0].edges(&layer(&s)).unwrap(), (700.0, 1400.0));
    let out = s.preview("path.insertNode", &json!({"glyph": "I", "segment": [0, 1]})).unwrap();
    assert!(out.get("hintUpkeep").is_none(), "nothing was unlinked or removed: {out}");
    assert!(s.commit().unwrap());
    let h = &ps(&s)[0];
    assert_eq!((h.origin, h.other), (node(0, 0), Some(node(0, 4))));
    assert_eq!(h.edges(&layer(&s)).unwrap(), (0.0, 700.0), "the move was replaced by the insertion");
    assert_eq!(tt_place(&s, tt(&s)[0].target), (200.0, 700.0));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(layer(&s).info.hints, before);
    assert_eq!(layer(&s).shapes.len(), 1);
}

fn read_tree(dir: &std::path::Path, root: &std::path::Path, out: &mut fontcraft_io_glyphs::Files) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            read_tree(&p, root, out);
        } else {
            out.insert(p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"), std::fs::read(&p).unwrap());
        }
    }
}

/// The cost of the upkeep step after a node is inserted in one glyph of Inter Roman, with and
/// without hints on that glyph.
/// `cargo test --release -p fontcraft-engine --test engine hint_upkeep:: -- --ignored --nocapture`.
#[test]
#[ignore = "speed measure"]
fn upkeep_per_edit_on_inter() {
    use fontcraft_engine::model::Font;
    use fontcraft_hinting::{RemapMode, remap_after_edit};
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/inter/src/Inter-Roman.glyphspackage");
    if !dir.is_dir() {
        return;
    }
    let mut files = fontcraft_io_glyphs::Files::new();
    read_tree(&dir, &dir, &mut files);
    let plain = fontcraft_io_glyphs::read_package(&files).unwrap();
    let edit = |font: &Font| {
        let mut f = font.clone();
        let mut g = f.glyphs.get_mut("n").unwrap();
        let p = g.layers[0]
            .shapes
            .iter_mut()
            .find_map(|s| match s {
                Shape::Path(p) => Some(p),
                _ => None,
            })
            .unwrap();
        let n = p.nodes[0].clone();
        p.nodes.insert(0, Node { x: n.x + 1.0, ..n });
        drop(g);
        f
    };
    let hinted = {
        let mut f = plain.clone();
        let mut g = f.glyphs.get_mut("n").unwrap();
        let layer = &mut g.layers[0];
        set_ps_hints(layer, &[PsHint::stem(true, node(0, 2), node(0, 5))]).unwrap();
        set_tt_instructions(layer, &[TtInstruction::new(TtKind::Snap { zone: ZoneRef::Auto }, Direction::Vertical, tt_node(0, 3))]).unwrap();
        drop(g);
        f
    };
    const EDITS: u32 = 200;
    for (what, before) in [("no hints", plain), ("hints", hinted)] {
        let after = edit(&before);
        let mut total = std::time::Duration::ZERO;
        let mut changed = false;
        for _ in 0..EDITS {
            let next = after.clone();
            let t = std::time::Instant::now();
            let (out, _) = remap_after_edit(&before, next, RemapMode::Edit).unwrap();
            total += t.elapsed();
            changed = !out.glyph("n").is_some_and(|g| Arc::ptr_eq(g, after.glyph("n").unwrap()));
        }
        assert_eq!(changed, what == "hints");
        println!("{} glyphs, {what}: upkeep {:?}/edit", before.glyphs.len(), total / EDITS);
    }
}
