//! The engine's export stage: PreFilter, Filter, Scale to UPM and Export Folder parameters, as
//! File › Export and `font.compile` run them (`engine/src/export_steps.rs`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine as _;
use fontcraft_engine::Session;
use fontcraft_engine::model::plist::Value as PValue;
use fontcraft_engine::model::{
    Alignment, Component, ComponentInfo, ComponentInfoBox, Instance, Node, NodeKind, Param, Params, Path as Contour, Shape,
};
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
/// and changes nothing; so is a built-in filter's string that cannot be read and a Scale to UPM
/// that is not a number.
#[test]
fn extrude_and_unreadable_parameters_are_refused_with_a_warning() {
    let mut s = font();
    add_param(&mut s, false, "Filter", text("Extrude;10;45"));
    add_param(&mut s, false, "Filter", text("RoundCorner; 20; include:A; exclude:O"));
    add_param(&mut s, false, "Scale to UPM", text("big"));
    let (fonts, out) = export(&mut s, json!({"timestamp": 0}));
    let warnings = out["warnings"].to_string();
    assert!(warnings.contains("Extrude") && warnings.contains("RoundCorner") && warnings.contains("Scale to UPM"), "{warnings}");
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
/// warnings) where the session allows it, and an absolute path refused for that instance.
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

    // Outside the desktop app, a font whose Export Folder climbs out of the destination is skipped.
    let out = run(&mut s, "file.exportStatic", json!({"destination": dest, "timestamp": 0}));
    assert_eq!(names(&dir), ["out/Untitled-Plain.ttf", "out/web/fonts/Untitled-Web.ttf"]);
    let skipped = out["skipped"].as_array().unwrap();
    assert_eq!(skipped.len(), 2, "{out}");
    assert_eq!(skipped[0]["instance"], "Regular");
    assert!(skipped[0]["reason"].as_str().unwrap().contains("outside the export destination"), "{out}");

    // `..` back into the destination stays inside it.
    let mut inside = font();
    add_instance(&mut inside, "Back", vec![("Export Folder", text("../out/back"))]);
    let out = run(&mut inside, "file.exportStatic", json!({"destination": dest, "timestamp": 0}));
    assert!(out["skipped"].as_array().unwrap().is_empty(), "{out}");
    assert!(names(&dir).contains(&"out/back/Untitled-Back.ttf".to_string()));

    std::fs::remove_dir_all(&dir).unwrap();
    s.set_export_outside(true);
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
#[ignore = "the corpus font's instance lies between masters and its glyph `A.ss01` has incompatible layers, so the instance cannot be interpolated"]
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

/// Linked sources are read once per export, not once per font built: a missing file is one
/// warning however many instances export.
#[test]
fn linked_sources_are_read_once_per_export() {
    let mut s = font();
    add_instance(&mut s, "Regular", vec![]);
    add_instance(&mut s, "Bold", vec![]);
    add_param(&mut s, false, "Import Font", text("Missing.glyphs"));
    let out = run(&mut s, "font.compile", json!({"timestamp": 0}));
    assert_eq!(out["fonts"].as_array().unwrap().len(), 2, "{out}");
    let missing = out["warnings"].as_array().unwrap().iter().filter_map(Value::as_str).filter(|w| w.contains("Missing.glyphs")).count();
    assert_eq!(missing, 1, "{out}");
}

// ── filter modules named by Filter and PreFilter (M13.4) ──────────────────────────────────────────

fn returning_module(manifest: &str, output: &str) -> Vec<u8> {
    fontcraft_plugins::fixtures::returning(manifest, output)
}

/// A filter module `Square` (named "Test Square", with an integer parameter `dx`) that turns any
/// layer into one square from 0 to 300.
fn square_module() -> Vec<u8> {
    let n = |x: f64, y: f64| Node::new(x, y, NodeKind::Line);
    let square = Shape::Path(Contour::new(true, vec![n(0.0, 0.0), n(300.0, 0.0), n(300.0, 300.0), n(0.0, 300.0)]));
    let output = json!({"shapes": [square], "log": ["squared"]});
    returning_module(&fontcraft_plugins::wat::manifest("Square", "filter", r#"{"dx":{"type":"int"}}"#), &output.to_string())
}

/// [`font`] with the `Square` module installed in the session.
fn font_with_square() -> Session {
    let mut s = font();
    s.extensions_mut().registry.install_bytes(&square_module(), None).unwrap();
    s
}

fn export_error(s: &mut Session) -> String {
    match s.execute("file.exportStatic", &json!({"timestamp": 0})) {
        Ok(out) => panic!("the export should be refused: {out}"),
        Err(e) => e.to_string(),
    }
}

fn span(bytes: &[u8], c: char) -> (f32, f32) {
    let p = probe(bytes, c);
    (p.x_min, p.x_max)
}

/// `filter.customParameter.thirdParty`, `filter.menu.pluginFiltersListed`: a Filter string names
/// an installed filter module by id or by name, passes it its parameters, and runs it only on the
/// glyphs `include:` names; a PreFilter runs it before decomposition, so a composite shows its
/// component's new outline.
#[test]
fn filter_custom_parameter_third_party_a_module_filter_runs_only_on_included_glyphs() {
    let mut s = font_with_square();
    add_param(&mut s, false, "Filter", text("Square; dx:10; include:A,B"));
    let (fonts, out) = export(&mut s, json!({"timestamp": 0}));
    let bytes = &fonts[0].1;
    assert!(!out["warnings"].to_string().contains("Square"), "{out}");
    assert_eq!(span(bytes, 'A'), (0.0, 300.0));
    assert_eq!(span(bytes, 'B'), (0.0, 300.0), "B is decomposed first, then filtered");
    assert_eq!(span(bytes, 'O'), (50.0, 550.0), "O is not included");
    let a = &s.font().unwrap().glyph("A").unwrap().layers[0];
    assert!(matches!(&a.shapes[0], Shape::Path(p) if p.nodes[0].x == 100.0), "the source keeps its outline");

    // By the module's name, ASCII case ignored, as a PreFilter on A alone.
    let mut s = font_with_square();
    add_param(&mut s, false, "PreFilter", text("test square; include:A"));
    let (fonts, _) = export(&mut s, json!({"timestamp": 0}));
    let bytes = &fonts[0].1;
    assert_eq!(span(bytes, 'A'), (0.0, 300.0));
    assert_eq!(span(bytes, 'O'), (50.0, 550.0));
    assert!(is_composite(bytes, 'B'), "a PreFilter keeps the components");
    assert_eq!(span(bytes, 'B'), (0.0, 300.0), "B shows its component's new outline");
}

/// A name that is neither a built-in filter nor an installed module, a module that is turned off,
/// and modules in safe mode each refuse the export with a message naming them.
#[test]
fn filter_custom_parameter_third_party_an_unknown_or_unavailable_module_refuses_the_export() {
    let mut s = font();
    add_param(&mut s, false, "Filter", text("NoSuchFilter; 1"));
    let e = export_error(&mut s);
    assert!(e.contains("NoSuchFilter") && e.contains("neither a built-in filter nor an installed filter module"), "{e}");

    let mut s = font_with_square();
    add_param(&mut s, false, "Filter", text("Square"));
    s.extensions_mut().registry.set_enabled("Square", false).unwrap();
    let e = export_error(&mut s);
    assert!(e.contains("Square") && e.contains("turned off"), "{e}");

    s.extensions_mut().registry.set_enabled("Square", true).unwrap();
    s.extensions_mut().safe_mode = true;
    let e = export_error(&mut s);
    assert!(e.contains("Square") && e.contains("safe mode"), "{e}");
    s.extensions_mut().safe_mode = false;
    let (fonts, _) = export(&mut s, json!({"timestamp": 0}));
    assert_eq!(span(&fonts[0].1, 'A'), (0.0, 300.0));
}

/// Parameters a module cannot take, and a module that fails, are warnings, and that string changes
/// nothing.
#[test]
fn filter_custom_parameter_third_party_bad_parameters_and_a_failing_module_are_warnings() {
    let mut s = font_with_square();
    let failing = returning_module(&fontcraft_plugins::wat::manifest("Broken", "filter", "{}"), r#"{"error":"cannot do this"}"#);
    s.extensions_mut().registry.install_bytes(&failing, None).unwrap();
    add_param(&mut s, false, "Filter", text("Square; size:3"));
    add_param(&mut s, false, "Filter", text("Broken; include:O"));
    let (fonts, out) = export(&mut s, json!({"timestamp": 0}));
    let warnings = out["warnings"].to_string();
    assert!(warnings.contains("no parameter `size`"), "{warnings}");
    assert!(warnings.contains("Broken") && warnings.contains("cannot do this") && warnings.contains("`O`"), "{warnings}");
    assert_eq!(span(&fonts[0].1, 'A'), (100.0, 500.0));
    assert_eq!(span(&fonts[0].1, 'O'), (50.0, 550.0));
}

/// The same module run from the Filter menu (`plugin.runFilter`) gives the same outline as at
/// export.
#[test]
fn filter_menu_plugin_filters_listed_a_module_gives_the_same_outline_from_the_menu_and_at_export() {
    let mut menu = font_with_square();
    run(&mut menu, "plugin.runFilter", json!({"id": "Square", "params": {"dx": 10}, "glyphs": ["A"]}));
    let (from_menu, _) = export(&mut menu, json!({"timestamp": 0}));

    let mut at_export = font_with_square();
    add_param(&mut at_export, false, "Filter", text("Square; dx:10; include:A"));
    let (exported, _) = export(&mut at_export, json!({"timestamp": 0}));

    let (a, b) = (probe(&from_menu[0].1, 'A'), probe(&exported[0].1, 'A'));
    assert_eq!((a.x_min, a.x_max, a.points, a.curves), (b.x_min, b.x_max, b.points, b.curves));
    assert_eq!(a.x_max, 300.0);
}
