//! Smart glyph and smart component commands (M6.8): properties, layer poles, component values.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::model::{Component, Glyph, Layer, Path, Shape};
use fontcraft_geom::Shape as _;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// Runs a command that must be refused, and checks that it left the font as it was.
fn refused(s: &mut Session, id: &str, p: Value) -> String {
    let before = s.doc().unwrap().font.clone();
    let err = s.execute(id, &p).err().unwrap_or_else(|| panic!("{id} {p} was accepted"));
    assert!(Arc::ptr_eq(&before, &s.doc().unwrap().font), "{id} {p} changed the font");
    err.to_string()
}

/// `_part.bar`: a master layer holding a 200 × 100 rectangle and a layer `Wide` (id `wide`)
/// holding a 600 × 100 one, no properties yet. `A` holds a component of `_part.bar` at (10, 0)
/// (shape 0) and a component of `B` (shape 1).
fn session() -> Session {
    let mut s = Session::with_document();
    s.edit("setup", |font| {
        let mut part = Glyph::new("_part.bar");
        for (id, name, width) in [("m01", None, 200.0), ("wide", Some("Wide"), 600.0)] {
            let mut l = Layer::new("m01", 600.0);
            l.id = id.into();
            if let Some(name) = name {
                l.info.name = Some(name.into());
            }
            l.shapes.push(Shape::Path(Path::rect(0.0, 0.0, width, 100.0)));
            part.layers.push(l);
        }
        font.glyphs.push(part);
        let mut a = font.glyph_mut("A")?;
        let l = a.layer_mut("m01").unwrap();
        l.shapes.push(Shape::Component(Component::new("_part.bar", 10.0, 0.0)));
        l.shapes.push(Shape::Component(Component::new("B", 0.0, 0.0)));
        Ok(())
    })
    .unwrap();
    s
}

/// `session()` with a Width property from 200 to 600: the master layer at its minimum, `Wide` at
/// its maximum.
fn width_session() -> Session {
    let mut s = session();
    run(&mut s, "smartGlyph.addProperty", json!({"glyph": "_part.bar", "name": "Width", "min": 200, "max": 600}));
    run(&mut s, "smartGlyph.setLayerPole", json!({"glyph": "_part.bar", "layer": "m01", "property": "Width", "pole": "min"}));
    run(&mut s, "smartGlyph.setLayerPole", json!({"glyph": "_part.bar", "layer": "Wide", "property": "Width", "pole": "max"}));
    s
}

fn width_of_a(s: &Session) -> f64 {
    s.font().unwrap().outline("A", "m01").unwrap().bounding_box().width()
}

