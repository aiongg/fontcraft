//! Corner component tests. The cases are rebuilt from fontc's
//! `resources/testdata/glyphs3/CornerComponents.glyphs` (Apache-2.0), whose `.expectation`
//! glyphs hold the reference app's output; fontc took them from glyphsLib's corner component
//! tests (Apache-2.0).
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

use super::*;
use crate::{Anchor, Font, Layer, LayerInfo, LayerInfoBox, Node, NodeKind, Path, Shape};

/// A test glyph: name, corner hints (glyph, path, node, scale x, scale y, options), paths, and
/// the paths the reference app draws.
type Case = (&'static str, &'static [(&'static str, usize, usize, f64, f64, i64)], &'static [&'static str], &'static [&'static str]);
/// A corner glyph: name, anchors, paths.
type CornerGlyph = (&'static str, &'static [(&'static str, f64, f64)], &'static [&'static str]);

const CASES: &[Case] = &[
    (
        "aa_simple_angleinstroke",
        &[("_corner.one", 0, 0, 1.0, 1.0, 0)],
        &["c:(200,100,l),(414,100,l),(100,300,l)"],
        &["c:(175,150,l),(150,150,l),(150,100,l),(225,100,l),(414,100,l),(100,300,l)"],
    ),
    (
        "ab_simple_angled",
        &[("_corner.one", 0, 0, 1.0, 1.0, 0)],
        &["c:(100,100,l),(500,400,l),(100,400,l)"],
        &["c:(100,163,l),(30,110,l),(60,70,l),(120,115,l),(500,400,l),(100,400,l)"],
    ),
    (
        "ac_scale",
        &[("_corner.one", 0, 0, 1.2, 1.5, 0)],
        &["c:(38,86,l),(302,86,l),(412,500,l),(148,500,l)"],
        &["c:(58,161,l),(-22,161,l),(-22,86,l),(68,86,l),(302,86,l),(412,500,l),(148,500,l)"],
    ),
    (
        "ad_curved_instroke",
        &[("_corner.one", 0, 2, 1.0, 1.0, 0)],
        &["c:(162,279,o),(150,200,o),(100,100,c),(500,400,l),(100,400,l)"],
        &["c:(143,316,o),(150,253,o),(134,188,c),(30,110,l),(60,70,l),(120,115,l),(500,400,l),(100,400,l)"],
    ),
    (
        "ae_curved_corner_firstseg",
        &[("_corner.two", 0, 0, 1.0, 1.0, 0)],
        &["c:(100,0,l),(400,0,l),(400,500,l),(100,500,l)"],
        &["c:(100,50,l),(66,50,o),(13,50,o),(13,0,c),(121,0,l),(400,0,l),(400,500,l),(100,500,l)"],
    ),
    (
        "af_curved_corner_firstseg_slanted",
        &[("_corner.two", 0, 0, 1.0, 1.0, 0)],
        &["c:(100,0,l),(400,0,l),(534,500,l),(234,500,l)"],
        &["c:(113,50,l),(79,50,o),(13,50,o),(13,0,c),(121,0,l),(400,0,l),(534,500,l),(234,500,l)"],
    ),
    (
        "ag_curved_corner_bothsegs",
        &[("_corner.three", 0, 0, 1.0, 1.0, 0)],
        &["c:(100,0,l),(400,0,l),(400,500,l),(100,500,l)"],
        &["c:(100,50,l),(75,50,o),(26,53,o),(10,32,c),(0,10,l),(36,32,o),(84,14,o),(120,0,c),(400,0,l),(400,500,l),(100,500,l)"],
    ),
    (
        "ag_curved_corner_bothsegs_rotated",
        &[("_corner.three", 0, 0, 1.0, 1.0, 0)],
        &["c:(-12,83,l),(248,-67,l),(498,367,l),(238,517,l)"],
        &["c:(13,126,l),(-9,139,o),(-50,166,o),(-74,156,c),(-94,142,l),(-51,143,o),(-19,103,o),(5,73,c),(248,-67,l),(498,367,l),(238,517,l)"],
    ),
    (
        "ah_origin",
        &[("_corner.four", 0, 0, 1.0, 1.0, 0)],
        &["c:(50,0,l),(210,0,l),(210,500,l),(50,500,l)"],
        &["c:(50,50,l),(20,2,l),(53,-26,l),(100,0,l),(210,0,l),(210,500,l),(50,500,l)"],
    ),
    (
        "ai_curved_outstroke",
        &[("_corner.one", 0, 0, 1.0, 1.0, 0)],
        &["c:(550,252,l),(526,294,o),(175,497,o),(90,448,c),(290,102,l)"],
        &["c:(505,226,l),(549,181,l),(585,216,l),(533,270,l),(456,334,o),(166,492,o),(90,448,c),(290,102,l)"],
    ),
    (
        "aj_right_alignment",
        &[("_corner.one", 0, 1, 1.0, 1.0, 1)],
        &["c:(84,0,l),(234,0,l),(234,500,l),(84,500,l)"],
        &["c:(84,0,l),(184,0,l),(184,-50,l),(234,-50,l),(234,25,l),(234,500,l),(84,500,l)"],
    ),
    (
        "ak_right_slanted",
        &[("_corner.one", 0, 1, 1.0, 1.0, 1)],
        &["c:(84,0,l),(234,0,l),(534,500,l),(384,500,l)"],
        &["c:(84,0,l),(184,0,l),(184,-50,l),(234,-50,l),(245,18,l),(534,500,l),(384,500,l)"],
    ),
    (
        "al_unaligned",
        &[("_corner.one", 0, 0, 1.0, 1.0, 4)],
        &["c:(236,-29,l),(496,121,l),(246,554,l),(-14,404,l)"],
        &["c:(207,21,l),(186,21,l),(186,-29,l),(255,-18,l),(496,121,l),(246,554,l),(-14,404,l)"],
    ),
    (
        "am_middle",
        &[("_corner.one", 0, 1, 1.0, 1.0, 2)],
        &["c:(325,0,l),(425,0,l),(125,500,l),(25,500,l)"],
        &["c:(325,0,l),(373,0,l),(390,-62,l),(438,-48,l),(413,21,l),(125,500,l),(25,500,l)"],
    ),
    (
        "an_flippy",
        &[("_corner.two", 0, 0, -1.0, 1.0, 0)],
        &["c:(41,0,l),(141,0,l),(141,500,l),(41,500,l)"],
        &["c:(41,21,l),(41,-87,l),(91,-87,o),(91,-34,o),(91,0,c),(141,0,l),(141,500,l),(41,500,l)"],
    ),
    (
        "ao_firstnode",
        &[("_corner.two", 0, 3, 1.0, 1.0, 0)],
        &["c:(41,0,l),(141,0,l),(141,500,l),(41,500,l)"],
        &["c:(91,534,o),(91,587,o),(41,587,c),(41,479,l),(41,0,l),(141,0,l),(141,500,l),(91,500,l)"],
    ),
    (
        "ap_twoofthem",
        &[("_corner.two", 0, 0, 1.0, 1.0, 0), ("_corner.two", 0, 1, -1.0, 1.0, 1)],
        &["c:(148,0,l),(283,50,l),(283,500,l),(148,450,l)"],
        &["c:(148,53,l),(116,42,o),(49,17,o),(66,-30,c),(168,7,l),(263,43,l),(364,80,l),(347,127,o),(297,109,o),(283,103,c),(283,500,l),(148,450,l)"],
    ),
    (
        "aq_rightleg",
        &[("_corner.slab", 0, 1, -1.0, 1.0, 1)],
        &["c:(88,0,l),(188,0,l),(284,500,l),(184,500,l)"],
        &["c:(88,0,l),(179,0,l),(284,0,l),(284,27,l),(193,27,l),(284,500,l),(184,500,l)"],
    ),
    (
        "ar_leftleg",
        &[("_corner.slab", 0, 1, -1.0, 1.0, 1)],
        &["c:(433,0,l),(483,0,l),(110,381,l),(84,359,l)"],
        &["c:(433,0,l),(474,0,l),(579,0,l),(579,27,l),(457,27,l),(110,381,l),(84,359,l)"],
    ),
    (
        "as_closedpaths",
        &[("_corner.pretty", 0, 0, 1.0, 1.0, 0), ("_corner.pretty", 0, 2, 1.0, 1.0, 0)],
        &["c:(152,1,l),(452,1,l),(452,501,l),(152,501,l)"],
        &[
            "c:(152,28,l),(113,66,o),(28,12,o),(7,-51,c),(47,-76,o),(97,-85,o),(161,1,c),(452,1,l),(452,474,l),(491,436,o),(576,490,o),(597,553,c),(557,578,o),(507,587,o),(443,501,c),(152,501,l)",
            "c:(44,-50,o),(34,-40,o),(34,-28,cs),(34,-17,o),(44,-7,o),(55,-7,cs),(67,-7,o),(77,-17,o),(77,-28,cs),(77,-40,o),(67,-50,o),(55,-50,cs)",
            "c:(101,-101,o),(39,-93,o),(-13,-62,c),(16,20,o),(123,86,o),(171,38,c),(179,46,l),(122,101,o),(2,23,o),(-25,-66,c),(30,-101,o),(101,-117,o),(191,3,c),(183,9,l)",
            "c:(560,552,o),(570,542,o),(570,530,cs),(570,519,o),(560,509,o),(549,509,cs),(537,509,o),(527,519,o),(527,530,cs),(527,542,o),(537,552,o),(549,552,cs)",
            "c:(503,603,o),(565,595,o),(617,564,c),(588,482,o),(481,416,o),(433,464,c),(425,456,l),(482,401,o),(602,479,o),(629,568,c),(574,603,o),(503,619,o),(413,499,c),(421,493,l)",
        ],
    ),
    (
        "at_unaligned_lastseg",
        &[("_corner.five", 0, 0, 1.0, 1.0, 0)],
        &["c:(96,0,l),(396,0,l),(396,500,l),(96,500,l)"],
        &["c:(96,28,l),(11,-22,l),(41,-72,l),(163,0,l),(396,0,l),(396,500,l),(96,500,l)"],
    ),
    (
        "aw_direction",
        &[("_corner.large", 0, 4, 1.0, 1.0, 0)],
        &["c:(246,0,l),(246,217,l),(198,154,o),(92,112,o),(37,112,c),(8,0,l)"],
        &["c:(246,0,l),(246,217,l),(198,154,o),(92,112,o),(37,112,c),(24,107,l),(0,12,l),(8,0,l),(8,0,l)"],
    ),
    (
        "ax_curved_instroke2",
        &[("_corner.one", 0, 2, 1.0, 1.0, 0)],
        &["c:(372,469,o),(106,256,o),(106,128,c),(382,0,l),(501,469,l)"],
        &["c:(388,469,o),(169,305,o),(117,178,c),(82,194,l),(61,149,l),(129,117,l),(382,0,l),(501,469,l)"],
    ),
    (
        "av_left_anchoroffpath",
        &[("_corner.seven", 0, 0, 1.0, 1.0, 0)],
        &["c:(100,0,l),(400,0,l),(500,500,l),(200,500,l)"],
        &["c:(130,150,l),(-30,150,l),(-30,0,l),(145,0,l),(400,0,l),(500,500,l),(200,500,l)"],
    ),
];
const CORNERS: &[CornerGlyph] = &[
    ("_corner.five", &[], &["o:(0,24,l),(-84,24,l),(-84,-34,l),(58,-34,l)"]),
    ("_corner.four", &[("origin", 0.0, 500.0)], &["o:(0,550,l),(-30,502,l),(3,474,l),(50,500,l)"]),
    ("_corner.large", &[], &["o:(0,0,l),(8,-11,l),(106,-11,l),(116,0,l)"]),
    ("_corner.one", &[], &["o:(0,50,l),(-50,50,l),(-50,0,l),(25,0,l)"]),
    (
        "_corner.pretty",
        &[],
        &[
            "o:(0,27,l),(-39,65,o),(-124,11,o),(-145,-52,c),(-105,-77,o),(-55,-86,o),(9,0,c)",
            "c:(-108,-51,o),(-118,-41,o),(-118,-29,cs),(-118,-18,o),(-108,-8,o),(-97,-8,cs),(-85,-8,o),(-75,-18,o),(-75,-29,cs),(-75,-41,o),(-85,-51,o),(-97,-51,cs)",
            "c:(-51,-102,o),(-113,-94,o),(-165,-63,c),(-136,19,o),(-29,85,o),(19,37,c),(27,45,l),(-30,100,o),(-150,22,o),(-177,-67,c),(-122,-102,o),(-51,-118,o),(39,2,c),(31,8,l)",
        ],
    ),
    ("_corner.seven", &[("left", 0.0, 100.0)], &["o:(0,150,l),(-150,150,l),(-150,0,l),(25,0,l)"]),
    ("_corner.slab", &[], &["o:(0,27,l),(-96,27,l),(-96,0,l),(9,0,l)"]),
    ("_corner.three", &[], &["o:(0,50,l),(-25,50,o),(-74,53,o),(-90,32,c),(-100,10,l),(-64,32,o),(-16,14,o),(20,0,c)"]),
    ("_corner.two", &[], &["o:(0,50,l),(-34,50,o),(-87,50,o),(-87,0,c),(21,0,l)"]),
];

/// A path from `c:(x,y,l),(x,y,o),(x,y,cs)…` (`o:` for an open path), as `.glyphs` writes nodes.
fn path(text: &str) -> Path {
    let (kind, list) = text.split_once(':').unwrap();
    let nodes = list
        .trim_matches(|c| c == '(' || c == ')')
        .split("),(")
        .map(|n| {
            let parts: Vec<&str> = n.split(',').collect();
            let (x, y) = (parts[0].parse().unwrap(), parts[1].parse().unwrap());
            let mut node = Node::new(
                x,
                y,
                match parts[2].trim_end_matches('s') {
                    "l" => NodeKind::Line,
                    "c" => NodeKind::Curve,
                    "q" => NodeKind::QCurve,
                    "o" => NodeKind::OffCurve,
                    other => panic!("node kind {other}"),
                },
            );
            node.smooth = parts[2].len() == 2;
            node
        })
        .collect();
    let mut nodes: Vec<Node> = nodes;
    // Written in the reference's file order, a closed path's start node last; the reader moves
    // it to the front.
    if kind == "c" {
        nodes.rotate_right(1);
    }
    Path::new(kind == "c", nodes)
}

fn points(shapes: &[Shape]) -> Vec<Vec<(f64, f64)>> {
    shapes.iter().filter_map(|s| if let Shape::Path(p) = s { Some(p.nodes.iter().map(|n| (n.x, n.y)).collect()) } else { None }).collect()
}

/// Draws `name` on master `m01`, adding the glyph unless the starting glyph set has it.
fn add_glyph(font: &mut Font, name: &str, anchors: &[(&str, f64, f64)], paths: &[&str]) {
    if font.glyph(name).is_none() {
        font.add_glyph(name, Vec::new(), 600.0).unwrap();
    }
    let mut g = font.glyph_mut(name).unwrap();
    let l = g.layer_mut("m01").unwrap();
    l.shapes = paths.iter().map(|p| Shape::Path(path(p))).collect();
    l.anchors = anchors.iter().map(|&(n, x, y)| Anchor::new(n, x, y)).collect();
}

fn set_hints(font: &mut Font, glyph: &str, hints: Vec<Hint>) {
    let mut g = font.glyph_mut(glyph).unwrap();
    g.layer_mut("m01").unwrap().info.hints = hints;
}

/// A font with every corner glyph of the cases and the case glyph `name`.
fn case_font(case: &Case) -> Font {
    let (name, hints, paths, _) = *case;
    let mut font = Font::new("Corners");
    for &(corner, anchors, corner_paths) in CORNERS {
        add_glyph(&mut font, corner, anchors, corner_paths);
    }
    add_glyph(&mut font, name, &[], paths);
    let hints = hints
        .iter()
        .map(|&(corner, shape, node, sx, sy, options)| {
            let mut h = CornerHint::new(corner, shape, node);
            h.scale = Vec2::new(sx, sy);
            h.alignment = CornerAlignment::from_option(options as f64);
            h.to_hint()
        })
        .collect();
    set_hints(&mut font, name, hints);
    font
}

fn fitted(font: &Font, glyph: &str) -> Vec<Shape> {
    insert_corners(font, font.glyph(glyph).unwrap().layer("m01").unwrap()).unwrap().unwrap()
}

/// Every case of the test source, mirrored corners (`an_flippy`, `ap_twoofthem`, `aq_rightleg`,
/// `ar_leftleg`), closed paths in the corner glyph and an `origin` anchor included, gives the
/// reference app's nodes.
#[test]
fn corner_components_fit_as_the_reference_draws_them() {
    let mut wrong = Vec::new();
    for case in CASES.iter().filter(|c| c.0 != "av_left_anchoroffpath") {
        let font = case_font(case);
        let got = points(&fitted(&font, case.0));
        let want: Vec<Shape> = case.3.iter().map(|p| Shape::Path(path(p))).collect();
        if got != points(&want) {
            wrong.push(format!("{}: got {got:?}", case.0));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// A `left` anchor above the origin on a slanted stem: the corner slides along its baseline until
/// the anchor is on the stem, as the reference app draws `av_left_anchoroffpath`.
#[test]
fn the_entry_anchor_moves_the_corner_onto_the_stem() {
    let case = CASES.iter().find(|c| c.0 == "av_left_anchoroffpath").unwrap();
    let font = case_font(case);
    let want: Vec<Shape> = case.3.iter().map(|p| Shape::Path(path(p))).collect();
    assert_eq!(points(&fitted(&font, case.0)), points(&want));
}

/// A `right` anchor stands in for the end node: the corner is not tilted by an end node drawn
/// above the baseline, and the end node stays where it was drawn.
#[test]
fn the_exit_anchor_keeps_the_end_node_where_it_was_drawn() {
    let mut font = Font::new("Cup");
    add_glyph(&mut font, "_corner.cup", &[("right", 25.0, 0.0)], &["o:(0,50,l),(-50,50,l),(-50,0,l),(25,5,l)"]);
    add_glyph(&mut font, "I", &[], &["c:(100,0,l),(400,0,l),(400,500,l),(100,500,l)"]);
    set_hints(&mut font, "I", vec![CornerHint::new("_corner.cup", 0, 0).to_hint()]);
    // In the model's order: the start node first.
    let want = vec![vec![(100.0, 500.0), (100.0, 50.0), (50.0, 50.0), (50.0, 0.0), (125.0, 5.0), (400.0, 0.0), (400.0, 500.0)]];
    assert_eq!(points(&fitted(&font, "I")), want);
}

/// The outline of a glyph has its corners, and so has a composite that uses it; the stored
/// shapes keep the hint.
#[test]
fn outlines_have_their_corners_fitted() {
    let case = CASES.iter().find(|c| c.0 == "aa_simple_angleinstroke").unwrap();
    let mut font = case_font(case);
    let want = path(case.3[0]).to_bez().unwrap();
    assert_eq!(font.outline(case.0, "m01").unwrap(), want);
    font.add_glyph("user", Vec::new(), 600.0).unwrap();
    font.glyph_mut("user").unwrap().layer_mut("m01").unwrap().shapes.push(Shape::Component(crate::Component::new(case.0, 0.0, 0.0)));
    assert_eq!(font.outline("user", "m01").unwrap(), want);
    assert_eq!(font.glyph(case.0).unwrap().layer("m01").unwrap().info.hints.len(), 1);
}

/// A corner on an extra node (an intersection of two paths) is left out; the rest is drawn.
#[test]
fn a_corner_on_an_extra_node_is_left_out() {
    let mut font = Font::new("Extra");
    add_glyph(&mut font, "_corner.one", &[], &["o:(0,50,l),(-50,50,l),(-50,0,l),(25,0,l)"]);
    add_glyph(&mut font, "T", &[], &["c:(0,400,l),(500,400,l),(500,500,l),(0,500,l)", "c:(200,0,l),(300,0,l),(300,600,l),(200,600,l)"]);
    let mut hint = CornerHint::new("_corner.one", 0, 1);
    hint.target = CornerTarget::Intersection { shape: 0, node: 1, other_shape: 1, other_node: 2 };
    set_hints(&mut font, "T", vec![hint.to_hint()]);
    let layer = font.glyph("T").unwrap().layer("m01").unwrap();
    assert_eq!(corner_hint(&layer.info.hints[0]).unwrap(), Some(hint));
    assert_eq!(insert_corners(&font, layer).unwrap().unwrap(), layer.shapes);
    assert!(font.outline("T", "m01").is_ok());
}

/// Hostile corners fail with a message and never panic: a scale of 10^9, a corner next to a
/// zero-length segment, a path index past the shapes, a corner glyph whose path has one node.
/// The outline is then drawn without the corners, and composites using the glyph still resolve.
#[test]
fn bad_corners_fail_with_a_message() {
    let mut font = Font::new("Bad");
    add_glyph(&mut font, "_corner.one", &[], &["o:(0,50,l),(-50,50,l),(-50,0,l),(25,0,l)"]);
    add_glyph(&mut font, "_corner.dot", &[], &["o:(0,50,l)"]);
    add_glyph(&mut font, "I", &[], &["c:(100,0,l),(100,0,l),(400,0,l),(400,500,l),(100,500,l)"]);
    add_glyph(&mut font, "Icomp", &[], &[]);
    font.glyph_mut("Icomp").unwrap().layer_mut("m01").unwrap().shapes.push(Shape::Component(crate::Component::new("I", 0.0, 0.0)));
    let plain = font.outline("I", "m01").unwrap();
    let mut huge = CornerHint::new("_corner.one", 0, 2);
    huge.scale = Vec2::new(1e9, 1.0);
    for hint in [huge, CornerHint::new("_corner.one", 0, 1), CornerHint::new("_corner.one", 7, 0), CornerHint::new("_corner.dot", 0, 3)] {
        set_hints(&mut font, "I", vec![hint.to_hint()]);
        let layer = font.glyph("I").unwrap().layer("m01").unwrap();
        assert!(insert_corners(&font, layer).is_err(), "{hint:?}");
        assert_eq!(font.outline("I", "m01").unwrap(), plain, "{hint:?}");
        assert_eq!(font.outline("Icomp", "m01").unwrap(), plain, "{hint:?}");
    }
    // The path a fitting corner sits on is deleted.
    set_hints(&mut font, "I", vec![CornerHint::new("_corner.one", 0, 3).to_hint()]);
    assert_ne!(font.outline("I", "m01").unwrap(), plain);
    font.glyph_mut("I").unwrap().layer_mut("m01").unwrap().shapes.clear();
    assert!(font.outline("I", "m01").unwrap().elements().is_empty());
    assert!(font.outline("Icomp", "m01").unwrap().elements().is_empty());
    // A corner glyph that does not exist is skipped, as the reference compilers skip it.
    set_hints(&mut font, "I", vec![CornerHint::new("_corner.none", 0, 3).to_hint()]);
    assert!(font.outline("I", "m01").is_ok());
}

/// The hint keys of both format versions read back to the same corner.
#[test]
fn corner_hints_read_both_format_versions() {
    let mut hint = CornerHint::new("_corner.cut", 0, 7);
    hint.scale = Vec2::new(1.1, 0.9);
    hint.alignment = CornerAlignment::Right;
    assert_eq!(corner_hint(&hint.to_hint()).unwrap(), Some(hint.clone()));
    let v2 = fontcraft_plist::openstep::parse_value("{name = _corner.cut; options = 1; origin = \"{0, 7}\"; scale = \"{1.1, 0.9}\"; type = Corner;}")
        .unwrap();
    assert_eq!(corner_hint(&Hint { family: Family::Glyphs, value: v2 }).unwrap(), Some(hint));
}

/// Decomposing the first of two corners on a path gives the same outline as fitting both, and
/// the second hint follows its node.
#[test]
fn decomposing_one_corner_keeps_the_other_on_its_node() {
    let case = CASES.iter().find(|c| c.0 == "ap_twoofthem").unwrap();
    let font = case_font(case);
    let layer = font.glyph(case.0).unwrap().layer("m01").unwrap();
    let once = decompose_corners(&font, layer, &[0]).unwrap();
    assert_eq!(once.info.hints.len(), 1);
    let Some(CornerHint { target: CornerTarget::Node { node, .. }, .. }) = corner_hint(&once.info.hints[0]).unwrap() else { panic!() };
    assert_eq!(node, 5);
    let both = fitted(&font, case.0);
    assert_eq!(points(&insert_corners(&font, &once).unwrap().unwrap()), points(&both));
    let all = decompose_corners(&font, &once, &[0]).unwrap();
    assert!(all.info.hints.is_empty());
    assert_eq!(points(&all.shapes), points(&both));
}

/// An intermediate layer at 150 % width: a 125 % scale draws the outline halfway between the
/// master layer and that layer, and a height scale still stretches.
#[test]
fn intermediate_width_layers_are_interpolated() {
    let host = ["c:(100,0,l),(400,0,l),(400,500,l),(100,500,l)"];
    let mut layered = Font::new("Layered");
    add_glyph(&mut layered, "_corner.wide", &[], &["o:(0,50,l),(-50,50,l),(-50,0,l),(25,0,l)"]);
    let mut wide = Layer::new("m01", 600.0);
    wide.id = "wide".into();
    wide.info = LayerInfoBox::new(LayerInfo { name: Some("Width 150 Height 100".into()), ..Default::default() });
    wide.shapes.push(Shape::Path(path("o:(0,50,l),(-100,50,l),(-100,0,l),(25,0,l)")));
    layered.glyph_mut("_corner.wide").unwrap().layers.push(wide);
    add_glyph(&mut layered, "I", &[], &host);
    let mut plain = Font::new("Plain");
    add_glyph(&mut plain, "_corner.wide", &[], &["o:(0,50,l),(-75,50,l),(-75,0,l),(25,0,l)"]);
    add_glyph(&mut plain, "I", &[], &host);
    for (sy, stretch) in [(1.0, 1.0), (2.0, 2.0)] {
        let mut a = CornerHint::new("_corner.wide", 0, 0);
        a.scale = Vec2::new(1.25, sy);
        set_hints(&mut layered, "I", vec![a.to_hint()]);
        let mut b = CornerHint::new("_corner.wide", 0, 0);
        b.scale = Vec2::new(1.0, stretch);
        set_hints(&mut plain, "I", vec![b.to_hint()]);
        assert_eq!(points(&fitted(&layered, "I")), points(&fitted(&plain, "I")), "height {sy}");
    }
}
