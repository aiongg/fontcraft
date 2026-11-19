//! Higher-order interpolation commands (M9.30, `hoi.*`) on a three-master `.glyphs` version 4
//! font built in the test: each command's curve, written back and read again after every step.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use fontcraft_engine::Session;
use fontcraft_interp::hoi::{self, Span};
use fontcraft_model::{Font, HoiCenter, HoiCurve, HoiDirection, Shape};
use serde_json::{Value, json};

/// A Weight axis and the masters given as (id, weight, spread): glyph `A` is a triangle whose base
/// widens by the spread on each side.
fn font_text(masters: &[(&str, f64, f64)]) -> String {
    let mut t = String::from("{\n.appVersion = \"4004\";\n.formatVersion = 4;\naxes = (\n{\nname = Weight;\ntag = wght;\n},\n);\nfontMaster = (\n");
    for (id, weight, _) in masters {
        t += &format!("{{\naxesValues = (\n{weight},\n);\nid = {id};\n}},\n");
    }
    t += ");\nglyphs = (\n{\nglyphname = A;\nlayers = (\n";
    for (id, _, spread) in masters {
        t += &format!(
            "{{\nlayerId = {id};\nshapes = (\n{{\nclosed = 1;\nnodes = (\n({},0,l),\n(300,{},l),\n({},0,l),\n);\n}},\n);\nwidth = 600;\n}},\n",
            100.0 - spread,
            700.0 + spread,
            500.0 + spread
        );
    }
    t += ");\n},\n);\nunitsPerEm = 1000;\nversionMajor = 1;\nversionMinor = 0;\n}\n";
    t
}

fn session(masters: &[(&str, f64, f64)]) -> Session {
    let font = fontcraft_io_glyphs::read(&font_text(masters)).unwrap();
    let mut s = Session::new();
    s.add_document(font, None).unwrap();
    s
}

