//! Bitmap strikes (`bitmap.importStrikes`, `bitmap.exportStrikes`, `bitmap.setStrikes`): a BDF
//! file goes into a strike by code then name, exports back to the same strike as BDF and PCF,
//! strikes are added and removed with undo, renames carry strike glyphs along, formats without a
//! place for strikes report them, and hostile files fail without changing the font. New strikes
//! are rasterised from the outlines, and strikes go into compiled fonts as `EBDT`/`EBLC`, embedded
//! or as a bitmap-only font, which the TTX reader and writer read back unchanged.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, clippy::string_slice)]

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

/// A strike written as a TeX PK file reads back with the same bitmaps, by import and by open.
#[test]
fn a_strike_exports_as_pk_and_reads_back() {
    let dir = temp("pk");
    let path = bdf_file(&dir, "pixel.bdf", BDF);
    let mut s = session();
    run(&mut s, "bitmap.importStrikes", json!({ "paths": [path] }));
    let mut expected = strikes(&s).remove(0);
    expected.glyphs[2].code = Some(u32::from('c'));
    let pk = dir.join("pixel.75pk");
    let out = run(&mut s, "bitmap.exportStrikes", json!({ "path": pk }));
    assert_eq!(out["format"], "pk");

    let mut fresh = session();
    run(&mut fresh, "bitmap.importStrikes", json!({ "paths": [pk] }));
    let back = strikes(&fresh).remove(0);
    assert_eq!((back.ppem, back.depth, back.resolution), (8, 1, [75, 75]));
    assert_eq!(back.glyphs, expected.glyphs);

    let mut opened = Session::new();
    run(&mut opened, "file.open", json!({ "path": pk }));
    let font = opened.font().unwrap();
    let names: Vec<&str> = font.glyphs.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(names, ["A", "B", "c"]);
    assert_eq!(font.bitmap_strikes.len(), 1);
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
    assert!(refused(&mut s, "bitmap.setStrikes", json!({ "add": [{"ppem": 20, "depth": 3}] })).contains("depth"));
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
        ("text.bdf", b"hello".to_vec(), "not a bitmap font"),
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

#[test]
fn removing_a_glyph_removes_its_bitmaps_and_undo_brings_them_back() {
    let dir = temp("remove-glyph");
    let path = bdf_file(&dir, "pixel.bdf", BDF);
    let mut s = session();
    run(&mut s, "bitmap.importStrikes", json!({ "paths": [path] }));
    let names = |s: &Session| strikes(s)[0].glyphs.iter().map(|g| g.glyph.clone()).collect::<Vec<_>>();
    run(&mut s, "glyph.removeGlyph", json!({ "glyphs": ["A"] }));
    assert_eq!(names(&s), ["B", "c"]);
    // A new glyph with the removed glyph's name starts without a bitmap.
    run(&mut s, "glyph.newGlyph", json!({ "name": "A" }));
    assert!(strikes(&s)[0].glyph("A").is_none());
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(names(&s), ["A", "B", "c"], "undo brings the bitmap back");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A document whose `A` is a 500-unit square, `B` the same with its top a quarter pixel above
/// row 8 at 16 ppem, and `C` three quarters.
fn squares() -> Session {
    let mut s = Session::with_document();
    for (glyph, top) in [("A", 500.0), ("B", 515.625), ("C", 546.875)] {
        run(&mut s, "path.addPath", json!({"glyph": glyph, "nodes": fontcraft_testkit::rect(0.0, 0.0, 500.0, top), "closed": true}));
    }
    s
}

/// A document with `count` glyphs, each a 500-unit square: 1,024 × 1,024 pixels at 2,048 ppem.
/// The new document's `.notdef` is removed: it would be over 1,024 pixels tall.
fn big_squares(count: usize) -> Session {
    let mut s = Session::with_document();
    run(&mut s, "glyph.removeGlyph", json!({ "glyphs": [".notdef"] }));
    for i in 0..count {
        let name = format!("sq{i}");
        run(&mut s, "glyph.newGlyph", json!({ "name": name }));
        run(&mut s, "path.addPath", json!({"glyph": name, "nodes": fontcraft_testkit::rect(0.0, 0.0, 500.0, 500.0), "closed": true}));
    }
    s
}

/// Rasterising measures every new strike before drawing any, and refuses a bitmap over 1,024
/// pixels a side, a strike over 64 Mi pixels and strikes over 256 Mi pixels together, changing
/// nothing.
#[test]
fn rasterising_refuses_oversized_bitmaps_and_strikes_before_drawing() {
    let mut s = squares();
    let e = refused(&mut s, "bitmap.setStrikes", json!({ "add": [{"ppem": 2100}] }));
    // The new document's `.notdef` (400 × 700 units) is measured first.
    assert!(e.contains("glyph `.notdef` would be 840 × 1470") && e.contains("at most 1024 on a side"), "{e}");
    assert!(strikes(&s).is_empty());

    // 65 bitmaps of 1 Mi pixels each.
    let mut s = big_squares(65);
    let e = refused(&mut s, "bitmap.setStrikes", json!({ "add": [{"ppem": 2048, "depth": 8}] }));
    assert!(e.contains("more than 67108864 pixels"), "{e}");

    // Under the limit for each strike, over it for five together.
    let mut s = big_squares(60);
    let add: Vec<Value> = (0..5).map(|i| json!({ "ppem": 2048 - i })).collect();
    let e = refused(&mut s, "bitmap.setStrikes", json!({ "add": add }));
    assert!(e.contains("more than 268435456 pixels to draw for the strikes added together"), "{e}");
    assert!(strikes(&s).is_empty());
}

#[test]
fn a_new_strike_is_rasterised_from_the_outlines() {
    let mut s = squares();
    let out = run(&mut s, "bitmap.setStrikes", json!({ "add": [{"ppem": 16}, {"ppem": 16, "depth": 8}] }));
    assert_eq!(out["added"], json!(["16", "16@8"]));
    let all = strikes(&s);
    let (mono, grey) = (&all[0], &all[1]);
    assert_eq!((mono.ascent, mono.descent), (13, 3), "800 and -200 units at 16 ppem");

    let a = mono.glyph("A").unwrap();
    assert_eq!((a.width, a.height, a.left, a.bottom, a.advance), (8, 8, 0, 0, 10));
    assert_eq!(a.data, vec![0xff; 8]);
    let b = mono.glyph("B").unwrap();
    assert_eq!((b.width, b.height), (8, 9));
    assert_eq!(b.data[0], 0, "a quarter-covered row is off at 1 bit");
    assert!(b.data[1..].iter().all(|r| *r == 0xff));
    assert_eq!(mono.glyph("C").unwrap().data[0], 0xff, "a row covered three quarters is on");
    let space = mono.glyph("space").unwrap();
    assert_eq!((space.width, space.height, space.advance, space.data.len()), (0, 0, 4, 0));

    assert_eq!(grey.glyph("A").unwrap().data, vec![0xff; 64]);
    let b = grey.glyph("B").unwrap();
    assert_eq!(b.data.len(), 72);
    assert!(b.data[..8].iter().all(|v| (48..=80).contains(v)), "a quarter-covered row is a quarter grey: {:?}", &b.data[..8]);

    run(&mut s, "edit.undo", json!({}));
    assert!(strikes(&s).is_empty());
}

#[test]
fn strikes_go_into_compiled_fonts_and_read_back_through_ttx() {
    use fontcraft_tables::ttx::{DumpOptions, ImportOptions, dump, import};
    let dir = temp("sfnt");
    let mut s = squares();
    run(&mut s, "bitmap.setStrikes", json!({ "add": [{"ppem": 16}, {"ppem": 300}], "remove": [], "rasterize": true }));

    let otb = dir.join("Squares.otb");
    let out = run(&mut s, "bitmap.exportStrikes", json!({ "strikes": [{"ppem": 16}], "path": otb }));
    assert_eq!(out["format"], "otb");
    assert_eq!(out["files"][0]["strike"], "16");
    let font = fontcraft_tables::read(&std::fs::read(&otb).unwrap(), 0).unwrap();
    for tag in [b"head", b"hhea", b"hmtx", b"maxp", b"cmap", b"name", b"OS/2", b"post", b"glyf", b"loca", b"EBDT", b"EBLC"] {
        assert!(font.get(*tag).is_some(), "{}", String::from_utf8_lossy(tag));
    }
    assert_eq!(font.tables.len(), 12);
    assert_eq!(font.get(*b"glyf"), Some(&[][..]));

    let options = DumpOptions { tables: vec![*b"EBLC", *b"EBDT"], strict: true, ..DumpOptions::default() };
    let text = dump(&font, &options).unwrap();
    let a = text.find("<ebdt_bitmap_format_1 name=\"A\">").expect("A has a bitmap");
    assert!(text[a..].contains("ffffffff ffffffff"), "{}", &text[a..a + 400]);
    let again = import(font.clone(), &text, &ImportOptions::default()).unwrap();
    assert_eq!(again.get(*b"EBDT"), font.get(*b"EBDT"));
    assert_eq!(again.get(*b"EBLC"), font.get(*b"EBLC"));

    // Embedded in the open font compiled, then into that file again: the same tables each time.
    let ttf = dir.join("Squares.ttf");
    let out = run(&mut s, "bitmap.exportStrikes", json!({ "format": "embedded", "strikes": [{"ppem": 16}], "path": ttf }));
    assert_eq!(out["replaced"], false);
    let embedded = fontcraft_tables::read(&std::fs::read(&ttf).unwrap(), 0).unwrap();
    assert!(!embedded.get(*b"glyf").unwrap().is_empty());
    assert_eq!(embedded.get(*b"EBLC"), font.get(*b"EBLC"));
    let again = dir.join("Again.ttf");
    let out = run(&mut s, "bitmap.exportStrikes", json!({ "format": "embedded", "strikes": [{"ppem": 16}], "font": ttf, "path": again }));
    assert_eq!(out["replaced"], true);
    let again = fontcraft_tables::read(&std::fs::read(&again).unwrap(), 0).unwrap();
    assert_eq!(again.get(*b"EBDT"), font.get(*b"EBDT"));
    assert_eq!(again.get(*b"EBLC"), font.get(*b"EBLC"));

    let cases = [
        (json!({ "format": "embedded", "strikes": [{"ppem": 300}], "path": dir.join("big.ttf") }), "255"),
        (json!({ "format": "otb", "font": ttf, "path": dir.join("x.otb") }), "embedded"),
        (json!({ "format": "embedded", "destination": dir }), "path"),
        (json!({ "format": "bdf", "font": ttf, "path": dir.join("x.bdf") }), "embedded"),
    ];
    for (params, words) in cases {
        let message = refused(&mut s, "bitmap.exportStrikes", params.clone());
        assert!(message.contains(words), "{params}: {message}");
    }
    std::fs::write(dir.join("text.ttf"), b"hello").unwrap();
    let message = refused(&mut s, "bitmap.exportStrikes", json!({ "format": "embedded", "font": dir.join("text.ttf"), "path": dir.join("y.ttf") }));
    assert!(message.contains("not a TrueType"), "{message}");
    assert!(!dir.join("big.ttf").exists() && !dir.join("y.ttf").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Each bitmap of a strike by glyph name: its code, advance and lit pixels from the origin (x
/// to the right, y up), so strikes whose bitmaps are boxed differently compare alike.
fn shapes(strike: &BitmapStrike) -> Vec<(String, Option<u32>, i32, std::collections::BTreeSet<(i32, i32)>)> {
    let mut out: Vec<_> = strike
        .glyphs
        .iter()
        .map(|g| {
            let stride = usize::from(g.width).div_ceil(8);
            let mut ink = std::collections::BTreeSet::new();
            for y in 0..usize::from(g.height) {
                for x in 0..usize::from(g.width) {
                    if g.data[y * stride + x / 8] & (0x80 >> (x % 8)) != 0 {
                        ink.insert((g.left + x as i32, g.bottom + i32::from(g.height) - 1 - y as i32));
                    }
                }
            }
            (g.glyph.clone(), g.code, g.advance, ink)
        })
        .collect();
    out.sort();
    out
}

#[test]
fn a_strike_exports_as_pcf_gz_fnt_and_fon_and_reads_back() {
    let dir = temp("windows");
    let path = bdf_file(&dir, "pixel.bdf", BDF);
    let mut s = session();
    run(&mut s, "bitmap.importStrikes", json!({ "paths": [path] }));
    let mut expected = strikes(&s).remove(0);
    expected.glyphs[2].code = Some(u32::from('c'));

    let gz = dir.join("out.pcf.gz");
    let out = run(&mut s, "bitmap.exportStrikes", json!({ "path": gz }));
    assert_eq!(out["format"], "pcf.gz");
    let out = run(&mut s, "bitmap.exportStrikes", json!({ "format": "fnt", "destination": dir.join("fnt") }));
    let fnt = PathBuf::from(out["files"][0]["path"].as_str().unwrap());
    assert_eq!(fnt.file_name().unwrap(), "Pixel-Test-8.fnt");
    assert!(out["files"][0].get("warnings").is_none(), "{out}");
    let fon = dir.join("out.fon");
    let out = run(&mut s, "bitmap.exportStrikes", json!({ "path": fon }));
    assert_eq!((&out["format"], &out["files"][0]["strikes"]), (&json!("fon"), &json!(["8"])));

    let mut fresh = session();
    run(&mut fresh, "bitmap.importStrikes", json!({ "paths": [gz] }));
    assert_eq!(strikes(&fresh)[0].glyphs, expected.glyphs, "a .pcf.gz reads back as the PCF it holds");
    // FNT bitmaps are cells as wide as the advance and as tall as the strike; the pixels are the same.
    for path in [&fnt, &fon] {
        let mut fresh = session();
        run(&mut fresh, "bitmap.importStrikes", json!({ "paths": [path] }));
        let back = strikes(&fresh).remove(0);
        assert_eq!((back.ppem, back.depth, back.ascent, back.descent), (8, 1, 7, 1), "{}", path.display());
        assert_eq!(shapes(&back), shapes(&expected), "{}", path.display());
    }
    assert!(refused(&mut s, "bitmap.exportStrikes", json!({ "format": "fon", "destination": dir })).contains("path"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_fon_opens_with_a_strike_per_size_and_a_pcf_gz_opens() {
    use fontcraft_io_bitmap as io;
    let dir = temp("open-windows");
    let small = io::read_bdf(BDF.as_bytes()).unwrap().font;
    let mut large = small.clone();
    (large.pixel_size, large.ascent) = (10, 9);
    let fon = dir.join("pair.fon");
    let written = io::write_fon(&[small.clone(), large]).unwrap();
    // `c` has no code and the private-use glyph none in code page 1252.
    assert!(written.warnings.iter().any(|w| w.contains("2 glyphs have no code")), "{:?}", written.warnings);
    std::fs::write(&fon, written.bytes).unwrap();
    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": fon }));
    let names: Vec<String> = s.font().unwrap().glyphs.iter().map(|g| g.name.clone()).collect();
    assert_eq!(names, ["uni0041", "uni0042"]);
    let got = strikes(&s);
    let sizes: Vec<(u16, usize)> = got.iter().map(|s| (s.ppem, s.glyphs.len())).collect();
    assert_eq!(sizes, [(8, 2), (10, 2)]);

    let gz = dir.join("pixel.pcf.gz");
    std::fs::write(&gz, io::write_pcf_gz(&small, io::PcfLayout::default()).unwrap()).unwrap();
    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": gz }));
    let font = s.font().unwrap();
    assert_eq!((font.family_name.as_str(), font.glyphs.len()), ("pixel", 4));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Two fonts of one size in one FON file import as one strike, the first, with a warning; they
/// are not merged.
#[test]
fn a_fon_with_two_fonts_of_one_size_imports_the_first() {
    use fontcraft_io_bitmap as io;
    let dir = temp("fon-one-size");
    let regular = io::read_bdf(BDF.as_bytes()).unwrap().font;
    let mut bold = regular.clone();
    bold.name = "Pixel Bold".into();
    bold.glyphs[0].bitmap.data = vec![0xF8; 4];
    let both = dir.join("both.fon");
    std::fs::write(&both, io::write_fon(&[regular.clone(), bold]).unwrap().bytes).unwrap();
    let first = dir.join("first.fon");
    std::fs::write(&first, io::write_fon(&[regular]).unwrap().bytes).unwrap();

    let mut s = session();
    let out = run(&mut s, "bitmap.importStrikes", json!({ "paths": [both] }));
    assert_eq!(out["strikes"].as_array().unwrap().len(), 1, "{out}");
    assert_eq!(out["strikes"][0]["replaced"], false);
    let warnings = out["warnings"].as_array().unwrap();
    assert!(warnings.iter().any(|w| w.as_str().unwrap().contains("a further strike of 8 was left out")), "{out}");
    let mut alone = session();
    run(&mut alone, "bitmap.importStrikes", json!({ "paths": [first] }));
    assert_eq!(strikes(&s), strikes(&alone), "the first font's strike, untouched by the second");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A FON whose symbol font matches no glyph imports its other font, with a warning.
#[test]
fn a_fon_whose_symbol_font_matches_nothing_imports_the_other() {
    use fontcraft_io_bitmap as io;
    let dir = temp("fon-symbol");
    let ansi = io::read_bdf(BDF.as_bytes()).unwrap().font;
    let mut symbol = ansi.clone();
    (symbol.pixel_size, symbol.ascent) = (10, 9);
    for p in &mut symbol.properties {
        match p.name.as_str() {
            "CHARSET_REGISTRY" => p.value = io::PropertyValue::Text("MICROSOFT".into()),
            "CHARSET_ENCODING" => p.value = io::PropertyValue::Text("CHARSET2".into()),
            "PIXEL_SIZE" => p.value = io::PropertyValue::Integer(10),
            _ => {}
        }
    }
    let fon = dir.join("pair.fon");
    std::fs::write(&fon, io::write_fon(&[ansi, symbol]).unwrap().bytes).unwrap();
    let mut s = session();
    let out = run(&mut s, "bitmap.importStrikes", json!({ "paths": [fon] }));
    assert_eq!(out["strikes"].as_array().unwrap().len(), 1, "{out}");
    assert_eq!(out["strikes"][0]["ppem"], 8);
    let warnings = out["warnings"].as_array().unwrap();
    assert!(warnings.iter().any(|w| w.as_str().unwrap().contains("strike was left out: none of its")), "{out}");
    assert_eq!(strikes(&s).len(), 1);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// TeX codes are not Unicode: a PK character of code 123 (an en dash in cmr10) does not go to
/// the font's `braceleft` (U+007B); letters still match by name.
#[test]
fn pk_codes_match_glyphs_by_name_only() {
    use fontcraft_io_bitmap as io;
    let dir = temp("pk-codes");
    let mut tex = io::read_bdf(BDF.as_bytes()).unwrap().font;
    tex.glyphs[1].encoding = Some(123);
    let pk = dir.join("tex.72pk");
    std::fs::write(&pk, io::write_pk(&tex).unwrap()).unwrap();
    let mut s = session();
    run(&mut s, "glyph.newGlyph", json!({ "name": "braceleft", "unicodes": [0x7B] }));
    let out = run(&mut s, "bitmap.importStrikes", json!({ "paths": [pk] }));
    let left_out = out["strikes"][0]["leftOut"].as_array().unwrap();
    assert!(left_out.contains(&json!("char123")), "{out}");
    let strike = strikes(&s).remove(0);
    assert!(strike.glyph("braceleft").is_none());
    assert_eq!(strike.glyph("A").unwrap().code, Some(65));
    std::fs::remove_dir_all(&dir).unwrap();
}
