//! Automatic alignment on fonts built in code: the setups of `docs/spec/09-reusing-shapes.md`
//! § 5.4 and each stored alignment value.

use std::sync::Arc;

use fontcraft_model::{Alignment, Anchor, Component, ComponentInfo, ComponentInfoBox, Font, Glyph, GlyphTable, Layer, Path, Shape, Transform};

use crate::align::{AlignMode, align_layer, aligned_components, aligned_placement, component_alignment, is_aligned};

pub(crate) const M: &str = "m01";

/// A font whose glyphs are exactly `glyphs`, on the one master `m01`.
pub(crate) fn font(glyphs: Vec<Glyph>) -> Font {
    let mut f = Font::new("Test");
    f.glyphs = GlyphTable::from_glyphs(glyphs.into_iter().map(Arc::new).collect());
    f
}

/// A drawn glyph: one rectangle, a width and anchors.
pub(crate) fn drawn(name: &str, width: f64, rect: (f64, f64, f64, f64), anchors: &[(&str, f64, f64)]) -> Glyph {
    let mut g = Glyph::new(name);
    let mut l = Layer::new(M, width);
    l.shapes.push(Shape::Path(Path::rect(rect.0, rect.1, rect.2, rect.3)));
    l.anchors = anchors.iter().map(|(n, x, y)| Anchor::new(*n, *x, *y)).collect();
    g.layers.push(l);
    g
}

/// A glyph made of `parts`, with a stale width of 1.
pub(crate) fn composite(name: &str, parts: Vec<Component>) -> Glyph {
    let mut g = Glyph::new(name);
    let mut l = Layer::new(M, 1.0);
    l.shapes = parts.into_iter().map(Shape::Component).collect();
    g.layers.push(l);
    g
}

pub(crate) fn part(glyph: &str) -> Component {
    Component::new(glyph, 0.0, 0.0)
}

pub(crate) fn part_at(glyph: &str, x: f64, y: f64) -> Component {
    Component::new(glyph, x, y)
}

pub(crate) fn with_info(mut c: Component, info: ComponentInfo) -> Component {
    c.info = ComponentInfoBox::new(info);
    c
}

pub(crate) fn scaled(glyph: &str, x: f64, y: f64, sx: f64, sy: f64) -> Component {
    let mut c = Component::new(glyph, 0.0, 0.0);
    c.transform = Transform::Decomposed { pos: (x, y), scale: (sx, sy), angle: 0.0, slant: (0.0, 0.0) };
    c
}

/// `A`: width 600, outline 20–580, `top` at (300, 700), `bottom` at (300, 0).
pub(crate) fn letter_a() -> Glyph {
    drawn("A", 600.0, (20.0, 0.0, 580.0, 700.0), &[("top", 300.0, 700.0), ("bottom", 300.0, 0.0), ("top_viet", 350.0, 720.0)])
}

/// A zero-width top mark: `_top` at (0, 500), `top` at (0, 650).
pub(crate) fn top_mark(name: &str) -> Glyph {
    drawn(name, 0.0, (-50.0, 520.0, 50.0, 620.0), &[("_top", 0.0, 500.0), ("top", 0.0, 650.0)])
}

/// The position of the component at shape index `i` of `glyph`'s master layer in `layer`.
pub(crate) fn pos(layer: &Layer, i: usize) -> (f64, f64) {
    match &layer.shapes[i] {
        Shape::Component(c) => {
            let k = c.affine().as_coeffs();
            (k[4], k[5])
        }
        other => panic!("shape {i} is not a component: {other:?}"),
    }
}

fn aligned(f: &Font, glyph: &str) -> Layer {
    align_layer(f, glyph, M).unwrap().unwrap_or_else(|| panic!("{glyph} should change"))
}

