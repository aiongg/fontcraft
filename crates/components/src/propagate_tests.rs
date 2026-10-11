//! Behaviour tests for anchor propagation. Every expected list is the output of the reference
//! compiler (glyphsLib 6.15.0 `propagate_all_anchors`) on the same font, built the same way,
//! order included.

use std::sync::Arc;

use fontcraft_model::plist::Value;
use fontcraft_model::{Anchor, Component, Font, Glyph, GlyphTable, Layer, Param, Shape, Transform};

use crate::propagate::{AnchorCache, AnchorSource, effective_anchors, effective_anchors_for_master, master_anchors, propagate_anchors_enabled};
use crate::{ComponentsError, MAX_COMPONENT_DEPTH};

const M: &str = "m01";
const ID: [f64; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

fn glyph(name: &str, category: &str, sub: &str, anchors: &[(&str, f64, f64)], comps: &[(&str, [f64; 6], Option<&str>)]) -> Glyph {
    let mut g = Glyph::new(name);
    g.info.category = Some(category.to_string());
    g.info.sub_category = Some(sub.to_string());
    let mut layer = Layer::new(M, 600.0);
    layer.anchors = anchors.iter().map(|(n, x, y)| Anchor::new(*n, *x, *y)).collect();
    for (base, m, attr) in comps {
        let mut c = Component::new(*base, 0.0, 0.0);
        c.transform = Transform::Matrix(*m);
        if let Some(a) = attr {
            c.info.anchor = Some(a.to_string());
        }
        layer.shapes.push(Shape::Component(c));
    }
    g.layers.push(layer);
    g
}

fn letter(name: &str, anchors: &[(&str, f64, f64)], comps: &[(&str, [f64; 6], Option<&str>)]) -> Glyph {
    glyph(name, "Letter", "Lowercase", anchors, comps)
}

fn mark(name: &str, anchors: &[(&str, f64, f64)], comps: &[(&str, [f64; 6], Option<&str>)]) -> Glyph {
    glyph(name, "Mark", "Nonspacing", anchors, comps)
}

fn liga(name: &str, comps: &[(&str, [f64; 6], Option<&str>)]) -> Glyph {
    glyph(name, "Letter", "Ligature", &[], comps)
}

fn at(dx: f64, dy: f64) -> [f64; 6] {
    [1.0, 0.0, 0.0, 1.0, dx, dy]
}

fn font(glyphs: Vec<Glyph>) -> Font {
    let mut f = Font::new("Propagation");
    f.glyphs = GlyphTable::from_glyphs(glyphs.into_iter().map(Arc::new).collect());
    f
}

/// The marks and letters most tests share.
fn base_glyphs() -> Vec<Glyph> {
    vec![
        letter("a", &[("top", 250.0, 500.0), ("bottom", 250.0, 0.0), ("ogonek", 400.0, 0.0)], &[]),
        mark("acutecomb", &[("_top", 100.0, 500.0), ("top", 100.0, 700.0)], &[]),
        mark("gravecomb", &[("_top", 100.0, 500.0), ("top", 110.0, 720.0)], &[]),
        letter("f", &[("top", 100.0, 700.0), ("bottom", 100.0, 0.0)], &[]),
        letter("i", &[("top", 50.0, 700.0), ("bottom", 50.0, 0.0)], &[]),
    ]
}

fn with(extra: Vec<Glyph>) -> Font {
    let mut g = base_glyphs();
    g.extend(extra);
    font(g)
}

#[track_caller]
fn assert_anchors(font: &Font, glyph: &str, want: &[(&str, f64, f64)]) {
    let got = effective_anchors(font, glyph, M).unwrap();
    let got: Vec<(String, f64, f64)> = got.into_iter().map(|a| (a.name, a.x, a.y)).collect();
    let same = got.len() == want.len() && got.iter().zip(want).all(|(g, w)| g.0 == w.0 && (g.1 - w.1).abs() < 1e-6 && (g.2 - w.2).abs() < 1e-6);
    assert!(same, "{glyph}: got {got:?}, want {want:?}");
}

#[test]
fn marks_stack_and_the_last_mark_gives_the_top_anchor() {
    let f = with(vec![
        letter("aacute", &[], &[("a", ID, None), ("acutecomb", at(150.0, 0.0), None)]),
        letter("aacutegrave", &[], &[("a", ID, None), ("acutecomb", at(150.0, 0.0), None), ("gravecomb", at(150.0, 200.0), None)]),
    ]);
    assert_anchors(&f, "aacute", &[("top", 250.0, 700.0), ("bottom", 250.0, 0.0), ("ogonek", 400.0, 0.0)]);
    assert_anchors(&f, "aacutegrave", &[("top", 260.0, 920.0), ("bottom", 250.0, 0.0), ("ogonek", 400.0, 0.0)]);
}

#[test]
fn an_own_anchor_replaces_the_propagated_one_in_place() {
    let f = with(vec![letter("aacute.own", &[("top", 1.0, 2.0)], &[("a", ID, None), ("acutecomb", at(150.0, 0.0), None)])]);
    assert_anchors(&f, "aacute.own", &[("top", 1.0, 2.0), ("bottom", 250.0, 0.0), ("ogonek", 400.0, 0.0)]);
    let anchors = effective_anchors(&f, "aacute.own", M).unwrap();
    assert_eq!(anchors.first().map(|a| &a.source), Some(&AnchorSource::Own));
    assert_eq!(anchors.get(1).map(|a| &a.source), Some(&AnchorSource::Component { component: 0, base: "a".into() }));
}

#[test]
fn a_mark_made_of_marks_keeps_the_first_attaching_anchor_unless_it_has_anchors_of_its_own() {
    let f = with(vec![
        mark("acutegravecomb", &[], &[("acutecomb", ID, None), ("gravecomb", at(0.0, 200.0), None)]),
        mark("acutegravecomb.own", &[("_top", 5.0, 6.0)], &[("acutecomb", ID, None), ("gravecomb", at(0.0, 200.0), None)]),
    ]);
    assert_anchors(&f, "acutegravecomb", &[("_top", 100.0, 500.0), ("top", 110.0, 920.0)]);
    assert_anchors(&f, "acutegravecomb.own", &[("_top", 5.0, 6.0)]);
}

#[test]
fn a_vertically_flipped_mark_attaches_as_a_bottom_mark() {
    let f = with(vec![
        letter("g", &[("top", 250.0, 500.0), ("bottom", 250.0, -200.0)], &[]),
        mark("commaturnedabovecomb", &[("_top", 100.0, 500.0), ("top", 100.0, 650.0)], &[]),
        letter("gcommaaccent", &[], &[("g", ID, None), ("commaturnedabovecomb", [1.0, 0.0, 0.0, -1.0, 150.0, 300.0], None)]),
    ]);
    assert_anchors(&f, "gcommaaccent", &[("top", 250.0, 500.0), ("bottom", 250.0, -350.0)]);
}

#[test]
fn flips_and_rotations_rename_anchors() {
    let arrow =
        [("top", 10.0, 20.0), ("topleft", 13.0, 23.0), ("left", 16.0, 26.0), ("entry", 18.0, 28.0), ("#exit", 21.0, 31.0), ("center", 26.0, 36.0)];
    let f = with(vec![
        glyph("arrow", "Symbol", "Arrow", &arrow, &[]),
        glyph("arrow.rot180", "Symbol", "Arrow", &[], &[("arrow", [-1.0, 0.0, 0.0, -1.0, 0.0, 0.0], None)]),
        glyph("arrow.mirror", "Symbol", "Arrow", &[], &[("arrow", [-1.0, 0.0, 0.0, 1.0, 100.0, 0.0], None)]),
        glyph("arrow.shear", "Symbol", "Arrow", &[], &[("arrow", [-1.0, 0.5, 0.0, 1.0, 0.0, 0.0], None)]),
        glyph("arrow.rot90", "Symbol", "Arrow", &[], &[("arrow", [0.0, 1.0, -1.0, 0.0, 0.0, 0.0], None)]),
    ]);
    assert_anchors(
        &f,
        "arrow.rot180",
        &[
            ("bottom", -10.0, -20.0),
            ("bottomright", -13.0, -23.0),
            ("right", -16.0, -26.0),
            ("exit", -18.0, -28.0),
            ("#entry", -21.0, -31.0),
            ("center", -26.0, -36.0),
        ],
    );
    assert_anchors(
        &f,
        "arrow.mirror",
        &[("top", 90.0, 20.0), ("topright", 87.0, 23.0), ("right", 84.0, 26.0), ("exit", 82.0, 28.0), ("#entry", 79.0, 31.0), ("center", 74.0, 36.0)],
    );
    // A mirror with a vertical shear counts as a rotation plus a vertical flip.
    assert_anchors(
        &f,
        "arrow.shear",
        &[
            ("bottom", -10.0, 25.0),
            ("bottomleft", -13.0, 29.5),
            ("left", -16.0, 34.0),
            ("entry", -18.0, 37.0),
            ("#exit", -21.0, 41.5),
            ("center", -26.0, 49.0),
        ],
    );
    assert_anchors(
        &f,
        "arrow.rot90",
        &[
            ("top", -20.0, 10.0),
            ("topleft", -23.0, 13.0),
            ("left", -26.0, 16.0),
            ("entry", -28.0, 18.0),
            ("#exit", -31.0, 21.0),
            ("center", -36.0, 26.0),
        ],
    );
}

#[test]
fn a_decomposed_180_degree_rotation_renames_like_the_matrix() {
    let mut f = with(vec![glyph("arrow", "Symbol", "Arrow", &[("top", 10.0, 20.0), ("left", 16.0, 26.0)], &[])]);
    let mut g = glyph("arrow.turned", "Symbol", "Arrow", &[], &[("arrow", ID, None)]);
    if let Some(Shape::Component(c)) = g.layers.first_mut().and_then(|l| l.shapes.first_mut()) {
        c.transform = Transform::Decomposed { pos: (0.0, 0.0), scale: (1.0, 1.0), angle: 180.0, slant: (0.0, 0.0) };
    }
    f.glyphs.push(g);
    assert_anchors(&f, "arrow.turned", &[("bottom", -10.0, -20.0), ("right", -16.0, -26.0)]);
}

#[test]
fn ligatures_number_anchors_by_base_and_an_attribute_picks_a_numbered_anchor() {
    let f = with(vec![
        liga("f_f", &[("f", ID, None), ("f", at(200.0, 0.0), None)]),
        liga("f_f_i", &[("f_f", ID, None), ("i", at(400.0, 0.0), None)]),
        liga("f_f.acute2", &[("f", ID, None), ("f", at(200.0, 0.0), None), ("acutecomb", at(210.0, 10.0), Some("top_2"))]),
    ]);
    assert_anchors(&f, "f_f", &[("top_1", 100.0, 700.0), ("bottom_1", 100.0, 0.0), ("top_2", 300.0, 700.0), ("bottom_2", 300.0, 0.0)]);
    assert_anchors(
        &f,
        "f_f_i",
        &[
            ("top_1", 100.0, 700.0),
            ("bottom_1", 100.0, 0.0),
            ("top_2", 300.0, 700.0),
            ("bottom_2", 300.0, 0.0),
            ("top_3", 450.0, 700.0),
            ("bottom_3", 450.0, 0.0),
        ],
    );
    assert_anchors(&f, "f_f.acute2", &[("top_1", 100.0, 700.0), ("bottom_1", 100.0, 0.0), ("top_2", 310.0, 710.0), ("bottom_2", 300.0, 0.0)]);
}

#[test]
fn the_anchor_attribute_renames_the_marks_top_anchor() {
    let f = with(vec![
        letter("a.viet", &[("top", 250.0, 500.0), ("top_viet", 260.0, 520.0)], &[]),
        letter("aacute.viet", &[], &[("a.viet", ID, None), ("acutecomb", at(150.0, 0.0), Some("top_viet"))]),
    ]);
    assert_anchors(&f, "aacute.viet", &[("top", 250.0, 500.0), ("top_viet", 250.0, 700.0)]);
}

#[test]
fn an_origin_anchor_shifts_the_bases_anchors_before_the_transform() {
    let f = with(vec![
        mark("ring.origin", &[("_top", 100.0, 500.0), ("top", 100.0, 800.0), ("*origin", 50.0, 20.0)], &[]),
        letter("aring.origin", &[], &[("a", ID, None), ("ring.origin", [2.0, 0.0, 0.0, 2.0, 100.0, 0.0], None)]),
    ]);
    // The base itself keeps its anchors as stored.
    assert_anchors(&f, "ring.origin", &[("_top", 100.0, 500.0), ("top", 100.0, 800.0), ("*origin", 50.0, 20.0)]);
    assert_anchors(&f, "aring.origin", &[("top", 200.0, 1560.0), ("bottom", 250.0, 0.0), ("ogonek", 400.0, 0.0)]);
}

#[test]
fn entry_comes_from_the_first_component_and_exit_from_the_last_base() {
    let stroke = [("entry", 1.0, 1.0), ("exit", 2.0, 2.0), ("#entry", 3.0, 3.0), ("#exit", 4.0, 4.0), ("top", 5.0, 5.0)];
    let f = with(vec![
        letter("stroke", &stroke, &[]),
        letter("stroke.first", &[], &[("stroke", ID, None), ("i", at(100.0, 0.0), None)]),
        letter("stroke.last", &[], &[("i", ID, None), ("stroke", at(100.0, 0.0), None)]),
        letter("stroke.mark", &[], &[("stroke", ID, None), ("acutecomb", at(100.0, 0.0), None)]),
    ]);
    assert_anchors(
        &f,
        "stroke.first",
        &[("entry", 1.0, 1.0), ("#entry", 3.0, 3.0), ("#exit", 4.0, 4.0), ("top", 150.0, 700.0), ("bottom", 150.0, 0.0)],
    );
    assert_anchors(
        &f,
        "stroke.last",
        &[("top", 105.0, 5.0), ("bottom", 50.0, 0.0), ("exit", 102.0, 2.0), ("#entry", 103.0, 3.0), ("#exit", 104.0, 4.0)],
    );
    // A mark after the stroke does not end it.
    assert_anchors(&f, "stroke.mark", &[("entry", 1.0, 1.0), ("exit", 2.0, 2.0), ("#entry", 3.0, 3.0), ("#exit", 4.0, 4.0), ("top", 200.0, 700.0)]);
}

#[test]
fn a_letter_with_a_top_attaching_anchor_passes_on_no_bottom() {
    let f = with(vec![
        letter("hook", &[("_top", 10.0, 20.0), ("top", 12.0, 22.0), ("bottom", 11.0, 21.0)], &[]),
        letter("hook.used", &[], &[("a", ID, None), ("hook", at(0.0, 100.0), None)]),
    ]);
    assert_anchors(&f, "hook.used", &[("top", 12.0, 122.0), ("bottom", 250.0, 0.0), ("ogonek", 400.0, 0.0)]);
}

#[test]
fn the_propagate_anchors_parameter_turns_propagation_off() {
    let mut f = with(vec![letter("aacute", &[], &[("a", ID, None), ("acutecomb", at(150.0, 0.0), None)])]);
    assert!(propagate_anchors_enabled(&f));
    Arc::make_mut(&mut f.info).params.0.push(Param::new("Propagate Anchors", Value::Bool(false)));
    assert!(!propagate_anchors_enabled(&f));
    assert_anchors(&f, "aacute", &[]);
}

#[test]
fn a_missing_base_passes_nothing_and_a_cycle_is_an_error() {
    let f = with(vec![
        letter("ghost", &[], &[("nothere", ID, None), ("acutecomb", at(150.0, 0.0), None)]),
        letter("loop1", &[], &[("loop2", ID, None)]),
        letter("loop2", &[], &[("loop1", ID, None)]),
        letter("abovelLoop", &[], &[("loop1", ID, None)]),
    ]);
    assert_anchors(&f, "ghost", &[("top", 250.0, 700.0)]);
    assert!(matches!(effective_anchors(&f, "loop1", M), Err(ComponentsError::Cycle(_))));
    assert!(matches!(effective_anchors(&f, "abovelLoop", M), Err(ComponentsError::Cycle(_))));
    assert!(matches!(effective_anchors(&f, "a", "nolayer"), Err(ComponentsError::MissingLayer { .. })));
    // A whole master gives the glyphs of the cycle and those using it their own anchors.
    let all = effective_anchors_for_master(&f, M).unwrap();
    let failed: Vec<&str> = all.failed.iter().map(|(g, _)| g.as_str()).collect();
    assert_eq!(failed, ["loop1", "loop2", "abovelLoop"]);
    assert!(all.failed.iter().all(|(_, e)| matches!(e, ComponentsError::Cycle(_))));
    assert_eq!(all.anchors.get("abovelLoop"), Some(&Vec::new()));
    assert_eq!(all.anchors.get("ghost"), Some(&effective_anchors(&f, "ghost", M).unwrap()));
}

/// One glyph that cannot be worked out keeps its own anchors and passes them on; the rest of
/// the master propagates.
#[test]
fn a_failing_glyph_does_not_stop_the_rest_of_the_master() {
    let many: Vec<(String, f64, f64)> = (0..=crate::MAX_LAYER_ANCHORS).map(|i| (format!("x{i}"), 0.0, 0.0)).collect();
    let many: Vec<(&str, f64, f64)> = many.iter().map(|(n, x, y)| (n.as_str(), *x, *y)).collect();
    let f = with(vec![
        letter("nan", &[("top", 300.0, 600.0)], &[("a", [f64::NAN, 0.0, 0.0, 1.0, 0.0, 0.0], None)]),
        letter("nanuser", &[], &[("nan", at(10.0, 0.0), None)]),
        letter("big", &many, &[("a", ID, None)]),
        letter("aacute", &[], &[("a", ID, None), ("acutecomb", at(150.0, 0.0), None)]),
    ]);
    let all = effective_anchors_for_master(&f, M).unwrap();
    let failed: Vec<&str> = all.failed.iter().map(|(g, _)| g.as_str()).collect();
    assert_eq!(failed, ["nan", "big"]);
    let names = |g: &str| all.anchors[g].iter().map(|a| (a.name.clone(), a.x, a.y)).collect::<Vec<_>>();
    assert_eq!(names("nan"), [("top".to_string(), 300.0, 600.0)]);
    assert_eq!(names("nanuser"), [("top".to_string(), 310.0, 600.0)]);
    assert_eq!(all.anchors["big"].len(), crate::MAX_LAYER_ANCHORS + 1);
    assert_eq!(all.anchors.get("aacute"), Some(&effective_anchors(&f, "aacute", M).unwrap()));
}

#[test]
fn hostile_nesting_and_transforms_fail_with_a_message() {
    let mut glyphs = base_glyphs();
    for i in 0..1000 {
        let next = if i == 0 { "a".to_string() } else { format!("deep{}", i - 1) };
        glyphs.push(letter(&format!("deep{i}"), &[], &[(&next, ID, None)]));
    }
    glyphs.push(letter("nan", &[], &[("a", [f64::NAN, 0.0, 0.0, 1.0, 0.0, 0.0], None)]));
    let f = font(glyphs);
    assert!(matches!(effective_anchors(&f, "deep999", M), Err(ComponentsError::TooDeep { .. })));
    assert!(effective_anchors(&f, &format!("deep{}", MAX_COMPONENT_DEPTH - 1), M).is_ok());
    assert!(matches!(effective_anchors(&f, "nan", M), Err(ComponentsError::BadPlacement { .. })));
}

#[test]
fn the_whole_master_matches_single_glyph_results() {
    let f = with(vec![
        letter("aacute", &[], &[("a", ID, None), ("acutecomb", at(150.0, 0.0), None)]),
        liga("f_f", &[("f", ID, None), ("f", at(200.0, 0.0), None)]),
    ]);
    let all = effective_anchors_for_master(&f, M).unwrap();
    assert!(all.failed.is_empty());
    for g in f.glyphs.iter() {
        assert_eq!(all.anchors.get(&g.name), Some(&effective_anchors(&f, &g.name, M).unwrap()), "{}", g.name);
    }
}

#[test]
fn the_cache_reuses_results_until_a_reached_glyph_changes() {
    let mut f = with(vec![letter("aacute", &[], &[("a", ID, None), ("acutecomb", at(150.0, 0.0), None)])]);
    let mut cache = AnchorCache::new();
    let first = cache.get(&f, "aacute", M).unwrap();
    let again = cache.get(&f, "aacute", M).unwrap();
    assert!(Arc::ptr_eq(&first, &again));
    // An unrelated glyph changes: the entry stays.
    if let Some(mut g) = f.glyphs.get_mut("i") {
        g.layers.clear();
    }
    cache.retain_current(&f);
    assert_eq!(cache.len(), 1);
    assert!(Arc::ptr_eq(&first, &cache.get(&f, "aacute", M).unwrap()));
    // The mark moves its top anchor: the composite follows.
    if let Some(mut g) = f.glyphs.get_mut("acutecomb")
        && let Some(l) = g.layers.first_mut()
        && let Some(a) = l.anchors.get_mut(1)
    {
        a.y = 800.0;
    }
    let after = cache.get(&f, "aacute", M).unwrap();
    assert_eq!(after.first().map(|a| (a.name.as_str(), a.y)), Some(("top", 800.0)));
}

/// The smart-components example font: `_part.shoulder` has two properties and three pole layers
/// whose `connect` anchor moves with `shoulderWidth`. A component of it passes on its anchors
/// interpolated at its values, not the master layer's. Expected values from glyphsLib on the
/// same font, and on a copy with `m`'s second shoulder removed (`m_one`).
#[test]
fn smart_components_pass_on_anchors_interpolated_at_their_values() {
    use fontcraft_model::{ComponentInfo, ComponentInfoBox, LayerAttrs, LayerInfo, LayerInfoBox, PartProperty, Path, SmartPole, SmartValue};
    let property = |name: &str, bottom_value: f64, top_value: f64| PartProperty {
        name: name.into(),
        bottom_value,
        top_value,
        bottom_name: None,
        top_name: None,
        extra: Default::default(),
    };
    let mut shoulder = Glyph::new("_part.shoulder");
    shoulder.info.parts = vec![property("shoulderWidth", 0.0, 100.0), property("crotchDepth", -100.0, 0.0)];
    let poles = [
        ("m01", None, 320.0, vec![("shoulderWidth", true), ("crotchDepth", true)]),
        ("narrow", Some("NarrowShoulder"), 290.0, vec![("shoulderWidth", false)]),
        ("low", Some("LowCrotch"), 320.0, vec![("crotchDepth", false)]),
    ];
    for (id, name, connect_x, ends) in poles {
        let mut l = Layer::new(M, 600.0);
        l.id = id.into();
        l.anchors = vec![Anchor::new("_connect", 136.0, 0.0), Anchor::new("connect", connect_x, 1.0)];
        l.shapes.push(Shape::Path(Path::rect(136.0, 0.0, connect_x + 40.0, 365.0)));
        l.info = LayerInfoBox::new(LayerInfo {
            name: name.map(str::to_string),
            attrs: LayerAttrs { smart_poles: ends.into_iter().map(|(p, top)| SmartPole { property: p.into(), top }).collect(), ..Default::default() },
            ..Default::default()
        });
        shoulder.layers.push(l);
    }
    let piece = |dx: f64, dy: f64, width: f64, crotch: f64| {
        let mut c = Component::new("_part.shoulder", dx, dy);
        c.info = ComponentInfoBox::new(ComponentInfo {
            smart_values: vec![
                SmartValue { property: "crotchDepth".into(), value: crotch },
                SmartValue { property: "shoulderWidth".into(), value: width },
            ],
            ..Default::default()
        });
        Shape::Component(c)
    };
    let user = |name: &str, shapes: Vec<Shape>| {
        let mut g = Glyph::new(name);
        let mut l = Layer::new(M, 600.0);
        l.shapes = shapes;
        g.layers.push(l);
        g
    };
    let stem = Shape::Component(Component::new("_part.stem", 0.0, 0.0));
    let f = font(vec![
        user("m", vec![stem.clone(), piece(17.0, 0.0, 28.78011, -44.28304), piece(180.0, 1.0, 0.0, -81.24796)]),
        user("m_one", vec![stem, piece(17.0, 0.0, 28.78011, -44.28304)]),
        user("n", vec![Shape::Path(Path::rect(26.0, 1.0, 193.0, 365.0)), piece(0.0, 0.0, 0.0, -100.0)]),
        shoulder,
        glyph("_part.stem", "Letter", "Lowercase", &[("connect", 153.0, 0.0)], &[]),
    ]);
    assert_anchors(&f, "m", &[("connect", 470.0, 2.0)]);
    assert_anchors(&f, "m_one", &[("connect", 315.634033, 1.0)]);
    assert_anchors(&f, "n", &[("_connect", 136.0, 0.0), ("connect", 290.0, 1.0)]);
    assert_anchors(&f, "_part.shoulder", &[("_connect", 136.0, 0.0), ("connect", 320.0, 1.0)]);
}

/// Past the master's work limit the glyphs not yet worked out keep their own anchors and are
/// listed as failed.
#[test]
fn a_master_past_its_work_limit_keeps_the_rest_on_their_own_anchors() {
    let f = with(vec![
        letter("aacute", &[], &[("a", ID, None), ("acutecomb", at(150.0, 0.0), None)]),
        letter("agrave", &[("ogonek", 9.0, 9.0)], &[("a", ID, None), ("gravecomb", at(150.0, 0.0), None)]),
    ]);
    let limit_hit = |limit| ComponentsError::LimitExceeded { what: "anchor propagation steps on one master", limit };
    // aacute reads 3 anchors from a and 2 from acutecomb; agrave needs more.
    let all = master_anchors(&f, None, M, 5).unwrap();
    assert_eq!(all.failed, [("agrave".to_string(), limit_hit(5))]);
    assert_eq!(all.anchors.get("aacute"), Some(&effective_anchors(&f, "aacute", M).unwrap()));
    let names = |all: &crate::propagate::MasterAnchors, g: &str| all.anchors[g].iter().map(|a| (a.name.clone(), a.x, a.y)).collect::<Vec<_>>();
    assert_eq!(names(&all, "agrave"), [("ogonek".to_string(), 9.0, 9.0)]);
    let none = master_anchors(&f, None, M, 0).unwrap();
    let failed: Vec<&str> = none.failed.iter().map(|(g, _)| g.as_str()).collect();
    assert_eq!(failed, ["aacute", "agrave"], "glyphs without components need no work");
    assert_eq!(names(&none, "a"), names(&all, "a"));
}

/// A chain of components longer than the nesting limit fails the same glyphs in any glyph order.
#[test]
fn nesting_past_the_limit_fails_the_same_glyphs_in_any_order() {
    let chain: Vec<Glyph> = (0..MAX_COMPONENT_DEPTH + 4)
        .map(|i| {
            let base = if i == 0 { "a".to_string() } else { format!("chain{}", i - 1) };
            letter(&format!("chain{i}"), &[], &[(&base, at(1.0, 0.0), None)])
        })
        .collect();
    let run = |chain: Vec<Glyph>| {
        let mut all = effective_anchors_for_master(&with(chain), M).unwrap();
        all.failed.sort_by(|a, b| a.0.cmp(&b.0));
        all
    };
    let forward = run(chain.clone());
    let backward = run(chain.into_iter().rev().collect());
    assert_eq!(forward, backward);
    // chain{i} nests i + 1 deep.
    let mut want: Vec<String> = (MAX_COMPONENT_DEPTH..MAX_COMPONENT_DEPTH + 4).map(|i| format!("chain{i}")).collect();
    want.sort();
    let failed: Vec<String> = forward.failed.iter().map(|(g, _)| g.clone()).collect();
    assert_eq!(failed, want);
    assert!(forward.failed.iter().all(|(g, e)| *e == ComponentsError::TooDeep { glyph: g.clone(), limit: MAX_COMPONENT_DEPTH }));
}
