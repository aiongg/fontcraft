//! Cap component tests on the cap test font of `docs/plans/M13.md` § 8, rebuilt in code. No
//! export from the reference app exists yet, so the expected outlines are the geometry the
//! module documentation describes (the rows' guesses), worked out by hand.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

use std::sync::Arc;

use super::*;
use crate::{Anchor, Font, Glyph, Layer, Node, NodeKind, Path, Shape};

fn lines(closed: bool, points: &[(f64, f64)]) -> Shape {
    Shape::Path(Path::new(closed, points.iter().map(|&(x, y)| Node::new(x, y, NodeKind::Line)).collect()))
}

fn glyph(name: &str, layers: &[(&str, f64, Vec<Shape>, Vec<Anchor>)]) -> Glyph {
    let mut g = Glyph::new(name);
    for (master, width, shapes, anchors) in layers {
        let mut l = Layer::new(*master, *width);
        l.shapes = shapes.clone();
        l.anchors = anchors.clone();
        g.layers.push(l);
    }
    g
}

/// Adds `g` in place of a glyph of the same name.
fn put(f: &mut Font, g: Glyph) {
    f.glyphs.remove(&g.name);
    f.glyphs.push(g);
}

/// `_cap.flag` (origin anchor, advance 100, a 4-node open path), `_cap.round` (node1 and node2
/// 50 units apart, an open half round), `l` (80-unit stem, 120 in Bold), `i` (120-unit stem) and
/// `slash` (a slanted stem whose sides are not parallel), on masters Regular and Bold.
fn cap_font() -> Font {
    let mut f = Font::new("Caps");
    let masters = Arc::make_mut(&mut f.masters);
    let mut bold = masters[0].clone();
    bold.id = "m02".into();
    bold.name = "Bold".into();
    masters.push(bold);
    let flag = lines(false, &[(0.0, 20.0), (-40.0, -30.0), (100.0, -30.0), (100.0, 20.0)]);
    let n = |x, y, k| Node::new(x, y, k);
    use NodeKind::{Curve as C, Line as L, OffCurve as O};
    let round = Shape::Path(Path::new(
        false,
        vec![n(0.0, 10.0, L), n(0.0, -14.0, O), n(11.0, -25.0, O), n(25.0, -25.0, C), n(39.0, -25.0, O), n(50.0, -14.0, O), n(50.0, 10.0, C)],
    ));
    let both =
        |shapes: Vec<Shape>, width: f64, anchors: Vec<Anchor>| vec![("m01", width, shapes.clone(), anchors.clone()), ("m02", width, shapes, anchors)];
    let rect = |x1: f64, y1: f64| vec![lines(true, &[(100.0, 0.0), (x1, 0.0), (x1, y1), (100.0, y1)])];
    put(&mut f, glyph("_cap.flag", &both(vec![flag], 100.0, vec![Anchor::new("origin", 0.0, 0.0)])));
    put(&mut f, glyph("_cap.round", &both(vec![round], 50.0, vec![Anchor::new("node1", 0.0, 0.0), Anchor::new("node2", 50.0, 0.0)])));
    put(&mut f, glyph("l", &[("m01", 280.0, rect(180.0, 700.0), vec![]), ("m02", 320.0, rect(220.0, 700.0), vec![])]));
    put(&mut f, glyph("i", &both(rect(220.0, 500.0), 320.0, vec![])));
    put(&mut f, glyph("slash", &both(vec![lines(true, &[(100.0, 0.0), (180.0, 0.0), (400.0, 700.0), (300.0, 700.0)])], 500.0, vec![])));
    f
}

/// File-order node 1 of a 4-node rectangle is its top right corner (model node 2), node 3 its
/// bottom left (model node 0).
const TOP: usize = 1;
const BOTTOM: usize = 3;

fn cap(name: &str, node: usize, fit: bool) -> CapHint {
    CapHint { fit, alignment: CornerAlignment::Both, ..CapHint::new(name, 0, node) }
}

