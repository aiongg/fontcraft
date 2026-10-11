//! Linked sources (Import Font, Import Master): glyphs and masters of other files shown in a
//! document, locked, and left out when it is saved.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use fontcraft_engine::Session;
use fontcraft_engine::io::{SaveFormat, open, save};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-linked-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A host with A and B on one master, linking `Linked.glyphs` both ways.
const HOST: &str = r#"{
.appVersion = "3343";
.formatVersion = 3;
axes = (
{
name = Weight;
tag = wght;
}
);
customParameters = (
{
name = "Import Font";
value = {
Path = "Linked.glyphs";
};
},
{
name = "Import Master";
value = "Linked.glyphs;Bold";
}
);
familyName = "Host Sans";
fontMaster = (
{
axesValues = (
400
);
id = m01;
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
unitsPerEm = 1000;
versionMajor = 1;
versionMinor = 0;
}
"#;

/// Two masters, A and C, and an Import Font back to the host (a cycle).
const LINKED: &str = r#"{
.appVersion = "3343";
.formatVersion = 3;
axes = (
{
name = Weight;
tag = wght;
}
);
customParameters = (
{
name = "Import Font";
value = {
Path = "Host.glyphs";
};
}
);
familyName = "Linked Sans";
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
width = 610;
},
{
layerId = m02;
width = 650;
}
);
unicode = 65;
},
{
glyphname = C;
layers = (
{
layerId = m01;
width = 620;
},
{
layerId = m02;
width = 660;
}
);
unicode = 67;
}
);
unitsPerEm = 1000;
versionMajor = 1;
versionMinor = 0;
}
"#;

fn warnings(reply: &Value) -> Vec<String> {
    reply["warnings"].as_array().map(|a| a.iter().filter_map(|w| w.as_str().map(String::from)).collect()).unwrap_or_default()
}

#[test]
fn import_font_shows_the_missing_glyphs_locked_and_saving_leaves_them_out() {
    let dir = temp("font");
    let host = dir.join("Host.glyphs");
    std::fs::write(&host, HOST).unwrap();
    std::fs::write(dir.join("Linked.glyphs"), LINKED).unwrap();
    let mut s = Session::new();
    let opened = run(&mut s, "file.open", json!({ "path": host }));
    let w = warnings(&opened);
    assert!(w.iter().any(|w| w.contains("not followed")), "the linked file's own Import Font is not followed: {w:?}");

    let font = s.doc().unwrap().font.clone();
    let names: Vec<&str> = font.glyphs.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(names, ["A", "B", "C"], "C is added after the host's own glyphs; A stays the host's");
    let a = font.glyphs.get("A").unwrap();
    assert!(!a.info.locked);
    assert_eq!(a.layers.first().map(|l| l.width), Some(600.0), "the host's own A is kept");
    let c = font.glyphs.get("C").unwrap();
    assert!(c.info.locked, "a linked glyph is locked");
    assert_eq!(c.layers.first().map(|l| (l.master.as_str(), l.width)), Some(("m01", 620.0)));

    let err = s.execute("spacing.setWidth", &json!({ "glyphs": ["C"], "value": 640 })).unwrap_err().to_string();
    assert!(err.contains("locked"), "{err}");

    // Saving writes what the host file holds without its linked sources.
    let (plain, _) = open(&host, None).unwrap();
    let expected = dir.join("Expected.glyphs");
    save(&plain, &expected, SaveFormat::Glyphs).unwrap();
    run(&mut s, "file.save", json!({ "force": true }));
    let text = std::fs::read_to_string(&host).unwrap();
    assert_eq!(text, std::fs::read_to_string(&expected).unwrap());
    assert!(!text.contains("glyphname = C;") && !text.contains("com.fontcraft.linkedSource"), "{text}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn import_master_adds_the_named_master_with_its_layers_and_saving_leaves_it_out() {
    let dir = temp("master");
    let host = dir.join("Host.glyphs");
    std::fs::write(&host, HOST).unwrap();
    std::fs::write(dir.join("Linked.glyphs"), LINKED).unwrap();
    let mut s = Session::new();
    let opened = run(&mut s, "file.open", json!({ "path": host }));
    assert_eq!(opened["masters"], 2, "{opened}");

    let font = s.doc().unwrap().font.clone();
    let bold = font.masters.get(1).unwrap();
    assert_eq!((bold.id.as_str(), bold.name.as_str(), bold.location.as_slice()), ("m02", "Bold", [700.0].as_slice()));
    let widths = |name: &str| -> Vec<(String, f64)> { font.glyphs.get(name).unwrap().layers.iter().map(|l| (l.master.clone(), l.width)).collect() };
    assert_eq!(widths("A"), [("m01".to_string(), 600.0), ("m02".to_string(), 650.0)]);
    assert_eq!(widths("B"), [("m01".to_string(), 600.0)], "B is not in the linked file");
    assert_eq!(widths("C"), [("m01".to_string(), 620.0), ("m02".to_string(), 660.0)]);

    run(&mut s, "file.save", json!({ "force": true }));
    let (back, _) = open(&host, None).unwrap();
    assert_eq!(back.masters.len(), 1, "the linked master is not saved into the host");
    assert_eq!(back.glyphs.get("A").unwrap().layers.len(), 1);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_missing_linked_file_is_a_warning_naming_it() {
    let dir = temp("missing");
    let host = dir.join("Host.glyphs");
    std::fs::write(&host, HOST.replace("Linked.glyphs", "Gone.glyphs")).unwrap();
    let mut s = Session::new();
    let opened = run(&mut s, "file.open", json!({ "path": host }));
    let w = warnings(&opened);
    // Both parameters name the file; it is tried, and named, once.
    assert_eq!(w.iter().filter(|w| w.contains("`Gone.glyphs`")).count(), 1, "{w:?}");
    assert_eq!((opened["glyphs"].as_u64(), opened["masters"].as_u64()), (Some(2), Some(1)));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_file_linking_to_itself_or_back_does_not_recurse() {
    let dir = temp("cycle");
    let host = dir.join("Host.glyphs");
    // The host imports itself; the linked file imports the host.
    std::fs::write(&host, HOST.replace("Path = \"Linked.glyphs\";", "Path = \"Host.glyphs\";")).unwrap();
    std::fs::write(dir.join("Linked.glyphs"), LINKED).unwrap();
    let mut s = Session::new();
    let opened = run(&mut s, "file.open", json!({ "path": host }));
    let w = warnings(&opened);
    assert!(w.iter().any(|w| w.contains("names the document itself")), "{w:?}");
    assert_eq!(opened["glyphs"], 2, "{opened}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_format_samples_with_import_font_open() {
    let root = std::env::var_os("FONTCRAFT_CORPUS")
        .map_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus"), PathBuf::from)
        .join("glyphssdk/GlyphsFileFormat");
    if !root.is_dir() {
        eprintln!("skipped: no corpus at {}", root.display());
        return;
    }
    for name in ["GlyphsFileFormatv2.glyphs", "GlyphsFileFormatv3.glyphs", "GlyphsFileFormatv3.glyphspackage"] {
        let mut s = Session::new();
        let opened = run(&mut s, "file.open", json!({ "path": root.join(name) }));
        let w = warnings(&opened);
        if name.contains("v2") {
            // Its Import Font is an empty dictionary.
            assert!(w.iter().any(|w| w.contains("Import Font: the parameter names no file")), "{name}: {w:?}");
        } else {
            // `files/LinkedFontv3.glyphs` sits beside the sample.
            assert!(!w.iter().any(|w| w.contains("Import Font `") && w.contains("cannot")), "{name}: {w:?}");
        }
    }
}
