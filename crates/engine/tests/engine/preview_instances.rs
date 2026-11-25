//! The engine half of the preview of instances: the instance views the preview draws
//! (`SessionInterpExt::instance_view`, `instance_layer`), `preview.render` and
//! `preview.toggleInstanceVisibility`.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use base64::Engine as _;
use fontcraft_engine::Session;
use fontcraft_engine::interp_cache::view_master_id;
use fontcraft_engine::model::{Axis, Font, Glyph, GlyphTable, Instance, Layer, Master, Path, Shape};
use fontcraft_engine::prelude::*;
use serde_json::{Value, json};

/// A box `w` wide, advance `w + 100`.
fn layer(master: &str, w: f64) -> Layer {
    let mut l = Layer::new(master, w + 100.0);
    l.shapes = vec![Shape::Path(Path::rect(0.0, 0.0, w, 700.0))];
    l
}

fn glyph(name: &str, unicode: u32, thin: f64, black: f64) -> Arc<Glyph> {
    let mut g = Glyph::new(name);
    g.unicodes = vec![unicode];
    g.layers = vec![layer("thin", thin), layer("black", black)];
    Arc::new(g)
}

/// Thin (100) and Black (900); instances Light (300), Regular (500) and a variable font
/// setting; `a` is 100 to 300 wide, `b` 200 to 600; `a b` kerned −40 on Thin and −80 on Black.
fn session() -> Session {
    let mut f = Font::new("Preview");
    f.axes = Arc::new(vec![Axis { name: "Weight".into(), tag: "wght".into(), ..Axis::default() }]);
    f.masters = Arc::new(
        [("thin", 100.0), ("black", 900.0)]
            .iter()
            .map(|(id, w)| Master { id: (*id).into(), name: (*id).into(), location: vec![*w], ..Master::default() })
            .collect(),
    );
    f.instances = Arc::new(vec![
        Instance { name: "Light".into(), location: vec![300.0], ..Instance::default() },
        Instance { name: "Regular".into(), location: vec![500.0], ..Instance::default() },
        Instance { name: "Variable".into(), kind: Some("variable".into()), ..Instance::default() },
    ]);
    f.glyphs = GlyphTable::from_glyphs(vec![glyph("a", 0x61, 100.0, 300.0), glyph("b", 0x62, 200.0, 600.0)]);
    let mut s = Session::new();
    s.add_document(f, None).unwrap();
    for (master, value) in [("thin", -40), ("black", -80)] {
        run(&mut s, "kerning.setPair", json!({"left": "a", "right": "b", "value": value, "master": master}));
    }
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn run_err(s: &mut Session, id: &str, p: Value) -> String {
    let before = s.doc().unwrap().font.clone();
    let e = s.execute(id, &p).err().unwrap_or_else(|| panic!("{id} {p} succeeded")).to_string();
    assert!(Arc::ptr_eq(&before, &s.doc().unwrap().font), "{id} {p} failed but changed the font");
    e
}

#[test]
fn an_instance_view_has_the_instance_widths_and_kerning_and_an_edit_redoes_only_its_glyph() {
    let mut s = session();
    let names = ["a".to_string(), "b".to_string()];
    let view = s.instance_view(1, &names).unwrap();
    let id = view_master_id(1);
    assert_eq!(view.glyph("a").unwrap().layer(&id).unwrap().width, 300.0);
    assert_eq!(view.glyph("b").unwrap().layer(&id).unwrap().width, 500.0);
    let pairs = &view.kerning.ltr.0;
    assert_eq!(pairs.len(), 1);
    assert_eq!(pairs[0].master, id, "the kerning is the view master's");

    let b = s.instance_layer("b", 1).unwrap();
    s.edit("widen a", |font| {
        font.glyph_mut("a")?.layers[1].width = 600.0;
        Ok(())
    })
    .unwrap();
    assert_eq!(s.instance_layer("a", 1).unwrap().width, 400.0, "the edited glyph follows the edit");
    assert!(Arc::ptr_eq(&s.instance_layer("b", 1).unwrap(), &b), "the other glyph is not interpolated again");
    assert!(s.instance_layer("a", 9).is_err());
    assert!(s.instance_layer("missing", 1).is_err());
}

#[test]
fn render_draws_the_line_at_the_instance_with_its_widths() {
    let mut s = session();
    let master = run(&mut s, "preview.render", json!({"glyphs": ["a"], "size": 120}));
    let regular = run(&mut s, "preview.render", json!({"glyphs": ["a"], "size": 120, "instance": "Regular"}));
    assert_eq!(master["instance"], "master");
    assert_eq!(master["master"], "thin");
    assert_eq!(regular["instance"], "Regular");
    assert_eq!(regular["glyphs"], json!(["a"]));
    assert_eq!(regular["height"], 120);
    // 8 pixels of padding on each side; the advance is 200 at Thin and 300 at Regular.
    let line = |r: &Value| r["width"].as_f64().unwrap() - 16.0;
    assert!((line(&regular) - 1.5 * line(&master)).abs() <= 1.5, "{} vs {}", line(&regular), line(&master));
    let png = base64::engine::general_purpose::STANDARD.decode(regular["png"].as_str().unwrap()).unwrap();
    assert_eq!(&png[1..4], b"PNG");

    // Text maps characters to glyphs; a character no glyph has is skipped with a warning.
    let text = run(&mut s, "preview.render", json!({"text": "abz", "format": "svg", "invert": true, "flip": true, "blur": 2}));
    assert_eq!(text["glyphs"], json!(["a", "b"]));
    assert!(text["svg"].as_str().unwrap().starts_with("<svg"));
    assert_eq!(text["warnings"].as_array().unwrap().len(), 1);
    let features = run(&mut s, "preview.render", json!({"glyphs": ["a"], "features": ["liga"]}));
    assert_eq!(features["warnings"].as_array().unwrap().len(), 1, "a feature the font does not have is named");

    run_err(&mut s, "preview.render", json!({}));
    run_err(&mut s, "preview.render", json!({"glyphs": ["a"], "format": "gif"}));
    run_err(&mut s, "preview.render", json!({"glyphs": ["a"], "size": 0}));
    run_err(&mut s, "preview.render", json!({"glyphs": ["a"], "instance": "Nope"}));
    run_err(&mut s, "preview.render", json!({"glyphs": ["a"], "instance": "Regular", "master": "thin"}));
    run_err(&mut s, "preview.render", json!({"glyphs": ["missing"]}));
    run_err(&mut s, "preview.render", json!({"glyphs": vec!["a"; 1001]}));
}

#[test]
fn render_shapes_with_the_features_asked_for() {
    let mut s = session();
    s.edit("glyphs", |font| {
        let mut all: Vec<Arc<Glyph>> = font.glyphs.iter().cloned().collect();
        let mut add = |name: &str, unicode: Option<u32>| {
            let mut g = Glyph::clone(&glyph(name, 0, 150.0, 150.0));
            g.unicodes = unicode.into_iter().collect();
            all.push(Arc::new(g));
        };
        add("a_b", None);
        add("beh-ar", Some(0x628));
        for form in ["init", "medi", "fina"] {
            add(&format!("beh-ar.{form}"), None);
        }
        font.glyphs = GlyphTable::from_glyphs(all);
        Ok(())
    })
    .unwrap();
    run(&mut s, "font.features.addFeature", json!({"tag": "liga", "code": "sub a b by a_b;"}));
    for form in ["init", "medi", "fina"] {
        run(&mut s, "font.features.addFeature", json!({"tag": form, "code": format!("script arab;\nsub beh-ar by beh-ar.{form};")}));
    }
    let plain = run(&mut s, "preview.render", json!({"text": "ab"}));
    assert_eq!((plain["glyphs"].clone(), plain["direction"].clone()), (json!(["a", "b"]), json!("ltr")));
    for instance in ["master", "Regular"] {
        let r = run(&mut s, "preview.render", json!({"text": "ab", "features": ["liga"], "instance": instance}));
        assert_eq!((r["glyphs"].clone(), r["warnings"].clone()), (json!(["a_b"]), json!([])), "{instance}: {r}");
    }
    let r = run(&mut s, "preview.render", json!({"text": "\u{628}\u{628}\u{628}", "features": ["init", "medi", "fina"]}));
    assert_eq!((r["glyphs"].clone(), r["direction"].clone()), (json!(["beh-ar.init", "beh-ar.medi", "beh-ar.fina"]), json!("rtl")), "{r}");
    run_err(&mut s, "preview.render", json!({"text": "ab", "features": ["toolong"]}));
    run_err(&mut s, "preview.render", json!({"text": "ab", "direction": "up"}));
}

#[test]
fn render_draws_locations_layers_nodes_and_a_frame_by_scale() {
    let mut s = session();
    let png = |r: &Value| r["png"].as_str().unwrap().to_string();
    // Regular lies at 500: the location there draws the same outline.
    let instance = run(&mut s, "preview.render", json!({"glyphs": ["a"], "instance": "Regular", "perEm": 100}));
    let location = run(&mut s, "preview.render", json!({"glyphs": ["a"], "location": {"wght": 500}, "perEm": 100}));
    assert_eq!(png(&instance), png(&location));
    assert_eq!((location["instance"].clone(), location["width"].clone()), (Value::Null, json!(46)), "300 units at 0.1 and 8 pixels around");
    let nodes = run(&mut s, "preview.render", json!({"glyphs": ["a"], "location": {"wght": 500}, "perEm": 100, "showNodes": true}));
    assert_ne!(png(&nodes), png(&location));
    let black = run(&mut s, "preview.render", json!({"glyphs": ["a"], "master": "black", "perEm": 100}));
    let layer = run(&mut s, "preview.render", json!({"glyphs": ["a"], "layer": "black", "perEm": 100}));
    assert_eq!((png(&black), json!("black")), (png(&layer), layer["master"].clone()));
    let mixed = run(&mut s, "preview.render", json!({"glyphs": ["a", "b"], "layers": {"b": "black"}, "perEm": 100}));
    assert_eq!(mixed["width"], json!(102), "a thin (200), b black (700), kerned -40");
    for bad in [
        json!({"glyphs": ["a"], "location": {"opsz": 1}}),
        json!({"glyphs": ["a"], "location": {"wght": "x"}}),
        json!({"glyphs": ["a"], "location": {"wght": 500}, "instance": "Regular"}),
        json!({"glyphs": ["a", "b"], "layer": "black"}),
        json!({"glyphs": ["a"], "layer": "nope"}),
        json!({"glyphs": ["a"], "layers": {"a": "black"}, "instance": "Regular"}),
        json!({"glyphs": ["a"], "palette": 1}),
        json!({"glyphs": ["a"], "color": true, "palette": 9}),
        json!({"glyphs": ["a"], "perEm": 0}),
        json!({"glyphs": ["a"], "scale": 1e9}),
        json!({"glyphs": ["a"], "scale": 1, "size": 100}),
    ] {
        run_err(&mut s, "preview.render", bad);
    }
}

#[test]
fn compare_marks_the_glyph_an_edit_changed() {
    let mut s = session();
    let revision = s.doc().unwrap().revision;
    run(&mut s, "path.addRectangle", json!({"glyph": "a", "layer": "thin", "x": 300, "y": 0, "width": 50, "height": 50}));
    let r = run(&mut s, "preview.compare", json!({"revision": revision, "glyphs": ["a", "b"], "perEm": 100}));
    assert_eq!((r["changed"].clone(), r["changedCount"].clone(), r["kerningChanged"].clone()), (json!(["a"]), json!(1), json!(false)));
    let slots: Vec<bool> = r["glyphs"].as_array().unwrap().iter().map(|g| g["changed"].as_bool().unwrap()).collect();
    assert_eq!(slots, [true, false]);
    let default = run(&mut s, "preview.compare", json!({}));
    assert_eq!(default["glyphs"].as_array().unwrap().len(), 1, "the changed glyphs, one undo step back");
    // Undone, the document is at a new revision; the step is found among the redo steps.
    let edited = s.doc().unwrap().revision;
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(run(&mut s, "preview.compare", json!({"revision": edited}))["changed"], json!(["a"]));
    let same = run(&mut s, "preview.compare", json!({"steps": 0}));
    assert_eq!((same["changedCount"].clone(), same.get("png").is_some()), (json!(0), false));
    let removed = run(&mut s, "preview.compare", json!({"steps": 0, "glyphs": ["a"], "instance": "Regular"}));
    assert_eq!(removed["glyphs"][0]["changed"], json!(false));
    for bad in
        [json!({"revision": u64::MAX}), json!({"steps": 1000}), json!({"steps": 1, "revision": 1}), json!({"steps": -1}), json!({"color": true})]
    {
        run_err(&mut s, "preview.compare", bad);
    }
}

#[test]
fn toggle_instance_visibility_is_one_undo_step_and_a_no_op_changes_nothing() {
    let mut s = session();
    assert!(s.font().unwrap().instances[0].visible);
    let r = run(&mut s, "preview.toggleInstanceVisibility", json!({"instance": "Light"}));
    assert_eq!(r, json!({"instance": 0, "name": "Light", "on": false}));
    assert!(!s.font().unwrap().instances[0].visible);
    run(&mut s, "edit.undo", json!({}));
    assert!(s.font().unwrap().instances[0].visible);

    let before = s.doc().unwrap().font.clone();
    let r = run(&mut s, "preview.toggleInstanceVisibility", json!({"instance": 1, "on": true}));
    assert_eq!(r["on"], true);
    assert!(Arc::ptr_eq(&before, &s.doc().unwrap().font), "setting the flag it already has changes nothing");

    run_err(&mut s, "preview.toggleInstanceVisibility", json!({}));
    run_err(&mut s, "preview.toggleInstanceVisibility", json!({"instance": "Nope"}));
    run_err(&mut s, "preview.toggleInstanceVisibility", json!({"instance": 0, "on": "yes"}));
}
