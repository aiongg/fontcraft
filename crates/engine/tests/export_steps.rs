//! The engine's export stage: PreFilter, Filter, Scale to UPM and Export Folder parameters, as
//! File › Export and `font.compile` run them (`engine/src/export_steps.rs`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine as _;
use fontcraft_engine::Session;
use fontcraft_engine::model::plist::Value as PValue;
use fontcraft_engine::model::{Alignment, Component, ComponentInfo, ComponentInfoBox, Instance, Node, NodeKind, Param, Params, Shape};
use serde_json::{Value, json};
use skrifa::MetadataProvider;
use skrifa::raw::TableProvider;
use skrifa::raw::tables::glyf::Glyph as GlyfGlyph;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn text(value: &str) -> PValue {
    PValue::String(value.into())
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-export-steps-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A new font: `A` a rectangle from x 100 to 500, `O` one from 50 to 550, `B` a composite of `A`
/// placed as it is.
fn font() -> Session {
    let mut s = Session::with_document();
    let rect = |x0: f64, x1: f64| json!([{"x": x0, "y": 0}, {"x": x1, "y": 0}, {"x": x1, "y": 700}, {"x": x0, "y": 700}]);
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(100.0, 500.0)}));
    run(&mut s, "path.addPath", json!({"glyph": "O", "nodes": rect(50.0, 550.0)}));
    s.edit("composite", |font| {
        let mut c = Component::new("A", 0.0, 0.0);
        c.info = ComponentInfoBox::new(ComponentInfo { alignment: Alignment::Disabled, ..Default::default() });
        font.glyph_mut("B")?.layers[0].shapes.push(Shape::Component(c));
        Ok(())
    })
    .unwrap();
    s
}

/// Adds a static instance at the only master with these parameters.
fn add_instance(s: &mut Session, name: &str, params: Vec<(&str, PValue)>) {
    s.edit("instance", |font| {
        let params = Params(params.into_iter().map(|(k, v)| Param::new(k, v)).collect());
        Arc::make_mut(&mut font.instances).push(Instance { name: name.into(), params, ..Default::default() });
        Ok(())
    })
    .unwrap();
}

/// Adds a parameter to the font (`master: false`) or to its master.
fn add_param(s: &mut Session, master: bool, name: &str, value: PValue) {
    s.edit("parameter", |font| {
        let param = Param::new(name, value);
        if master {
            Arc::make_mut(&mut font.masters)[0].params.0.push(param);
        } else {
            Arc::make_mut(&mut font.info).params.0.push(param);
        }
        Ok(())
    })
    .unwrap();
}

/// The fonts of an in-memory `file.exportStatic`, by instance name, and the result.
fn export(s: &mut Session, params: Value) -> (Vec<(String, Vec<u8>)>, Value) {
    let out = run(s, "file.exportStatic", params);
    let fonts = out["fonts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            let data = base64::engine::general_purpose::STANDARD.decode(f["files"][0]["dataBase64"].as_str().unwrap()).unwrap();
            (f["instance"].as_str().unwrap().to_string(), data)
        })
        .collect();
    (fonts, out)
}

/// What a glyph's drawing holds: the smallest and largest x, and how many cubic curves.
#[derive(Default)]
struct Probe {
    x_min: f32,
    x_max: f32,
    curves: usize,
    points: usize,
}

impl Probe {
    fn at(&mut self, x: f32) {
        if self.points == 0 || x < self.x_min {
            self.x_min = x;
        }
        if self.points == 0 || x > self.x_max {
            self.x_max = x;
        }
        self.points += 1;
    }
}

impl skrifa::outline::OutlinePen for Probe {
    fn move_to(&mut self, x: f32, _: f32) {
        self.at(x);
    }
    fn line_to(&mut self, x: f32, _: f32) {
        self.at(x);
    }
    fn quad_to(&mut self, _: f32, _: f32, x: f32, _: f32) {
        self.at(x);
    }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, x: f32, _: f32) {
        self.curves += 1;
        self.at(x);
    }
    fn close(&mut self) {}
}

fn probe(bytes: &[u8], c: char) -> Probe {
    let font = skrifa::FontRef::new(bytes).unwrap();
    let glyph = font.outline_glyphs().get(font.charmap().map(c).unwrap()).unwrap();
    let mut pen = Probe::default();
    let settings = skrifa::outline::DrawSettings::unhinted(skrifa::instance::Size::unscaled(), skrifa::instance::LocationRef::default());
    glyph.draw(settings, &mut pen).unwrap();
    pen
}

/// Whether glyph `c` of a TrueType font is stored as a composite.
fn is_composite(bytes: &[u8], c: char) -> bool {
    let font = skrifa::FontRef::new(bytes).unwrap();
    let (loca, glyf) = (font.loca(None).unwrap(), font.glyf().unwrap());
    matches!(loca.get_glyf(font.charmap().map(c).unwrap(), &glyf).unwrap(), Some(GlyfGlyph::Composite(_)))
}

