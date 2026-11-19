//! Designspace helpers: Set Master Locations moves every source on an axis through one map,
//! Adjust by Variation interpolates and extrapolates master layers, Add Variation copies a
//! master onto a new axis in one undo step, and Merge Fonts to Masters builds a font whose
//! instances compile to the inputs.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_compile::{Flavour, Options, compile};
use fontcraft_engine::Session;
use fontcraft_model::plist::{Number, Value as Plist};
use fontcraft_model::{Axis, AxisRule, Font, Glyph, GlyphTable, Instance, Layer, Path, Shape, Target};
use serde_json::{Value, json};
use skrifa::FontRef;
use skrifa::raw::TableProvider;
use skrifa::raw::types::Tag;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

/// Runs a command that must fail and checks that the font is the same object as before and no
/// document opened.
fn refused(s: &mut Session, id: &str, p: Value) -> String {
    let before = s.doc().unwrap().font.clone();
    let docs = s.documents().len();
    let e = s.execute(id, &p).expect_err(id).to_string();
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "{id} {p}: a refused command changed the font");
    assert_eq!(s.documents().len(), docs, "{id} {p}: a refused command opened a document");
    e
}

fn session(font: Font) -> Session {
    let mut s = Session::new();
    s.add_document(font, None).unwrap();
    s
}

/// The right edge of a layer's paths.
fn right(layer: &Layer) -> f64 {
    layer
        .shapes
        .iter()
        .filter_map(|s| match s {
            Shape::Path(p) => Some(p),
            _ => None,
        })
        .flat_map(|p| p.nodes.iter().map(|n| n.x))
        .fold(f64::MIN, f64::max)
}

/// A master layer of `I`: a stem from x 100 to `stem_right`.
fn stem(master: &str, stem_right: f64, width: f64) -> Layer {
    let mut l = Layer::new(master, width);
    l.shapes.push(Shape::Path(Path::rect(100.0, 0.0, stem_right, 700.0)));
    l
}

/// A weight axis with Regular at `regular` and Bold at `bold`; one glyph `I` whose stem is 100
/// units wide in Regular and 200 in Bold.
fn two_masters(regular: f64, bold: f64) -> Font {
    let mut f = Font::new("Vary Sans");
    f.axes = Arc::new(vec![Axis { name: "Weight".into(), tag: "wght".into(), ..Default::default() }]);
    let mut masters = (*f.masters).clone();
    masters[0].location = vec![regular];
    let mut b = masters[0].clone();
    b.id = "m02".into();
    b.name = "Bold".into();
    b.location = vec![bold];
    masters.push(b);
    f.masters = Arc::new(masters);
    let mut i = Glyph::new("I");
    i.unicodes.push(0x49);
    i.layers.push(stem("m01", 200.0, 300.0));
    i.layers.push(stem("m02", 300.0, 400.0));
    f.glyphs = GlyphTable::from_glyphs(vec![Arc::new(i)]);
    f
}

#[test]
fn adjust_by_variation_interpolates_and_extrapolates_a_master_layer() {
    let mut s = session(two_masters(400.0, 700.0));
    let r = run(&mut s, "glyph.replaceWithInstance", json!({"glyphs": ["I"], "master": "Regular", "location": {"wght": 150}, "relative": true}));
    assert_eq!(r["glyphs"], json!(["I"]));
    assert_eq!(r["location"], json!({"wght": 550.0}));
    let f = s.font().unwrap();
    let i = f.glyph("I").unwrap();
    assert_eq!((right(i.layer("m01").unwrap()), i.layer("m01").unwrap().width), (250.0, 350.0), "halfway to Bold");
    assert_eq!(right(i.layer("m02").unwrap()), 300.0, "the other master stays");

    // Absolute, beyond the masters, every glyph with a layer for the master.
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "glyph.replaceWithInstance", json!({"master": "m02", "location": {"wght": 850}}));
    let i = s.font().unwrap().glyph("I").unwrap().clone();
    assert_eq!((right(i.layer("m02").unwrap()), i.layer("m02").unwrap().width), (350.0, 450.0), "extrapolated past Bold");
    assert_eq!(right(i.layer("m01").unwrap()), 200.0);

    refused(&mut s, "glyph.replaceWithInstance", json!({"location": {"opsz": 12}}));
    refused(&mut s, "glyph.replaceWithInstance", json!({"location": {}}));
}

