//! Small caps, subscripts and superscripts (M13.19): `glyph.addSmallCaps` and
//! `glyph.addSubSuperscripts`.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use fontcraft_engine::Session;
use fontcraft_model::Shape;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn add_polygon(s: &mut Session, glyph: &str, points: &[(i32, i32)]) {
    let nodes: Vec<Value> = points.iter().map(|(x, y)| json!({"x": x, "y": y})).collect();
    run(s, "path.addPath", json!({"glyph": glyph, "nodes": nodes}));
}

/// The nodes of the glyph's first path on the first master, in path order.
fn nodes(s: &Session, glyph: &str) -> Vec<(f64, f64)> {
    let font = s.font().unwrap();
    let g = font.glyph(glyph).unwrap_or_else(|| panic!("no glyph `{glyph}`"));
    let layer = g.layer(&font.masters[0].id).unwrap();
    let Some(Shape::Path(path)) = layer.shapes.first() else { panic!("`{glyph}` has no path") };
    path.nodes.iter().map(|n| (n.x, n.y)).collect()
}

/// The code of the font's feature `tag`.
fn feature(s: &Session, tag: &str) -> String {
    let font = s.font().unwrap();
    let f = font.features.features.iter().find(|f| f.name == tag).unwrap_or_else(|| panic!("no feature `{tag}`"));
    f.code.clone()
}

fn near(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance
}

/// `H`: 80-unit stems at 0 and 520, a 60-unit crossbar from 300 to 360, cap height 700. `n`: two
/// 60-unit stems and a 60-unit arch top at the x-height 500.
fn fixture() -> Session {
    let mut s = Session::with_document();
    add_polygon(
        &mut s,
        "H",
        &[(0, 0), (80, 0), (80, 300), (520, 300), (520, 0), (600, 0), (600, 700), (520, 700), (520, 360), (80, 360), (80, 700), (0, 700)],
    );
    add_polygon(&mut s, "n", &[(0, 0), (60, 0), (60, 440), (240, 440), (240, 0), (300, 0), (300, 500), (0, 500)]);
    s
}

#[test]
fn add_small_caps_scales_the_capital_to_the_x_height_with_lowercase_stems() {
    let mut s = fixture();
    let h_before = nodes(&s, "H");
    let out = run(&mut s, "glyph.addSmallCaps", json!({"glyphs": ["H", "space"]}));
    assert_eq!(out["created"], json!(["h.sc"]), "{out}");
    assert_eq!(out["skipped"][0]["glyph"], "space", "{out}");

    // The capital's 80-unit stems, scaled by 500/700, are thickened to the n's 60 units, and the
    // zones stay at the baseline and the x-height.
    let sc = nodes(&s, "h.sc");
    assert_eq!(sc.len(), 12);
    let (left, right) = (sc[1].0 - sc[0].0, sc[5].0 - sc[4].0);
    assert!(near(left, 60.0, 1.5) && near(right, 60.0, 1.5), "stems {left} and {right}: {sc:?}");
    assert!(near(sc[0].1, 0.0, 1.0) && near(sc[6].1, 500.0, 1.0), "heights: {sc:?}");

    // The original is untouched; the features map both cases to the new glyph.
    assert_eq!(nodes(&s, "H"), h_before);
    assert!(feature(&s, "smcp").contains("sub h by h.sc;"));
    assert!(feature(&s, "c2sc").contains("sub H by h.sc;"));

    // A second run remakes the glyph and adds no line twice.
    let again = run(&mut s, "glyph.addSmallCaps", json!({"glyphs": ["h"]}));
    assert_eq!(again["replaced"], json!(["h.sc"]), "{again}");
    assert_eq!(again["created"], json!([]));
    assert_eq!(feature(&s, "smcp").matches("sub h by h.sc;").count(), 1);
    assert_eq!(feature(&s, "c2sc").matches("sub H by h.sc;").count(), 1);
}