fn instance<'a>(fonts: &'a [(String, Vec<u8>)], name: &str) -> &'a [u8] {
    &fonts.iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("no font for {name}")).1
}

/// `filter.customParameter.preFilter`: a PreFilter moves the master's points while composites stay
/// composites.
#[test]
fn a_prefilter_moves_points_before_decomposition() {
    let mut s = font();
    add_param(&mut s, false, "PreFilter", text("Transformations;OffsetX:100;include:A"));
    let (fonts, out) = export(&mut s, json!({"timestamp": 0}));
    assert_eq!(out["warnings"], json!([]), "{out}");
    let bytes = &fonts[0].1;
    assert_eq!(probe(bytes, 'A').x_min, 200.0, "A moved by the PreFilter");
    assert_eq!(probe(bytes, 'O').x_min, 50.0, "O is not included");
    assert!(is_composite(bytes, 'B'), "PreFilter runs with components in place");
    // The document itself is untouched.
    let a = s.font().unwrap().glyph("A").unwrap().layers[0].clone();
    assert!(matches!(&a.shapes[0], Shape::Path(p) if p.nodes[0].x == 100.0));
}

/// `filter.customParameter.transformations`, `.includeExclude`, `.syntax`: an instance's Filter
/// strings run in order, each on the glyphs it includes.
#[test]
fn transformations_filters_honour_include_and_exclude() {
    let mut s = font();
    add_instance(
        &mut s,
        "Regular",
        vec![("Filter", text("Transformations; OffsetX: 50; include: A")), ("Filter", text("Transformations;OffsetX:7;exclude:A,B"))],
    );
    let (fonts, out) = export(&mut s, json!({"timestamp": 0}));
    assert_eq!(out["warnings"], json!([]), "{out}");
    let bytes = instance(&fonts, "Regular");
    assert_eq!(probe(bytes, 'A').x_min, 150.0);
    assert_eq!(probe(bytes, 'O').x_min, 57.0);
    // B was decomposed before the filters ran and is in neither filter.
    assert_eq!(probe(bytes, 'B').x_min, 100.0);
}

/// `Filter` on a master: every export of that master gets it, after the font's own.
#[test]
fn a_filter_on_a_master_reaches_every_export_of_it() {
    let mut s = font();
    add_param(&mut s, false, "Filter", text("Transformations;OffsetX:1;include:A"));
    add_param(&mut s, true, "Filter", text("Transformations;OffsetX:30;include:A"));
    add_instance(&mut s, "Regular", vec![]);
    add_instance(&mut s, "Text", vec![("Filter", text("Transformations;OffsetX:5;include:A"))]);
    let (fonts, _) = export(&mut s, json!({"timestamp": 0}));
    assert_eq!(probe(instance(&fonts, "Regular"), 'A').x_min, 131.0);
    assert_eq!(probe(instance(&fonts, "Text"), 'A').x_min, 136.0);
}

/// `filter.customParameter.addExtremes`, `extremes.addedAtExportByFilter`,
/// `path.addExtremes.parameterName`: AddExtremes adds the extreme points in the exported font only.
#[test]
fn add_extremes_at_export() {
    let mut s = font();
    s.edit("arch", |font| {
        let n = |x: f64, y: f64, kind: NodeKind| Node::new(x, y, kind);
        let arch = fontcraft_engine::model::Path::new(
            true,
            vec![n(0.0, 0.0, NodeKind::Line), n(0.0, 400.0, NodeKind::OffCurve), n(400.0, 400.0, NodeKind::OffCurve), n(400.0, 0.0, NodeKind::Curve)],
        );
        font.glyph_mut("o")?.layers[0].shapes.push(Shape::Path(arch));
        Ok(())
    })
    .unwrap();
    let curves = |s: &mut Session| {
        let (fonts, out) = export(s, json!({"timestamp": 0, "outlines": "cff", "removeOverlap": false}));
        assert_eq!(out["warnings"], json!([]), "{out}");
        probe(&fonts[0].1, 'o').curves
    };
    assert_eq!(curves(&mut s), 1);
    add_param(&mut s, false, "Filter", text("AddExtremes"));
    assert_eq!(curves(&mut s), 2, "the curve is split at its top");
    let o = &s.font().unwrap().glyph("o").unwrap().layers[0];
    assert!(matches!(&o.shapes[0], Shape::Path(p) if p.nodes.len() == 4), "the source keeps its nodes");
}

