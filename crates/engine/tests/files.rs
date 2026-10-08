//! Opening and saving source files through the commands, on disk.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use fontcraft_engine::Session;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-files-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const V3: &str = "{
.appVersion = \"3343\";
.formatVersion = 3;
familyName = \"Test Sans\";
fontMaster = (
{
id = m01;
metricValues = (
{
over = 16;
pos = 800;
}
);
name = Regular;
}
);
glyphs = (
{
glyphname = A;
layers = (
{
layerId = m01;
width = 600;
}
);
unicode = 65;
},
{
glyphname = B;
layers = (
{
layerId = m01;
width = 600;
}
);
unicode = 66;
}
);
metrics = (
{
type = ascender;
}
);
unitsPerEm = 1000;
versionMajor = 1;
versionMinor = 0;
}
";

#[test]
fn a_glyphs_file_opens_edits_and_saves_in_its_own_format() {
    let dir = temp("single");
    let path = dir.join("Test.glyphs");
    std::fs::write(&path, V3).unwrap();
    let mut s = Session::new();
    let opened = run(&mut s, "file.open", json!({ "path": path }));
    assert_eq!((opened["format"].as_str(), opened["version"].as_u64(), opened["glyphs"].as_u64()), (Some("glyphs"), Some(3), Some(2)));
    // Saving an unedited document gives back the same bytes.
    run(&mut s, "file.save", json!({}));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), V3);
    run(&mut s, "spacing.setWidth", json!({ "glyphs": ["B"], "value": 640 }));
    let saved = run(&mut s, "file.save", json!({}));
    assert_eq!(saved["format"], "glyphs");
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text, V3.replacen("width = 600;\n}\n);\nunicode = 66;", "width = 640;\n}\n);\nunicode = 66;", 1));
    assert!(!s.doc().unwrap().modified());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_file_changed_on_disk_is_not_overwritten_without_force() {
    let dir = temp("stamp");
    let path = dir.join("Test.glyphs");
    std::fs::write(&path, V3).unwrap();
    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": path }));
    // Another program rewrites the file (one more byte, so the change shows even within one
    // modification-time tick).
    std::fs::write(&path, format!("{V3}\n")).unwrap();
    let err = s.execute("file.save", &json!({})).unwrap_err().to_string();
    assert!(err.contains("changed on disk"), "{err}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), format!("{V3}\n"), "the other program's version is untouched");
    run(&mut s, "file.save", json!({ "force": true }));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), V3);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_package_rewrites_only_what_changed() {
    let dir = temp("package");
    let single = dir.join("Test.glyphs");
    std::fs::write(&single, V3).unwrap();
    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": single }));
    let pkg = dir.join("Test.glyphspackage");
    let first = run(&mut s, "file.saveAs", json!({ "path": pkg }));
    assert_eq!(first["format"], "glyphspackage");
    assert_eq!(first["filesWritten"], 4, "fontinfo.plist, order.plist and two glyph files");
    assert!(pkg.join("glyphs/A_.glyph").is_file() && pkg.join("glyphs/B_.glyph").is_file());

    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": pkg }));
    run(&mut s, "spacing.setWidth", json!({ "glyphs": ["A"], "value": 650 }));
    let saved = run(&mut s, "file.save", json!({}));
    assert_eq!((saved["filesWritten"].as_u64(), saved["filesRemoved"].as_u64()), (Some(1), Some(0)), "{saved}");
    run(&mut s, "glyph.removeGlyph", json!({ "glyphs": ["B"] }));
    let saved = run(&mut s, "file.save", json!({}));
    assert_eq!(saved["filesRemoved"], 1);
    assert!(!pkg.join("glyphs/B_.glyph").exists());
    // Display strings are model data in either form; a file the model has no place for is kept
    // in the package and reported when saving to a single file.
    std::fs::write(pkg.join("UIState.plist"), "{\ndisplayStrings = (\nA\n);\n}\n").unwrap();
    std::fs::write(pkg.join("README.txt"), "notes for the team\n").unwrap();
    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": pkg }));
    assert_eq!(s.font().unwrap().display_strings.as_slice(), ["A"]);
    let saved = run(&mut s, "file.saveAs", json!({ "path": dir.join("Flat.glyphs") }));
    assert_eq!(saved["losses"][0]["files"], json!(["README.txt"]));
    assert!(std::fs::read_to_string(dir.join("Flat.glyphs")).unwrap().contains("DisplayStrings = (\nA\n);"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn saving_reports_what_the_format_cannot_hold() {
    let dir = temp("losses");
    // A version 1 native file whose unknown key is kept as a native-family extra entry.
    let v1 = r#"{"format":"fontcraft","version":1,"font":{"family_name":"Old","units_per_em":1000,"fromTheFuture":1,
        "masters":[{"id":"m01","name":"Regular","ascender":800,"cap_height":700,"x_height":500,"descender":-200}],
        "glyphs":[{"name":"A","layers":[{"master":"m01","width":600}]}]}}"#;
    let path = dir.join("Old.fontcraft");
    std::fs::write(&path, v1).unwrap();
    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": path }));
    let saved = run(&mut s, "file.saveAs", json!({ "path": dir.join("Old.glyphs") }));
    assert_eq!(
        saved["losses"],
        json!([{ "family": "fontcraft", "entries": 1, "what": "keys read from that format, which this one has no place for" }])
    );
    // The native format keeps everything.
    let saved = run(&mut s, "file.saveAs", json!({ "path": dir.join("Again.fontcraft") }));
    assert_eq!(saved["losses"], json!([]));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn bad_targets_are_refused() {
    let dir = temp("bad");
    let mut s = Session::with_document();
    assert!(s.execute("file.saveAs", &json!({ "path": dir.join("x.unknown") })).is_err());
    assert!(s.execute("file.saveAs", &json!({ "path": dir.join("x"), "format": "nope" })).is_err());
    assert!(s.execute("file.open", &json!({ "path": dir })).is_err(), "a folder that is not a source");
    std::fs::write(dir.join("x.ttf"), b"\0\x01\0\0rest").unwrap();
    assert!(s.execute("file.open", &json!({ "path": dir.join("x.ttf") })).is_err(), "a damaged compiled font");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_real_family_saves_back_unchanged() {
    let oswald = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/oswald/sources/Oswald.glyphs");
    if !oswald.is_file() {
        eprintln!("corpus/ absent: skipped (cargo xtask corpus --fetch)");
        return;
    }
    let dir = temp("oswald");
    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": oswald }));
    let copy = dir.join("Oswald.glyphs");
    run(&mut s, "file.saveAs", json!({ "path": copy }));
    assert!(std::fs::read(&copy).unwrap() == std::fs::read(&oswald).unwrap(), "Oswald.glyphs came back different");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_compiled_font_opens_view_only_and_saves_as_a_source() {
    let dir = temp("binary");
    let mut s = Session::with_document();
    let ttf = dir.join("Untitled.ttf");
    run(&mut s, "file.exportStatic", json!({ "destination": ttf, "timestamp": 0 }));
    let opened = run(&mut s, "file.open", json!({ "path": ttf }));
    assert_eq!((opened["format"].as_str(), opened["viewOnly"].as_bool()), (Some("binary"), Some(true)));
    assert!(opened["warnings"].is_array());
    let err = s.execute("file.save", &json!({})).unwrap_err().to_string();
    assert!(err.contains("Save As"), "{err}");
    let saved = run(&mut s, "file.saveAs", json!({ "path": dir.join("FromBinary.glyphs") }));
    assert_eq!(saved["format"], "glyphs");
    let text = std::fs::read_to_string(dir.join("FromBinary.glyphs")).unwrap();
    assert!(text.contains("name = glyphOrder;"), "the glyph order is recorded");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn files_are_inspected_without_being_opened() {
    let dir = temp("inspect");
    let mut s = Session::with_document();
    let ttf = dir.join("Untitled.ttf");
    run(&mut s, "file.exportStatic", json!({ "destination": ttf, "timestamp": 0 }));
    let docs = s.documents().len();
    let info = run(&mut s, "font.inspect", json!({ "input": ttf, "sections": ["format", "names", "metrics", "masters"] }));
    assert_eq!(info["format"]["kind"], "TrueType");
    assert_eq!(info["names"]["family"], "Untitled");
    assert_eq!(info["metrics"]["unitsPerEm"], 1000);
    assert!(info["masters"]["error"].is_string(), "a section that does not apply reports an error, not a failure");
    assert!(info.get("cmap").is_none(), "only the sections asked for");
    let tables = run(&mut s, "tables.list", json!({ "font": ttf, "sort": "tag" }));
    let tags: Vec<&str> = tables["faces"][0]["tables"].as_array().unwrap().iter().map(|t| t["tag"].as_str().unwrap()).collect();
    let mut sorted = tags.clone();
    sorted.sort();
    assert_eq!(tags, sorted);
    assert!(tags.contains(&"glyf") && tags.contains(&"OS/2"));
    let glyphs = run(&mut s, "font.listGlyphs", json!({ "font": ttf }));
    assert_eq!(glyphs["glyphs"][0]["name"], ".notdef");
    // A source is summarised too.
    let src = dir.join("Untitled.glyphs");
    run(&mut s, "file.saveAs", json!({ "path": src }));
    let info = run(&mut s, "font.inspect", json!({ "input": src, "sections": ["format", "masters"] }));
    assert_eq!((info["format"]["kind"].as_str(), info["masters"][0]["ascender"].as_f64()), (Some("glyphs"), Some(800.0)));
    assert_eq!(s.documents().len(), docs, "nothing was opened");
    assert!(s.execute("font.inspect", &json!({ "input": ttf, "sections": ["nope"] })).is_err());
    assert!(s.execute("tables.list", &json!({ "font": src })).is_err(), "a source has no table directory");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_compressed_native_file_stays_compressed() {
    let dir = temp("gzip");
    let mut s = Session::with_document();
    let path = dir.join("Small.fontcraft");
    run(&mut s, "file.saveAs", json!({ "path": path, "compress": true }));
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[..2], &[0x1f, 0x8b]);
    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": path }));
    run(&mut s, "spacing.setWidth", json!({ "glyphs": ["A"], "value": 610 }));
    run(&mut s, "file.save", json!({}));
    assert_eq!(&std::fs::read(&path).unwrap()[..2], &[0x1f, 0x8b], "saved compressed again");
    assert!(s.execute("file.saveAs", &json!({ "path": dir.join("x.glyphs"), "compress": true })).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Every file under `dir`, relative, with its bytes.
fn tree(dir: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    let mut out = std::collections::BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.insert(p.strip_prefix(dir).unwrap().to_string_lossy().replace('\\', "/"), std::fs::read(&p).unwrap());
            }
        }
    }
    out
}

#[test]
fn a_ufo_opens_and_saves_only_what_changed() {
    let dir = temp("ufo");
    let single = dir.join("Test.glyphs");
    std::fs::write(&single, V3).unwrap();
    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": single }));
    let ufo = dir.join("Test.ufo");
    let first = run(&mut s, "file.saveAs", json!({ "path": ufo }));
    assert_eq!((first["format"].as_str(), first["version"].as_u64()), (Some("ufo"), Some(3)), "{first}");
    for f in ["metainfo.plist", "fontinfo.plist", "lib.plist", "layercontents.plist", "glyphs/contents.plist", "glyphs/A_.glif", "glyphs/B_.glif"] {
        assert!(ufo.join(f).is_file(), "{f} missing");
    }

    let mut s = Session::new();
    let opened = run(&mut s, "file.open", json!({ "path": ufo }));
    assert_eq!((opened["format"].as_str(), opened["version"].as_u64()), (Some("ufo"), Some(3)));
    let before = tree(&ufo);
    let saved = run(&mut s, "file.save", json!({}));
    assert_eq!((saved["filesWritten"].as_u64(), saved["filesRemoved"].as_u64()), (Some(0), Some(0)), "{saved}");
    assert_eq!(tree(&ufo), before, "an unchanged UFO is not touched");
    run(&mut s, "spacing.setWidth", json!({ "glyphs": ["A"], "value": 650 }));
    let saved = run(&mut s, "file.save", json!({}));
    assert_eq!(saved["filesWritten"], 1, "{saved}");
    assert!(std::fs::read_to_string(ufo.join("glyphs/A_.glif")).unwrap().contains("<advance width=\"650\"/>"));
    run(&mut s, "glyph.removeGlyph", json!({ "glyphs": ["B"] }));
    let saved = run(&mut s, "file.save", json!({}));
    assert_eq!(
        (saved["filesWritten"].as_u64(), saved["filesRemoved"].as_u64()),
        (Some(1), Some(1)),
        "contents.plist rewritten, B_.glif removed: {saved}"
    );
    assert!(!ufo.join("glyphs/B_.glif").exists());
    // A two-master document does not fit one UFO.
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn font_information_reaches_fontinfo_and_comes_back() {
    let dir = temp("fontinfo");
    let source = dir.join("Info.glyphs");
    let text = V3.replacen(
        "familyName = \"Test Sans\";",
        "customParameters = (\n{\nname = typoAscender;\nvalue = 820;\n},\n{\nname = \"Variable Font Origin\";\nvalue = m01;\n}\n);\n\
         date = \"2026-10-08 12:00:00 +0000\";\nfamilyName = \"Test Sans\";\nproperties = (\n{\nkey = designers;\nvalues = (\n{\nlanguage = dflt;\n\
         value = \"A. Designer\";\n},\n{\nlanguage = DEU;\nvalue = \"A. Gestalterin\";\n}\n);\n}\n);",
        1,
    );
    std::fs::write(&source, text).unwrap();
    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": source }));
    let ufo = dir.join("Info.ufo");
    let saved = run(&mut s, "file.saveAs", json!({ "path": ufo }));
    let info = saved["losses"].as_array().unwrap().iter().find(|l| l["what"] == "font information a UFO has no place for yet").cloned();
    assert_eq!(info.map(|l| l["items"].clone()), Some(json!(["parameter `Variable Font Origin`"])), "{saved}");
    let fontinfo = std::fs::read_to_string(ufo.join("fontinfo.plist")).unwrap();
    for expected in [
        "<key>openTypeOS2TypoAscender</key>\n  <integer>820</integer>",
        "<key>openTypeNameDesigner</key>\n  <string>A. Designer</string>",
        "<key>openTypeHeadCreated</key>\n  <string>2026/10/08 12:00:00</string>",
        "<key>openTypeOS2Type</key>\n  <array>\n    <integer>3</integer>",
        "<integer>1031</integer>",
    ] {
        assert!(fontinfo.contains(&expected.replace("\n  ", "\n    ")), "{expected} in {fontinfo}");
    }
    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": ufo }));
    let saved = run(&mut s, "file.saveAs", json!({ "path": dir.join("Back.glyphs") }));
    assert!(saved["losses"].as_array().unwrap().iter().all(|l| l["family"] == "ufo"), "only the UFO's file records: {saved}");
    let back = std::fs::read_to_string(dir.join("Back.glyphs")).unwrap();
    assert!(back.contains("name = typoAscender;\nvalue = 820;"), "{back}");
    assert!(back.contains("language = DEU;\nvalue = \"A. Gestalterin\";"), "{back}");
    assert!(back.contains("date = \"2026-10-08 12:00:00 +0000\";"), "{back}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn zones_stems_and_guides_reach_fontinfo_and_come_back() {
    let dir = temp("design");
    let source = dir.join("Design.glyphs");
    let text = V3
        .replacen(
            "id = m01;\nmetricValues = (\n{\nover = 16;\npos = 800;\n}\n);\nname = Regular;\n",
            "guides = (\n{\nlocked = 1;\nname = \"top [#g1]\";\npos = (0,600);\n},\n{\nangle = 90;\npos = (250,0);\n}\n);\nid = m01;\n\
             metricValues = (\n{\nover = 16;\npos = 800;\n},\n{\nover = -12;\n}\n);\nname = Regular;\nstemValues = (\n80,\n30\n);\n",
            1,
        )
        .replacen(
            "metrics = (\n{\ntype = ascender;\n}\n);",
            "metrics = (\n{\ntype = ascender;\n},\n{\ntype = baseline;\n}\n);\nstems = (\n{\nname = stem;\n},\n{\nhorizontal = 1;\nname = bar;\n}\n);",
            1,
        );
    std::fs::write(&source, &text).unwrap();
    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": source }));
    let ufo = dir.join("Design.ufo");
    let saved = run(&mut s, "file.saveAs", json!({ "path": ufo }));
    let info = saved["losses"].as_array().unwrap().iter().find(|l| l["what"] == "font information a UFO has no place for yet").cloned();
    assert_eq!(info.map(|l| l["items"].clone()), Some(json!(["the font's stem names and filters (2)"])), "{saved}");
    let fontinfo = std::fs::read_to_string(ufo.join("fontinfo.plist")).unwrap();
    let list = |key: &str, items: &[i64]| {
        let body: String = items.iter().map(|i| format!("\n      <integer>{i}</integer>")).collect();
        format!("<key>{key}</key>\n    <array>{body}\n    </array>")
    };
    for expected in [
        list("postscriptBlueValues", &[-12, 0, 800, 816]),
        list("postscriptStemSnapH", &[30]),
        list("postscriptStemSnapV", &[80]),
        "<key>identifier</key>\n        <string>g1</string>\n        <key>name</key>\n        <string>top [locked]</string>\n        <key>y</key>\n        <integer>600</integer>".into(),
        "<dict>\n        <key>x</key>\n        <integer>250</integer>\n      </dict>".into(),
    ] {
        assert!(fontinfo.contains(&expected), "{expected} in {fontinfo}");
    }
    assert!(!fontinfo.contains("postscriptOtherBlues"), "{fontinfo}");

    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": ufo }));
    let saved = run(&mut s, "file.saveAs", json!({ "path": dir.join("Back.glyphs") }));
    assert!(saved["losses"].as_array().unwrap().iter().all(|l| l["family"] == "ufo"), "only the UFO's file records: {saved}");
    let back = std::fs::read_to_string(dir.join("Back.glyphs")).unwrap();
    for expected in [
        "{\nlocked = 1;\nname = \"top [#g1]\";\npos = (0,600);\n}",
        "{\nangle = 90;\npos = (250,0);\n}",
        "{\nover = -12;\n}",
        "stemValues = (\n30,\n80\n);",
        "stems = (\n{\nhorizontal = 1;\nname = \"\";\n},\n{\nname = \"\";\n}\n);",
    ] {
        assert!(back.contains(expected), "{expected} in {back}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_ufo_zone_at_no_metric_is_reported_when_saved_as_version_3() {
    let dir = temp("zones");
    std::fs::write(dir.join("Z.glyphs"), V3).unwrap();
    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": dir.join("Z.glyphs") }));
    let ufo = dir.join("Z.ufo");
    run(&mut s, "file.saveAs", json!({ "path": ufo }));
    // A zone from 400 to 408, where the document has no metric.
    let info = std::fs::read_to_string(ufo.join("fontinfo.plist")).unwrap();
    let blues = "<key>postscriptBlueValues</key>\n    <array>\n      <integer>400</integer>\n      <integer>408</integer>\n";
    let edited = info.replacen("<key>postscriptBlueValues</key>\n    <array>\n", blues, 1);
    assert_ne!(edited, info, "{info}");
    std::fs::write(ufo.join("fontinfo.plist"), edited).unwrap();
    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": ufo }));
    let saved = run(&mut s, "file.saveAs", json!({ "path": dir.join("Back.glyphs") }));
    let lost = saved["losses"].as_array().unwrap().iter().find(|l| l["what"] == "font information this .glyphs version has no place for").cloned();
    assert_eq!(lost.map(|l| l["items"].clone()), Some(json!(["master `Regular`: alignment zones other than overshoots (1)"])), "{saved}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn instance_fields_travel_in_a_designspace_but_not_a_ufo() {
    let dir = temp("instances");
    let text = V3.replacen(
        "familyName = \"Test Sans\";",
        "familyName = \"Test Sans\";\ninstances = (\n{\nisBold = 1;\nname = Bold;\nvisible = 0;\nweightClass = 700;\n}\n);",
        1,
    );
    std::fs::write(dir.join("I.glyphs"), text).unwrap();
    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": dir.join("I.glyphs") }));
    let items = |saved: &Value, what: &str| saved["losses"].as_array().unwrap().iter().find(|l| l["what"] == what).map(|l| l["items"].clone());
    let saved = run(&mut s, "file.saveAs", json!({ "path": dir.join("I.designspace") }));
    assert_eq!(
        items(&saved, "font information a designspace and its UFOs have no place for yet"),
        Some(json!(["instance `Bold`: visibility"])),
        "{saved}"
    );
    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": dir.join("I.glyphs") }));
    let saved = run(&mut s, "file.saveAs", json!({ "path": dir.join("I.ufo") }));
    assert_eq!(
        items(&saved, "font information a UFO has no place for yet"),
        Some(json!(["instance `Bold`: visibility, weight class, style linking"])),
        "{saved}"
    );
    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": dir.join("I.designspace") }));
    let back = &s.doc().unwrap().font.instances[0];
    assert_eq!((back.name.as_str(), back.is_bold, back.weight_class.clone()), ("Bold", true, Some(fontcraft_model::StyleClass::Number(700.0))));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The test document with a second master, Bold, whose glyphs are wider.
fn two_masters() -> String {
    V3.replacen(
        "name = Regular;\n}\n);",
        "name = Regular;\n},\n{\nid = m02;\nmetricValues = (\n{\nover = 16;\npos = 800;\n}\n);\nname = Bold;\n}\n);",
        1,
    )
    .replace("layerId = m01;\nwidth = 600;\n}\n);", "layerId = m01;\nwidth = 600;\n},\n{\nlayerId = m02;\nwidth = 700;\n}\n);")
}

#[test]
fn a_designspace_saves_its_ufos_and_comes_back() {
    let dir = temp("designspace");
    let single = dir.join("Duo.glyphs");
    std::fs::write(&single, two_masters()).unwrap();
    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": single }));
    assert!(s.execute("file.saveAs", &json!({ "path": dir.join("One.ufo") })).is_err(), "two masters do not fit one UFO");
    let ds = dir.join("Duo.designspace");
    let saved = run(&mut s, "file.saveAs", json!({ "path": ds }));
    assert_eq!(saved["format"], "designspace");
    assert!(dir.join("TestSans-Regular.ufo/glyphs/A_.glif").is_file() && dir.join("TestSans-Bold.ufo/glyphs/A_.glif").is_file(), "{saved}");

    let mut s = Session::new();
    let opened = run(&mut s, "file.open", json!({ "path": ds }));
    assert_eq!((opened["format"].as_str(), opened["masters"].as_u64()), (Some("designspace"), Some(2)));
    let before = tree(&dir);
    assert_eq!(run(&mut s, "file.save", json!({}))["filesWritten"], 0);
    assert_eq!(tree(&dir), before);
    run(&mut s, "spacing.setWidth", json!({ "glyphs": ["B"], "value": 610, "layer": "m02" }));
    let saved = run(&mut s, "file.save", json!({}));
    assert_eq!(saved["filesWritten"], 1, "{saved}");
    let changed: Vec<String> = tree(&dir).into_iter().filter(|(k, v)| before.get(k) != Some(v)).map(|(k, _)| k).collect();
    assert_eq!(changed, ["TestSans-Bold.ufo/glyphs/B_.glif"]);

    // A UFO edited elsewhere since opening is not overwritten.
    std::fs::write(dir.join("TestSans-Regular.ufo/glyphs/A_.glif"), "changed").unwrap();
    run(&mut s, "spacing.setWidth", json!({ "glyphs": ["A"], "value": 611 }));
    assert!(s.execute("file.save", &json!({})).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn masters_export_as_ufos() {
    let dir = temp("export-ufo");
    let single = dir.join("Duo.glyphs");
    std::fs::write(&single, two_masters()).unwrap();
    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": single }));
    let out = run(&mut s, "file.exportUfo", json!({ "destination": dir.join("ufo") }));
    assert_eq!(out["files"].as_array().unwrap().len(), 2, "{out}");
    assert!(dir.join("ufo/TestSans-Bold.ufo/glyphs/A_.glif").is_file());
    assert!(std::fs::read_to_string(dir.join("ufo/TestSans-Bold.ufo/glyphs/A_.glif")).unwrap().contains("width=\"700\""));
    let one = run(&mut s, "file.exportUfo", json!({ "destination": dir.join("one"), "masters": ["m01"] }));
    assert_eq!(one["files"].as_array().unwrap().len(), 1);
    assert!(s.execute("file.exportUfo", &json!({ "destination": dir.join("x"), "masters": ["nope"] })).is_err());
    assert!(s.execute("file.exportUfo", &json!({ "destination": dir.join("x"), "productionNames": true })).is_err());
    // The document keeps its own file and format.
    assert_eq!(s.doc().unwrap().path.as_deref(), Some(single.as_path()));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_ufo_is_not_written_into_a_folder_that_is_not_one() {
    let dir = temp("ufo-refuse");
    let target = dir.join("Folder.ufo");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("notes.txt"), "mine").unwrap();
    let mut s = Session::new();
    run(&mut s, "file.new", json!({ "familyName": "New" }));
    assert!(s.execute("file.saveAs", &json!({ "path": target })).is_err());
    assert_eq!(std::fs::read_to_string(target.join("notes.txt")).unwrap(), "mine");
    std::fs::remove_dir_all(&dir).unwrap();
}
