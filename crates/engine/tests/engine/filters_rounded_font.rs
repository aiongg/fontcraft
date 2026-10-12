//! Rounded Font (M3.12): `filter.roundedFont` and its export string `RoundedFont; stem`.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::model::StemDef;
use fontcraft_testkit::rect;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn shapes(s: &mut Session, glyph: &str) -> Value {
    run(s, "layer.inspect", json!({"glyph": glyph}))["shapes"].clone()
}

/// The on-curve nodes of the first path of a glyph's first master layer.
fn on_curve(s: &mut Session, glyph: &str) -> Vec<(f64, f64)> {
    shapes(s, glyph)[0]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["type"] != "offcurve")
        .map(|n| (n["x"].as_f64().unwrap(), n["y"].as_f64().unwrap()))
        .collect()
}

fn font(s: &Session) -> Value {
    serde_json::to_value(s.font().unwrap()).unwrap()
}

/// `I`: an 80-wide bar from (100, 0) to (180, 700); `L`: a stem 80 wide with an arm 60 thick;
/// `B`: a 300-unit square. With `stem`, the master has one vertical stem of that width.
fn fixture(stem: Option<f64>) -> Session {
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "I", "nodes": rect(100.0, 0.0, 180.0, 700.0)}));
    let l = json!([{"x": 0, "y": 0}, {"x": 300, "y": 0}, {"x": 300, "y": 60}, {"x": 80, "y": 60}, {"x": 80, "y": 700}, {"x": 0, "y": 700}]);
    run(&mut s, "path.addPath", json!({"glyph": "L", "nodes": l}));
    run(&mut s, "path.addPath", json!({"glyph": "B", "nodes": rect(0.0, 0.0, 300.0, 300.0)}));
    if let Some(width) = stem {
        s.edit("stems", |font| {
            Arc::make_mut(&mut font.stems).push(StemDef { name: "stem".into(), ..StemDef::default() });
            for m in Arc::make_mut(&mut font.masters) {
                m.stem_values.push(width);
            }
            Ok(())
        })
        .unwrap();
    }
    s
}

#[test]
fn stem_endings_become_half_circles_with_an_overshoot() {
    let mut s = fixture(Some(80.0));
    let v = run(&mut s, "filter.roundedFont", json!({"glyph": "I"}));
    assert_eq!(v["glyphs"], json!(["I"]));
    assert_eq!(v["string"], "RoundedFont;");
    // Radius 40: the sides end 40·π/4 inside the old ends, the tips stand 40·(1 − π/4) past them,
    // rounded to the grid.
    assert_eq!(on_curve(&mut s, "I"), vec![(100.0, 31.0), (140.0, -9.0), (180.0, 31.0), (180.0, 669.0), (140.0, 709.0), (100.0, 669.0)]);
    let nodes = shapes(&mut s, "I")[0]["nodes"].as_array().unwrap().clone();
    assert_eq!(nodes.len(), 14);

    // Both ends of the L are rounded, the arm's too (thinner than the stem); its corners stay.
    run(&mut s, "filter.roundedFont", json!({"glyph": "L"}));
    let l = on_curve(&mut s, "L");
    assert_eq!(l.len(), 8, "{l:?}");
    assert!(l.contains(&(0.0, 0.0)) && l.contains(&(80.0, 60.0)));

    // A square wider than the stem has no endings.
    let before = shapes(&mut s, "B");
    run(&mut s, "filter.roundedFont", json!({"glyph": "B"}));
    assert_eq!(shapes(&mut s, "B"), before);

    // A stem given in the parameters takes the place of the master's: a narrower one leaves the
    // bar alone.
    let mut s = fixture(Some(80.0));
    let v = run(&mut s, "filter.roundedFont", json!({"glyph": "I", "stem": 50}));
    assert_eq!(v["string"], "RoundedFont;50;");
    assert_eq!(on_curve(&mut s, "I"), vec![(100.0, 0.0), (180.0, 0.0), (180.0, 700.0), (100.0, 700.0)]);
}