#[test]
fn a_mark_puts_its_underscore_anchor_on_the_base_anchor() {
    let f = font(vec![letter_a(), top_mark("acutecomb"), composite("Aacute", vec![part_at("A", 37.0, 11.0), part("acutecomb")])]);
    let l = aligned(&f, "Aacute");
    assert_eq!(pos(&l, 0), (0.0, 0.0), "the base sits at the origin");
    assert_eq!(pos(&l, 1), (300.0, 200.0), "_top (0, 500) lands on top (300, 700)");
    assert_eq!(l.width, 600.0, "the width is the base's");
    // Aligned again, nothing moves.
    let mut again = f.clone();
    again.glyphs.get_mut("Aacute").unwrap().layers[0] = l;
    assert_eq!(align_layer(&again, "Aacute", M).unwrap(), None);
}

#[test]
fn marks_stack_in_the_order_they_were_added() {
    let f = font(vec![
        letter_a(),
        top_mark("acutecomb"),
        top_mark("gravecomb"),
        composite("Aacutegrave", vec![part("A"), part("acutecomb"), part("gravecomb")]),
    ]);
    let l = aligned(&f, "Aacutegrave");
    assert_eq!(pos(&l, 1), (300.0, 200.0));
    assert_eq!(pos(&l, 2), (300.0, 350.0), "the grave sits on the acute's top at (300, 850)");
}

#[test]
fn the_anchor_attribute_picks_an_alternative_and_falls_back_when_it_is_gone() {
    let viet = with_info(part("acutecomb"), ComponentInfo { anchor: Some("top_viet".into()), ..Default::default() });
    let gone = with_info(part("acutecomb"), ComponentInfo { anchor: Some("top_gone".into()), ..Default::default() });
    let f = font(vec![letter_a(), top_mark("acutecomb"), composite("viet", vec![part("A"), viet]), composite("gone", vec![part("A"), gone])]);
    assert_eq!(pos(&aligned(&f, "viet"), 1), (350.0, 220.0), "top_viet at (350, 720)");
    assert_eq!(pos(&aligned(&f, "gone"), 1), (300.0, 200.0), "a vanished name falls back to top");
}

#[test]
fn a_vertically_flipped_mark_attaches_through_its_swapped_anchor() {
    // A turned comma: its `_bottom` is on top once flipped, and attaches to `top`.
    let comma = drawn("commaaccentcomb", 0.0, (-30.0, -200.0, 30.0, -20.0), &[("_bottom", 0.0, 10.0), ("bottom", 0.0, -220.0)]);
    let f = font(vec![letter_a(), comma, composite("Acomma", vec![part("A"), scaled("commaaccentcomb", 0.0, 0.0, 1.0, -1.0)])]);
    let l = aligned(&f, "Acomma");
    assert_eq!(pos(&l, 1), (300.0, 710.0), "flipped `_bottom` (0, -10) meets top (300, 700)");
    match &l.shapes[1] {
        Shape::Component(c) => assert!(matches!(c.transform, Transform::Decomposed { scale: (1.0, -1.0), .. }), "the flip is kept"),
        _ => panic!("not a component"),
    }
}

#[test]
fn several_bases_sit_side_by_side_and_marks_follow_the_base_before_them() {
    let n = drawn("n", 500.0, (50.0, 0.0, 450.0, 500.0), &[("top", 250.0, 500.0)]);
    let o = drawn("o", 520.0, (40.0, 0.0, 480.0, 500.0), &[("top", 260.0, 500.0)]);
    let f = font(vec![n, o, top_mark("acutecomb"), composite("n_o", vec![part("n"), part("acutecomb"), part_at("o", 3.0, 4.0), part("acutecomb")])]);
    let l = aligned(&f, "n_o");
    assert_eq!(pos(&l, 1), (250.0, 0.0), "the first acute sits on n");
    assert_eq!(pos(&l, 2), (500.0, 0.0), "o follows n's advance");
    assert_eq!(pos(&l, 3), (760.0, 0.0), "the second acute sits on o, not on n");
    assert_eq!(l.width, 1020.0, "the advances add up, with no kerning");
}

