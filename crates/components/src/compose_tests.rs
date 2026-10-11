//! Behaviour of composition and default anchors (M6.3): `docs/spec/09-reusing-shapes.md` § 2.1,
//! the rows named in `compose.rs` and `default_anchors.rs`.

use fontcraft_glyphdata::{GlyphData, Record};
use fontcraft_model::{Anchor, Component, Font, Node, NodeKind, Path, Shape, metric};

use crate::ComponentsError;
use crate::compose::{components_for, composite_glyph};
use crate::default_anchors::{default_anchor_names, default_anchors};

const MASTER: &str = "m01";

fn data() -> GlyphData {
    GlyphData::new().unwrap()
}

/// The model's starter font (A–Z, a–z, one master `m01` with x-height 500, cap height 700 and
/// ascender 800) plus empty glyphs with these names.
fn font_with(names: &[&str]) -> Font {
    let mut font = Font::new("Compose");
    for n in names {
        font.add_glyph(n, Vec::new(), 0.0).unwrap();
    }
    font
}

/// Gives `name` (added when missing) a width, an optional box outline and anchors on `m01`.
fn draw(font: &mut Font, name: &str, width: f64, rect: Option<(f64, f64, f64, f64)>, anchors: &[(&str, f64, f64)]) {
    if font.glyph(name).is_none() {
        font.add_glyph(name, Vec::new(), width).unwrap();
    }
    let mut g = font.glyph_mut(name).unwrap();
    let layer = g.layer_mut(MASTER).unwrap();
    layer.width = width;
    layer.shapes = rect.map(|(x0, y0, x1, y1)| Shape::Path(Path::rect(x0, y0, x1, y1))).into_iter().collect();
    layer.anchors = anchors.iter().map(|(n, x, y)| Anchor::new(*n, *x, *y)).collect();
}

fn parts(font: &Font, name: &str) -> Vec<String> {
    components_for(font, &data(), name).unwrap().unwrap_or_else(|| panic!("no composition for {name}"))
}

fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn missing(glyph: &str, parts: &[&str]) -> ComponentsError {
    ComponentsError::MissingParts { glyph: glyph.into(), missing: names(parts) }
}

// ── components_for ──

#[test]
fn accented_letters_decompose_into_base_and_mark() {
    let font = font_with(&["gravecomb", "acutecomb", "ringcomb"]);
    assert_eq!(parts(&font, "Agrave"), names(&["A", "gravecomb"]));
    assert_eq!(parts(&font, "agrave"), names(&["a", "gravecomb"]));
    assert_eq!(parts(&font, "eacute"), names(&["e", "acutecomb"]));
    // The decomposition is expanded fully: U+01FA is Aring plus acute, Aring is A plus ring.
    assert_eq!(parts(&font, "Aringacute"), names(&["A", "ringcomb", "acutecomb"]));
}

#[test]
fn composition_is_all_or_nothing_and_names_every_missing_part() {
    let font = font_with(&[]);
    assert_eq!(components_for(&font, &data(), "Agrave"), Err(missing("Agrave", &["gravecomb"])));
    assert_eq!(components_for(&font, &data(), "Aringacute"), Err(missing("Aringacute", &["ringcomb", "acutecomb"])));
}

#[test]
fn names_no_rule_decomposes_give_none() {
    let font = font_with(&["gravecomb"]);
    for name in ["A", "myGlyph", "Lslash", "period", "", "_part.stem", "uniF8FF"] {
        assert_eq!(components_for(&font, &data(), name), Ok(None), "{name}");
    }
    let long = "a".repeat(300);
    assert!(matches!(components_for(&font, &data(), &long), Err(ComponentsError::LimitExceeded { .. })));
}

#[test]
fn uppercase_composites_prefer_case_marks() {
    let font = font_with(&["gravecomb", "gravecomb.case"]);
    assert_eq!(parts(&font, "Agrave"), names(&["A", "gravecomb.case"]));
    assert_eq!(parts(&font, "agrave"), names(&["a", "gravecomb"]));
}

