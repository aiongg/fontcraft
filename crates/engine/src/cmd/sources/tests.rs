//! Behaviour tests of `source.diff`, `source.merge` and `font.compareGlyphs` on two versions of a
//! synthetic source that differ in every part.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use fontcraft_model::{
    Axis, Extra, Features, Font, Glyph, Instance, KernFirst, KernTable, Kerning, Layer, MasterKerning, Path as Contour, Property, PropertyValue,
    Shape, Snippet,
};
use serde_json::{Value, json};

use crate::Session;
use crate::io::SaveFormat;

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("fontcraft-sources-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn snippet(name: &str, code: &str) -> Snippet {
    serde_json::from_value(json!({ "name": name, "code": code })).unwrap()
}

fn instance(name: &str, wght: f64) -> Instance {
    serde_json::from_value(json!({ "name": name, "location": [wght] })).unwrap()
}

fn kern(entries: &[(&str, &str, &str, f64)]) -> Kerning {
    let table = entries
        .iter()
        .map(|(m, first, second, v)| MasterKerning {
            master: m.to_string(),
            firsts: vec![KernFirst { first: first.to_string(), seconds: vec![(second.to_string(), *v)] }],
        })
        .collect();
    Kerning { ltr: KernTable::new(table), ..Kerning::default() }
}

fn set_glyph(font: &mut Font, name: &str, f: impl FnOnce(&mut Glyph)) {
    let mut g = font.glyphs.get_mut(name).unwrap();
    f(&mut g);
}

/// Version A: two masters on a weight axis, one instance, a designer, a kerning pair with a
/// kerning group, a feature.
fn version_a() -> Font {
    let mut f = Font::new("Sample");
    f.axes = Arc::new(vec![Axis { name: "Weight".into(), tag: "wght".into(), ..Axis::default() }]);
    let mut masters = (*f.masters).clone();
    masters[0].location = vec![400.0];
    let mut bold = masters[0].clone();
    bold.id = "m02".into();
    bold.name = "Bold".into();
    bold.location = vec![700.0];
    masters.push(bold);
    f.masters = Arc::new(masters);
    f.instances = Arc::new(vec![instance("Regular", 400.0)]);
    Arc::make_mut(&mut f.info).properties.0.push(Property {
        key: "designers".into(),
        value: PropertyValue::Single("Ann".into()),
        extra: Extra::new(),
    });
    for name in ["A", "B", "C"] {
        set_glyph(&mut f, name, |g| {
            g.layers[0].shapes.push(Shape::Path(Contour::rect(50.0, 0.0, 550.0, 700.0)));
            let mut bold = Layer::new("m02", 650.0);
            bold.shapes.push(Shape::Path(Contour::rect(40.0, 0.0, 610.0, 700.0)));
            g.layers.push(bold);
        });
    }
    // C is told apart from B by its code point only; make its drawing its own too.
    set_glyph(&mut f, "C", |g| g.layers[0].shapes.push(Shape::Path(Contour::rect(200.0, 200.0, 300.0, 300.0))));
    set_glyph(&mut f, "A", |g| g.kern_groups.left = Some("A".into()));
    f.kerning = Arc::new(kern(&[("m01", "A", "V", -50.0)]));
    f.features = Arc::new(Features { features: vec![snippet("liga", "sub f i by fi;")], ..Features::default() });
    f
}

/// Version B: A changed in every part.
fn version_b() -> Font {
    let mut f = version_a();
    f.family_name = "Sample Pro".into();
    Arc::make_mut(&mut f.info).properties.0[0].value = PropertyValue::Single("Ann and Bo".into());
    Arc::make_mut(&mut f.axes)[0].hidden = true;
    let masters = Arc::make_mut(&mut f.masters);
    masters[1].name = "Heavy".into();
    let mut black = masters[1].clone();
    black.id = "m03".into();
    black.name = "Black".into();
    black.location = vec![900.0];
    masters.push(black);
    Arc::make_mut(&mut f.instances).push(instance("Bold", 700.0));
    set_glyph(&mut f, "A", |g| {
        g.layers[0].shapes = vec![Shape::Path(Contour::rect(60.0, 0.0, 560.0, 700.0))];
        g.layers.push(Layer::new("m03", 700.0));
        g.kern_groups.left = Some("A2".into());
    });
    set_glyph(&mut f, "C", |g| g.name = "C.alt".into());
    set_glyph(&mut f, "a", |g| g.unicodes = vec![0x61, 0xAA]);
    set_glyph(&mut f, "space", |g| g.layers[0].width = 260.0);
    f.glyphs.remove("B");
    let mut dcroat = Glyph::new("Dcroat");
    dcroat.unicodes.push(0x110);
    dcroat.layers.push(Layer::new("m01", 700.0));
    f.glyphs.push(dcroat);
    f.kerning = Arc::new(kern(&[("m01", "A", "V", -60.0), ("m02", "T", "o", -30.0)]));
    let features = Arc::make_mut(&mut f.features);
    features.features[0].code = "sub f l by fl;".into();
    features.classes.push(snippet("caps", "A B C"));
    f
}

