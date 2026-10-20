//! Bitmap strikes (`bitmap.importStrikes`, `bitmap.exportStrikes`, `bitmap.setStrikes`): a BDF
//! file goes into a strike by code then name, exports back to the same strike as BDF and PCF,
//! strikes are added and removed with undo, renames carry strike glyphs along, formats without a
//! place for strikes report them, and hostile files fail without changing the font.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::PathBuf;
use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::model::{BitmapStrike, Font};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// Runs a command that must fail, checks that the font is the same object as before, and
/// returns the message.
fn refused(s: &mut Session, id: &str, p: Value) -> String {
    let before = s.doc().unwrap().font.clone();
    let e = s.execute(id, &p).expect_err(id).to_string();
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "{id}: a refused command changed the font");
    e
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-bitmap-strikes-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn session() -> Session {
    let mut s = Session::new();
    s.add_document(Font::new("Pixel Test"), None).unwrap();
    s
}

fn strikes(s: &Session) -> Vec<BitmapStrike> {
    s.font().unwrap().bitmap_strikes.to_vec()
}

/// An 8 pixel strike: `A` by its code, `bee` (code 66) going to `B` by code, `c` without a code
/// going to `c` by name, and a private-use glyph the font lacks.
const BDF: &str = "STARTFONT 2.1
FONT -Test-Pixel-Medium-R-Normal--8-80-75-75-C-60-ISO10646-1
SIZE 8 75 75
FONTBOUNDINGBOX 6 8 0 -1
STARTPROPERTIES 5
PIXEL_SIZE 8
CHARSET_REGISTRY \"ISO10646\"
CHARSET_ENCODING \"1\"
FONT_ASCENT 7
FONT_DESCENT 1
ENDPROPERTIES
CHARS 4
STARTCHAR A
ENCODING 65
SWIDTH 750 0
DWIDTH 6 0
BBX 5 4 0 0
BITMAP
70
88
F8
88
ENDCHAR
STARTCHAR bee
ENCODING 66
SWIDTH 750 0
DWIDTH 6 0
BBX 4 4 0 0
BITMAP
E0
F0
90
F0
ENDCHAR
STARTCHAR c
ENCODING -1
SWIDTH 625 0
DWIDTH 5 0
BBX 3 3 0 0
BITMAP
60
80
60
ENDCHAR
STARTCHAR orphan
ENCODING 57344
SWIDTH 750 0
DWIDTH 6 0
BBX 1 1 0 0
BITMAP
80
ENDCHAR
ENDFONT
";

