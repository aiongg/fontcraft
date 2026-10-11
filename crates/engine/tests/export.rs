//! Exporting and compiling static fonts through the commands: flavours, containers, instances,
//! file names, the compile plan, and failures that leave the disk as it was.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use base64::Engine as _;
use fontcraft_engine::Session;
use fontcraft_engine::container::{self, sfnt};
use serde_json::{Value, json};
use skrifa::MetadataProvider;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-export-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Value {
    json!([{"x": x0, "y": y0}, {"x": x1, "y": y0}, {"x": x1, "y": y1}, {"x": x0, "y": y1}])
}

/// A new font with outlines in `A` and `O`.
fn drawn() -> Session {
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(60.0, 0.0, 560.0, 700.0)}));
    run(&mut s, "path.addPath", json!({"glyph": "O", "nodes": rect(40.0, -10.0, 600.0, 710.0)}));
    s
}

/// Two masters (Regular 400, Bold 700) and four instances: Regular and Bold at the masters
/// (Bold with a fileName parameter), Medium between them, Hidden at Bold but not exported.
const TWO_MASTERS: &str = r#"{
.appVersion = "3343";
.formatVersion = 3;
axes = (
{
name = Weight;
tag = wght;
}
);
familyName = "Test Sans";
fontMaster = (
{
axesValues = (
400
);
id = m01;
name = Regular;
},
{
axesValues = (
700
);
id = m02;
name = Bold;
}
);
glyphs = (
{
glyphname = A;
layers = (
{
layerId = m01;
shapes = (
{
closed = 1;
nodes = (
(100,0,l),
(500,0,l),
(500,700,l),
(100,700,l)
);
}
);
width = 600;
},
{
layerId = m02;
shapes = (
{
closed = 1;
nodes = (
(80,0,l),
(620,0,l),
(620,700,l),
(80,700,l)
);
}
);
width = 700;
}
);
unicode = 65;
}
);
instances = (
{
axesValues = (
400
);
name = Regular;
},
{
axesValues = (
550
);
name = Medium;
},
{
axesValues = (
700
);
customParameters = (
{
name = fileName;
value = "TestSans-Heavy";
}
);
name = Bold;
},
{
axesValues = (
700
);
exports = 0;
name = Hidden;
}
);
unitsPerEm = 1000;
versionMajor = 1;
versionMinor = 0;
}
"#;

fn two_masters(dir: &Path, text: &str) -> Session {
    let path = dir.join("TestSans.glyphs");
    std::fs::write(&path, text).unwrap();
    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": path }));
    s
}

/// The tables of an sfnt, by tag, with `head.checkSumAdjustment` zeroed (it depends on where
/// the container puts each table) and the `head` flag WOFF2 sets (bit 11) cleared.
fn tables(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let font = sfnt::parse(bytes).unwrap();
    let mut out: Vec<(String, Vec<u8>)> = font
        .tables
        .iter()
        .map(|t| {
            let tag = sfnt::tag_str(t.tag);
            let mut data = t.data.to_vec();
            if tag == "head" {
                data.splice(8..12, [0; 4]);
                if let Some(flags) = data.get_mut(16) {
                    *flags &= !0x08;
                }
            }
            (tag, data)
        })
        .collect();
    out.sort();
    out
}

fn names(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = walk(dir).iter().map(|p| p.strip_prefix(dir).unwrap().to_string_lossy().replace('\\', "/")).collect();
    out.sort();
    out
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries.flatten().flat_map(|e| if e.path().is_dir() { walk(&e.path()) } else { vec![e.path()] }).collect()
}

