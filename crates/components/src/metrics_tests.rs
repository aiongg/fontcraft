//! Derived metrics, offset keys and the re-alignment hook, on fonts built in code.

use std::sync::Arc;

use fontcraft_model::{Font, Layer, Master, Path, Shape};

use crate::ComponentsError;
use crate::align_tests::{M, composite, drawn, font, letter_a, part, part_at, pos, top_mark};
use crate::metrics::{DerivedMetrics, changed_glyphs, derived_metrics, parse_offset, realign, realign_glyphs};

fn layer(f: &Font, glyph: &str) -> Layer {
    f.glyph(glyph).unwrap().layers[0].clone()
}

/// `A`, two top marks, `Aacute`, `Agrave` and `Aacute.ss01` (a single `Aacute` component), all
/// aligned.
fn accents() -> Font {
    let f = font(vec![
        letter_a(),
        top_mark("acutecomb"),
        top_mark("gravecomb"),
        composite("Aacute", vec![part("A"), part("acutecomb")]),
        composite("Agrave", vec![part("A"), part("gravecomb")]),
        composite("Aacute.ss01", vec![part("Aacute")]),
        drawn("B", 610.0, (60.0, 0.0, 560.0, 700.0), &[]),
    ]);
    let all: Vec<String> = f.glyphs.iter().map(|g| g.name.clone()).collect();
    realign_glyphs(f, &all).unwrap()
}

#[test]
fn offsets_parse_only_signed_numbers_after_an_equals_sign() {
    assert_eq!(parse_offset("=+18"), Some(18.0));
    assert_eq!(parse_offset("=-12"), Some(-12.0));
    assert_eq!(parse_offset(" = +2.5 "), Some(2.5));
    for other in ["=18", "+18", "=n+5", "=+", "=+inf", "=+NaN", "=+1e3", "=|", "", "=+-3"] {
        assert_eq!(parse_offset(other), None, "{other:?}");
    }
}

#[test]
fn offset_keys_shift_and_widen_an_aligned_composite() {
    let mut f = accents();
    {
        let mut g = f.glyphs.get_mut("Aacute").unwrap();
        g.info.metrics_keys.left = Some("=+20".into());
        g.layers[0].info.metrics_keys.right = Some("=-10".into());
    }
    let f = realign_glyphs(f, &["Aacute".to_string()]).unwrap();
    let l = layer(&f, "Aacute");
    assert_eq!(pos(&l, 0), (20.0, 0.0), "the left offset moves everything right");
    assert_eq!(pos(&l, 1), (320.0, 200.0));
    assert_eq!(l.width, 610.0, "600 + 20 - 10");
    let m = derived_metrics(&f, "Aacute", M).unwrap().unwrap();
    assert_eq!(m, DerivedMetrics { width: 610.0, lsb: Some(40.0), rsb: Some(10.0) });
}

#[test]
fn marks_attached_by_anchors_leave_the_sidebearings_alone() {
    let dotless = drawn("idotless", 200.0, (50.0, 0.0, 150.0, 500.0), &[("top", 100.0, 500.0)]);
    let macron = drawn("macroncomb", 0.0, (-150.0, 560.0, 150.0, 600.0), &[("_top", 0.0, 500.0)]);
    let f = font(vec![dotless, macron, composite("imacron", vec![part("idotless"), part("macroncomb")])]);
    let m = derived_metrics(&f, "imacron", M).unwrap().unwrap();
    assert_eq!(m, DerivedMetrics { width: 200.0, lsb: Some(50.0), rsb: Some(50.0) }, "the macron sticks out; the metrics ignore it");
    assert_eq!(derived_metrics(&f, "idotless", M).unwrap(), None, "a drawn glyph has no derived metrics");
}

#[test]
fn a_base_edit_moves_every_composite_and_its_metrics() {
    let old = accents();
    let mut new = old.clone();
    {
        let mut a = new.glyphs.get_mut("A").unwrap();
        let l = &mut a.layers[0];
        l.width = 650.0;
        l.anchors.iter_mut().filter(|an| an.name == "top").for_each(|an| an.x = 325.0);
    }
    let out = realign(&old, new).unwrap();
    assert_eq!(pos(&layer(&out, "Aacute"), 1), (325.0, 200.0));
    assert_eq!(pos(&layer(&out, "Agrave"), 1), (325.0, 200.0));
    assert_eq!(layer(&out, "Aacute").width, 650.0);
    assert_eq!(layer(&out, "Aacute.ss01").width, 650.0, "a composite of a composite follows too");
    assert!(Arc::ptr_eq(out.glyph("B").unwrap(), old.glyph("B").unwrap()), "unrelated glyphs stay shared");
}