#[test]
fn add_small_caps_appends_to_a_hand_written_feature_and_makes_petite_caps() {
    let mut s = fixture();
    run(&mut s, "font.features.addFeature", json!({"tag": "smcp", "code": "sub a by a.sc;\n"}));
    let out = run(&mut s, "glyph.addSmallCaps", json!({"glyphs": ["H"]}));
    assert_eq!(out["features"][0], json!({"tag": "smcp", "action": "appended", "lines": 1}), "{out}");
    let code = feature(&s, "smcp");
    assert!(code.contains("sub a by a.sc;") && code.contains("sub h by h.sc;"), "{code}");
    assert_eq!(s.font().unwrap().features.features.iter().filter(|f| f.name == "smcp").count(), 1);

    let petite = run(&mut s, "glyph.addSmallCaps", json!({"glyphs": ["H"], "petite": true, "height": 450}));
    assert_eq!(petite["created"], json!(["h.pc"]), "{petite}");
    assert!(feature(&s, "pcap").contains("sub h by h.pc;"));
    assert!(feature(&s, "c2pc").contains("sub H by h.pc;"));
    let pc = nodes(&s, "h.pc");
    assert!(near(pc[6].1, 450.0, 1.0), "{pc:?}");
}

#[test]
fn add_sub_superscripts_scales_shifts_and_keeps_stems_darker() {
    let mut s = fixture();
    run(&mut s, "glyph.newGlyph", json!({"name": "one"}));
    add_polygon(&mut s, "one", &[(200, 0), (280, 0), (280, 700), (200, 700)]);
    let before = nodes(&s, "one");
    let out = run(&mut s, "glyph.addSubSuperscripts", json!({"glyphs": ["one"], "suffix": "sups", "feature": "sups"}));
    assert_eq!(out["created"], json!(["one.sups"]), "{out}");

    // Two thirds of the size, raised 450 units, the 80-unit stem at three quarters (60), not two
    // thirds (53).
    let sup = nodes(&s, "one.sups");
    let stem = sup[1].0 - sup[0].0;
    assert!(near(stem, 60.0, 1.5), "stem {stem}: {sup:?}");
    assert!(near(sup[0].1, 450.0, 1.0) && near(sup[2].1, 450.0 + 700.0 * 2.0 / 3.0, 1.0), "{sup:?}");
    assert_eq!(nodes(&s, "one"), before);
    assert!(feature(&s, "sups").contains("sub one by one.sups;"));

    // A subscript feature drops below the baseline by default; a second sups run repeats nothing.
    let sub = run(&mut s, "glyph.addSubSuperscripts", json!({"glyphs": ["one"], "suffix": ".subs", "feature": "subs"}));
    assert_eq!(sub["created"], json!(["one.subs"]), "{sub}");
    assert!(near(nodes(&s, "one.subs")[0].1, -150.0, 1.0));
    run(&mut s, "glyph.addSubSuperscripts", json!({"glyphs": ["one"], "suffix": "sups", "feature": "sups"}));
    assert_eq!(feature(&s, "sups").matches("sub one by one.sups;").count(), 1);
}

#[test]
fn derived_glyph_commands_refuse_bad_parameters_and_change_nothing() {
    let mut s = fixture();
    let before = serde_json::to_value(s.font().unwrap()).unwrap();
    for (id, p) in [
        ("glyph.addSubSuperscripts", json!({"glyphs": ["H"], "suffix": "", "feature": "sups"})),
        ("glyph.addSubSuperscripts", json!({"glyphs": ["H"], "suffix": "sups", "feature": "sup"})),
        ("glyph.addSubSuperscripts", json!({"glyphs": ["H"], "suffix": "sups", "feature": "sups", "scale": 0})),
        ("glyph.addSubSuperscripts", json!({"glyphs": ["H"], "suffix": "sups", "feature": "sups", "masters": ["nope"]})),
        ("glyph.addSubSuperscripts", json!({"suffix": "sups", "feature": "sups"})),
        ("glyph.addSmallCaps", json!({"glyphs": ["H"], "height": -5})),
        ("glyph.addSmallCaps", json!({"glyphs": ["H"], "stemScale": {"v": 1e9}})),
        ("glyph.addSmallCaps", json!({"glyphs": ["H"], "stemScale": {"w": 1}})),
        ("glyph.addSmallCaps", json!({"glyphs": ["nope"]})),
    ] {
        assert!(s.execute(id, &p).is_err(), "{id} {p} should fail");
    }
    assert_eq!(serde_json::to_value(s.font().unwrap()).unwrap(), before);
}
