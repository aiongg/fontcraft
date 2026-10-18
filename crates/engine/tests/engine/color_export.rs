//! `color.describeExport`: the colour tables an export writes, per instance, from the same plan
//! the compiler reads.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_model::plist::Value as PlistValue;
use fontcraft_model::{Glyph, Instance, Layer, PaletteEntry, Path, Shape};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

/// Two palettes of two colours; `heart` has palette layers on rows 1 and 0, `plain` none; the
/// instance `Regular` exports, `NoColr` has Export COLR Table off.
fn color_font() -> Session {
    let mut s = Session::with_document();
    for c in ["#FF0000", "#00FF00"] {
        run(&mut s, "colorPalette.addColor", json!({"color": c}));
    }
    run(&mut s, "colorPalette.addPalette", json!({}));
    s.edit("glyphs", |font| {
        let master = font.masters[0].id.clone();
        let mut heart = Glyph::new("heart");
        heart.layers.push(Layer::new(master.as_str(), 500.0));
        for (id, row) in [("back", 1), ("front", 0)] {
            let mut l = Layer::new(master.as_str(), 500.0);
            l.id = id.into();
            l.info.attrs.color_palette = Some(PaletteEntry::Index(row));
            l.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 400.0, 400.0)));
            heart.layers.push(l);
        }
        font.glyphs.push(heart);
        let mut plain = Glyph::new("plain");
        let mut l = Layer::new(master.as_str(), 500.0);
        l.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 300.0, 300.0)));
        plain.layers.push(l);
        font.glyphs.push(plain);
        let instances = Arc::make_mut(&mut font.instances);
        instances.push(Instance { name: "Regular".into(), exports: true, ..Instance::default() });
        let mut off = Instance { name: "NoColr".into(), exports: true, ..Instance::default() };
        off.params.set("Export COLR Table", PlistValue::Bool(false));
        instances.push(off);
        Ok(())
    })
    .unwrap();
    s
}

#[test]
fn color_describe_export_reports_the_tables_layers_and_switches_of_each_instance() {
    let mut s = color_font();
    let out = run(&mut s, "color.describeExport", json!({}));
    let exports = out["exports"].as_array().unwrap();
    assert_eq!(exports.len(), 2, "{out}");

    let regular = &exports[0];
    assert_eq!(regular["instance"], json!("Regular"));
    assert_eq!(regular["tables"], json!(["COLR", "CPAL"]));
    assert_eq!(regular["colr"]["version"], json!(0));
    // Only the glyph with colour layers, its layers bottom first.
    assert_eq!(regular["colr"]["glyphs"], json!([{"glyph": "heart", "layers": ["back", "front"]}]));
    assert_eq!(regular["cpal"], json!({"palettes": 2, "colors": 2}));
    assert_eq!(regular["switches"]["exportColrTable"], json!(true));
    assert_eq!(regular["sbix"], Value::Null);
    assert_eq!(regular["refused"], json!(false));

    // The instance's own parameter switches the table off.
    let off = &exports[1];
    assert_eq!(off["instance"], json!("NoColr"));
    assert_eq!(off["tables"], json!([]));
    assert_eq!(off["switches"]["exportColrTable"], json!(false));

    // One instance by name or index, a master, and the CBDT option.
    let one = run(&mut s, "color.describeExport", json!({"instance": 1}));
    assert_eq!(one["exports"].as_array().unwrap().len(), 1);
    assert_eq!(one["exports"][0]["instance"], json!("NoColr"));
    let master_id = s.font().unwrap().masters[0].id.clone();
    let m = run(&mut s, "color.describeExport", json!({"master": master_id, "cbdt": true}));
    assert_eq!(m["exports"][0]["master"], json!(master_id));
    assert_eq!(m["exports"][0]["instance"], Value::Null);
    assert_eq!(m["exports"][0]["switches"]["cbdt"], json!(true));

    // Unknown targets are refused; the query changes nothing either way.
    assert!(s.execute("color.describeExport", &json!({"instance": "Bold"})).is_err());
    assert!(s.execute("color.describeExport", &json!({"instance": 0, "master": master_id})).is_err());
}