/// `smartGlyph.addProperty` and `smart.propertyRangeRules`: the default range is 0 to 100,
/// negatives are allowed, the minimum must be below the maximum, names are unique.
#[test]
fn smart_glyph_add_property_follows_the_range_rules() {
    let mut s = session();
    let r = run(&mut s, "smartGlyph.addProperty", json!({"glyph": "_part.bar", "name": "Contrast"}));
    assert_eq!(r["property"], json!({"name": "Contrast", "min": 0.0, "max": 100.0, "toggle": false}));
    assert_eq!(r["index"], 0);
    let r = run(&mut s, "smartGlyph.addProperty", json!({"glyph": "_part.bar", "name": "Depth", "min": -50, "max": -10}));
    assert_eq!((r["property"]["min"].as_f64(), r["property"]["max"].as_f64()), (Some(-50.0), Some(-10.0)));
    for bad in [
        json!({"glyph": "_part.bar", "name": "Equal", "min": 5, "max": 5}),
        json!({"glyph": "_part.bar", "name": "Reversed", "min": 10, "max": 0}),
        json!({"glyph": "_part.bar", "name": "Contrast"}),
        json!({"glyph": "_part.bar", "name": ""}),
        json!({"glyph": "_part.bar", "name": "Huge", "max": 2e6}),
        json!({"glyph": "nope", "name": "Width"}),
    ] {
        refused(&mut s, "smartGlyph.addProperty", bad);
    }
    let parts = &s.font().unwrap().glyph("_part.bar").unwrap().info.parts;
    assert_eq!(parts.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Contrast", "Depth"]);
}

/// `smartGlyph.setProperty`: a rename carries the poles and the components' values; the range
/// after the change must keep the minimum below the maximum.
#[test]
fn smart_glyph_set_property_renames_everywhere_and_keeps_min_below_max() {
    let mut s = width_session();
    run(&mut s, "smartComponent.setValue", json!({"glyph": "A", "component": 0, "property": "Width", "value": 400}));
    let msg = refused(&mut s, "smartGlyph.setProperty", json!({"glyph": "_part.bar", "property": "Width", "min": 700}));
    assert!(msg.contains("below"), "{msg}");
    refused(&mut s, "smartGlyph.setProperty", json!({"glyph": "_part.bar", "property": "Width"}));
    refused(&mut s, "smartGlyph.setProperty", json!({"glyph": "_part.bar", "property": "Height", "min": 0}));

    let r = run(&mut s, "smartGlyph.setProperty", json!({"glyph": "_part.bar", "property": "Width", "name": "Breite", "max": 800}));
    assert_eq!(r["property"], json!({"name": "Breite", "min": 200.0, "max": 800.0, "toggle": false}));
    assert_eq!(r["renamedValues"], 1);
    let list = run(&mut s, "smartGlyph.listProperties", json!({"glyph": "_part.bar"}));
    assert_eq!(list["layers"][0]["poles"], json!([{"property": "Breite", "pole": "min"}]));
    assert_eq!(list["layers"][1]["poles"], json!([{"property": "Breite", "pole": "max"}]));
    let font = s.font().unwrap();
    let Shape::Component(c) = &font.glyph("A").unwrap().layer("m01").unwrap().shapes[0] else { panic!() };
    assert_eq!((c.info.smart_values[0].property.as_str(), c.info.smart_values[0].value), ("Breite", 400.0));
    // 400 on 200…800 is a third of the way from the 200-unit pole to the 600-unit one.
    assert!((width_of_a(&s) - (200.0 + 400.0 / 3.0)).abs() < 1e-6);

    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.font().unwrap().glyph("_part.bar").unwrap().info.parts[0].name, "Width");
}

/// `smartGlyph.removeProperty`: the property and its poles go; values on components stay.
#[test]
fn smart_glyph_remove_property_drops_its_poles() {
    let mut s = width_session();
    run(&mut s, "smartGlyph.addProperty", json!({"glyph": "_part.bar", "name": "Contrast"}));
    run(&mut s, "smartComponent.setValue", json!({"glyph": "A", "component": 0, "property": "Width", "value": 300}));
    let r = run(&mut s, "smartGlyph.removeProperty", json!({"glyph": "_part.bar", "property": "Width"}));
    assert_eq!((r["removed"].as_str(), r["polesRemoved"].as_u64()), (Some("Width"), Some(2)));
    let list = run(&mut s, "smartGlyph.listProperties", json!({"glyph": "_part.bar"}));
    assert_eq!(list["properties"].as_array().unwrap().len(), 1);
    assert!(list["layers"].as_array().unwrap().iter().all(|l| l["poles"] == json!([])));
    let font = s.font().unwrap();
    let Shape::Component(c) = &font.glyph("A").unwrap().layer("m01").unwrap().shapes[0] else { panic!() };
    assert_eq!(c.info.smart_values.len(), 1);
    refused(&mut s, "smartGlyph.removeProperty", json!({"glyph": "_part.bar", "property": "Width"}));
}