#[test]
fn a_suffix_goes_to_the_base_and_preferably_to_the_marks() {
    let mut font = font_with(&["a.sc", "gravecomb", "gravecomb.sc", "gravecomb.case"]);
    assert_eq!(parts(&font, "agrave.sc"), names(&["a.sc", "gravecomb.sc"]));
    font.remove_glyph("gravecomb.sc").unwrap();
    assert_eq!(parts(&font, "agrave.sc"), names(&["a.sc", "gravecomb"]));
    font.remove_glyph("a.sc").unwrap();
    assert_eq!(components_for(&font, &data(), "agrave.sc"), Err(missing("agrave.sc", &["a.sc"])));
}

#[test]
fn i_and_j_are_built_from_the_dotless_letters() {
    let font = font_with(&["idotless", "jdotless", "dotaccentcomb"]);
    assert_eq!(parts(&font, "i"), names(&["idotless", "dotaccentcomb"]));
    assert_eq!(parts(&font, "j"), names(&["jdotless", "dotaccentcomb"]));
    // A small capital i has no dot: without idotless.sc there is nothing to compose.
    assert_eq!(components_for(&font, &data(), "i.sc"), Ok(None));
}

#[test]
fn marks_above_take_the_dot_off_i_and_j_and_prefer_narrow_marks() {
    let mut font = font_with(&["idotless", "jdotless", "macroncomb", "circumflexcomb", "ogonekcomb"]);
    assert_eq!(parts(&font, "imacron"), names(&["idotless", "macroncomb"]));
    assert_eq!(parts(&font, "jcircumflex"), names(&["jdotless", "circumflexcomb"]));
    // A mark below keeps the dot.
    assert_eq!(parts(&font, "iogonek"), names(&["i", "ogonekcomb"]));
    font.add_glyph("macroncomb.i", Vec::new(), 0.0).unwrap();
    font.add_glyph("circumflexcomb.narrow", Vec::new(), 0.0).unwrap();
    assert_eq!(parts(&font, "imacron"), names(&["idotless", "macroncomb.i"]));
    assert_eq!(parts(&font, "jcircumflex"), names(&["jdotless", "circumflexcomb.narrow"]));
    // Uppercase I keeps its shape and takes case marks, not narrow ones.
    font.add_glyph("macroncomb.case", Vec::new(), 0.0).unwrap();
    assert_eq!(parts(&font, "Imacron"), names(&["I", "macroncomb.case"]));
}

#[test]
fn ligatures_are_built_from_their_name_parts() {
    let mut font = font_with(&["f.ss01"]);
    assert_eq!(parts(&font, "f_f_l"), names(&["f", "f", "l"]));
    assert_eq!(parts(&font, "f_f_l.ss01"), names(&["f.ss01", "f.ss01", "l"]));
    font.remove_glyph("l").unwrap();
    assert_eq!(components_for(&font, &data(), "f_f_l"), Err(missing("f_f_l", &["l"])));
}

#[test]
fn our_exceptions_to_the_unicode_decomposition() {
    let mut font = font_with(&["commaaccentcomb", "commaturnedabovecomb", "cedillacomb", "caroncomb", "acutecomb", "horncomb"]);
    // Unicode has the cedilla in these; the letters are drawn with a comma.
    assert_eq!(parts(&font, "Gcommaaccent"), names(&["G", "commaaccentcomb"]));
    assert_eq!(parts(&font, "ncommaaccent"), names(&["n", "commaaccentcomb"]));
    assert_eq!(parts(&font, "gcommaaccent"), names(&["g", "commaturnedabovecomb"]));
    assert_eq!(parts(&font, "scedilla"), names(&["s", "cedillacomb"]));
    // The vertical caron, when the font has one.
    assert_eq!(parts(&font, "dcaron"), names(&["d", "caroncomb"]));
    font.add_glyph("caroncomb.alt", Vec::new(), 0.0).unwrap();
    assert_eq!(parts(&font, "dcaron"), names(&["d", "caroncomb.alt"]));
    assert_eq!(parts(&font, "Lcaron"), names(&["L", "caroncomb.alt"]));
    assert_eq!(parts(&font, "ccaron"), names(&["c", "caroncomb"]));
    // A letter with a horn stays whole when the font has it.
    assert_eq!(parts(&font, "Ohornacute"), names(&["O", "horncomb", "acutecomb"]));
    font.add_glyph("Ohorn", Vec::new(), 0.0).unwrap();
    assert_eq!(parts(&font, "Ohornacute"), names(&["Ohorn", "acutecomb"]));
}

