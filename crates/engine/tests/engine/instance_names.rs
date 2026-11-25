//! `font.instance.names` (M7.32): what the exports write, against the fonts they build.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use fontcraft_compile::var::{VariableOptions, compile_variable};
use fontcraft_compile::{Flavour, Options, compile};
use fontcraft_engine::cmd::instance_names::names_of;
use fontcraft_engine::model::{Axis, Font, Glyph, GlyphTable, Instance, Target};
use fontcraft_engine::{EngineError, Session, work};
use serde_json::{Value, json};
use skrifa::FontRef;
use skrifa::raw::TableProvider;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

/// Two masters on a weight axis, three instances and a variable font setting.
fn family() -> Font {
    let mut f = Font::new("Slice Sans");
    f.axes = Arc::new(vec![Axis { name: "Weight".into(), tag: "wght".into(), ..Default::default() }]);
    let mut masters = (*f.masters).clone();
    masters[0].location = vec![400.0];
    let mut bold = masters[0].clone();
    bold.id = "m02".into();
    bold.name = "Bold".into();
    bold.location = vec![700.0];
    masters.push(bold);
    f.masters = Arc::new(masters);
    let glyphs = f
        .glyphs
        .iter()
        .map(|g| {
            let mut g = Glyph::clone(g);
            let mut layer = g.layers[0].clone();
            layer.id = "m02".into();
            layer.master = "m02".into();
            layer.width += 100.0;
            g.layers.push(layer);
            Arc::new(g)
        })
        .collect();
    f.glyphs = GlyphTable::from_glyphs(glyphs);
    let instance =
        |name: &str, at: f64, bold: bool| Instance { name: name.into(), exports: true, location: vec![at], is_bold: bold, ..Default::default() };
    let mut medium = instance("Medium", 550.0, false);
    medium.properties.set("descriptions", "{{{A:width}}} {{{nothing}}}", true);
    f.instances = Arc::new(vec![
        instance("Regular", 400.0, false),
        medium,
        instance("Bold", 700.0, true),
        Instance { name: "Roman".into(), exports: true, kind: Some("variable".into()), ..Default::default() },
    ]);
    f
}

fn records(bytes: &[u8]) -> Vec<(u16, u16, u16, String)> {
    let font = FontRef::new(bytes).unwrap();
    let table = font.name().unwrap();
    let data = table.string_data();
    let mut out: Vec<_> = table
        .name_record()
        .iter()
        .map(|r| (r.platform_id(), r.language_id(), r.name_id().to_u16(), r.string(data).unwrap().chars().collect()))
        .collect();
    out.sort();
    out
}

fn answered(entry: &Value) -> Vec<(u16, u16, u16, String)> {
    let n = |r: &Value, key: &str| u16::try_from(r[key].as_u64().unwrap()).unwrap();
    let mut out: Vec<_> = entry["names"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (n(r, "platform"), n(r, "language"), n(r, "id"), r["text"].as_str().unwrap().to_string()))
        .collect();
    out.sort();
    out
}

fn options() -> Options {
    Options { version_stamp: true, skip_features: true, remove_overlap: false, ..Options::default() }
}

/// The names and classes `font.instance.names` gives for `index` are those of the font built from it.
fn assert_same(font: &Font, entry: &Value, index: usize, what: &str) {
    let built = compile(font, &Target::Instance(index), Flavour::TrueType, &options()).unwrap_or_else(|e| panic!("{what}: {e}"));
    assert_eq!(answered(entry), records(&built.bytes), "{what}");
    let os2 = FontRef::new(&built.bytes).unwrap().os2().unwrap();
    assert_eq!(entry["weightClass"], json!(os2.us_weight_class()), "{what}");
    assert_eq!(entry["widthClass"], json!(os2.us_width_class()), "{what}");
}