/// `filter.customParameter.extrudeUndocumented`: an Extrude Filter is not used, with a warning,
/// and changes nothing; so is an unknown filter and a Scale to UPM that is not a number.
#[test]
fn extrude_and_unreadable_parameters_are_refused_with_a_warning() {
    let mut s = font();
    add_param(&mut s, false, "Filter", text("Extrude;10;45"));
    add_param(&mut s, false, "Filter", text("NoSuchFilter;1"));
    add_param(&mut s, false, "Scale to UPM", text("big"));
    let (fonts, out) = export(&mut s, json!({"timestamp": 0}));
    let warnings = out["warnings"].to_string();
    assert!(warnings.contains("Extrude") && warnings.contains("NoSuchFilter") && warnings.contains("Scale to UPM"), "{warnings}");
    let bytes = &fonts[0].1;
    assert_eq!(probe(bytes, 'A').x_min, 100.0);
    assert!(is_composite(bytes, 'B'), "no usable Filter, so nothing is decomposed");
}

/// `Filter`, `filter.customParameter.filter`: a Filter decomposes every composite first, so a
/// TrueType export has none, and the component's outline is filtered once.
#[test]
fn a_filter_decomposes_composites_in_a_truetype_export() {
    let mut s = font();
    s.edit("lock", |font| {
        font.glyph_mut("A")?.info.locked = true;
        Ok(())
    })
    .unwrap();
    add_instance(&mut s, "Regular", vec![("Filter", text("Transformations;OffsetX:10"))]);
    let (fonts, out) = export(&mut s, json!({"timestamp": 0}));
    assert_eq!(out["warnings"], json!([]), "{out}");
    let bytes = instance(&fonts, "Regular");
    assert!(!is_composite(bytes, 'B'));
    assert_eq!(probe(bytes, 'A').x_min, 110.0, "a locked glyph is filtered at export");
    assert_eq!(probe(bytes, 'B').x_min, 110.0, "B is A's outline moved once");
    // The same build through the compile command.
    let built = run(&mut s, "font.compile", json!({"timestamp": 0}));
    assert_eq!(built["fonts"].as_array().unwrap().len(), 1);
}

fn names(dir: &Path) -> Vec<String> {
    fn walk(dir: &Path) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
        entries.flatten().flat_map(|e| if e.path().is_dir() { walk(&e.path()) } else { vec![e.path()] }).collect()
    }
    let mut out: Vec<String> = walk(dir).iter().map(|p| p.strip_prefix(dir).unwrap().to_string_lossy().replace('\\', "/")).collect();
    out.sort();
    out
}

/// `export.instanceExportFolder`: nested folders, `..` out of the destination (named in the
/// warnings), and an absolute path refused for that instance.
#[test]
fn export_folder_places_each_instance_and_refuses_an_absolute_path() {
    let dir = temp("folder");
    let dest = dir.join("out");
    let mut s = font();
    add_instance(&mut s, "Regular", vec![("Export Folder", text("../Trial Versions"))]);
    add_instance(&mut s, "Web", vec![("Export Folder", text("web/fonts/"))]);
    add_instance(&mut s, "Plain", vec![]);
    add_instance(&mut s, "Bad", vec![("Export Folder", text("/tmp/elsewhere"))]);

    let plan = run(&mut s, "font.compilePlan", json!({"destination": dest, "sections": ["outputs"]}));
    let planned: Vec<&str> = plan["outputs"].as_array().unwrap().iter().map(|o| o["files"][0]["name"].as_str().unwrap()).collect();
    assert_eq!(planned, ["../Trial Versions/Untitled-Regular.ttf", "web/fonts/Untitled-Web.ttf", "Untitled-Plain.ttf"]);

    let out = run(&mut s, "file.exportStatic", json!({"destination": dest, "timestamp": 0}));
    assert_eq!(names(&dir), ["Trial Versions/Untitled-Regular.ttf", "out/Untitled-Plain.ttf", "out/web/fonts/Untitled-Web.ttf"]);
    let skipped = out["skipped"].as_array().unwrap();
    assert_eq!(skipped.len(), 1, "{out}");
    assert_eq!(skipped[0]["instance"], "Bad");
    assert!(skipped[0]["reason"].as_str().unwrap().contains("absolute"), "{out}");
    let outside: Vec<&str> =
        out["warnings"].as_array().unwrap().iter().filter_map(Value::as_str).filter(|w| w.contains("outside the export destination")).collect();
    assert_eq!(outside.len(), 1, "{out}");
    assert!(outside[0].contains("Untitled-Regular.ttf"), "{out}");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The corpus source with a HatchOutlineFilter Filter parameter exports (skipped without the
/// corpus).
#[test]
#[ignore = "M9: the corpus font's instance lies between masters, and interpolated instances are not built yet"]
fn the_corpus_source_with_a_filter_parameter_exports() {
    let corpus = std::env::var_os("FONTCRAFT_CORPUS").map_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus"), PathBuf::from);
    let path = corpus.join("glyphssdk/GlyphsFileFormat/GlyphsFileFormatv3.glyphs");
    if !path.exists() {
        return;
    }
    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": path }));
    let out = run(&mut s, "font.compile", json!({"timestamp": 0}));
    let warnings = out["warnings"].to_string();
    assert!(!warnings.contains("HatchOutlineFilter"), "{warnings}");
}