fn bdf_file(dir: &std::path::Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

#[test]
fn import_matches_bitmaps_by_code_then_name_as_one_undo_step() {
    let dir = temp("import");
    let path = bdf_file(&dir, "pixel.bdf", BDF);
    let mut s = session();
    let steps = s.doc().unwrap().history.undo.len();
    let out = run(&mut s, "bitmap.importStrikes", json!({ "paths": [path] }));
    assert_eq!(out["strikes"][0]["ppem"], 8);
    assert_eq!(out["strikes"][0]["glyphs"], 3);
    assert_eq!(out["strikes"][0]["replaced"], false);
    assert_eq!(out["strikes"][0]["leftOut"], json!(["orphan"]));
    assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1, "one undo step");

    let got = strikes(&s);
    assert_eq!(got.len(), 1);
    let strike = &got[0];
    assert_eq!((strike.ppem, strike.depth, strike.ascent, strike.descent), (8, 1, 7, 1));
    let names: Vec<&str> = strike.glyphs.iter().map(|g| g.glyph.as_str()).collect();
    assert_eq!(names, ["A", "B", "c"], "bee goes to B by its code, c by its name");
    let b = strike.glyph("B").unwrap();
    assert_eq!((b.width, b.height, b.advance, b.code), (4, 4, 6, Some(66)));
    assert_eq!(b.data, [0xE0, 0xF0, 0x90, 0xF0]);

    // Importing again replaces the bitmaps it holds and keeps the strike's others.
    let mut fewer = BDF.replace("CHARS 4", "CHARS 1");
    let start = fewer.find("STARTCHAR bee").unwrap();
    let end = fewer.find("ENDFONT").unwrap();
    fewer.replace_range(start..end, "");
    let fewer = fewer.replace("70\n88\nF8\n88", "F8\n88\n88\n88");
    let again = bdf_file(&dir, "fewer.bdf", &fewer);
    let out = run(&mut s, "bitmap.importStrikes", json!({ "paths": [again] }));
    assert_eq!(out["strikes"][0]["replaced"], true);
    let strike = &strikes(&s)[0];
    assert_eq!(strike.glyphs.len(), 3);
    assert_eq!(strike.glyph("A").unwrap().data, [0xF8, 0x88, 0x88, 0x88]);

    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert!(strikes(&s).is_empty(), "undo removes the imported strike");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_imported_strike_exports_back_to_the_same_strike_as_bdf_and_pcf() {
    let dir = temp("export");
    let path = bdf_file(&dir, "pixel.bdf", BDF);
    let mut s = session();
    run(&mut s, "bitmap.importStrikes", json!({ "paths": [path] }));
    let original = strikes(&s).remove(0);

    let bdf = dir.join("out.bdf");
    let out = run(&mut s, "bitmap.exportStrikes", json!({ "path": bdf }));
    assert_eq!(out["format"], "bdf");
    assert_eq!(out["files"][0]["glyphs"], 3);
    let pcf_dir = dir.join("pcf");
    let out = run(&mut s, "bitmap.exportStrikes", json!({ "strikes": [{"ppem": 8}], "format": "pcf", "destination": pcf_dir }));
    let pcf = PathBuf::from(out["files"][0]["path"].as_str().unwrap());
    assert_eq!(pcf.file_name().unwrap(), "Pixel-Test-8.pcf");

    // `c` had no code in the file; the export gives it its glyph's code point.
    let mut expected = original.clone();
    expected.glyphs[2].code = Some(u32::from('c'));
    let mut fresh = session();
    run(&mut fresh, "bitmap.importStrikes", json!({ "paths": [bdf] }));
    assert_eq!(strikes(&fresh), [expected.clone()], "a BDF export reads back as the same strike");

    let mut fresh = session();
    run(&mut fresh, "bitmap.importStrikes", json!({ "paths": [pcf] }));
    let back = strikes(&fresh).remove(0);
    // PCF adds the properties it needs; the bitmaps and metrics are the same.
    assert_eq!((back.ppem, back.depth, back.ascent, back.descent), (8, 1, 7, 1));
    assert_eq!(back.glyphs, expected.glyphs);

    assert!(refused(&mut s, "bitmap.exportStrikes", json!({ "strikes": [{"ppem": 9}], "path": dir.join("x.bdf") })).contains("no strike 9"));
    assert!(refused(&mut s, "bitmap.exportStrikes", json!({})).contains("destination"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn strikes_are_added_and_removed_with_undo_and_follow_renamed_glyphs() {
    let dir = temp("set");
    let path = bdf_file(&dir, "pixel.bdf", BDF);
    let mut s = session();
    run(&mut s, "bitmap.importStrikes", json!({ "paths": [path] }));

    let out = run(&mut s, "bitmap.setStrikes", json!({ "add": [{"ppem": 16, "depth": 4}, {"ppem": 12}], "rasterize": false }));
    assert_eq!(out["strikes"], json!(["8", "12", "16@4"]));
    assert!(refused(&mut s, "bitmap.setStrikes", json!({ "add": [{"ppem": 20}] })).contains("rasterize"));
    assert!(refused(&mut s, "bitmap.setStrikes", json!({ "add": [{"ppem": 12}], "rasterize": false })).contains("already"));
    assert!(refused(&mut s, "bitmap.setStrikes", json!({ "remove": [{"ppem": 9}] })).contains("no strike 9"));
    assert!(refused(&mut s, "bitmap.setStrikes", json!({})).contains("add"));

    let out = run(&mut s, "bitmap.setStrikes", json!({ "remove": [{"ppem": 8}] }));
    assert_eq!(out["removed"], json!(["8"]));
    assert_eq!(out["strikes"], json!(["12", "16@4"]));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(strikes(&s).iter().map(BitmapStrike::label).collect::<Vec<_>>(), ["8", "12", "16@4"]);
    assert_eq!(strikes(&s)[0].glyphs.len(), 3, "undo brings the removed strike back with its bitmaps");

    run(&mut s, "glyph.rename", json!({ "glyph": "B", "newName": "Bee", "updateInfo": false }));
    let strike = &strikes(&s)[0];
    assert!(strike.glyph("B").is_none() && strike.glyph("Bee").is_some(), "the strike glyph follows the rename");

    // A .glyphs file has no place for strikes, and saving says so; the native file keeps them.
    let saved = run(&mut s, "file.saveAs", json!({ "path": dir.join("Pixel.glyphs") }));
    let loss = saved["losses"].as_array().unwrap().iter().find(|l| l["what"] == "bitmap strikes, which this format has no place for").cloned();
    assert_eq!(loss.map(|l| l["items"].clone()), Some(json!(["8", "12", "16@4"])));
    let saved = run(&mut s, "file.saveAs", json!({ "path": dir.join("Pixel.fontcraft") }));
    assert_eq!(saved["losses"], json!([]));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn hostile_or_unmatched_files_are_refused_and_change_nothing() {
    let dir = temp("hostile");
    let mut s = session();
    let unmatched = BDF
        .replace("STARTCHAR A\nENCODING 65", "STARTCHAR zz\nENCODING 57345")
        .replace("ENCODING 66", "ENCODING 57346")
        .replace("STARTCHAR c\n", "STARTCHAR zzz\n");
    // Each file, and words its message must hold.
    let cases: [(&str, Vec<u8>, &str); 5] = [
        ("wide.bdf", BDF.replace("BBX 1 1 0 0", "BBX 5000 1 0 0").into_bytes(), "limit"),
        ("many.bdf", BDF.replace("STARTPROPERTIES 5", "STARTPROPERTIES 99999").into_bytes(), "limit"),
        ("none.bdf", unmatched.into_bytes(), "none of"),
        ("packed.pcf.gz", vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0], "gzip"),
        ("text.bdf", b"hello".to_vec(), "neither"),
    ];
    for (name, bytes, words) in &cases {
        std::fs::write(dir.join(name), bytes).unwrap();
        let message = refused(&mut s, "bitmap.importStrikes", json!({ "paths": [dir.join(name)] }));
        assert!(message.contains(words), "{name}: {message}");
    }
    assert!(refused(&mut s, "bitmap.importStrikes", json!({ "paths": [] })).contains("empty"));
    assert!(strikes(&s).is_empty());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_strike_holds_whatever_the_bitmap_readers_accept() {
    use fontcraft_engine::model::strike;
    use fontcraft_io_bitmap as io;
    const { assert!(strike::MAX_STRIKE_BITMAP_SIDE >= io::MAX_BITMAP_SIDE) };
    const { assert!(strike::MAX_STRIKE_BYTES >= io::MAX_TOTAL_BITMAP_BYTES) };
    const { assert!(strike::MAX_STRIKE_PROPERTIES >= io::MAX_PROPERTIES) };
    const { assert!(strike::MAX_STRIKE_STRING >= io::MAX_STRING_BYTES) };
    const { assert!(strike::MAX_STRIKE_COORDINATE >= io::MAX_COORDINATE) };
}