#[test]
fn two_marks_become_their_mark_ligature_when_the_font_has_it() {
    let mut font = font_with(&["circumflexcomb", "gravecomb"]);
    assert_eq!(parts(&font, "Acircumflexgrave"), names(&["A", "circumflexcomb", "gravecomb"]));
    font.add_glyph("circumflexcomb_gravecomb", Vec::new(), 0.0).unwrap();
    assert_eq!(parts(&font, "Acircumflexgrave"), names(&["A", "circumflexcomb_gravecomb"]));
    font.add_glyph("circumflexcomb_gravecomb.case", Vec::new(), 0.0).unwrap();
    assert_eq!(parts(&font, "Acircumflexgrave"), names(&["A", "circumflexcomb_gravecomb.case"]));
    // The mark ligature itself is composed from its parts.
    assert_eq!(parts(&font, "circumflexcomb_gravecomb"), names(&["circumflexcomb", "gravecomb"]));
}

#[test]
fn a_custom_glyph_data_file_overrides_the_decomposition() {
    let font = font_with(&["gravecomb", "gravecomb.alt"]);
    let custom = GlyphData::with_custom(vec![vec![
        Record { name: "Agrave".into(), decompose: Some("A, gravecomb.alt".into()), ..Record::default() },
        Record { name: "selfish".into(), decompose: Some("selfish".into()), ..Record::default() },
    ]])
    .unwrap();
    assert_eq!(components_for(&font, &custom, "Agrave"), Ok(Some(names(&["A", "gravecomb.alt"]))));
    assert!(matches!(components_for(&font_with(&["selfish"]), &custom, "selfish"), Err(ComponentsError::Cycle(_))));
}

// ── composite_glyph ──

#[test]
fn composite_glyph_holds_the_parts_aligned_on_every_master() {
    let mut font = font_with(&[]);
    draw(&mut font, "A", 600.0, Some((0.0, 0.0, 600.0, 700.0)), &[("top", 300.0, 700.0)]);
    draw(&mut font, "gravecomb", 0.0, Some((-50.0, 550.0, 50.0, 650.0)), &[("_top", 50.0, 500.0)]);
    let g = composite_glyph(&font, &data(), "Agrave", &names(&["A", "gravecomb"])).unwrap();
    assert_eq!(g.name, "Agrave");
    assert_eq!(g.unicodes, vec![0xC0]);
    let layer = g.layer(MASTER).unwrap();
    let comps: Vec<&Component> = layer.shapes.iter().filter_map(|s| if let Shape::Component(c) = s { Some(c) } else { None }).collect();
    assert_eq!(comps.iter().map(|c| c.glyph.as_str()).collect::<Vec<_>>(), ["A", "gravecomb"]);
    assert_eq!(layer.width, 600.0);
    let offset = |c: &Component| {
        let a = c.affine().as_coeffs();
        (a[4], a[5])
    };
    assert_eq!(offset(comps[0]), (0.0, 0.0));
    // The mark's _top sits on the base's top (automatic alignment).
    assert_eq!(offset(comps[1]), (250.0, 200.0));
}

#[test]
fn composite_glyph_sets_bases_side_by_side() {
    let mut font = font_with(&[]);
    draw(&mut font, "t", 300.0, Some((0.0, 0.0, 300.0, 600.0)), &[]);
    draw(&mut font, "s", 400.0, Some((0.0, 0.0, 400.0, 500.0)), &[]);
    let g = composite_glyph(&font, &data(), "t_s", &names(&["t", "s"])).unwrap();
    let layer = g.layer(MASTER).unwrap();
    assert_eq!(layer.width, 700.0);
    let xs: Vec<f64> = layer.shapes.iter().filter_map(|s| if let Shape::Component(c) = s { Some(c.affine().as_coeffs()[4]) } else { None }).collect();
    assert_eq!(xs, vec![0.0, 300.0]);
}

#[test]
fn composite_glyph_replaces_shapes_and_keeps_anchors_of_an_existing_glyph() {
    let mut font = font_with(&["gravecomb", "Agrave"]);
    draw(&mut font, "Agrave", 500.0, Some((0.0, 0.0, 10.0, 10.0)), &[("top", 1.0, 2.0)]);
    let g = composite_glyph(&font, &data(), "Agrave", &names(&["A", "gravecomb"])).unwrap();
    let layer = g.layer(MASTER).unwrap();
    assert!(layer.shapes.iter().all(|s| matches!(s, Shape::Component(_))));
    assert_eq!(layer.shapes.len(), 2);
    assert_eq!(layer.anchors, vec![Anchor::new("top", 1.0, 2.0)]);
}