/// `smartGlyph.setLayerPole` and `smartGlyph.listProperties`: layers by id or name, `none`
/// clears a pole, unknown properties and layers are refused; a 0 to 1 range is a toggle
/// (`smart.zeroOneRangeIsToggle`).
#[test]
fn smart_glyph_set_layer_pole_and_list_properties() {
    let mut s = width_session();
    run(&mut s, "smartGlyph.addProperty", json!({"glyph": "_part.bar", "name": "Serif", "min": 0, "max": 1}));
    let list = run(&mut s, "smartGlyph.listProperties", json!({"glyph": "_part.bar"}));
    assert_eq!(list["smart"], true);
    assert_eq!(list["properties"][1], json!({"name": "Serif", "min": 0.0, "max": 1.0, "toggle": true}));
    assert_eq!(list["layers"][0], json!({"id": "m01", "name": "Regular", "master": "m01", "poles": [{"property": "Width", "pole": "min"}]}));
    assert_eq!(list["layers"][1]["name"], "Wide");

    let r = run(&mut s, "smartGlyph.setLayerPole", json!({"glyph": "_part.bar", "layer": "Regular", "property": "Serif", "pole": "max"}));
    assert_eq!((r["layer"].as_str(), r["pole"].as_str()), (Some("m01"), Some("max")));
    run(&mut s, "smartGlyph.setLayerPole", json!({"glyph": "_part.bar", "layer": "wide", "property": "Width", "pole": "none"}));
    let list = run(&mut s, "smartGlyph.listProperties", json!({"glyph": "_part.bar"}));
    assert_eq!(list["layers"][0]["poles"], json!([{"property": "Width", "pole": "min"}, {"property": "Serif", "pole": "max"}]));
    assert_eq!(list["layers"][1]["poles"], json!([]));

    refused(&mut s, "smartGlyph.setLayerPole", json!({"glyph": "_part.bar", "layer": "Narrow", "property": "Width", "pole": "min"}));
    refused(&mut s, "smartGlyph.setLayerPole", json!({"glyph": "_part.bar", "layer": "m01", "property": "Height", "pole": "min"}));
    refused(&mut s, "smartGlyph.setLayerPole", json!({"glyph": "_part.bar", "layer": "m01", "property": "Width", "pole": "top"}));
    assert_eq!(run(&mut s, "smartGlyph.listProperties", json!({"glyph": "B"}))["smart"], false);
}

/// `smartComponent.setValue` and `smart.valuesExtrapolate`: a value inside the range
/// interpolates, one beyond it is accepted and extrapolates; components of glyphs without the
/// property are refused.
#[test]
fn smart_component_set_value_interpolates_and_extrapolates() {
    let mut s = width_session();
    assert!((width_of_a(&s) - 200.0).abs() < 1e-6, "no value draws the master layer's pole");
    let r = run(&mut s, "smartComponent.setValue", json!({"glyph": "A", "component": 0, "property": "Width", "value": 400}));
    assert_eq!((r["extrapolates"].as_bool(), r["layer"].as_str()), (Some(false), Some("m01")));
    assert!((width_of_a(&s) - 400.0).abs() < 1e-6);
    let r = run(&mut s, "smartComponent.setValue", json!({"glyph": "A", "component": 0, "property": "Width", "value": 700}));
    assert_eq!(r["extrapolates"], true);
    assert!((width_of_a(&s) - 700.0).abs() < 1e-6);

    for (bad, says) in [
        (json!({"glyph": "A", "component": 1, "property": "Width", "value": 1}), "no smart properties"),
        (json!({"glyph": "A", "component": 0, "property": "Height", "value": 1}), "no property"),
        (json!({"glyph": "A", "component": 5, "property": "Width", "value": 1}), "does not resolve"),
        (json!({"glyph": "A", "component": 0, "property": "Width", "value": 2e6}), "within"),
        (json!({"glyph": "A", "component": 0, "property": "Width"}), "required"),
    ] {
        let msg = refused(&mut s, "smartComponent.setValue", bad);
        assert!(msg.contains(says), "{msg}");
    }
}