#[test]
fn set_master_locations_moves_every_source_on_the_axis_through_one_map() {
    let mut f = two_masters(40.0, 160.0);
    f.instances = Arc::new(vec![Instance { name: "Mid".into(), location: vec![100.0], ..Default::default() }]);
    let mut glyph = Glyph::clone(f.glyph("I").unwrap());
    let mut brace = stem("m01", 250.0, 350.0);
    brace.id = "brace".into();
    brace.info.attrs.coordinates = vec![Some(100.0)];
    let mut bracket = stem("m02", 320.0, 400.0);
    bracket.id = "bracket".into();
    bracket.info.attrs.axis_rules = vec![AxisRule { min: Some(120.0), max: None }];
    glyph.layers.extend([brace, bracket]);
    f.glyphs = GlyphTable::from_glyphs(vec![Arc::new(glyph)]);
    let mut s = session(f);

    let r = run(&mut s, "font.setMasterLocations", json!({"axis": "wght", "map": [{"from": 160, "to": 900}, {"from": 40, "to": 100}]}));
    assert_eq!((r["masters"].clone(), r["layers"].clone()), (json!(["m01", "m02"]), json!(2)));
    let f = s.font().unwrap();
    let locations: Vec<f64> = f.masters.iter().map(|m| m.location[0]).collect();
    assert_eq!(locations, [100.0, 900.0]);
    assert_eq!(f.instances[0].location, [500.0], "halfway between the masters before and after");
    let layers = &f.glyph("I").unwrap().layers;
    let layer = |id: &str| layers.iter().find(|l| l.id == id).unwrap();
    assert_eq!(layer("brace").info.attrs.coordinates, [Some(500.0)]);
    let min = layer("bracket").info.attrs.axis_rules[0].min.unwrap();
    assert!((min - 1900.0 / 3.0).abs() < 1e-9, "the range starts at {min}");

    // A list moves only what it names.
    run(&mut s, "font.setMasterLocations", json!({"axis": "Weight", "scale": 2, "masters": ["Bold"]}));
    let f = s.font().unwrap();
    let locations: Vec<f64> = f.masters.iter().map(|m| m.location[0]).collect();
    assert_eq!(locations, [100.0, 1800.0]);
    assert_eq!(f.instances[0].location, [500.0]);
    assert_eq!(f.glyph("I").unwrap().layers.iter().find(|l| l.id == "brace").unwrap().info.attrs.coordinates, [Some(500.0)]);

    let e = refused(
        &mut s,
        "font.setMasterLocations",
        json!({"axis": "wght", "map": [{"from": 100, "to": 100}, {"from": 500, "to": 50}, {"from": 1800, "to": 900}]}),
    );
    assert!(e.contains("order"), "{e}");
    refused(&mut s, "font.setMasterLocations", json!({"axis": "wght", "offset": 1_999_000}));
    refused(&mut s, "font.setMasterLocations", json!({"axis": "wght", "map": [{"from": 100, "to": 0}], "scale": 2}));
}