fn save(font: &Font, path: &Path) {
    crate::io::save(font, path, SaveFormat::of_path(path).unwrap()).unwrap();
}

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn pair(name: &str) -> (PathBuf, PathBuf, PathBuf) {
    let d = dir(name);
    let (a, b) = (d.join("A.fontcraft"), d.join("B.fontcraft"));
    save(&version_a(), &a);
    save(&version_b(), &b);
    (a, b, d)
}

#[test]
fn source_diff_of_a_source_with_itself_is_empty() {
    let (a, _, _) = pair("self");
    let r = run(&mut Session::new(), "source.diff", json!({ "a": a, "b": a }));
    assert_eq!(r["same"], json!(true));
    assert_eq!(r["changes"], json!([]));
}

#[test]
fn source_diff_lists_changes_in_every_part() {
    let (a, b, _) = pair("parts");
    let r = run(&mut Session::new(), "source.diff", json!({ "a": a, "b": b }));
    assert_eq!(r["same"], json!(false));
    for part in ["glyphs", "info", "masters", "instances", "axes", "kerning", "features"] {
        assert!(r["summary"][part].as_u64().unwrap() > 0, "no change in {part}: {}", r["summary"]);
    }
    let changes = r["changes"].as_array().unwrap();
    let find = |kind: &str, glyph: &str, field: Option<&str>| {
        changes.iter().any(|c| c["kind"] == kind && c["glyph"] == glyph && field.is_none_or(|f| c["field"] == f))
    };
    assert!(find("renamed", "C.alt", Some("name")), "C to C.alt is a rename");
    assert!(find("removed", "B", None));
    assert!(find("added", "Dcroat", None));
    assert!(find("changed", "A", Some("outlines")));
    assert!(find("added", "A", Some("layer")), "the layer of the added master");
    assert!(find("changed", "a", Some("unicodes")));
    assert!(find("changed", "space", Some("width")));
    assert!(changes.iter().any(|c| c["part"] == "masters" && c["master"] == "Bold" && c["field"] == "name" && c["b"] == "Heavy"));
    assert!(changes.iter().any(|c| c["part"] == "masters" && c["kind"] == "added" && c["master"] == "Black"));
    assert!(changes.iter().any(|c| c["part"] == "kerning" && c["field"] == "ltr A V" && c["a"] == json!(-50.0) && c["b"] == json!(-60.0)));
    // Ids are stable: the same comparison gives the same ids.
    let again = run(&mut Session::new(), "source.diff", json!({ "a": a, "b": b }));
    assert_eq!(r["changes"], again["changes"]);
    // A tolerance hides outline moves smaller than it.
    let near = run(&mut Session::new(), "source.diff", json!({ "a": a, "b": b, "parts": ["glyphs"], "glyphs": ["A"], "tolerance": 20 }));
    assert!(!near["changes"].as_array().unwrap().iter().any(|c| c["field"] == "outlines"));
}

#[test]
fn source_merge_take_all_turns_a_into_b() {
    let (a, b, d) = pair("all");
    let out = d.join("merged.fontcraft");
    let mut s = Session::new();
    let r = run(&mut s, "source.merge", json!({ "base": a, "other": b, "take": "all", "output": out }));
    assert!(!r["applied"].as_array().unwrap().is_empty());
    let check = run(&mut s, "source.diff", json!({ "a": out, "b": b }));
    assert_eq!(check["changes"], json!([]), "the merge left differences");
    assert_eq!(check["same"], json!(true));
    // BASE is untouched.
    let base = run(&mut s, "source.diff", json!({ "a": a, "b": a }));
    assert_eq!(base["same"], json!(true));
    assert_eq!(crate::io::open(&a, None).unwrap().0.family_name, "Sample");
}