#[test]
fn a_mark_without_a_matching_anchor_is_set_after_the_base() {
    let bare = drawn("bare", 400.0, (0.0, 0.0, 400.0, 500.0), &[]);
    let f = font(vec![bare, top_mark("acutecomb"), composite("bareacute", vec![part("bare"), part_at("acutecomb", 9.0, 9.0)])]);
    let l = aligned(&f, "bareacute");
    assert_eq!(pos(&l, 1), (400.0, 0.0));
    assert_eq!(l.width, 400.0, "the zero-width mark adds nothing");
}

#[test]
fn exit_and_entry_anchors_connect_bases() {
    let stem = drawn("_part.stem", 120.0, (20.0, 0.0, 100.0, 500.0), &[("#exit", 100.0, 400.0)]);
    let arch = drawn("_part.arch", 300.0, (0.0, 300.0, 300.0, 500.0), &[("#entry", 0.0, 400.0), ("#exit", 300.0, 400.0)]);
    let f = font(vec![stem, arch, composite("m", vec![part("_part.stem"), part("_part.arch"), part("_part.arch")])]);
    let l = aligned(&f, "m");
    assert_eq!(pos(&l, 1), (100.0, 0.0), "#entry meets the stem's #exit, not its advance at 120");
    assert_eq!(pos(&l, 2), (400.0, 0.0), "the chain continues from the first arch's #exit");
    assert_eq!(l.width, 700.0, "the width ends at the last base's advance");
}

#[test]
fn a_numbered_anchor_attribute_reaches_its_ligature_base() {
    let f_glyph = drawn("f", 300.0, (40.0, 0.0, 260.0, 700.0), &[("top", 150.0, 700.0)]);
    let first = with_info(part("acutecomb"), ComponentInfo { anchor: Some("top_1".into()), ..Default::default() });
    let f = font(vec![f_glyph, top_mark("acutecomb"), composite("f_f", vec![part("f"), part("f"), first])]);
    let l = aligned(&f, "f_f");
    assert_eq!(pos(&l, 1), (300.0, 0.0));
    assert_eq!(pos(&l, 2), (150.0, 200.0), "top_1 is the first f's top, though the second f came last");
}

#[test]
fn a_single_number_component_aligns_horizontally_and_keeps_its_height() {
    let six = drawn("six", 560.0, (40.0, -10.0, 520.0, 710.0), &[]);
    let mut nine = composite("nine", vec![scaled("six", 7.0, 700.0, -1.0, -1.0)]);
    nine.unicodes.push(0x39);
    let f = font(vec![six, nine]);
    assert_eq!(component_alignment(&f, &f.glyph("nine").unwrap().layers[0], 0), Some(AlignMode::Horizontal));
    let l = aligned(&f, "nine");
    assert_eq!(pos(&l, 0), (560.0, 700.0), "turned around, it is shifted by its width; the height is the designer's");
    assert_eq!(l.width, 560.0);
}

#[test]
fn a_single_flipped_component_keeps_the_base_sidebearings_mirrored() {
    let left = drawn("parenleft", 300.0, (80.0, -100.0, 260.0, 700.0), &[]);
    let f = font(vec![left, composite("parenright", vec![scaled("parenleft", 0.0, 50.0, -1.0, 1.0)])]);
    let l = aligned(&f, "parenright");
    assert_eq!(pos(&l, 0), (300.0, 0.0), "outline 40–220: the sidebearings 80 and 40 swap");
    assert_eq!(l.width, 300.0);
}