fn fitted(font: &Font, glyph: &str, master: &str, cap: &CapHint) -> Vec<(f64, f64)> {
    let layer = font.glyph(glyph).unwrap().layer(master).unwrap();
    let shapes = fit_cap(font, master, &layer.shapes, cap).unwrap_or_else(|e| panic!("{glyph}: {e}"));
    let Shape::Path(p) = &shapes[0] else { panic!("not a path") };
    p.nodes.iter().map(|n| (n.x, n.y)).collect()
}

fn fit_error(font: &Font, glyph: &str, cap: &CapHint) -> String {
    let layer = font.glyph(glyph).unwrap().layer("m01").unwrap();
    fit_cap(font, "m01", &layer.shapes, cap).unwrap_err().to_string()
}

#[test]
fn an_origin_cap_on_the_top_of_a_stem_turns_round_and_replaces_the_stem_end() {
    let f = cap_font();
    // Fit on: the 100-unit cap is sized to the 80-unit stem; the sides end at the cap's ends.
    let fit = fitted(&f, "l", "m01", &cap("_cap.flag", TOP, true));
    assert_eq!(fit, [(100.0, 0.0), (180.0, 0.0), (180.0, 680.0), (212.0, 730.0), (100.0, 730.0), (100.0, 680.0)]);
    // Fit off: drawn at its own size, so its end lies past the left side, which bends to it.
    let plain = fitted(&f, "l", "m01", &cap("_cap.flag", TOP, false));
    assert_eq!(plain, [(100.0, 0.0), (180.0, 0.0), (180.0, 680.0), (220.0, 730.0), (80.0, 730.0), (80.0, 680.0)]);
    // The 120-unit stem of `i`, Fit on.
    let wide = fitted(&f, "i", "m01", &cap("_cap.flag", TOP, true));
    assert_eq!(wide, [(100.0, 0.0), (220.0, 0.0), (220.0, 480.0), (268.0, 530.0), (100.0, 530.0), (100.0, 480.0)]);
}

#[test]
fn an_origin_cap_on_the_bottom_of_a_stem_is_drawn_as_designed() {
    let f = cap_font();
    // The cap's first node took the place of the path's start node, so it starts the path.
    let nodes = fitted(&f, "l", "m01", &cap("_cap.flag", BOTTOM, true));
    assert_eq!(nodes, [(100.0, 20.0), (68.0, -30.0), (180.0, -30.0), (180.0, 20.0), (180.0, 700.0), (100.0, 700.0)]);
}

#[test]
fn a_node1_node2_cap_fits_across_the_stem_and_keeps_its_height() {
    let f = cap_font();
    for (glyph, right) in [("l", 180.0), ("i", 220.0)] {
        let nodes = fitted(&f, glyph, "m01", &cap("_cap.round", BOTTOM, true));
        assert_eq!(nodes.first(), Some(&(100.0, 10.0)), "{glyph}");
        assert!(nodes.contains(&(right, 10.0)), "{glyph}: the cap ends on the right side: {nodes:?}");
        let low = nodes.iter().map(|p| p.1).fold(f64::MAX, f64::min);
        assert_eq!(low, -25.0, "{glyph}: Fit scales across the stem only");
    }
    // Fit off: the cap keeps its 50 units and the right side bends to its end.
    let plain = fitted(&f, "l", "m01", &cap("_cap.round", BOTTOM, false));
    assert!(plain.contains(&(150.0, 10.0)), "{plain:?}");
}

#[test]
fn scale_applies_in_the_cap_frame_and_fit_replaces_the_horizontal_scale() {
    let f = cap_font();
    let scaled = |sx: f64, sy: f64, fit: bool| fitted(&f, "l", "m01", &CapHint { scale: Vec2::new(sx, sy), ..cap("_cap.flag", BOTTOM, fit) });
    assert_eq!(&scaled(0.5, 0.5, false)[..4], [(100.0, 10.0), (80.0, -15.0), (150.0, -15.0), (150.0, 10.0)]);
    assert_eq!(&scaled(1.0, 2.0, false)[..4], [(100.0, 40.0), (60.0, -60.0), (200.0, -60.0), (200.0, 40.0)]);
    assert_eq!(&scaled(0.5, 2.0, true)[..4], [(100.0, 40.0), (68.0, -60.0), (180.0, -60.0), (180.0, 40.0)]);
}