/// Light, Regular and Bold.
fn three() -> Session {
    session(&[("l", 300.0, 0.0), ("r", 400.0, 20.0), ("b", 700.0, 60.0)])
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn node_xy(font: &Font, layer: &str, node: usize) -> (f64, f64) {
    let g = font.glyph("A").unwrap();
    let Shape::Path(p) = &g.layers.iter().find(|l| l.id == layer).unwrap().shapes[0] else { panic!("not a path") };
    (p.nodes[node].x, p.nodes[node].y)
}

fn curve(font: &Font, layer: &str, node: usize) -> Option<HoiCurve> {
    let g = font.glyph("A").unwrap();
    let Shape::Path(p) = &g.layers.iter().find(|l| l.id == layer).unwrap().shapes[0] else { panic!("not a path") };
    p.nodes[node].info.hoi.get("wght").cloned()
}

/// The span of node `node` from `lower` to `upper`, centred as the commands centre it.
fn span(font: &Font, lower: &str, upper: &str, node: usize) -> Span {
    let g = font.glyph("A").unwrap();
    let path = |id: &str| match &g.layers.iter().find(|l| l.id == id).unwrap().shapes[0] {
        Shape::Path(p) => p.clone(),
        _ => panic!("not a path"),
    };
    hoi::node_span(&path(lower), &path(upper), node).unwrap()
}

fn close(a: (f64, f64), b: (f64, f64)) -> bool {
    (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6
}

/// Writes the font as `.glyphs` version 4 and reads it back: every node's curves come back equal.
fn assert_round_trips(s: &Session, step: &str) {
    let font = s.font().unwrap();
    let text = fontcraft_io_glyphs::write(font).unwrap();
    assert!(text.contains(".formatVersion = 4;"), "{step}: written as version 4");
    let back = fontcraft_io_glyphs::read(&text).unwrap();
    for id in ["l", "r", "b"] {
        for n in 0..3 {
            assert_eq!(curve(&back, id, n), curve(font, id, n), "{step}: layer {id} node {n}");
        }
    }
}

fn target(layer: &str, extra: Value) -> Value {
    let mut p = json!({"glyph": "A", "layer": layer, "points": [[0, 1]], "axis": "wght"});
    if let (Value::Object(m), Value::Object(e)) = (&mut p, extra) {
        m.extend(e);
    }
    p
}

#[test]
fn each_command_sets_its_curve_and_the_font_round_trips() {
    let mut s = three();
    let sp = span(s.font().unwrap(), "r", "b", 1);

    // A new intermediate point sits halfway along the straight line, on the lower layer's node.
    let out = run(&mut s, "hoi.setInterpolationType", target("r", json!({"type": "intermediatePoint"})));
    assert_eq!(out["layer"], "r");
    assert_eq!(out["changed"], json!([[0, 1]]));
    let mid = sp.linear(0.5);
    assert_eq!(curve(s.font().unwrap(), "r", 1), Some(HoiCurve::Point { x: mid.0, y: mid.1 }));
    assert_round_trips(&s, "intermediate point");

    // Dragging the knob, then Return: the handles follow the same path.
    run(&mut s, "hoi.setControlPoints", target("r", json!({"positions": [[380.0, 760.0]]})));
    let bent = HoiCurve::Point { x: 380.0, y: 760.0 };
    assert_eq!(curve(s.font().unwrap(), "r", 1), Some(bent.clone()));
    run(&mut s, "hoi.convertToHandles", target("r", json!({})));
    let handles = curve(s.font().unwrap(), "r", 1).unwrap();
    assert!(matches!(handles, HoiCurve::Timing(_)));
    for t in [0.25, 0.5, 0.75] {
        assert!(close(hoi::position(&handles, &sp, t), hoi::position(&bent, &sp, t)), "the path is kept at {t}");
    }
    assert_round_trips(&s, "handles");

    // Two dragged handles are stored relative to the span and come back where they were put.
    run(&mut s, "hoi.setControlPoints", target("r", json!({"positions": [[320.0, 760.0], [330.0, 770.0]]})));
    let Some(HoiCurve::Timing(timing)) = curve(s.font().unwrap(), "r", 1) else { panic!("handles expected") };
    let (h1, h2) = hoi::handles(&sp, &timing);
    assert!(close(h1, (320.0, 760.0)) && close(h2, (330.0, 770.0)), "{h1:?} {h2:?}");
    assert_round_trips(&s, "dragged handles");

    // Rotation: automatic centre, then a moved centre, a direction, back to the centre.
    run(&mut s, "hoi.setInterpolationType", target("r", json!({"type": "rotation"})));
    assert_eq!(curve(s.font().unwrap(), "r", 1), Some(HoiCurve::Rotation { center: Some(HoiCenter::Auto), direction: None }));
    run(&mut s, "hoi.setControlPoints", target("r", json!({"positions": [[300.0, 350.0]]})));
    run(&mut s, "hoi.setRotationDirection", target("r", json!({"direction": "clockwise"})));
    let moved = HoiCurve::Rotation { center: Some(HoiCenter::At { x: 300.0, y: 350.0 }), direction: Some(HoiDirection::Clockwise) };
    assert_eq!(curve(s.font().unwrap(), "r", 1), Some(moved));
    assert_round_trips(&s, "rotation");
    run(&mut s, "hoi.centerRotation", target("r", json!({})));
    assert_eq!(
        curve(s.font().unwrap(), "r", 1),
        Some(HoiCurve::Rotation { center: Some(HoiCenter::Auto), direction: Some(HoiDirection::Clockwise) })
    );
    run(&mut s, "hoi.setRotationDirection", target("r", json!({"direction": "automatic"})));
    assert_eq!(curve(s.font().unwrap(), "r", 1), Some(HoiCurve::Rotation { center: Some(HoiCenter::Auto), direction: None }));
    assert_round_trips(&s, "centred rotation");

    // Each command is one undo step.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(
        curve(s.font().unwrap(), "r", 1),
        Some(HoiCurve::Rotation { center: Some(HoiCenter::Auto), direction: Some(HoiDirection::Clockwise) })
    );

    // Delete, and Linear, remove the curve.
    run(&mut s, "hoi.removeCurve", target("r", json!({})));
    assert_eq!(curve(s.font().unwrap(), "r", 1), None);
    assert_round_trips(&s, "removed");
    run(&mut s, "hoi.setInterpolationType", target("r", json!({"type": "handles"})));
    run(&mut s, "hoi.setInterpolationType", target("r", json!({"type": "linear"})));
    assert_eq!(curve(s.font().unwrap(), "r", 1), None);
}

#[test]
fn toward_down_edits_the_node_of_the_layer_below() {
    let mut s = three();
    let out = run(&mut s, "hoi.setInterpolationType", target("b", json!({"toward": "down", "type": "intermediatePoint"})));
    assert_eq!(out["layer"], "r", "the span Regular → Bold is kept on Regular");
    let font = s.font().unwrap();
    let (a, b) = (node_xy(font, "r", 1), node_xy(font, "b", 1));
    assert_eq!(curve(font, "r", 1), Some(HoiCurve::Point { x: (a.0 + b.0) / 2.0, y: (a.1 + b.1) / 2.0 }));
    assert_eq!(curve(font, "b", 1), None);
    assert_round_trips(&s, "toward down");
}

#[test]
fn refusals_change_nothing() {
    // One master: no span to curve.
    let mut one = session(&[("r", 400.0, 0.0)]);
    let e = one.execute("hoi.setInterpolationType", &target("r", json!({"type": "rotation"}))).unwrap_err();
    assert!(e.to_string().contains("two masters"), "{e}");

    let mut s = three();
    let before = s.font().unwrap().clone();
    let refused = [
        ("hoi.setInterpolationType", target("b", json!({"type": "intermediatePoint"})), "no layer above"),
        ("hoi.setInterpolationType", target("l", json!({"toward": "down", "type": "handles"})), "no layer below"),
        ("hoi.setInterpolationType", target("r", json!({"axis": "wdth", "type": "handles"})), "no axis"),
        ("hoi.convertToHandles", target("r", json!({})), "intermediate point"),
        ("hoi.centerRotation", target("r", json!({})), "rotation"),
        ("hoi.removeCurve", target("r", json!({})), "curve"),
        ("hoi.setControlPoints", target("r", json!({"positions": [[1.0, 2.0], [3.0, 4.0], [5.0, 6.0]]})), "one point"),
        ("hoi.setControlPoints", target("r", json!({"points": [[0, 0], [0, 1]], "positions": [[1.0, 2.0]]})), "exactly one node"),
        ("hoi.setInterpolationType", target("r", json!({"points": [[0, 9]], "type": "rotation"})), "does not resolve"),
    ];
    for (id, p, why) in refused {
        let e = s.execute(id, &p).unwrap_err();
        assert!(e.to_string().contains(why), "{id} {p}: {e}");
    }
    assert_eq!(*s.font().unwrap(), before);
}
