//! Instance and master commands: Add Instance for Each Master, Instance as Master (the
//! handbook's Thin–Regular–Bold example), Add Variable Font Setting, Add Other Font (several fonts
//! combined as masters) and Generate Instances, on fonts built in code.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::interp::Designspace;
use fontcraft_engine::interp::designspace::Location;
use fontcraft_engine::interp::instance::{InterpOptions, instance_font, master_weights};
use fontcraft_engine::{EngineError, Session};
use fontcraft_model::plist::{Str, Value as Plist};
use fontcraft_model::{Axis, Font, Glyph, Instance, Layer, Master, Param, Path, Shape};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

/// Runs a command that must be refused, with a message holding `needle`, and leave the font as it
/// was: the very same pointer.
fn refused(s: &mut Session, id: &str, p: Value, needle: &str) {
    let before = Arc::clone(&s.doc().unwrap().font);
    match s.execute(id, &p) {
        Err(EngineError::BadParams { message, .. }) => assert!(message.contains(needle), "{id} {p}: {message}"),
        other => panic!("{id} {p}: expected a refusal naming `{needle}`, got {other:?}"),
    }
    assert!(Arc::ptr_eq(&before, &s.doc().unwrap().font), "{id} {p}: a refused command changed the font");
}

fn font(s: &Session) -> &Font {
    s.font().unwrap()
}

fn x_height(font: &Font, master: usize) -> f64 {
    let i = font.metrics.iter().position(|d| d.kind == "x-height").unwrap();
    font.masters[master].metric_values[i].pos
}

/// The right edge of the first path of `glyph`'s layer `layer`.
fn right_edge(font: &Font, glyph: &str, layer: &str) -> f64 {
    let l = font.glyph(glyph).unwrap().layers.iter().find(|l| l.id == layer).unwrap();
    match l.shapes.first() {
        Some(Shape::Path(p)) => p.nodes.iter().map(|n| n.x).fold(f64::NEG_INFINITY, f64::max),
        other => panic!("{glyph}/{layer}: expected a path, got {other:?}"),
    }
}

/// The handbook's example: masters Thin (`m01`, wght 100) and Bold (`m02`, wght 700); instances
/// Thin, Light (250), Regular (400), Semibold (550) and Bold. `a` is a box whose right edge runs
/// from 150 to 350; `b` has an intermediate layer under Bold at wght 400 with its right edge at
/// 300, off the straight line. x-height 500 and 540; T–o −40 and −100.
fn thin_bold() -> Session {
    let mut f = Font::new("Test");
    Arc::make_mut(&mut f.axes).push(Axis { name: "Weight".into(), tag: "wght".into(), ..Default::default() });
    {
        let masters = Arc::make_mut(&mut f.masters);
        let thin = &mut masters[0];
        thin.name = "Thin".into();
        thin.location = vec![100.0];
        let bold =
            Master { id: "m02".into(), name: "Bold".into(), location: vec![700.0], metric_values: thin.metric_values.clone(), ..Default::default() };
        masters.push(bold);
    }
    f.set_metric_pos("m01", "x-height", 500.0).unwrap();
    f.set_metric_pos("m02", "x-height", 540.0).unwrap();
    for i in 0..f.glyphs.len() {
        let mut g = f.glyphs.get_mut_at(i).unwrap();
        let width = g.layers[0].width;
        let mut bold = Layer::new("m02", width);
        match g.name.as_str() {
            "a" => {
                g.layers[0].width = 200.0;
                g.layers[0].shapes.push(Shape::Path(Path::rect(50.0, 0.0, 150.0, 500.0)));
                bold.width = 400.0;
                bold.shapes.push(Shape::Path(Path::rect(50.0, 0.0, 350.0, 500.0)));
            }
            "b" => {
                g.layers[0].width = 200.0;
                g.layers[0].shapes.push(Shape::Path(Path::rect(50.0, 0.0, 150.0, 500.0)));
                bold.width = 400.0;
                bold.shapes.push(Shape::Path(Path::rect(50.0, 0.0, 350.0, 500.0)));
                let mut brace = Layer::new("m02", 300.0);
                brace.id = "brace-b".into();
                brace.info.name = Some("{400}".into());
                brace.info.attrs.coordinates = vec![Some(400.0)];
                brace.shapes.push(Shape::Path(Path::rect(50.0, 0.0, 300.0, 500.0)));
                g.layers.push(bold);
                g.layers.push(brace);
                continue;
            }
            _ => {}
        }
        g.layers.push(bold);
    }
    let k = Arc::make_mut(&mut f.kerning);
    k.ltr.set("m01", "T", "o", -40.0);
    k.ltr.set("m02", "T", "o", -100.0);
    let instances = Arc::make_mut(&mut f.instances);
    for (name, w) in [("Thin", 100.0), ("Light", 250.0), ("Regular", 400.0), ("Semibold", 550.0), ("Bold", 700.0)] {
        instances.push(Instance { name: name.into(), location: vec![w], ..Instance::default() });
    }
    let mut s = Session::new();
    s.add_document(f, None).unwrap();
    s
}