#[test]
fn the_four_alignments_on_a_slanted_stem() {
    let f = cap_font();
    // The flag's bottom edge runs from (-40, -30) to (100, -30): its angle is the cap's turn.
    let turn = |node: usize, alignment: CornerAlignment| {
        let nodes = fitted(&f, "slash", "m01", &CapHint { alignment, ..cap("_cap.flag", node, false) });
        // Bottom end: the path starts with the cap; top end: the cap follows the two bottom nodes.
        let start = if node == BOTTOM { 0 } else { 2 };
        let (p, q) = (nodes[start + 1], nodes[start + 2]);
        (q.1 - p.1).atan2(q.0 - p.0)
    };
    let quarter = std::f64::consts::FRAC_PI_2;
    let close = |a: f64, b: f64| ((a - b + 3.0 * std::f64::consts::PI).rem_euclid(2.0 * std::f64::consts::PI) - std::f64::consts::PI).abs() < 0.02;
    // Bottom end: left follows the side before the first node, right the side after the second.
    assert!(close(turn(BOTTOM, CornerAlignment::Left), 700f64.atan2(200.0) - quarter));
    assert!(close(turn(BOTTOM, CornerAlignment::Right), 700f64.atan2(220.0) - quarter));
    assert!(close(turn(BOTTOM, CornerAlignment::Both), 0.0));
    assert!(close(turn(BOTTOM, CornerAlignment::Unaligned), 0.0));
    // Top end: centre follows the nodes and turns the cap round; none leaves it as drawn.
    assert!(close(turn(TOP, CornerAlignment::Both), std::f64::consts::PI));
    assert!(close(turn(TOP, CornerAlignment::Unaligned), 0.0));
}

#[test]
fn decompose_gives_the_drawn_outline_and_moves_the_other_hints() {
    let mut f = cap_font();
    {
        let mut l = f.glyph_mut("l").unwrap();
        let hints = &mut l.layers[0].info.hints;
        hints.push(cap("_cap.flag", TOP, true).to_hint());
        hints.push(cap("_cap.round", BOTTOM, true).to_hint());
    }
    let drawn = f.outline("l", "m01").unwrap();
    let layer = f.glyph("l").unwrap().layer("m01").unwrap().clone();
    // Both at once, and one after the other through the moved hint: the same outline.
    let all = decompose_attachments(&f, &layer, &[0, 1]).unwrap();
    assert!(all.info.hints.is_empty());
    let first = decompose_attachments(&f, &layer, &[0]).unwrap();
    let moved = cap_hint(&first.info.hints[0]).unwrap().unwrap();
    assert_eq!(moved.node, 5, "the bottom cap's first node is the path's last in file order");
    let second = decompose_attachments(&f, &first, &[0]).unwrap();
    assert_eq!(second.shapes, all.shapes);
    f.glyph_mut("l").unwrap().layers[0] = all;
    assert_eq!(f.outline("l", "m01").unwrap(), drawn, "decompose equals the drawn outline");
}

#[test]
fn a_corner_and_a_cap_on_one_path_are_both_fitted() {
    let mut f = cap_font();
    put(&mut f, glyph("_corner.one", &[("m01", 0.0, vec![lines(false, &[(0.0, 50.0), (-50.0, 50.0), (-50.0, 0.0), (25.0, 0.0)])], vec![])]));
    let corner = crate::corner::CornerHint::new("_corner.one", 0, BOTTOM).to_hint();
    let cornered = {
        let mut l = f.glyph("l").unwrap().layer("m01").unwrap().clone();
        l.info.hints.push(corner.clone());
        crate::corner::insert_corners(&f, &l).unwrap().unwrap()
    };
    let mut l = f.glyph("l").unwrap().layer("m01").unwrap().clone();
    l.info.hints.push(cap("_cap.flag", TOP, true).to_hint());
    l.info.hints.push(corner);
    let shapes = insert_attachments(&f, &l).unwrap().unwrap();
    let nodes = |s: &[Shape]| {
        let Shape::Path(p) = &s[0] else { panic!() };
        p.nodes.iter().map(|n| (n.x, n.y)).collect::<Vec<_>>()
    };
    let (both, corner_only) = (nodes(&shapes), nodes(&cornered));
    let low = |v: &[(f64, f64)]| {
        let mut out = v.iter().copied().filter(|p| p.1 < 300.0).collect::<Vec<_>>();
        out.sort_by(|a, b| a.partial_cmp(b).unwrap());
        out
    };
    assert_eq!(low(&both), low(&corner_only), "the corner sits where it sits without the cap");
    assert!(both.contains(&(212.0, 730.0)), "the cap is fitted: {both:?}");
}

