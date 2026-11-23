//! Segment component and brush commands (M13.41): `segmentComponent.add`, `decompose` and
//! `setGlyph`, `brush.add` on several segments and on end nodes, `decompose` and `setGlyph`,
//! their refusals, Set Anchors on segment and brush glyphs, and the hints' `.glyphs` round trip.
//! The bending itself is tested in `fontcraft_model::attach`.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use fontcraft_engine::Session;
use fontcraft_geom::PathEl;
use fontcraft_model::attach::{Attachment, SegmentHint, attachment};
use fontcraft_model::{Anchor, Font, Node, NodeKind, Path, Shape};
use fontcraft_testkit::snapshot;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn run_err(s: &mut Session, id: &str, p: Value) -> String {
    let before = snapshot(s);
    let e = s.execute(id, &p).err().unwrap_or_else(|| panic!("{id} {p} succeeded")).to_string();
    assert_eq!(snapshot(s), before, "{id} {p} failed but changed the session");
    e
}

fn lines(closed: bool, points: &[(f64, f64)]) -> Shape {
    Shape::Path(Path::new(closed, points.iter().map(|&(x, y)| Node::new(x, y, NodeKind::Line)).collect()))
}

/// `_segment.wave` and `_segment.peak` (open paths, 200 units start to end), `_segment.dot`
/// (start and end on one point), `_brush.bar` (a closed bar, start and end 100 apart),
/// `wave` (not a segment glyph), `box` (a closed 400 × 400 rectangle), `zig` (an open path
/// of three straight segments), and `_segment.new` and `_brush.new` without anchors.
fn session() -> Session {
    let mut font = Font::new("Strokes");
    let ends = |s: f64, e: f64| vec![Anchor::new("start", s, 0.0), Anchor::new("end", e, 0.0)];
    let wave = lines(false, &[(0.0, 0.0), (100.0, 30.0), (200.0, 0.0)]);
    let glyphs: [(&str, Vec<Shape>, f64, Vec<Anchor>); 9] = [
        ("_segment.wave", vec![wave.clone()], 200.0, ends(0.0, 200.0)),
        ("_segment.peak", vec![lines(false, &[(0.0, 0.0), (100.0, 60.0), (200.0, 0.0)])], 200.0, ends(0.0, 200.0)),
        ("_segment.dot", vec![wave.clone()], 200.0, ends(50.0, 50.0)),
        ("_brush.bar", vec![lines(true, &[(0.0, -10.0), (100.0, -10.0), (100.0, 10.0), (0.0, 10.0)])], 100.0, ends(0.0, 100.0)),
        ("wave", vec![wave], 200.0, vec![]),
        ("box", vec![lines(true, &[(100.0, 0.0), (500.0, 0.0), (500.0, 400.0), (100.0, 400.0)])], 600.0, vec![]),
        ("zig", vec![lines(false, &[(0.0, 0.0), (100.0, 0.0), (200.0, 100.0), (300.0, 100.0)])], 400.0, vec![]),
        ("_segment.new", vec![lines(false, &[(10.0, 0.0), (60.0, 20.0), (110.0, 0.0)])], 140.0, vec![]),
        ("_brush.new", vec![lines(true, &[(0.0, -10.0), (140.0, -10.0), (140.0, 10.0), (0.0, 10.0)])], 140.0, vec![]),
    ];
    for (name, shapes, width, anchors) in glyphs {
        if font.glyph(name).is_none() {
            font.add_glyph(name, Vec::new(), width).unwrap();
        }
        let mut g = font.glyph_mut(name).unwrap();
        let l = g.layer_mut("m01").unwrap();
        l.shapes = shapes;
        l.width = width;
        l.anchors = anchors;
    }
    let mut s = Session::new();
    s.add_document(font, None).unwrap();
    s
}

fn attachments(s: &Session, glyph: &str) -> Vec<Attachment> {
    let layer = s.font().unwrap().glyph(glyph).unwrap().layer("m01").unwrap();
    layer.info.hints.iter().map(|h| attachment(h).unwrap().unwrap()).collect()
}