fn instance_index(font: &Font, name: &str) -> usize {
    font.instances.iter().position(|i| i.name == name).unwrap()
}

#[test]
fn add_instance_for_each_master_places_one_instance_on_every_master() {
    let mut s = thin_bold();
    let before = font(&s).instances.len();
    let out = run(&mut s, "font.instance.addForEachMaster", json!({}));
    let made = out["instances"].as_array().unwrap();
    assert_eq!(made.len(), 2);
    assert_eq!(made[0]["name"], "Thin");
    assert_eq!(made[1]["master"], "m02");
    let f = font(&s);
    assert_eq!(f.instances.len(), before + 2);
    let bold = &f.instances[before + 1];
    assert_eq!((bold.name.as_str(), bold.location.as_slice(), bold.kind.as_deref()), ("Bold", &[700.0][..], None));
    // Both names were taken already: each addition says so.
    assert_eq!(out["warnings"].as_array().unwrap().len(), 2);

    run(&mut s, "edit.undo", json!({}));
    assert_eq!(font(&s).instances.len(), before, "one undo step removes every instance it added");
    let out = run(&mut s, "font.instance.addForEachMaster", json!({"masters": ["Bold"]}));
    assert_eq!(out["instances"].as_array().unwrap().len(), 1);
    refused(&mut s, "font.instance.addForEachMaster", json!({"masters": ["Black"]}), "Black");
}

#[test]
fn instance_as_master_matches_the_handbook_thin_regular_bold_example() {
    let mut s = thin_bold();
    let opts = InterpOptions::default();
    let before = Arc::clone(&s.doc().unwrap().font);
    let light_before = instance_font(&before, instance_index(&before, "Light"), &opts).unwrap();
    let semibold_before = instance_font(&before, instance_index(&before, "Semibold"), &opts).unwrap();

    let out = run(&mut s, "font.instance.toMaster", json!({"instance": "Regular"}));
    assert_eq!(out["master"]["name"], "Regular");
    let id = out["master"]["id"].as_str().unwrap().to_string();
    let f = Arc::clone(&s.doc().unwrap().font);
    assert_eq!(f.masters.len(), 3);
    let regular = f.masters.iter().position(|m| m.id == id).unwrap();
    assert_eq!(f.masters[regular].location, vec![400.0]);
    // Halfway between Thin and Bold: outlines, width, metrics and kerning.
    assert!((right_edge(&f, "a", &id) - 250.0).abs() < 1e-9);
    assert!((f.glyph("a").unwrap().layers.iter().find(|l| l.id == id).unwrap().width - 300.0).abs() < 1e-9);
    assert!((x_height(&f, regular) - 520.0).abs() < 1e-9);
    assert!((f.kerning.ltr.get(&id, "T", "o").unwrap() + 70.0).abs() < 1e-9);

    // Light now interpolates Thin and Regular, Regular uses its master alone, Semibold
    // interpolates Regular and Bold.
    let ds = Designspace::new(&f).unwrap();
    let weights = |w: f64| -> Vec<(String, f64)> {
        master_weights(&ds, &Location(vec![w]), &opts).unwrap().into_iter().filter(|(_, x)| x.abs() > 1e-12).collect()
    };
    let names = |w: f64| -> Vec<String> {
        let mut n: Vec<String> = weights(w).into_iter().map(|(id, _)| f.masters.iter().find(|m| m.id == id).unwrap().name.clone()).collect();
        n.sort();
        n
    };
    assert_eq!(names(250.0), ["Regular", "Thin"]);
    assert_eq!(names(400.0), ["Regular"]);
    assert_eq!(names(550.0), ["Bold", "Regular"]);

    // The neighbouring instances export as they did: the new master lies on the line between the
    // old ones.
    let light_after = instance_font(&f, instance_index(&f, "Light"), &opts).unwrap();
    let semibold_after = instance_font(&f, instance_index(&f, "Semibold"), &opts).unwrap();
    let edge = |font: &Font| right_edge(font, "a", &font.masters[0].id);
    assert!((edge(&light_before) - edge(&light_after)).abs() < 1e-9);
    assert!((edge(&semibold_before) - edge(&semibold_after)).abs() < 1e-9);
    assert!((edge(&light_after) - 200.0).abs() < 1e-9);

    run(&mut s, "edit.undo", json!({}));
    assert_eq!(font(&s).masters.len(), 2, "Instance as Master is one undo step");
}

