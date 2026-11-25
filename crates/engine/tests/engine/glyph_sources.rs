//! Glyph sources: glyph.setSourceActive and view.showSourceInfluence, on a font built in code.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::interp::instance::{InterpOptions, instance_font};
use fontcraft_engine::model::{Axis, Font, Glyph, GlyphTable, Instance, Layer, Master, Path, Shape};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// The command is refused and the font stays as it was.
fn refused(s: &mut Session, id: &str, p: Value) -> String {
    let before = s.documents()[0].font.clone();
    let message = s.execute(id, &p).expect_err("a refusal").to_string();
    assert!(Arc::ptr_eq(&before, &s.documents()[0].font), "{id}: a refused command changed the font");
    message
}

fn layer(master: &str, id: &str, w: f64) -> Layer {
    let mut l = Layer::new(master, w + 100.0);
    l.id = id.into();
    l.shapes = vec![Shape::Path(Path::rect(0.0, 0.0, w, 700.0))];
    l
}

/// Thin (100), Black (900) and Wide, which `e` lacks; `e` is 100 and 300 wide with an intermediate layer 280 wide at
/// 500, and an instance stands at 500.
fn session() -> Session {
    let mut f = Font::new("Sources");
    f.axes = Arc::new(vec![
        Axis { name: "Weight".into(), tag: "wght".into(), ..Axis::default() },
        Axis { name: "Width".into(), tag: "wdth".into(), ..Axis::default() },
    ]);
    let master = |id: &str, location: [f64; 2]| Master { id: id.into(), name: id.into(), location: location.to_vec(), ..Master::default() };
    f.masters = Arc::new(vec![master("thin", [100.0, 100.0]), master("black", [900.0, 100.0]), master("wide", [100.0, 200.0])]);
    f.instances = Arc::new(vec![Instance { name: "Medium".into(), location: vec![500.0, 100.0], ..Instance::default() }]);
    let mut brace = layer("thin", "brace", 280.0);
    brace.info.attrs.coordinates = vec![Some(500.0), None];
    let mut e = Glyph::new("e");
    e.layers = vec![layer("thin", "thin", 100.0), layer("black", "black", 300.0), brace, layer("thin", "backup", 1.0)];
    f.glyphs = GlyphTable::from_glyphs(vec![Arc::new(e)]);
    let mut s = Session::new();
    s.add_document(f, None).unwrap();
    s
}

fn instance_width(s: &Session) -> f64 {
    let f = instance_font(s.font().unwrap(), 0, &InterpOptions::default()).unwrap();
    f.glyph("e").unwrap().layers[0].width
}

fn weights(s: &mut Session, location: Value) -> Vec<(String, f64)> {
    let r = s.query("view.showSourceInfluence", &json!({"glyph": "e", "location": location})).unwrap();
    r["sources"].as_array().unwrap().iter().map(|x| (x["layer"].as_str().unwrap().to_string(), x["weight"].as_f64().unwrap())).collect()
}

#[test]
fn an_inactive_source_leaves_the_instance_and_comes_back_with_undo() {
    let mut s = session();
    assert_eq!(instance_width(&s), 380.0);
    let r = run(&mut s, "glyph.setSourceActive", json!({"glyph": "e", "source": "brace", "active": false}));
    assert_eq!(r["changed"], json!(true));
    assert_eq!(instance_width(&s), 300.0, "halfway between the masters");
    let again = run(&mut s, "glyph.setSourceActive", json!({"glyph": "e", "source": "brace", "active": false}));
    assert_eq!(again["changed"], json!(false));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(instance_width(&s), 380.0, "one undo step");

    refused(&mut s, "glyph.setSourceActive", json!({"glyph": "e", "source": "thin", "active": false}));
    refused(&mut s, "glyph.setSourceActive", json!({"glyph": "e", "source": "backup", "active": false}));
    refused(&mut s, "glyph.setSourceActive", json!({"glyph": "e", "source": "brace"}));
}

#[test]
fn an_inactive_master_layer_can_be_re_interpolated_from_the_other_sources() {
    let mut s = session();
    run(&mut s, "glyph.setSourceActive", json!({"glyph": "e", "source": "black", "active": false}));
    run(&mut s, "layers.reInterpolate", json!({"glyph": "e", "layer": "black"}));
    let black = s.font().unwrap().glyph("e").unwrap().layers.iter().find(|l| l.id == "black").unwrap().width;
    assert_eq!(black, 560.0, "the intermediate layer's line carried on from Thin");
}

#[test]
fn weights_sum_to_one_and_name_what_is_left_out() {
    let mut s = session();
    for (wght, wdth) in [(100.0, 100.0), (300.0, 100.0), (500.0, 150.0), (700.0, 200.0), (900.0, 100.0)] {
        let w = weights(&mut s, json!({"wght": wght, "wdth": wdth}));
        assert!((w.iter().map(|x| x.1).sum::<f64>() - 1.0).abs() < 1e-9, "{w:?}");
    }
    assert_eq!(weights(&mut s, json!({"wght": 300.0})), [("thin".to_string(), 0.5), ("black".into(), 0.0), ("brace".into(), 0.5)]);
    run(&mut s, "glyph.setSourceActive", json!({"glyph": "e", "source": "brace", "active": false}));
    let r = s.query("view.showSourceInfluence", &json!({"glyph": "e", "location": {"wght": 300.0}})).unwrap();
    assert_eq!(r["missing"], json!([{"master": "wide", "name": "wide"}]));
    let brace = r["sources"].as_array().unwrap().iter().find(|x| x["layer"] == json!("brace")).unwrap();
    assert_eq!((&brace["active"], &brace["weight"], &brace["kind"]), (&json!(false), &json!(0.0), &json!("intermediate")));
    assert!(s.query("view.showSourceInfluence", &json!({"glyph": "e", "location": {"opsz": 1}})).is_err());
}