#[test]
fn each_stored_alignment_value_decides_the_component() {
    let forced = with_info(part_at("acutecomb", 5.0, 5.0), ComponentInfo { alignment: Alignment::Forced, ..Default::default() });
    let off = with_info(part_at("acutecomb", 5.0, 5.0), ComponentInfo { alignment: Alignment::Disabled, ..Default::default() });
    let horizontal = with_info(part_at("acutecomb", 5.0, 5.0), ComponentInfo { alignment: Alignment::Horizontal, ..Default::default() });
    let mut mixed = composite("mixed", vec![part("A"), forced]);
    mixed.layers[0].shapes.insert(0, Shape::Path(Path::rect(0.0, 0.0, 10.0, 10.0)));
    let f = font(vec![
        letter_a(),
        top_mark("acutecomb"),
        composite("plain", vec![part("A"), part_at("acutecomb", 5.0, 5.0)]),
        composite("off", vec![part("A"), off]),
        composite("horizontal", vec![part("A"), horizontal]),
        mixed,
    ]);
    let layer = |g: &str| f.glyph(g).unwrap().layers[0].clone();

    assert_eq!(aligned_components(&f, &layer("plain")), vec![(0, AlignMode::Full), (1, AlignMode::Full)], "default on a layer of components");
    assert_eq!(aligned_components(&f, &layer("off")), vec![(0, AlignMode::Full)]);
    assert_eq!(pos(&aligned(&f, "off"), 1), (5.0, 5.0), "a disabled component stays where it is");
    assert_eq!(pos(&aligned(&f, "horizontal"), 1), (300.0, 5.0), "horizontal only: the height is kept");

    // A path turns default components off; a forced one is still placed, and the width stays.
    assert_eq!(aligned_components(&f, &layer("mixed")), vec![(2, AlignMode::Full)]);
    assert_eq!(component_alignment(&f, &layer("mixed"), 0), None, "a path is not a component");
    let l = aligned(&f, "mixed");
    assert_eq!(pos(&l, 1), (0.0, 0.0), "the default base next to a path keeps its place");
    assert_eq!(pos(&l, 2), (300.0, 200.0));
    assert_eq!(l.width, 1.0, "a layer with a path keeps its width");
}

#[test]
fn the_font_setting_turns_every_component_off() {
    let forced = with_info(part_at("acutecomb", 5.0, 5.0), ComponentInfo { alignment: Alignment::Forced, ..Default::default() });
    let mut f = font(vec![letter_a(), top_mark("acutecomb"), composite("Aacute", vec![part_at("A", 9.0, 0.0), forced])]);
    Arc::make_mut(&mut f.info).settings.disables_automatic_alignment = Some(true);
    let l = f.glyph("Aacute").unwrap().layers[0].clone();
    assert!(!is_aligned(&f, &l), "the font setting wins over forced components");
    assert_eq!(align_layer(&f, "Aacute", M).unwrap(), None);
    assert!(aligned_placement(&f, "Aacute", M).unwrap().is_empty());
}

#[test]
fn placements_report_only_aligned_components() {
    let off = with_info(part("acutecomb"), ComponentInfo { alignment: Alignment::Disabled, ..Default::default() });
    let f = font(vec![letter_a(), top_mark("acutecomb"), composite("Aacute", vec![part_at("A", 9.0, 0.0), off])]);
    let p = aligned_placement(&f, "Aacute", M).unwrap();
    assert_eq!(p.len(), 1);
    assert_eq!((p[0].shape, p[0].mode), (0, AlignMode::Full));
    assert_eq!(p[0].transform, Transform::translate(0.0, 0.0));
    assert!(aligned_placement(&f, "nothing", M).is_err(), "a missing glyph is an error");
}

#[test]
fn hostile_placements_fail_with_an_error() {
    // Bases this wide add up past the model's coordinate limit.
    let wide = drawn("wide", 600_000.0, (0.0, 0.0, 10.0, 10.0), &[]);
    let f = font(vec![wide, composite("twice", vec![part("wide"), part("wide")])]);
    assert!(align_layer(&f, "twice", M).is_err());
    // A component of itself: no crash, whatever propagation reports.
    let f = font(vec![composite("loop", vec![part("loop")])]);
    let _ = align_layer(&f, "loop", M);
}