#[test]
fn instance_as_master_takes_the_place_of_an_intermediate_layer_at_its_location() {
    let mut s = thin_bold();
    let out = run(&mut s, "font.instance.toMaster", json!({"instance": "Regular"}));
    let id = out["master"]["id"].as_str().unwrap().to_string();
    assert_eq!(out["removedLayers"], json!([{"glyph": "b", "layer": "brace-b"}]));
    let f = font(&s);
    let b = f.glyph("b").unwrap();
    assert!(b.layers.iter().all(|l| l.id != "brace-b"));
    // The intermediate layer's design, not the straight line's 250.
    assert!((right_edge(f, "b", &id) - 300.0).abs() < 1e-9);
}

#[test]
fn instance_as_master_refuses_variable_settings_and_taken_locations() {
    let mut s = thin_bold();
    refused(&mut s, "font.instance.toMaster", json!({"instance": "Bold"}), "already sits");
    refused(&mut s, "font.instance.toMaster", json!({}), "`instance` is required");
    run(&mut s, "font.instance.addVariableFontSetting", json!({"name": "Variable"}));
    refused(&mut s, "font.instance.toMaster", json!({"instance": "Variable"}), "variable font setting");
}

#[test]
fn add_variable_font_setting_adds_a_variable_entry_and_warns_on_a_taken_name() {
    let mut s = thin_bold();
    let out = run(&mut s, "font.instance.addVariableFontSetting", json!({}));
    assert_eq!(out["name"], "Regular");
    let index = out["index"].as_u64().unwrap() as usize;
    let setting = &font(&s).instances[index];
    assert_eq!(setting.kind.as_deref(), Some("variable"));
    assert!(out["warnings"].as_array().unwrap().is_empty(), "a static instance called Regular does not clash with a setting");
    let again = run(&mut s, "font.instance.addVariableFontSetting", json!({"name": "Regular"}));
    assert_eq!(again["warnings"].as_array().unwrap().len(), 1);
    refused(&mut s, "font.instance.addVariableFontSetting", json!({"name": "  "}), "empty");
}

/// "Sans" (active, Regular at wght 400) and "Sans Black" (Black at wght 900: `A` drawn, no `b`,
/// an extra glyph, x-height 530, T–o −80).
fn two_fonts() -> Session {
    let mut sans = Font::new("Sans");
    Arc::make_mut(&mut sans.axes).push(Axis { name: "Weight".into(), tag: "wght".into(), ..Default::default() });
    Arc::make_mut(&mut sans.masters)[0].location = vec![400.0];
    let mut black = Font::new("Sans Black");
    Arc::make_mut(&mut black.axes).push(Axis { name: "Weight".into(), tag: "wght".into(), ..Default::default() });
    {
        let m = &mut Arc::make_mut(&mut black.masters)[0];
        m.name = "Black".into();
        m.location = vec![900.0];
    }
    black.glyph_mut("A").unwrap().layer_mut("m01").unwrap().shapes.push(Shape::Path(Path::rect(20.0, 0.0, 680.0, 700.0)));
    black.glyphs.remove("b");
    let mut extra = Glyph::new("extra");
    extra.layers.push(Layer::new("m01", 500.0));
    black.glyphs.push(extra);
    black.set_metric_pos("m01", "x-height", 530.0).unwrap();
    Arc::make_mut(&mut black.kerning).ltr.set("m01", "T", "o", -80.0);
    let mut s = Session::new();
    s.add_document(sans, None).unwrap();
    s.add_document(black, None).unwrap();
    run(&mut s, "document.activate", json!({"index": 0}));
    s
}