#[test]
fn two_masters_stay_compatible() {
    let mut f = cap_font();
    for i in 0..2 {
        f.glyph_mut("l").unwrap().layers[i].info.hints.push(cap("_cap.round", TOP, true).to_hint());
    }
    let shape = |master: &str| {
        let shapes = insert_attachments(&f, f.glyph("l").unwrap().layer(master).unwrap()).unwrap().unwrap();
        let Shape::Path(p) = &shapes[0] else { panic!() };
        p.nodes.iter().map(|n| n.kind).collect::<Vec<_>>()
    };
    assert_eq!(shape("m01"), shape("m02"));
    assert_eq!(shape("m01").len(), 9);
}

#[test]
fn the_hint_stores_alignment_and_fit_in_options() {
    let c = CapHint { scale: Vec2::new(1.0, 0.5), alignment: CornerAlignment::Right, fit: true, ..CapHint::new("_cap.flag", 0, 1) };
    let hint = c.to_hint();
    let d = hint.value.as_dict().unwrap();
    let keys: Vec<&str> = d.entries.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(keys, ["name", "options", "origin", "scale", "type"]);
    assert_eq!(d.get("options").and_then(number), Some(9.0));
    assert_eq!(cap_hint(&hint).unwrap(), Some(c));
    let plain = CapHint::new("_cap.flag", 0, 1).to_hint();
    assert!(plain.value.as_dict().unwrap().get("options").is_none(), "left without Fit is written without options");
}

#[test]
fn hostile_scales_and_zero_length_stems_fail_with_a_message() {
    let mut f = cap_font();
    let mut hint = cap("_cap.flag", TOP, false).to_hint();
    if let Value::Dict(d) = &mut hint.value {
        d.insert(
            "scale",
            Value::Array(Array { items: vec![Value::Number(Number::int(1_000_000_000)), Value::Number(Number::int(1))], ..Default::default() }),
        );
    }
    assert!(cap_hint(&hint).unwrap_err().contains("beyond"));
    // A Fit that would scale past the limit: a cap glyph 0.5 units wide on an 80-unit stem.
    let mut tiny = f.glyph("_cap.flag").unwrap().as_ref().clone();
    tiny.name = "_cap.tiny".into();
    tiny.layers[0].width = 0.5;
    put(&mut f, tiny);
    assert!(fit_error(&f, "l", &cap("_cap.tiny", TOP, true)).contains("beyond"));
    f.glyph_mut("_cap.tiny").unwrap().layers[0].width = 0.0;
    assert!(fit_error(&f, "l", &cap("_cap.tiny", TOP, true)).contains("no width"));
    // Two coinciding stem nodes.
    f.glyph_mut("i").unwrap().layers[0].shapes = vec![lines(true, &[(100.0, 0.0), (100.0, 0.0), (100.0, 500.0), (0.0, 500.0)])];
    assert!(fit_error(&f, "i", &cap("_cap.flag", BOTTOM, false)).contains("zero length"));
    // Nodes that are not neighbours: an open path's last node has none after it.
    f.glyph_mut("i").unwrap().layers[0].shapes = vec![lines(false, &[(100.0, 0.0), (200.0, 0.0)])];
    assert!(fit_error(&f, "i", &CapHint::new("_cap.flag", 0, 1)).contains("ends the open path"));
}