#[test]
fn the_names_are_those_of_the_exported_fonts() {
    let font = family();
    let mut s = Session::new();
    s.add_document(font.clone(), None).unwrap();
    let out = run(&mut s, "font.instance.names", json!({}));
    let entries = out["instances"].as_array().unwrap();
    assert_eq!(entries.len(), 4);
    for (index, entry) in entries.iter().enumerate().take(3) {
        assert_eq!(entry["error"], Value::Null, "{entry}");
        assert_same(&font, entry, index, entry["name"].as_str().unwrap());
    }
    // The bold is the Bold of the Regular's family; the medium is a family of its own.
    assert_eq!(entries[2]["styleGroup"], json!({"family": "Slice Sans", "style": "Bold", "entries": [0, 2]}));
    assert_eq!(entries[2]["fsSelection"], json!(["bold"]));
    assert_eq!(entries[1]["styleGroup"]["family"], "Slice Sans Medium");
    // A glyph token reads the interpolated glyph; a token that names nothing is listed with its row.
    let text = entries[1]["names"].as_array().unwrap().iter().find(|r| r["id"] == 10).unwrap()["text"].as_str().unwrap().to_string();
    let widths: Vec<f64> = font.glyph("A").unwrap().layers.iter().map(|l| l.width).collect();
    assert_eq!(text, format!("{} {{{{{{nothing}}}}}}", (widths[0] + widths[1]) / 2.0));
    assert_eq!(entries[1]["tokens"], json!([{"scope": "instance", "param": false, "key": "descriptions", "index": 0, "tokens": ["nothing"]}]));

    // The variable font's, with its named instances and axis values.
    let built = compile_variable(&font, Some(3), &options(), &VariableOptions::default()).unwrap();
    let entry = &entries[3];
    assert_eq!(answered(entry), records(&built.compiled.bytes));
    assert_eq!(entry["fileName"], json!(built.file_stem));
    let named: Vec<&str> = entry["namedInstances"].as_array().unwrap().iter().map(|i| i["name"].as_str().unwrap()).collect();
    let compiled = FontRef::new(&built.compiled.bytes).unwrap();
    assert_eq!(named.len(), usize::from(compiled.fvar().unwrap().instance_count()));
    let values: usize = entry["stat"]["axes"].as_array().unwrap().iter().map(|a| a["values"].as_array().unwrap().len()).sum();
    assert_eq!(values, usize::from(compiled.stat().unwrap().axis_value_count()));
}

#[test]
fn a_cancelled_query_gives_nothing() {
    let token = work::Token::new();
    assert!(token.cancel());
    let found = work::with(token, None, || names_of(&family(), "Slice Sans", &json!({})));
    assert!(matches!(found, Err(EngineError::Cancelled)), "{found:?}");
}

#[test]
fn two_exports_of_one_name_are_marked() {
    let mut s = Session::new();
    let mut font = family();
    let mut twin = font.instances[0].clone();
    twin.exports = false;
    Arc::make_mut(&mut font.instances).push(twin);
    s.add_document(font, None).unwrap();
    let first = run(&mut s, "font.instance.names", json!({"instances": [0]}));
    assert_eq!(first["instances"][0]["conflicts"], json!([]), "an inactive entry takes nothing");
    run(&mut s, "font.instance.setActive", json!({"instances": [4], "active": true}));
    let both = run(&mut s, "font.instance.names", json!({"instances": [0, 4]}));
    let fields = |e: &Value| e["conflicts"].as_array().unwrap().iter().map(|c| c["field"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    for (at, other) in [(0, 4), (1, 0)] {
        let entry = &both["instances"][at];
        assert_eq!(fields(entry), ["fileName", "postscriptName", "styleLinking"]);
        assert_eq!(entry["conflicts"][1]["entries"], json!([other]));
        assert_eq!(entry["conflicts"][1]["value"], "SliceSans-Regular");
    }
}

/// Every static instance of these corpus sources, at most four of each: the query against the compiled tables. Skipped
/// when the corpus is not fetched.
#[test]
fn corpus_instances_have_the_names_of_their_fonts() {
    let root = std::env::var_os("FONTCRAFT_CORPUS").map_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus"), PathBuf::from);
    for source in [
        "fontc/resources/testdata/glyphs3/InstancePostscript.glyphs",
        "fontc/resources/testdata/glyphs3/WghtVar_Avar.glyphs",
        "fontc/resources/testdata/glyphs3/InstanceNames.glyphs",
        "fontc/resources/testdata/glyphs2/BracketTestFontKerning.glyphs",
        "fontc/resources/testdata/glyphs3/InstanceClasses.glyphs",
        "fontc/resources/testdata/glyphs3/StaticWithInstance.glyphs",
    ] {
        let path = root.join(source);
        if !path.exists() {
            eprintln!("skipped: {source} is not in the corpus");
            continue;
        }
        let (font, _) = fontcraft_engine::io::open(&path, None).unwrap();
        let out = names_of(&font, "source", &json!({})).unwrap();
        let statics = out["instances"].as_array().unwrap().iter().filter(|e| e["type"] == "static" && e["error"].is_null());
        for entry in statics.take(4) {
            let index = usize::try_from(entry["index"].as_u64().unwrap()).unwrap();
            assert_same(&font, entry, index, source);
        }
    }
}