#[test]
fn add_variation_copies_a_master_onto_a_new_axis_in_one_undo_step() {
    let mut s = Session::with_document();
    let before = s.doc().unwrap().font.clone();
    let r = run(
        &mut s,
        "font.addVariation",
        json!({
            "axis": {"tag": "wdth", "name": "Width"},
            "location": 75,
            "commands": [{"command": "font.master.set", "params": {"masters": ["$master"], "metrics": {"x-height": {"position": 450}}}}],
        }),
    );
    let id = r["master"]["id"].as_str().unwrap().to_string();
    assert_eq!(r["master"]["name"], "Regular Width 75");
    assert_eq!(r["axis"], json!({"tag": "wdth", "name": "Width", "added": true}));
    let f = s.font().unwrap();
    assert_eq!(f.axes.iter().map(|a| a.tag.as_str()).collect::<Vec<_>>(), ["wdth"]);
    let copy = f.master(&id).unwrap();
    assert_eq!((f.masters[0].location.clone(), copy.location.clone()), (vec![0.0], vec![75.0]));
    assert_eq!(f.metric_pos(copy, "x-height", 0.0), 450.0, "the command ran on the copy");
    assert_eq!(f.metric_pos(&f.masters[0], "x-height", 0.0), 500.0);
    assert!(f.glyphs.iter().all(|g| g.layer(&id).is_some()), "every glyph has a layer for the copy");

    run(&mut s, "edit.undo", json!({}));
    assert!(*s.doc().unwrap().font == *before, "one undo step takes all of it back");

    let e = refused(
        &mut s,
        "font.addVariation",
        json!({"axis": "wdth", "location": 75, "commands": [{"command": "font.master.set", "params": {"masters": ["no such master"]}}]}),
    );
    assert!(e.contains("commands[0]"), "{e}");
    refused(&mut s, "font.addVariation", json!({"axis": "wdth", "location": 0}));
}

/// A static font of `I` with the weight class `weight` and a stem from 100 to `stem_right`.
fn static_font(name: &str, weight: i64, stem_right: f64, width: f64) -> Font {
    let mut f = Font::new("Merge Sans");
    let mut glyphs: Vec<Arc<Glyph>> = f.glyphs.iter().filter(|g| matches!(g.name.as_str(), ".notdef" | "space")).cloned().collect();
    let mut i = Glyph::new("I");
    i.unicodes.push(0x49);
    i.layers.push(stem("m01", stem_right, width));
    glyphs.push(Arc::new(i));
    f.glyphs = GlyphTable::from_glyphs(glyphs);
    Arc::make_mut(&mut f.info).params.set("weightClass", Plist::Number(Number::int(weight)));
    Arc::make_mut(&mut f.masters)[0].name = name.into();
    f
}

fn table(bytes: &[u8], tag: &[u8; 4]) -> Vec<u8> {
    FontRef::new(bytes).unwrap().table_data(Tag::new(tag)).unwrap().as_bytes().to_vec()
}

#[test]
fn merged_fonts_compile_to_the_inputs_at_the_masters() {
    let inputs = [static_font("Light", 300, 160.0, 260.0), static_font("Bold", 700, 320.0, 420.0)];
    let mut s = Session::new();
    for f in &inputs {
        s.add_document(f.clone(), None).unwrap();
    }
    let r = run(&mut s, "font.mergeFontsToMasters", json!({"fonts": ["Untitled", "Untitled 2"]}));
    assert_eq!(r["axes"], json!(["wght"]), "the fonts differ in weight only");
    assert_eq!(r["masters"][0]["location"], json!({"wght": 300.0}));
    assert_eq!(r["masters"][1]["location"], json!({"wght": 700.0}));
    assert_eq!(r["title"], s.doc().unwrap().title.as_str());
    let merged = s.font().unwrap().clone();
    assert_eq!(merged.masters.len(), 2);
    assert_ne!(merged.masters[0].id, merged.masters[1].id);

    let options = Options { skip_features: true, remove_overlap: false, ..Options::default() };
    for (i, input) in inputs.iter().enumerate() {
        let want = compile(input, &Target::Master("m01".into()), Flavour::TrueType, &options).unwrap();
        let got = compile(&merged, &Target::Instance(i), Flavour::TrueType, &options).unwrap();
        assert_eq!(got.glyph_order, want.glyph_order);
        for tag in [b"glyf", b"loca", b"hmtx"] {
            assert_eq!(table(&got.bytes, tag), table(&want.bytes, tag), "instance {i}: {}", String::from_utf8_lossy(tag));
        }
        let weight = FontRef::new(&got.bytes).unwrap().os2().unwrap().us_weight_class();
        assert_eq!(weight, [300, 700][i]);
    }

    let e = refused(&mut s, "font.mergeFontsToMasters", json!({"fonts": ["Untitled", "Untitled"]}));
    assert!(e.contains("same weight class"), "{e}");
    refused(&mut s, "font.mergeFontsToMasters", json!({"fonts": ["Untitled"]}));
}