#[test]
fn fonts_open_as_masters_through_add_other_font() {
    let mut s = two_fonts();
    let out = run(&mut s, "font.master.addFromOtherFont", json!({"sources": [{"font": 1, "master": "Black"}]}));
    let made = &out["masters"][0];
    assert_eq!(made["name"], "Black");
    assert_eq!(made["from"], json!({"document": 1, "master": "m01"}));
    let id = made["id"].as_str().unwrap().to_string();
    let f = font(&s);
    assert_eq!(f.family_name, "Sans");
    assert_eq!(f.masters.len(), 2);
    let m = f.masters.iter().position(|m| m.id == id).unwrap();
    assert_eq!(f.masters[m].location, vec![900.0]);
    assert!((x_height(f, m) - 530.0).abs() < 1e-9);
    assert_eq!(f.glyph("A").unwrap().layers.iter().find(|l| l.id == id).unwrap().shapes.len(), 1);
    // A glyph the other font lacks gets an empty layer; one only it has is not added.
    assert!(f.glyph("b").unwrap().layers.iter().find(|l| l.id == id).unwrap().shapes.is_empty());
    assert!(f.glyph("extra").is_none());
    assert_eq!(f.kerning.ltr.get(&id, "T", "o"), Some(-80.0));
    assert_eq!(out["warnings"].as_array().unwrap().len(), 2);
    // The other document is untouched.
    assert_eq!(s.documents()[1].font.masters.len(), 1);

    run(&mut s, "edit.undo", json!({}));
    assert_eq!(font(&s).masters.len(), 1, "Add Other Font is one undo step");
    refused(&mut s, "font.master.addFromOtherFont", json!({"sources": [{"font": 7, "master": "Black"}]}), "no open document");
    refused(&mut s, "font.master.addFromOtherFont", json!({"sources": [{"font": 1, "master": "Light"}]}), "Light");
    refused(&mut s, "font.master.addFromOtherFont", json!({"sources": []}), "empty");
}

#[test]
fn generate_instances_opens_a_one_master_document_per_instance() {
    let mut s = thin_bold();
    s.edit("parameters", |f| {
        let light = Arc::make_mut(&mut f.instances).iter_mut().find(|i| i.name == "Light").unwrap();
        light.params.0.push(Param::new("Rename Glyphs", Plist::String(Str::new("a=b"))));
        light.params.0.push(Param::new("InterpolationWeightY", Plist::String(Str::new("250"))));
        Ok(())
    })
    .unwrap();
    let out = run(&mut s, "file.generateInstances", json!({"instances": ["Light", "Regular"]}));
    let docs = out["documents"].as_array().unwrap();
    assert_eq!(docs.len(), 2);
    assert_eq!(s.documents().len(), 3);
    assert_eq!(s.documents()[0].font.masters.len(), 2, "the source document is untouched");

    let light = &s.documents()[docs[0]["index"].as_u64().unwrap() as usize].font;
    assert_eq!(light.masters.len(), 1);
    assert_eq!(light.masters[0].name, "Light");
    assert_eq!(light.family_name, "Test");
    assert!((right_edge(light, "a", &light.masters[0].id) - 200.0).abs() < 1e-9);
    assert!(light.glyph("b").unwrap().layers.len() == 1, "no special layers");
    assert_eq!(light.instances.len(), 1);
    // The export parameters stay; the interpolation ones were applied and are gone.
    let names: Vec<&str> = light.instances[0].params.0.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Rename Glyphs"]);
    assert_eq!(light.instances[0].location, light.masters[0].location);

    let regular = &s.documents()[docs[1]["index"].as_u64().unwrap() as usize].font;
    assert!((right_edge(regular, "b", &regular.masters[0].id) - 300.0).abs() < 1e-9, "the intermediate layer counts");
    assert!((regular.kerning.ltr.get(&regular.masters[0].id, "T", "o").unwrap() + 70.0).abs() < 1e-9);
}

#[test]
fn generate_instances_saves_into_a_folder_without_opening() {
    let mut s = thin_bold();
    let dir = fontcraft_testkit::temp_dir("generate-instances");
    let out = run(&mut s, "file.generateInstances", json!({"open": false, "destination": dir.to_string_lossy()}));
    let docs = out["documents"].as_array().unwrap();
    assert_eq!(docs.len(), 5, "every active static instance");
    assert_eq!(s.documents().len(), 1);
    let path = std::path::PathBuf::from(docs[2]["path"].as_str().unwrap());
    assert_eq!(path.file_name().unwrap().to_string_lossy(), "Test-Regular.fontcraft");
    assert!(path.exists());

    refused(&mut s, "file.generateInstances", json!({"open": false}), "destination");
    run(&mut s, "font.instance.addVariableFontSetting", json!({"name": "Variable"}));
    refused(&mut s, "file.generateInstances", json!({"instances": ["Variable"]}), "variable font setting");
}