#[test]
fn composite_glyph_refuses_missing_parts_cycles_and_empty_lists() {
    let mut font = font_with(&["gravecomb"]);
    assert_eq!(composite_glyph(&font, &data(), "Agrave", &names(&["A", "nothere"])), Err(missing("Agrave", &["nothere"])));
    assert!(matches!(composite_glyph(&font, &data(), "Agrave", &[]), Err(ComponentsError::BadRecipe(_))));
    let too_many = vec!["A".to_string(); crate::MAX_RECIPE_PARTS + 1];
    assert!(matches!(composite_glyph(&font, &data(), "X", &too_many), Err(ComponentsError::LimitExceeded { .. })));
    assert_eq!(composite_glyph(&font, &data(), "A", &names(&["A"])), Err(ComponentsError::Cycle(names(&["A", "A"]))));
    // Rebuilding A from a glyph that already uses A would make a circle.
    let agrave = composite_glyph(&font, &data(), "Agrave", &names(&["A", "gravecomb"])).unwrap();
    font.glyphs.push(agrave);
    assert_eq!(composite_glyph(&font, &data(), "A", &names(&["Agrave"])), Err(ComponentsError::Cycle(names(&["A", "Agrave", "A"]))));
}

// ── default anchors ──

fn anchor_map(font: &Font, glyph: &str) -> Vec<(String, f64, f64)> {
    let round = |v: f64| (v * 100.0).round() / 100.0;
    default_anchors(font, &data(), glyph, MASTER).unwrap().into_iter().map(|a| (a.name, round(a.x), round(a.y))).collect()
}

fn expect(v: &[(&str, f64, f64)]) -> Vec<(String, f64, f64)> {
    v.iter().map(|(n, x, y)| (n.to_string(), *x, *y)).collect()
}

#[test]
fn letters_get_top_and_bottom_on_the_metric_lines() {
    let mut font = font_with(&[]);
    draw(&mut font, "a", 500.0, Some((50.0, -10.0, 450.0, 510.0)), &[]);
    draw(&mut font, "d", 500.0, Some((50.0, -10.0, 450.0, 800.0)), &[]);
    draw(&mut font, "A", 600.0, Some((0.0, 0.0, 600.0, 700.0)), &[]);
    draw(&mut font, "n", 500.0, Some((50.0, 0.0, 450.0, 500.0)), &[]);
    assert_eq!(anchor_map(&font, "a"), expect(&[("top", 250.0, 500.0), ("bottom", 250.0, 0.0), ("ogonek", 450.0, 0.0)]));
    assert_eq!(
        anchor_map(&font, "d"),
        expect(&[("top", 250.0, 800.0), ("bottom", 250.0, 0.0), ("center", 250.0, 250.0), ("topright", 450.0, 800.0)])
    );
    assert_eq!(anchor_map(&font, "A"), expect(&[("top", 300.0, 700.0), ("bottom", 300.0, 0.0), ("ogonek", 600.0, 0.0)]));
    assert_eq!(anchor_map(&font, "n"), expect(&[("top", 250.0, 500.0), ("bottom", 250.0, 0.0)]));
}

#[test]
fn marks_get_their_attaching_and_stacking_anchors() {
    let mut font = font_with(&[]);
    draw(&mut font, "acutecomb", 0.0, Some((-50.0, 550.0, 50.0, 650.0)), &[]);
    draw(&mut font, "acutecomb.case", 0.0, Some((-50.0, 750.0, 50.0, 850.0)), &[]);
    draw(&mut font, "cedillacomb", 0.0, Some((-50.0, -150.0, 50.0, 0.0)), &[]);
    draw(&mut font, "ogonekcomb", 0.0, Some((-80.0, -160.0, 20.0, 0.0)), &[]);
    draw(&mut font, "horncomb", 0.0, Some((-10.0, 450.0, 90.0, 560.0)), &[]);
    draw(&mut font, "caroncomb.alt", 0.0, Some((-5.0, 600.0, 60.0, 790.0)), &[]);
    assert_eq!(anchor_map(&font, "acutecomb"), expect(&[("_top", 0.0, 500.0), ("top", 0.0, 650.0)]));
    assert_eq!(anchor_map(&font, "acutecomb.case"), expect(&[("_top", 0.0, 700.0), ("top", 0.0, 850.0)]));
    assert_eq!(anchor_map(&font, "cedillacomb"), expect(&[("_bottom", 0.0, 0.0), ("bottom", 0.0, -150.0)]));
    assert_eq!(anchor_map(&font, "ogonekcomb"), expect(&[("_ogonek", 20.0, 0.0)]));
    assert_eq!(anchor_map(&font, "horncomb"), expect(&[("_topright", -10.0, 500.0)]));
    assert_eq!(anchor_map(&font, "caroncomb.alt"), expect(&[("_topright", -5.0, 790.0)]));
}