fn closed_paths(s: &Session, glyph: &str) -> usize {
    s.font().unwrap().outline(glyph, "m01").unwrap().elements().iter().filter(|e| matches!(e, PathEl::ClosePath)).count()
}

#[test]
fn segment_component_add_stores_the_end_node_and_decompose_gives_the_drawn_outline() {
    let mut s = session();
    let before = s.font().unwrap().glyph("box").unwrap().clone();
    // The bottom segment ends at model node 1, which the file numbers 0.
    let r = run(&mut s, "segmentComponent.add", json!({"glyph": "box", "segments": [[0, 1]], "segmentGlyph": "_segment.wave"}));
    assert_eq!((r["segmentComponents"].clone(), r["replaced"].clone()), (json!([0]), json!(0)));
    assert_eq!(r["selection"]["segmentComponents"], json!([0]));
    assert_eq!(attachments(&s, "box"), [Attachment::Segment(SegmentHint::new("_segment.wave", 0, 0))]);
    let r = run(&mut s, "segmentComponent.add", json!({"glyph": "box", "segment": [0, 1], "segmentGlyph": "_segment.peak"}));
    assert_eq!(r["replaced"], json!(1), "one segment holds one segment component");
    let drawn = s.font().unwrap().outline("box", "m01").unwrap();
    let r = run(&mut s, "segmentComponent.decompose", json!({"glyph": "box"}));
    assert_eq!(r["decomposed"], json!(1));
    assert!(attachments(&s, "box").is_empty());
    assert_eq!(s.font().unwrap().outline("box", "m01").unwrap(), drawn);
    let layer = s.font().unwrap().glyph("box").unwrap().layer("m01").unwrap();
    let Shape::Path(p) = &layer.shapes[0] else { panic!("not a path") };
    let points: Vec<(f64, f64)> = p.nodes.iter().map(|n| (n.x, n.y)).collect();
    assert_eq!(points, [(100.0, 0.0), (300.0, 60.0), (500.0, 0.0), (500.0, 400.0), (100.0, 400.0)]);
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.font().unwrap().glyph("box").unwrap(), &before, "undo gives back the glyph");
}

#[test]
fn brush_add_takes_several_segments_and_end_nodes() {
    let mut s = session();
    let r = run(&mut s, "brush.add", json!({"glyph": "zig", "segments": [[0, 1], [0, 2]], "brush": "_brush.bar"}));
    assert_eq!(r["brushes"], json!([0, 1]));
    assert_eq!(closed_paths(&s, "zig"), 2, "each brush draws one closed outline");
    // The first node of the open path ends no segment; the last ends the third.
    let r = run(&mut s, "brush.add", json!({"glyph": "zig", "points": [[0, 0], [0, 3]], "brush": "_brush.bar"}));
    assert_eq!((r["brushes"].clone(), r["skipped"].clone()), (json!([2]), json!([[0, 0]])));
    assert_eq!(closed_paths(&s, "zig"), 3);
    let r = run(&mut s, "brush.decompose", json!({"glyph": "zig", "brushes": [1]}));
    assert_eq!(r["decomposed"], json!(1));
    let layer = s.font().unwrap().glyph("zig").unwrap().layer("m01").unwrap();
    assert_eq!(layer.shapes.len(), 2, "the host path stays and the brush's outline is added");
    assert_eq!(layer.info.hints.len(), 2);
    assert_eq!(closed_paths(&s, "zig"), 3);
}