#[test]
fn realign_gives_back_the_very_font_when_nothing_moves() {
    let old = accents();
    let mut new = old.clone();
    new.glyphs.get_mut("B").unwrap().layers[0].width = 620.0;
    let glyphs_before: Vec<_> = new.glyphs.as_arcs().to_vec();
    let out = realign(&old, new).unwrap();
    assert!(out.glyphs.as_arcs().iter().zip(&glyphs_before).all(|(a, b)| Arc::ptr_eq(a, b)));
    // Re-aligning an aligned font changes nothing either.
    let all: Vec<String> = out.glyphs.iter().map(|g| g.name.clone()).collect();
    let again = realign_glyphs(out.clone(), &all).unwrap();
    assert!(again.glyphs.as_arcs().iter().zip(out.glyphs.as_arcs()).all(|(a, b)| Arc::ptr_eq(a, b)));
}

#[test]
fn a_path_on_an_aligned_layer_stops_its_alignment() {
    let old = accents();
    let mut new = old.clone();
    new.glyphs.get_mut("Aacute").unwrap().layers[0].shapes.push(Shape::Path(Path::rect(0.0, 0.0, 10.0, 10.0)));
    let new = realign(&old, new).unwrap();
    let mut moved = new.clone();
    moved.glyphs.get_mut("A").unwrap().layers[0].width = 700.0;
    let out = realign(&new, moved).unwrap();
    let l = layer(&out, "Aacute");
    assert_eq!(l.width, 600.0, "the width stays");
    assert_eq!(pos(&l, 1), (300.0, 200.0), "the components keep their stored places");
    assert_eq!(layer(&out, "Agrave").width, 700.0, "a composite without a path follows");
}

#[test]
fn only_the_masters_an_edit_touched_are_realigned() {
    let mut f = accents();
    let mut second = f.masters[0].clone();
    second.id = "m02".into();
    Arc::make_mut(&mut f.masters).push(Master { name: "Bold".into(), ..second });
    for name in ["A", "acutecomb", "Aacute"] {
        let mut g = f.glyphs.get_mut(name).unwrap();
        let mut l = g.layers[0].clone();
        (l.id, l.master) = ("m02".into(), "m02".into());
        if name == "Aacute" {
            l.width = 999.0;
        }
        g.layers.push(l);
    }
    let mut new = f.clone();
    new.glyphs.get_mut("A").unwrap().layers[0].width = 640.0;
    let out = realign(&f, new).unwrap();
    let g = out.glyph("Aacute").unwrap();
    assert_eq!(g.layers[0].width, 640.0);
    assert_eq!(g.layers[1].width, 999.0, "the stale layer of the other master is left as stored");
}

#[test]
fn changed_glyphs_lists_changed_added_and_removed_glyphs() {
    let old = accents();
    let mut new = old.clone();
    new.glyphs.get_mut("Agrave").unwrap().layers[0].width = 5.0;
    new.glyphs.remove("B");
    new.glyphs.push(fontcraft_model::Glyph::new("C"));
    assert_eq!(changed_glyphs(&old, &new), vec!["Agrave".to_string(), "C".into(), "B".into()]);
    assert!(changed_glyphs(&old, &old.clone()).is_empty());
    let mut off = old.clone();
    Arc::make_mut(&mut off.info).settings.disables_automatic_alignment = Some(true);
    assert_eq!(changed_glyphs(&old, &off).len(), old.glyphs.len(), "the font setting touches every glyph");
}

#[test]
fn locked_composites_keep_their_drawing() {
    let old = accents();
    let mut new = old.clone();
    new.glyphs.get_mut("Aacute").unwrap().info.locked = true;
    let old = realign(&old, new).unwrap();
    let mut new = old.clone();
    new.glyphs.get_mut("A").unwrap().layers[0].width = 700.0;
    let out = realign(&old, new).unwrap();
    assert_eq!(layer(&out, "Aacute").width, 600.0);
    assert_eq!(layer(&out, "Agrave").width, 700.0);
}

#[test]
fn glyphs_in_a_cycle_are_skipped_and_their_users_too() {
    let f = font(vec![
        composite("a", vec![part("b")]),
        composite("b", vec![part("a")]),
        composite("c", vec![part_at("a", 5.0, 0.0)]),
        letter_a(),
        top_mark("acutecomb"),
        composite("Aacute", vec![part("A"), part("acutecomb")]),
    ]);
    let names: Vec<String> = ["a", "A"].iter().map(|s| s.to_string()).collect();
    let out = realign_glyphs(f.clone(), &names).unwrap();
    for g in ["a", "b", "c"] {
        assert!(Arc::ptr_eq(out.glyph(g).unwrap(), f.glyph(g).unwrap()), "{g} is on or above a cycle");
    }
    assert_eq!(layer(&out, "Aacute").width, 600.0, "the rest of the font is aligned");
}

#[test]
fn a_composite_past_the_coordinate_limit_fails_the_realignment() {
    let old = font(vec![drawn("wide", 400_000.0, (0.0, 0.0, 10.0, 10.0), &[]), composite("twice", vec![part("wide"), part("wide")])]);
    let mut new = old.clone();
    new.glyphs.get_mut("wide").unwrap().layers[0].width = 600_000.0;
    assert!(matches!(realign(&old, new), Err(ComponentsError::BadPlacement { .. })));
}
