//! Bitmap fonts opened as fonts (`file.open`): a BDF file opens as a bitmap-only font whose
//! glyphs are named and encoded from the file and whose strike holds the bitmaps; a strike
//! written into a compiled font (`EBDT`/`EBLC`, embedded or bitmap-only) opens back as the same
//! strike; hostile bitmap fonts are refused with a message and open nothing.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::PathBuf;

use fontcraft_engine::Session;
use fontcraft_engine::model::{BitmapStrike, SourceFormat, metric};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-bitmap-open-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// An 8 pixel font: `A` and `bee` with Unicode codes, `c` without a code, a second glyph named
/// `A` with the code of `A`, and an `H` that sets the cap height.
const BDF: &str = "STARTFONT 2.1
FONT -Test-Pixel-Medium-R-Normal--8-80-75-75-C-60-ISO10646-1
SIZE 8 75 75
FONTBOUNDINGBOX 6 8 0 -1
STARTPROPERTIES 6
FAMILY_NAME \"Pixel Test\"
PIXEL_SIZE 8
CHARSET_REGISTRY \"ISO10646\"
CHARSET_ENCODING \"1\"
FONT_ASCENT 7
FONT_DESCENT 1
ENDPROPERTIES
CHARS 5
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
BBX 4 5 0 -1
BITMAP
E0
F0
90
F0
80
ENDCHAR
STARTCHAR c
ENCODING -1
SWIDTH 625 0
DWIDTH 5 0
BBX 3 3 1 0
BITMAP
60
80
60
ENDCHAR
STARTCHAR A
ENCODING 65
SWIDTH 750 0
DWIDTH 6 0
BBX 1 1 0 0
BITMAP
80
ENDCHAR
STARTCHAR H
ENCODING 72
SWIDTH 750 0
DWIDTH 6 0
BBX 5 6 0 0
BITMAP
88
88
F8
88
88
88
ENDCHAR
ENDFONT
";

fn open(path: &std::path::Path) -> Session {
    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": path }));
    s
}

fn strikes(s: &Session) -> Vec<BitmapStrike> {
    s.font().unwrap().bitmap_strikes.to_vec()
}

#[test]
fn a_bdf_opens_as_a_bitmap_only_font() {
    let dir = temp("bdf");
    let path = dir.join("pixel.bdf");
    std::fs::write(&path, BDF).unwrap();
    let s = open(&path);
    let font = s.font().unwrap();
    assert_eq!((font.family_name.as_str(), font.units_per_em, font.source.format), ("Pixel Test", 1000, SourceFormat::Binary));
    let glyphs: Vec<(&str, Vec<u32>, f64, usize)> =
        font.glyphs.iter().map(|g| (g.name.as_str(), g.unicodes.clone(), g.layers[0].width, g.layers[0].shapes.len())).collect();
    assert_eq!(
        glyphs,
        [("A", vec![65], 750.0, 0), ("bee", vec![66], 750.0, 0), ("c", vec![], 625.0, 0), ("A.1", vec![], 750.0, 0), ("H", vec![72], 750.0, 0)]
    );
    let m = font.default_master().unwrap();
    assert_eq!(font.metric_pos(m, metric::ASCENDER, 0.0), 875.0);
    assert_eq!(font.metric_pos(m, metric::DESCENDER, 0.0), -125.0);
    assert_eq!(font.metric_pos(m, metric::CAP_HEIGHT, 0.0), 750.0);

    // The strike is the file's, every bitmap under the glyph made for it.
    let file = fontcraft_io_bitmap::read(BDF.as_bytes()).unwrap().font;
    let strike = strikes(&s).remove(0);
    assert_eq!((strike.ppem, strike.depth, strike.ascent, strike.descent), (8, 1, 7, 1));
    assert_eq!(strike.name, file.name);
    assert_eq!(strike.glyphs.len(), file.glyphs.len());
    for (have, want) in strike.glyphs.iter().zip(&file.glyphs) {
        assert_eq!((have.code, have.advance, have.width, have.height), (want.encoding, want.advance, want.bitmap.width, want.bitmap.height));
        assert_eq!((have.left, have.bottom, &have.data), (want.bitmap.left, want.bitmap.bottom, &want.bitmap.data));
    }
    assert_eq!(strike.glyphs[3].glyph, "A.1");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_strike_written_to_a_compiled_font_opens_as_the_same_strike() {
    let dir = temp("sfnt");
    let path = dir.join("pixel.bdf");
    std::fs::write(&path, BDF).unwrap();
    let mut s = open(&path);
    let written = strikes(&s).remove(0);

    let ttf = dir.join("embedded.ttf");
    run(&mut s, "bitmap.exportStrikes", json!({ "format": "embedded", "strikes": [{"ppem": 8}], "path": ttf }));
    let otb = dir.join("pixel.otb");
    run(&mut s, "bitmap.exportStrikes", json!({ "format": "otb", "strikes": [{"ppem": 8}], "path": otb }));
    for path in [ttf, otb] {
        let opened = open(&path);
        let back = strikes(&opened);
        assert_eq!(back.len(), 1, "{}", path.display());
        let back = &back[0];
        // The tables hold the size, depth, line metrics and images; names, codes and the file's
        // properties stay behind.
        assert_eq!((back.ppem, back.depth, back.ascent, back.descent), (written.ppem, written.depth, written.ascent, written.descent));
        let images = |strike: &BitmapStrike| {
            let mut all: Vec<_> =
                strike.glyphs.iter().map(|g| (g.glyph.clone(), g.advance, g.width, g.height, g.left, g.bottom, g.data.clone())).collect();
            all.sort();
            all
        };
        assert_eq!(images(back), images(&written), "{}", path.display());
        let font = opened.font().unwrap();
        assert_eq!(font.glyph("bee").unwrap().layers[0].width, 750.0);
        assert!(font.glyph("bee").unwrap().layers[0].shapes.is_empty());
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn hostile_bitmap_fonts_are_refused_and_open_nothing() {
    let dir = temp("hostile");
    let cases: [(&str, Vec<u8>, &str); 4] = [
        ("wide.bdf", BDF.replace("BBX 1 1 0 0", "BBX 5000 1 0 0").into_bytes(), "limit"),
        ("many.bdf", BDF.replace("STARTPROPERTIES 6", "STARTPROPERTIES 99999").into_bytes(), "limit"),
        ("short.bdf", BDF.replace("88\nENDCHAR\nENDFONT", "ENDCHAR\nENDFONT").into_bytes(), "line"),
        ("cut.pcf", b"\x01fcp\x40\x00\x00\x00".to_vec(), "PCF"),
    ];
    for (name, bytes, words) in &cases {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        let mut s = Session::new();
        let message = s.execute("file.open", &json!({ "path": path })).expect_err(name).to_string();
        assert!(message.contains("bitmap font") && message.contains(words), "{name}: {message}");
        assert!(s.doc().is_err(), "{name}: a refused file opened a document");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
