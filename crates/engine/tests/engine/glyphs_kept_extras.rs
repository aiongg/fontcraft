//! What a `.glyphs` file keeps beside the model (spellings, layouts) after an edit changes the
//! data it describes: saving leaves it out, in `.glyphs` files and in native files, and the
//! glyphs the edit did not touch are written as they were read.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};

use fontcraft_engine::Session;
use fontcraft_engine::model::Shape;
use serde_json::{Value, json};

/// The prefix of the keys under which the reader keeps a value's layout.
const LAYOUT: &str = "com.fontcraft.glyphsLayout.";

/// `A` with its code point spelled with a leading zero, and the master's guide list on one line.
const V3: &str = "{
.appVersion = \"3343\";
.formatVersion = 3;
familyName = Kept;
fontMaster = (
{
guides = ({pos = (100,0);});
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
unicode = 0065;
},
{
glyphname = B;
layers = (
{
layerId = m01;
width = 500;
}
);
unicode = 66;
}
);
unitsPerEm = 1000;
versionMajor = 1;
versionMinor = 0;
}
";

/// `Aacute` with a component transform spelled with a decimal point.
const V2: &str = "{
.appVersion = \"1192\";
familyName = Old;
fontMaster = (
{
id = m01;
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
unicode = 0041;
},
{
glyphname = Aacute;
layers = (
{
components = (
{
name = A;
},
{
alignment = -1;
name = acute;
transform = \"{1.0, 0, 0, 1, 285, 232}\";
}
);
layerId = m01;
width = 600;
}
);
unicode = 00C1;
}
);
unitsPerEm = 1000;
versionMajor = 1;
versionMinor = 0;
}
";

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-kept-extras-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn open(path: &Path) -> Session {
    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": path }));
    s
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

/// The text of glyph `name` in `text`, from its name to its last line `last`.
fn glyph_text<'a>(text: &'a str, name: &str, last: &str) -> &'a str {
    let start = text.find(&format!("glyphname = {name};")).unwrap();
    let end = start + text.get(start..).unwrap().find(last).unwrap() + last.len();
    text.get(start..end).unwrap()
}

#[test]
fn removing_code_points_leaves_their_kept_spelling_out() {
    let dir = scratch("unicode");
    let source = dir.join("Kept.glyphs");
    std::fs::write(&source, V3).unwrap();
    let mut s = open(&source);
    run(&mut s, "glyph.setUnicodes", json!({ "glyphs": ["A"], "unicodes": [] }));
    let saved = dir.join("Saved.glyphs");
    run(&mut s, "file.saveAs", json!({ "path": saved }));
    let text = read(&saved);
    assert!(!text.contains("0065"), "{text}");
    assert!(text.contains(glyph_text(V3, "B", "unicode = 66;")), "the untouched glyph is written as read:\n{text}");
    let back = open(&saved);
    assert!(back.font().unwrap().glyph("A").unwrap().unicodes.is_empty());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn resetting_a_version_2_component_leaves_its_kept_transform_out() {
    let dir = scratch("transform");
    let source = dir.join("Old.glyphs");
    std::fs::write(&source, V2).unwrap();
    let mut s = open(&source);
    run(&mut s, "component.setTransform", json!({ "glyph": "Aacute", "component": 1, "x": 0, "y": 0 }));
    let saved = dir.join("Saved.glyphs");
    run(&mut s, "file.saveAs", json!({ "path": saved }));
    let text = read(&saved);
    assert!(!text.contains("285"), "{text}");
    assert!(text.contains(glyph_text(V2, "A", "unicode = 0041;")), "the untouched glyph is written as read:\n{text}");
    let back = open(&saved);
    let font = back.font().unwrap();
    let Shape::Component(c) = &font.glyph("Aacute").unwrap().layers[0].shapes[1] else { panic!() };
    assert!(c.transform.is_identity(), "{:?}", c.transform);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_native_save_leaves_out_layouts_an_edit_made_stale() {
    let dir = scratch("native");
    let source = dir.join("Kept.glyphs");
    std::fs::write(&source, V3).unwrap();
    let mut s = open(&source);
    // Untouched, the guide list's layout travels in the native file.
    run(&mut s, "file.saveAs", json!({ "path": dir.join("Before.fontcraft") }));
    assert!(read(&dir.join("Before.fontcraft")).contains(LAYOUT));

    run(&mut s, "guide.addGlobal", json!({ "x": 200 }));
    run(&mut s, "file.saveAs", json!({ "path": dir.join("After.fontcraft") }));
    let native = read(&dir.join("After.fontcraft"));
    assert!(!native.contains(LAYOUT), "{native}");
    // The native file reads back to the same font: both write the same `.glyphs` text.
    run(&mut s, "file.saveAs", json!({ "path": dir.join("After.glyphs") }));
    let mut back = open(&dir.join("After.fontcraft"));
    assert_eq!(back.font().unwrap().masters[0].guides.len(), 2);
    run(&mut back, "file.saveAs", json!({ "path": dir.join("Back.glyphs") }));
    let text = read(&dir.join("After.glyphs"));
    assert_eq!(read(&dir.join("Back.glyphs")), text);
    assert!(text.contains(glyph_text(V3, "B", "unicode = 66;")), "{text}");
    std::fs::remove_dir_all(&dir).unwrap();
}