#[test]
fn wrong_targets_are_refused_with_nothing_changed() {
    let mut s = session();
    let e = run_err(&mut s, "segmentComponent.add", json!({"glyph": "box", "segments": [[0, 1]], "segmentGlyph": "wave"}));
    assert!(e.contains("not a segment glyph"), "{e}");
    let e = run_err(&mut s, "brush.add", json!({"glyph": "box", "segments": [[0, 1]], "brush": "_segment.wave"}));
    assert!(e.contains("not a brush glyph"), "{e}");
    let e = run_err(&mut s, "segmentComponent.add", json!({"glyph": "box", "points": [[0, 1]], "segmentGlyph": "_segment.wave"}));
    assert!(e.contains("no other target list"), "{e}");
    let e = run_err(&mut s, "brush.add", json!({"glyph": "zig", "points": [[0, 0]], "brush": "_brush.bar"}));
    assert!(e.contains("ends a segment"), "{e}");
    let e = run_err(&mut s, "brush.add", json!({"glyph": "zig", "segments": [[0, 0]], "brush": "_brush.bar"}));
    assert!(e.contains("does not end a segment"), "{e}");
    run(&mut s, "brush.add", json!({"glyph": "zig", "segments": [[0, 1]], "brush": "_brush.bar"}));
    let e = run_err(&mut s, "segmentComponent.decompose", json!({"glyph": "zig", "segmentComponents": [0]}));
    assert!(e.contains("not a segment component"), "{e}");
    let e = run_err(&mut s, "segmentComponent.decompose", json!({"glyph": "zig"}));
    assert!(e.contains("no segment components"), "{e}");
    let e = run_err(&mut s, "brush.setGlyph", json!({"glyph": "zig", "brushes": [3], "brush": "_brush.bar"}));
    assert!(e.contains("does not resolve"), "{e}");
}

#[test]
fn set_glyph_swaps_the_glyph_and_keeps_the_segment() {
    let mut s = session();
    run(&mut s, "segmentComponent.add", json!({"glyph": "box", "segments": [[0, 1], [0, 2]], "segmentGlyph": "_segment.wave"}));
    let r = run(&mut s, "segmentComponent.setGlyph", json!({"glyph": "box", "segmentComponents": [1], "segmentGlyph": "_segment.peak"}));
    assert_eq!(r["segmentComponents"], json!([1]));
    assert_eq!(
        attachments(&s, "box"),
        [Attachment::Segment(SegmentHint::new("_segment.wave", 0, 0)), Attachment::Segment(SegmentHint::new("_segment.peak", 0, 1))]
    );
}

#[test]
fn a_zero_length_segment_glyph_warns_and_refuses_to_decompose() {
    let mut s = session();
    let r = run(&mut s, "segmentComponent.add", json!({"glyph": "box", "segments": [[0, 1]], "segmentGlyph": "_segment.dot"}));
    assert!(r["warning"].as_str().is_some_and(|w| w.contains("zero length")), "{r}");
    let e = run_err(&mut s, "segmentComponent.decompose", json!({"glyph": "box"}));
    assert!(e.contains("zero length"), "{e}");
}

#[test]
fn set_anchors_places_start_and_end_on_segment_and_brush_glyphs() {
    let mut s = session();
    for name in ["_segment.new", "_brush.new"] {
        run(&mut s, "glyph.setAnchors", json!({"glyph": name}));
    }
    let anchors = |name: &str| -> Vec<(String, f64, f64)> {
        let l = s.font().unwrap().glyph(name).unwrap().layer("m01").unwrap().clone();
        l.anchors.iter().map(|a| (a.name.clone(), a.x, a.y)).collect()
    };
    assert_eq!(anchors("_segment.new"), [("start".to_string(), 10.0, 0.0), ("end".to_string(), 110.0, 0.0)]);
    assert_eq!(anchors("_brush.new"), [("start".to_string(), 0.0, 0.0), ("end".to_string(), 140.0, 0.0)]);
}

#[test]
fn segment_and_brush_hints_round_trip_through_a_glyphs_file() {
    let mut s = session();
    run(&mut s, "segmentComponent.add", json!({"glyph": "box", "segments": [[0, 0]], "segmentGlyph": "_segment.wave"}));
    run(&mut s, "brush.add", json!({"glyph": "zig", "segments": [[0, 2]], "brush": "_brush.bar"}));
    let font = s.font().unwrap();
    let text = fontcraft_io_glyphs::write(font).unwrap();
    assert!(text.contains("type = Segment;") && text.contains("type = Brush;"), "{text}");
    let back = fontcraft_io_glyphs::read(&text).unwrap();
    for glyph in ["box", "zig"] {
        let hints = |f: &Font| f.glyph(glyph).unwrap().layer("m01").unwrap().info.hints.clone();
        assert_eq!(hints(&back), hints(font), "{glyph}");
    }
    assert_eq!(fontcraft_io_glyphs::write(&back).unwrap(), text);
}