#[test]
fn source_merge_applies_only_the_chosen_change() {
    let (a, b, d) = pair("one");
    let mut s = Session::new();
    let diff = run(&mut s, "source.diff", json!({ "a": a, "b": b, "parts": ["info"] }));
    let id = diff["changes"].as_array().unwrap().iter().find(|c| c["field"] == "familyName").unwrap()["id"].clone();
    let out = d.join("one.fontcraft");
    run(&mut s, "source.merge", json!({ "base": a, "other": b, "take": [id], "output": out }));
    let merged = crate::io::open(&out, None).unwrap().0;
    assert_eq!(merged.family_name, "Sample Pro");
    assert!(merged.glyph("B").is_some(), "the glyph removal was not taken");
    assert_eq!(merged.masters.len(), 2);
}

#[test]
fn source_merge_refuses_a_stale_change_and_writes_nothing() {
    let (a, b, d) = pair("stale");
    let mut s = Session::new();
    let diff = run(&mut s, "source.diff", json!({ "a": a, "b": b }));
    let id = diff["changes"].as_array().unwrap().iter().find(|c| c["field"] == "familyName").unwrap()["id"].clone();
    // BASE changes after the comparison.
    let mut changed = version_a();
    changed.family_name = "Sample Two".into();
    save(&changed, &a);
    let out = d.join("stale.fontcraft");
    let err = s.execute("source.merge", &json!({ "base": a, "other": b, "take": [id], "output": out })).unwrap_err().to_string();
    assert!(err.contains("nothing was written"), "{err}");
    assert!(!out.exists());
    // An unknown part is refused before anything is read.
    let err = s.execute("source.merge", &json!({ "base": a, "other": b, "take": "all", "parts": ["hints"], "output": out })).unwrap_err();
    assert!(err.to_string().contains("unknown part"));
    assert!(!out.exists());
}

#[test]
fn source_merge_writes_base_format_and_needs_in_place_to_replace_base() {
    let d = dir("format");
    let (a, b) = (d.join("A.glyphs"), d.join("B.fontcraft"));
    save(&version_a(), &a);
    save(&version_b(), &b);
    let mut s = Session::new();
    let err = s.execute("source.merge", &json!({ "base": a, "other": b, "take": "all", "output": a })).unwrap_err();
    assert!(err.to_string().contains("inPlace"), "{err}");
    let err = s.execute("source.merge", &json!({ "base": a, "other": b, "take": "all", "output": d.join("x.ufo") })).unwrap_err();
    assert!(err.to_string().contains("format"), "{err}");
    let err = s.execute("source.merge", &json!({ "base": a, "other": b, "take": ["info:000000000000"], "inPlace": true })).unwrap_err();
    assert!(err.to_string().contains("nothing was written"), "{err}");
    let out = d.join("M.glyphs");
    let r = run(&mut s, "source.merge", json!({ "base": a, "other": b, "take": "all", "output": out }));
    assert_eq!(r["format"], json!("glyphs"));
    let merged = crate::io::open(&out, None).unwrap().0;
    assert_eq!(merged.source.format, fontcraft_model::SourceFormat::Glyphs);
    assert_eq!(merged.family_name, "Sample Pro");
    assert!(merged.glyph("C.alt").is_some() && merged.glyph("B").is_none());
}

#[test]
fn font_compare_glyphs_reports_and_copies_differences_to_the_background() {
    let (a, b, _) = pair("glyphs");
    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": a }));
    let r = run(&mut s, "font.compareGlyphs", json!({ "a": a, "b": b, "differencesToBackground": true }));
    assert_eq!(r["same"], json!(false));
    assert_eq!(r["onlyInA"], json!(["B", "C"]));
    assert_eq!(r["onlyInB"], json!(["C.alt", "Dcroat"]));
    assert_eq!(r["outlines"], json!([{ "glyph": "A", "master": "Regular" }]));
    assert_eq!(r["names"], json!([{ "glyph": "a", "a": [0x61], "b": [0x61, 0xAA] }]));
    assert_eq!(r["copiedToBackground"], json!(1));
    let font = s.font().unwrap();
    let bg = font.glyph("A").unwrap().layer("m01").unwrap().info.background.clone().unwrap();
    assert_eq!(bg.shapes, vec![Shape::Path(Contour::rect(60.0, 0.0, 560.0, 700.0))]);
    // Near matching accepts the 10-unit move.
    let near = run(&mut s, "font.compareGlyphs", json!({ "a": a, "b": b, "what": ["outlines"], "tolerance": 10 }));
    assert_eq!(near["outlines"], json!([]));
    let err = s.execute("font.compareGlyphs", &json!({ "a": a, "b": b, "what": ["bitmaps"] })).unwrap_err();
    assert!(err.to_string().contains("not supported"));
}