#[test]
fn set_anchors_adds_only_what_the_layer_lacks() {
    let mut font = font_with(&[]);
    draw(&mut font, "a", 500.0, Some((50.0, 0.0, 450.0, 500.0)), &[("top", 1.0, 2.0)]);
    let added: Vec<String> = default_anchors(&font, &data(), "a", MASTER).unwrap().into_iter().map(|a| a.name).collect();
    assert_eq!(added, names(&["bottom", "ogonek"]));
    assert_eq!(default_anchors(&font, &data(), "nothere", MASTER), Err(ComponentsError::MissingGlyph("nothere".into())));
    assert!(matches!(default_anchors(&font, &data(), "a", "no-layer"), Err(ComponentsError::MissingLayer { .. })));
}

#[test]
fn ligatures_get_carets_dividing_the_width() {
    let mut font = font_with(&[]);
    draw(&mut font, "f_f_l", 900.0, Some((0.0, 0.0, 900.0, 800.0)), &[]);
    assert_eq!(anchor_map(&font, "f_f_l"), expect(&[("caret_1", 300.0, 0.0), ("caret_2", 600.0, 0.0)]));
}

#[test]
fn segments_and_brushes_get_start_and_end() {
    let mut font = font_with(&[]);
    draw(&mut font, "_segment.stem", 300.0, None, &[]);
    {
        let mut g = font.glyph_mut("_segment.stem").unwrap();
        let line = |x, y| Node::new(x, y, NodeKind::Line);
        g.layer_mut(MASTER).unwrap().shapes = vec![Shape::Path(Path::new(false, vec![line(10.0, 0.0), line(150.0, 40.0), line(290.0, 0.0)]))];
    }
    draw(&mut font, "_brush.arrow", 400.0, Some((0.0, -50.0, 400.0, 50.0)), &[]);
    assert_eq!(anchor_map(&font, "_segment.stem"), expect(&[("start", 10.0, 0.0), ("end", 290.0, 0.0)]));
    assert_eq!(anchor_map(&font, "_brush.arrow"), expect(&[("start", 0.0, 0.0), ("end", 400.0, 0.0)]));
}

#[test]
fn centred_anchors_follow_the_italic_angle() {
    let mut font = font_with(&[]);
    font.set_metric_pos(MASTER, metric::ITALIC_ANGLE, 10.0).unwrap();
    draw(&mut font, "n", 500.0, Some((50.0, 0.0, 450.0, 500.0)), &[]);
    let shift = 250.0 * 10f64.to_radians().tan();
    let round = |v: f64| (v * 100.0).round() / 100.0;
    assert_eq!(anchor_map(&font, "n"), expect(&[("top", round(250.0 + shift), 500.0), ("bottom", round(250.0 - shift), 0.0)]));
}

#[test]
fn custom_anchor_lists_and_glyphs_without_anchors() {
    let font = font_with(&["period", "uni4E00"]);
    assert!(default_anchor_names(&font, &data(), "period").is_empty());
    assert!(default_anchor_names(&font, &data(), "uni4E00").is_empty());
    let custom = GlyphData::with_custom(vec![vec![Record { name: "a".into(), anchors: Some("top, , extra".into()), ..Record::default() }]]).unwrap();
    assert_eq!(default_anchor_names(&font, &custom, "a"), names(&["top", "extra"]));
    // A suffixed glyph takes its base glyph's custom list.
    assert_eq!(default_anchor_names(&font, &custom, "a.ss01"), names(&["top", "extra"]));
}