#[test]
fn the_export_string_and_the_parameters_give_the_same_font() {
    let forms = [
        ("filter.roundedFont", json!({"glyphs": ["I", "L"]})),
        ("filter.roundedFont", json!({"glyphs": ["I", "L"], "stem": 80})),
        ("filter.apply", json!({"glyphs": ["I", "L"], "string": "RoundedFont"})),
        ("filter.apply", json!({"glyphs": ["I", "L"], "string": "RoundedFont; 80"})),
        ("filter.apply", json!({"glyphs": ["I", "L"], "filter": "RoundedFont", "args": ["80"]})),
        ("filter.apply", json!({"glyphs": ["I", "L"], "filter": "filter.roundedFont", "args": {"stem": 80}})),
        ("filter.apply", json!({"string": "RoundedFont;include:I,L;"})),
        ("filter.apply", json!({"string": "RoundedFont;80;", "exclude": ["B"]})),
    ];
    let untouched = shapes(&mut fixture(Some(80.0)), "I");
    let mut results = Vec::new();
    for (id, params) in &forms {
        let mut s = fixture(Some(80.0));
        run(&mut s, id, params.clone());
        results.push((shapes(&mut s, "I"), shapes(&mut s, "L")));
    }
    assert_ne!(results[0].0, untouched, "the filter changed I");
    for (i, r) in results.iter().enumerate() {
        assert_eq!(r, &results[0], "form {i}: {}", forms[i].1);
    }
}

#[test]
fn filter_list_shows_the_optional_stem() {
    let mut s = fixture(None);
    let v = run(&mut s, "filter.list", json!({}));
    let f = v["filters"].as_array().unwrap().iter().find(|f| f["name"] == "RoundedFont").unwrap().clone();
    assert_eq!(f["command"], "filter.roundedFont");
    assert_eq!(f["label"], "Rounded Font");
    assert_eq!(f["string"], "RoundedFont;");
    let params = f["params"].as_array().unwrap();
    assert_eq!(params.len(), 1);
    assert_eq!(params[0]["key"], "stem");
    assert_eq!(params[0]["required"], false);
    assert_eq!(params[0]["default"].as_f64(), Some(0.0));
    assert_eq!(params[0]["slot"], json!({"position": 0}));
}

#[test]
fn a_missing_stem_and_hostile_arguments_are_errors_that_change_nothing() {
    let mut s = fixture(None);
    let before = font(&s);
    for (id, p) in [("filter.roundedFont", json!({"glyph": "I"})), ("filter.apply", json!({"string": "RoundedFont"}))] {
        let e = s.execute(id, &p).unwrap_err().to_string();
        assert!(e.contains("vertical stem"), "{id}: {e}");
        assert_eq!(font(&s), before, "{id} changed the font");
    }
    // A stem in the parameters needs none in the master.
    run(&mut s, "filter.roundedFont", json!({"glyph": "I", "stem": 80}));
    assert_eq!(on_curve(&mut s, "I").len(), 6);

    let mut s = fixture(Some(80.0));
    let before = font(&s);
    for text in ["RoundedFont;-5", "RoundedFont;NaN", "RoundedFont;1e300", "RoundedFont;80;3", "RoundedFont;Stem:80", "RoundedFont;abc"] {
        assert!(s.execute("filter.apply", &json!({"string": text})).is_err(), "{text}");
        assert_eq!(font(&s), before, "{text} changed the font");
    }
    let params = [
        json!({"glyph": "I", "stem": -1}),
        json!({"glyph": "I", "stem": "abc"}),
        json!({"glyph": "I", "stem": 2e6}),
        json!({"glyph": "I", "stem": true}),
        json!({"glyph": "I", "points": [[0, 0]]}),
        json!({"glyph": "Missing"}),
    ];
    for p in params {
        assert!(s.execute("filter.roundedFont", &p).is_err(), "{p}");
        assert_eq!(font(&s), before, "{p} changed the font");
    }
}