#[test]
fn cff_export_in_every_container_decodes_to_the_same_font() {
    let dir = temp("cff");
    let mut s = drawn();
    let out = run(
        &mut s,
        "file.exportStatic",
        json!({"outlines": "cff", "containers": ["plain", "woff", "woff2"], "destination": dir, "timestamp": 0, "removeOverlap": false}),
    );
    assert_eq!(names(&dir), ["Untitled-Regular.otf", "Untitled-Regular.woff", "Untitled-Regular.woff2"]);
    assert_eq!(out["warnings"], json!([]), "no overlap warning when removal was declined");
    let otf = std::fs::read(dir.join("Untitled-Regular.otf")).unwrap();
    assert_eq!(container::sniff(&otf), Some(container::Kind::Cff));
    let plain = tables(&otf);
    assert!(plain.iter().any(|(tag, _)| tag == "CFF "), "CFF outlines");
    for (file, kind) in [("Untitled-Regular.woff", container::Kind::Woff), ("Untitled-Regular.woff2", container::Kind::Woff2)] {
        let bytes = std::fs::read(dir.join(file)).unwrap();
        assert_eq!(container::sniff(&bytes), Some(kind));
        let sfnt = container::to_sfnt(&bytes).unwrap();
        assert_eq!(tables(&sfnt), plain, "{file} holds the plain font");
    }
    let font = skrifa::FontRef::new(&otf).unwrap();
    assert!(font.charmap().map('A').is_some());
    // The result lists each file with its container and size.
    let files = out["fonts"][0]["files"].as_array().unwrap();
    assert_eq!(files.iter().map(|f| f["container"].as_str().unwrap()).collect::<Vec<_>>(), ["plain", "woff", "woff2"]);
    assert_eq!(files[0]["bytes"].as_u64().unwrap() as usize, otf.len());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn truetype_woff2_export_decodes_to_the_plain_font() {
    let mut s = drawn();
    let plain = run(&mut s, "file.exportStatic", json!({"timestamp": 0}));
    let woff2 = run(&mut s, "file.exportStatic", json!({"containers": ["woff2"], "timestamp": 0}));
    let decode = |v: &Value| base64::engine::general_purpose::STANDARD.decode(v["dataBase64"].as_str().unwrap()).unwrap();
    let woff2 = decode(&woff2);
    assert_eq!(container::sniff(&woff2), Some(container::Kind::Woff2));
    let sfnt = container::to_sfnt(&woff2).unwrap();
    let font = skrifa::FontRef::new(&sfnt).unwrap();
    assert!(font.charmap().map('A').is_some());
    assert_eq!(sfnt::parse(&sfnt).unwrap().tables.len(), sfnt::parse(&decode(&plain)).unwrap().tables.len());
}

#[test]
fn export_all_builds_instances_at_masters_and_reports_the_rest() {
    let dir = temp("all");
    let mut s = two_masters(&dir, TWO_MASTERS);
    run(&mut s, "file.new", json!({"familyName": "Second"}));
    let out_dir = dir.join("out");
    let out = run(
        &mut s,
        "file.exportAll",
        json!({"outlines": ["truetype", "cff"], "containers": ["plain", "woff"], "destination": out_dir, "timestamp": 0}),
    );
    assert_eq!(
        names(&out_dir),
        [
            "otf/Second-Regular.otf",
            "otf/Second-Regular.woff",
            "otf/TestSans-Heavy.otf",
            "otf/TestSans-Heavy.woff",
            "otf/TestSans-Regular.otf",
            "otf/TestSans-Regular.woff",
            "ttf/Second-Regular.ttf",
            "ttf/Second-Regular.woff",
            "ttf/TestSans-Heavy.ttf",
            "ttf/TestSans-Heavy.woff",
            "ttf/TestSans-Regular.ttf",
            "ttf/TestSans-Regular.woff",
        ]
    );
    assert_eq!(out["written"], 12);
    let test_sans = &out["documents"][0];
    assert_eq!(test_sans["title"], "TestSans");
    let skipped = test_sans["skipped"].as_array().unwrap();
    assert_eq!(skipped.len(), 1, "{skipped:?}");
    assert_eq!(skipped[0]["instance"], "Medium");
    assert!(skipped[0]["reason"].as_str().unwrap().contains("between masters"));
    assert_eq!(out["documents"][1]["fonts"][0]["source"], "default", "a font without instances exports its first master");
    assert!(!out["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("overlap")), "overlaps are removed");

    // Each instance draws its master: the Bold instance has the Bold advance.
    let bold = std::fs::read(out_dir.join("ttf/TestSans-Heavy.ttf")).unwrap();
    let font = skrifa::FontRef::new(&bold).unwrap();
    let a = font.charmap().map('A').unwrap();
    let metrics = font.glyph_metrics(skrifa::instance::Size::unscaled(), skrifa::instance::LocationRef::default());
    assert_eq!(metrics.advance_width(a), Some(700.0));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn compile_plan_describes_what_compile_builds_without_writing() {
    let dir = temp("plan");
    let mut s = two_masters(&dir, TWO_MASTERS);
    let out_dir = dir.join("out");
    let request = json!({"formats": ["ttf", "otf"], "containers": ["plain"], "mastersAsInstances": true, "destination": out_dir});
    let plan = run(&mut s, "font.compilePlan", request.clone());
    assert!(!out_dir.exists(), "a plan writes nothing");
    let built = run(&mut s, "font.compile", request);

    let outputs = plan["outputs"].as_array().unwrap();
    let fonts = built["fonts"].as_array().unwrap();
    // Regular and Bold instances, then the two masters, each in two formats.
    assert_eq!(outputs.len(), 8);
    assert_eq!(fonts.len(), 8);
    for (planned, font) in outputs.iter().zip(fonts) {
        assert_eq!((&planned["instance"], &planned["format"]), (&font["instance"], &font["format"]));
        assert_eq!(planned["glyphs"], font["glyphs"]);
        let path = font["files"][0]["path"].as_str().unwrap();
        assert_eq!(planned["files"][0]["path"].as_str().unwrap(), path);
        let written: Vec<String> = tables(&std::fs::read(path).unwrap()).into_iter().map(|(tag, _)| tag).collect();
        assert_eq!(planned["tables"], json!(written), "{path}");
    }
    assert_eq!(outputs[0]["postscriptName"], "TestSans-Regular");
    assert_eq!(outputs[0]["removeOverlap"], true);
    assert_eq!(plan["glyphOrder"], json!([".notdef", "A"]));
    assert_eq!(plan["skipped"][0]["instance"], "Medium");
    let instances = plan["instances"].as_array().unwrap();
    assert_eq!(instances.iter().map(|i| i["master"].as_str().unwrap()).collect::<Vec<_>>(), ["m01", "m02", "m01", "m02"]);

    // In memory, font.compile only describes what it built.
    let summary = run(&mut s, "font.compile", json!({"instances": ["Bold"], "formats": ["otf"]}));
    assert_eq!(summary["fonts"][0]["files"][0]["name"], "TestSans-Heavy.otf");
    assert!(summary["fonts"][0]["files"][0].get("dataBase64").is_none());
    // Formats that are not built yet are refused by name.
    let e = s.execute("font.compile", &json!({"formats": ["variable"]})).unwrap_err().to_string();
    assert!(e.contains("variable") && e.contains("M9"), "{e}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_failing_export_changes_nothing_and_writes_nothing() {
    let dir = temp("fail");
    // Bold's `A` is wider than a TrueType outline can be; Regular builds first and fails nothing.
    let text = TWO_MASTERS.replacen("(620,0,l),\n(620,700,l)", "(20620,0,l),\n(20620,700,l)", 1);
    assert_ne!(text, TWO_MASTERS);
    let mut s = two_masters(&dir, &text);
    let out_dir = dir.join("out");
    std::fs::create_dir_all(&out_dir).unwrap();
    std::fs::write(out_dir.join("TestSans-Regular.ttf"), b"old").unwrap();
    let before = fontcraft_testkit::snapshot(&s);

    for destination in [out_dir.clone(), out_dir.join("new/deeper")] {
        let e = s.execute("file.exportStatic", &json!({"destination": destination, "containers": ["plain", "woff"]})).unwrap_err().to_string();
        assert!(e.contains("glyph `A`"), "{e}");
        let e = s.execute("file.exportAll", &json!({"destination": destination})).unwrap_err().to_string();
        assert!(e.contains("glyph `A`"), "{e}");
        let e = s.execute("font.compile", &json!({"destination": destination})).unwrap_err().to_string();
        assert!(e.contains("glyph `A`"), "{e}");
    }
    assert_eq!(names(&out_dir), ["TestSans-Regular.ttf"], "no new files, no temporary files, no new folders");
    assert_eq!(std::fs::read(out_dir.join("TestSans-Regular.ttf")).unwrap(), b"old");
    assert_eq!(fontcraft_testkit::snapshot(&s), before);
    assert!(!out_dir.join("new").exists());

    // Bad parameters name the parameter and write nothing either.
    for (params, key) in [
        (json!({"outlines": "type1", "destination": out_dir}), "outlines"),
        (json!({"containers": ["zip"], "destination": out_dir}), "containers"),
        (json!({"destination": out_dir.join("x.ttf"), "outlines": "cff"}), "destination"),
        (json!({"instances": ["Nope"], "destination": out_dir}), "Nope"),
    ] {
        let e = s.execute("file.exportStatic", &params).unwrap_err().to_string();
        assert!(e.contains(key), "{params}: {e}");
    }
    assert_eq!(names(&out_dir), ["TestSans-Regular.ttf"]);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn one_file_destination_takes_its_format_from_the_extension() {
    let dir = temp("single");
    let mut s = two_masters(&dir, TWO_MASTERS);
    let path = dir.join("one.woff");
    let out = run(&mut s, "file.exportStatic", json!({"destination": path, "timestamp": 0}));
    assert_eq!(out["path"].as_str().unwrap(), path.to_string_lossy());
    assert_eq!(container::sniff(&std::fs::read(&path).unwrap()), Some(container::Kind::Woff));
    assert_eq!(out["fonts"][0]["instance"], "Regular");
    let skipped: Vec<&str> = out["skipped"].as_array().unwrap().iter().map(|v| v["instance"].as_str().unwrap()).collect();
    assert_eq!(skipped, ["Medium", "Bold"]);
    // A named instance is built even when it does not export.
    let otf = dir.join("hidden.otf");
    run(&mut s, "file.exportStatic", json!({"destination": otf, "instances": ["Hidden"]}));
    assert_eq!(container::sniff(&std::fs::read(&otf).unwrap()), Some(container::Kind::Cff));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_export_that_cannot_place_every_file_leaves_the_folder_as_it_was() {
    let dir = temp("blocked");
    let mut s = two_masters(&dir, TWO_MASTERS);
    let out_dir = dir.join("out");
    std::fs::create_dir_all(out_dir.join("TestSans-Heavy.ttf")).unwrap();
    std::fs::write(out_dir.join("TestSans-Heavy.ttf/keep.txt"), b"kept").unwrap();
    std::fs::write(out_dir.join("TestSans-Regular.ttf"), b"old").unwrap();
    let e = s.execute("file.exportStatic", &json!({"destination": out_dir})).unwrap_err().to_string();
    assert!(e.contains("TestSans-Heavy.ttf") && e.contains("folder"), "{e}");
    assert_eq!(names(&out_dir), ["TestSans-Heavy.ttf/keep.txt", "TestSans-Regular.ttf"]);
    assert_eq!(std::fs::read(out_dir.join("TestSans-Regular.ttf")).unwrap(), b"old");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn file_names_that_differ_only_in_case_collide() {
    let dir = temp("case");
    let regular = "400\n);\nname = Regular;";
    assert!(TWO_MASTERS.contains(regular));
    let text =
        TWO_MASTERS.replacen(regular, "400\n);\ncustomParameters = (\n{\nname = fileName;\nvalue = \"testsans-heavy\";\n}\n);\nname = Regular;", 1);
    let mut s = two_masters(&dir, &text);
    let out_dir = dir.join("out");
    let out = run(&mut s, "file.exportStatic", json!({"destination": out_dir, "timestamp": 0}));
    assert_eq!(names(&out_dir), ["TestSans-Heavy.ttf"]);
    assert!(out["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("two fonts are written")), "{out}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_font_without_a_family_name_is_named_by_its_source_file() {
    let dir = temp("nofamily");
    let text = TWO_MASTERS.replacen("familyName = \"Test Sans\";\n", "", 1);
    assert_ne!(text, TWO_MASTERS);
    let path = dir.join("Draft Sans.glyphs");
    std::fs::write(&path, text).unwrap();
    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": path }));
    let out_dir = dir.join("out");
    run(&mut s, "file.exportStatic", json!({"destination": out_dir, "timestamp": 0}));
    assert_eq!(names(&out_dir), ["DraftSans-Regular.ttf", "TestSans-Heavy.ttf"]);
    let plan = run(&mut s, "font.compilePlan", json!({"destination": out_dir, "sections": ["outputs"]}));
    assert!(plan.to_string().contains("DraftSans-Regular.ttf"), "{plan}");
    // Only the file name: the name table keeps the source's empty family name.
    let bytes = std::fs::read(out_dir.join("DraftSans-Regular.ttf")).unwrap();
    let font = skrifa::FontRef::new(&bytes).unwrap();
    let ps = font.localized_strings(skrifa::string::StringId::POSTSCRIPT_NAME).english_or_first().unwrap().to_string();
    assert_eq!(ps, "-Regular");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// How many contours glyph `name` of a compiled font has.
fn contours(bytes: &[u8], name: char) -> usize {
    struct Count(usize);
    impl skrifa::outline::OutlinePen for Count {
        fn move_to(&mut self, _: f32, _: f32) {
            self.0 += 1;
        }
        fn line_to(&mut self, _: f32, _: f32) {}
        fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
        fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
        fn close(&mut self) {}
    }
    let font = skrifa::FontRef::new(bytes).unwrap();
    let glyph = font.outline_glyphs().get(font.charmap().map(name).unwrap()).unwrap();
    let mut pen = Count(0);
    let settings = skrifa::outline::DrawSettings::unhinted(skrifa::instance::Size::unscaled(), skrifa::instance::LocationRef::default());
    glyph.draw(settings, &mut pen).unwrap();
    pen.0
}

#[test]
fn export_removes_overlaps_unless_asked_to_keep_them() {
    let mut s = drawn();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(300.0, 300.0, 700.0, 900.0)}));
    for outlines in ["truetype", "cff"] {
        let font = |s: &mut Session, p: Value| {
            let out = run(s, "file.exportStatic", p);
            assert!(!out["warnings"].to_string().contains("overlap"), "{out}");
            base64::engine::general_purpose::STANDARD.decode(out["dataBase64"].as_str().unwrap()).unwrap()
        };
        let merged = font(&mut s, json!({"outlines": outlines, "timestamp": 0}));
        assert_eq!(contours(&merged, 'A'), 1, "{outlines}");
        let kept = font(&mut s, json!({"outlines": outlines, "timestamp": 0, "removeOverlap": false}));
        assert_eq!(contours(&kept, 'A'), 2, "{outlines}");
    }
    let plan = run(&mut s, "font.compilePlan", json!({"outlines": {"removeOverlaps": false}}));
    assert_eq!(plan["outputs"][0]["removeOverlap"], false);
    let built = run(&mut s, "font.compile", json!({"formats": ["otf"], "outlines": {"removeOverlaps": false}}));
    assert_eq!(built["warnings"], json!([]));
}

#[test]
fn export_refuses_an_invalid_glyph_name_while_compile_writes_a_clean_one() {
    let mut s = drawn();
    s.set_source_glyph_data(&[("GlyphData.xml", br#"<glyphData><glyph name="zz" production="myzz" category="Symbol"/></glyphData>"#)]).unwrap();
    s.edit("Add", |font| {
        font.add_glyph("A-cy", vec![0x410], 500.0)?;
        font.add_glyph("zz", vec![], 500.0)?;
        font.add_glyph("bad name", vec![0xE000], 500.0)?;
        Ok(())
    })
    .unwrap();
    let err = s.execute("file.exportStatic", &json!({})).unwrap_err().to_string();
    assert!(err.contains("glyph `bad name`: illegal glyph name") && err.contains("position 4"), "{err}");

    let plan = run(&mut s, "font.compilePlan", json!({"sections": ["productionNames"]}));
    // The document's own glyph data gives `zz` its production name.
    assert_eq!(plan["productionNames"], json!({"A-cy": "uni0410", "zz": "myzz", "bad name": "badname"}));
    let built = run(&mut s, "font.compile", json!({}));
    let warnings = built["fonts"][0]["warnings"].to_string();
    assert!(warnings.contains("`bad name` → `badname`"), "{warnings}");
}

/// The component flags of `font.compile`'s `outlines` group decompose and flatten composites.
#[test]
fn compile_component_flags_decompose_and_flatten() {
    use fontcraft_engine::model::{Alignment, Component, ComponentInfo, ComponentInfoBox, Shape, Transform};
    use skrifa::raw::TableProvider;
    use skrifa::raw::tables::glyf::Glyph;
    let mut s = drawn();
    s.edit("composites", |font| {
        let unaligned = |base: &str, transform: Transform| {
            let mut c = Component::new(base, 0.0, 0.0);
            c.transform = transform;
            c.info = ComponentInfoBox::new(ComponentInfo { alignment: Alignment::Disabled, ..Default::default() });
            Shape::Component(c)
        };
        font.glyph_mut("B")?.layers[0].shapes.push(unaligned("A", Transform::default()));
        font.glyph_mut("C")?.layers[0].shapes.push(unaligned("B", Transform::default()));
        font.glyph_mut("D")?.layers[0].shapes.push(unaligned("A", Transform::Matrix([0.5, 0.0, 0.0, 1.0, 0.0, 0.0])));
        Ok(())
    })
    .unwrap();
    let dir = temp("component-flags");
    // Per glyph B, C, D: None when simple, else the component glyph ids.
    let shape = |s: &mut Session, outlines: Value| -> Vec<Option<Vec<u16>>> {
        let path = dir.join("out.ttf");
        run(s, "font.compile", json!({"outlines": outlines, "destination": path, "timestamp": 0}));
        let bytes = std::fs::read(&path).unwrap();
        let font = skrifa::FontRef::new(&bytes).unwrap();
        let (loca, glyf) = (font.loca(None).unwrap(), font.glyf().unwrap());
        let gid = |c: char| font.charmap().map(c).unwrap();
        ['B', 'C', 'D']
            .iter()
            .map(|&c| match loca.get_glyf(gid(c), &glyf).unwrap() {
                Some(Glyph::Composite(g)) => Some(g.components().map(|c| c.glyph.to_u16()).collect()),
                _ => None,
            })
            .collect()
    };
    let plain = shape(&mut s, json!({}));
    assert!(plain.iter().all(Option::is_some), "composites stay composites by default: {plain:?}");
    let b_gid = plain[1].as_ref().unwrap()[0];
    let flat = shape(&mut s, json!({"flattenComponents": true}));
    assert_ne!(flat[1].as_ref().unwrap()[0], b_gid, "C points at A once flattened: {flat:?}");
    let transformed = shape(&mut s, json!({"decomposeTransformedComponents": true}));
    assert!(transformed[0].is_some() && transformed[2].is_none(), "only the scaled composite is decomposed: {transformed:?}");
    let all = shape(&mut s, json!({"decomposeComponents": true}));
    assert!(all.iter().all(Option::is_none), "every composite is decomposed: {all:?}");
    let e = s.execute("font.compile", &json!({"outlines": {"flattenComponents": 1}})).unwrap_err().to_string();
    assert!(e.contains("flattenComponents"), "{e}");
    std::fs::remove_dir_all(&dir).unwrap();
}
