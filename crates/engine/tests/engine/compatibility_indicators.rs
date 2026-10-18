//! The compatibility indicators' data: the per-glyph check the Edit View bar, the Font View
//! corner and the Master Compatibility view read (`Session::glyph_compat`), the Master Compatible
//! smart-filter rule and the built-in Incompatible masters filter.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::interp::params::ENFORCE_COMPATIBILITY_CHECK;
use fontcraft_engine::model::plist::Value as Plist;
use fontcraft_engine::model::{Axis, Font, Glyph, GlyphTable, Instance, Layer, Master, Path, Shape};
use serde_json::{Value, json};

fn rect(x0: f64, w: f64) -> Shape {
    Shape::Path(Path::rect(x0, 0.0, x0 + w, 700.0))
}

fn glyph(name: &str, thin: Vec<Shape>, black: Vec<Shape>) -> Arc<Glyph> {
    let mut g = Glyph::new(name);
    for (master, shapes) in [("thin", thin), ("black", black)] {
        let mut l = Layer::new(master, 500.0);
        l.shapes = shapes;
        g.layers.push(l);
    }
    Arc::new(g)
}

/// Thin (100) and Black (900) with an instance at 400; `e` has one box on Thin and two on Black,
/// `o` one box on each.
fn session() -> Session {
    let mut f = Font::new("Compat");
    f.axes = Arc::new(vec![Axis { name: "Weight".into(), tag: "wght".into(), ..Axis::default() }]);
    f.masters = Arc::new(
        [("thin", 100.0), ("black", 900.0)]
            .iter()
            .map(|(id, w)| Master { id: (*id).into(), name: (*id).into(), location: vec![*w], ..Master::default() })
            .collect(),
    );
    f.instances = Arc::new(vec![Instance { name: "Regular".into(), location: vec![400.0], ..Instance::default() }]);
    f.glyphs = GlyphTable::from_glyphs(vec![
        glyph("e", vec![rect(0.0, 100.0)], vec![rect(0.0, 300.0), rect(400.0, 50.0)]),
        glyph("o", vec![rect(0.0, 100.0)], vec![rect(0.0, 300.0)]),
    ]);
    let mut s = Session::new();
    s.add_document(f, None).unwrap();
    s
}

fn query(s: &mut Session, params: Value) -> Vec<String> {
    let r = s.execute("fontView.queryGlyphs", &params).unwrap();
    r["glyphs"].as_array().unwrap().iter().map(|g| g.as_str().unwrap().to_string()).collect()
}

#[test]
fn smart_filter_incompatible_masters_and_the_master_compatible_rule_follow_the_check() {
    let mut s = session();
    assert!(!s.glyph_compat("e").unwrap().ok);
    assert!(s.glyph_compat("o").unwrap().ok);
    assert!(s.glyph_compat("missing").is_none());
    assert!(s.shows_incompatible("e") && !s.shows_incompatible("o"));

    assert_eq!(query(&mut s, json!({"entries": ["filters/Incompatible masters"]})), ["e"]);
    let rule = |v: bool| json!({"rules": {"all": [{"property": "masterCompatible", "op": "is", "value": v}]}});
    assert_eq!(query(&mut s, rule(true)), ["o"]);
    assert_eq!(query(&mut s, rule(false)), ["e"]);

    // An edit that makes `e` compatible is seen at once: the cache follows the revision.
    s.edit("fix e", |font| {
        font.glyph_mut("e")?.layers[1].shapes.pop();
        Ok(())
    })
    .unwrap();
    assert!(s.glyph_compat("e").unwrap().ok);
    assert!(query(&mut s, json!({"entries": ["filters/Incompatible masters"]})).is_empty());
}

#[test]
fn enforce_compatibility_check_off_hides_the_indicators_but_not_the_filter() {
    let mut s = session();
    s.edit("indicators off", |font| {
        Arc::make_mut(&mut font.info).params.set(ENFORCE_COMPATIBILITY_CHECK, Plist::Bool(false));
        Ok(())
    })
    .unwrap();
    assert!(!s.shows_incompatible("e"), "the parameter unchecked hides the red bar and corner");
    assert_eq!(query(&mut s, json!({"entries": ["filters/Incompatible masters"]})), ["e"]);
}

#[test]
fn without_an_instance_between_the_masters_nothing_is_incompatible() {
    let mut s = session();
    s.edit("no instances", |font| {
        font.instances = Arc::new(Vec::new());
        Ok(())
    })
    .unwrap();
    assert!(s.glyph_compat("e").unwrap().ok, "no instance needs both masters (compatibility.checkScope)");
    assert!(!s.shows_incompatible("e"));
}
