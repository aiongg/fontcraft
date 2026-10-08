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
