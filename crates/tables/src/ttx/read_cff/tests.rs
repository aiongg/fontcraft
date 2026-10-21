//! `CFF `, `CFF2` and `VORG` against `ttx`'s compiler. The inputs are the dumps the writer tests
//! compare with (`../cff/testdata/`, written by fontTools 4.66.1 for fonts built in code); the
//! `testdata/*.digest` hold the length and SHA-256 of each table fontTools 4.66.1 compiles from
//! them, one table a line (see `oracle_digest`). fontTools refuses
//! `cff1.ttx` as it is (two charstrings end with operands, and a `VORG` record names a glyph
//! past the glyph order), so that dump is first edited as [`cff1_ttx`] does.
#![allow(clippy::string_slice)]

use super::{encode_float, format_g8};
use crate::sfnt::{Flavour, SfntFont};
use crate::ttx::oracle_digest::{assert_table, expected};
use crate::ttx::{DumpOptions, ImportOptions, dump, import};

const CFF1_TTX: &str = include_str!("../cff/testdata/cff1.ttx");
const CID_TTX: &str = include_str!("../cff/testdata/cid.ttx");
const CFF2_TTX: &str = include_str!("../cff/testdata/cff2.ttx");

fn cff1_ttx() -> String {
    let s = CFF1_TTX.replace("          5</CharString>", "          endchar\n        </CharString>");
    let s = s.replace("          100 200</CharString>", "          100 200 rlineto\n          endchar\n        </CharString>");
    let start = s.find("    <VOriginRecord>\n      <glyphName value=\"glyph00009\"/>").unwrap();
    let end = start + s[start..].find("</VOriginRecord>\n").unwrap() + "</VOriginRecord>\n".len();
    format!("{}{}", &s[..start], &s[end..])
}

fn check(text: &str, digest: &str) -> SfntFont {
    let font = import(SfntFont::new(Flavour::Cff), text, &ImportOptions::default()).unwrap();
    for (tag, len, sha) in expected(digest) {
        let got = font.get(tag).unwrap_or_else(|| panic!("no '{}' table", String::from_utf8_lossy(&tag)));
        assert_table(&tag, got, len, &sha);
    }
    font
}

#[test]
fn cff_dumps_compile_as_ttx_compiles() {
    check(&cff1_ttx(), include_str!("testdata/cff1.digest"));
    check(CID_TTX, include_str!("testdata/cid.digest"));
    check(CFF2_TTX, include_str!("testdata/cff2.digest"));
}

#[test]
fn our_cff_dumps_read_back_to_the_same_tables() {
    for text in [cff1_ttx(), CID_TTX.to_string(), CFF2_TTX.to_string()] {
        let font = import(SfntFont::new(Flavour::Cff), &text, &ImportOptions::default()).unwrap();
        let again = import(SfntFont::new(Flavour::Cff), &dump(&font, &DumpOptions::default()).unwrap(), &ImportOptions::default()).unwrap();
        for (tag, data) in &font.tables {
            assert_eq!(again.get(*tag), Some(&data[..]), "table '{}'", String::from_utf8_lossy(tag));
        }
    }
}

#[test]
fn operands_after_the_last_operator_are_refused_in_cff() {
    let err = import(SfntFont::new(Flavour::Cff), CFF1_TTX, &ImportOptions::default()).unwrap_err();
    assert!(err.to_string().contains("after last operator"), "{err}");
}

#[test]
fn reals_are_encoded_as_fonttools_encodes_them() {
    let enc = |f: f64| {
        let mut out = Vec::new();
        encode_float(f, &mut out).unwrap();
        out
    };
    assert_eq!(format_g8(0.039625), "0.039625");
    assert_eq!(format_g8(1e-5), "1E-05");
    assert_eq!(format_g8(123456789.0), "1.2345679E+08");
    assert_eq!(enc(0.0), [30, 0x0f]);
    assert_eq!(enc(0.039625), [30, 0x39, 0x62, 0x5c, 0x6f]);
    assert_eq!(enc(0.00015), [30, 0x15, 0xc5, 0xff]);
    assert_eq!(enc(-0.5), [30, 0xea, 0x5f]);
    assert_eq!(enc(1e-5), [30, 0x1c, 0x5f]);
    assert_eq!(enc(1000.0), [30, 0x1b, 0x3f]);
}

// ── Bounds ─────────────────────────────────────────────────────────────────────────────────────
//
// `testdata/bounds.ttx` is a CFF font whose `FontBBox`, `head` box and `hhea` extremes are wrong
// (1 2 3 4); glyph `a` is a curve whose top (750.75) lies between its points, and `Notice` holds a
// line break. `testdata/bounds-cff2.ttx` draws the same glyphs in `CFF2`. The expected values are
// what fontTools 4.66.1 gives (`TTFont.importXML`, then `save`, `recalcTimestamp` off).

const BOUNDS_TTX: &str = include_str!("testdata/bounds.ttx");
const BOUNDS_CFF2_TTX: &str = include_str!("testdata/bounds-cff2.ttx");
const BOUNDS: [i64; 4] = [-20, -150, 580, 751];
/// `advanceWidthMax`, `minLeftSideBearing`, `minRightSideBearing`, `xMaxExtent`.
const HHEA: [i64; 4] = [600, -20, -20, 580];
const AS_GIVEN: [i64; 4] = [1, 2, 3, 4];
const CFF_RECALCULATED: (usize, &str) = (118, "9d5d01e77f422056b53f6373cf5d4b54ea0be217e91ce81a7bd49ef2c32ba805");
const CFF_AS_GIVEN: (usize, &str) = (115, "ee2b85b57191fb4845ffed0c0784721a4f707dfa8e888991d8e9fd4a1cf6ef85");

/// The elements `names` of the TTX `text`, in that order.
fn elements(text: &str, names: &[&str]) -> String {
    let mut out = String::new();
    for name in names {
        let start = text.find(&format!("  <{name}>")).unwrap();
        let close = format!("  </{name}>\n");
        let end = start + text[start..].find(&close).unwrap() + close.len();
        out.push_str(&text[start..end]);
    }
    out
}

fn document(body: &str) -> String {
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<ttFont sfntVersion=\"OTTO\" ttLibVersion=\"4.66\">\n{body}</ttFont>\n")
}

fn read_bounds(base: SfntFont, text: &str, keep_bounds: bool) -> SfntFont {
    import(base, text, &ImportOptions { keep_bounds, ..ImportOptions::default() }).unwrap()
}

fn fields(font: &SfntFont, tag: &[u8; 4], offsets: [usize; 4]) -> [i64; 4] {
    let data = font.get(*tag).unwrap();
    offsets.map(|at| i64::from(i16::from_be_bytes([data[at], data[at + 1]])))
}

fn head_box(font: &SfntFont) -> [i64; 4] {
    fields(font, b"head", [36, 38, 40, 42])
}

fn hhea_extremes(font: &SfntFont) -> [i64; 4] {
    fields(font, b"hhea", [10, 12, 14, 16])
}

fn assert_cff(font: &SfntFont, (len, sha): (usize, &str)) {
    assert_table(b"CFF ", font.get(*b"CFF ").unwrap(), len, sha);
}

#[test]
fn ttx_read_cff_fonts_get_the_bounds_fonttools_computes() {
    let font = read_bounds(SfntFont::new(Flavour::Cff), BOUNDS_TTX, false);
    assert_cff(&font, CFF_RECALCULATED);
    assert_eq!(head_box(&font), BOUNDS);
    assert_eq!(hhea_extremes(&font), HHEA);

    let kept = read_bounds(SfntFont::new(Flavour::Cff), BOUNDS_TTX, true);
    assert_cff(&kept, CFF_AS_GIVEN);
    assert_eq!(head_box(&kept), AS_GIVEN);
    assert_eq!(hhea_extremes(&kept), AS_GIVEN);

    // Without outlines the FontBBox is the default, zeros, left out of the Top DICT; head says zeros.
    let empty = BOUNDS_TTX
        .replace("100 0 rmoveto 0 1001 400 0 0 -1001 rrcurveto endchar", "endchar")
        .replace("-20 -150 rmoveto 600 hlineto 900 vlineto -600 hlineto endchar", "endchar");
    let empty = read_bounds(SfntFont::new(Flavour::Cff), &empty, false);
    assert_cff(&empty, (83, "d0ba6f7a9a3a5f343044c7b37d5ed1087c9075ec2a3ff29f06fdd93831d0d793"));
    assert_eq!(head_box(&empty), [0; 4]);
    assert_eq!(hhea_extremes(&empty), [600, 0, 0, 0]);
}

/// fontTools reads the glyph order of a font with `maxp` and `post` before the text, which loads
/// a CFF font's `CFF ` table: the save compiles it again, so a text without `CFF ` still gets its
/// FontBBox, `head` box and `hhea` extremes recalculated. Compiling `head` loads `CFF2`, so a
/// CFF2 font's `hhea` is recalculated too.
#[test]
fn ttx_read_into_a_cff_font_recalculates_the_fonts_bounds() {
    let metrics = document(&elements(BOUNDS_TTX, &["head", "hhea", "hmtx"]));
    let base = read_bounds(SfntFont::new(Flavour::Cff), BOUNDS_TTX, true);

    let font = read_bounds(base.clone(), &metrics, false);
    assert_cff(&font, CFF_RECALCULATED);
    assert!(font.get(*b"CFF ").unwrap().windows(7).any(|w| w == b"one\ntwo"), "the line break in Notice is kept");
    assert_eq!(head_box(&font), BOUNDS);
    assert_eq!(hhea_extremes(&font), HHEA);

    let kept = read_bounds(base, &metrics, true);
    assert_cff(&kept, CFF_AS_GIVEN);
    assert_eq!(head_box(&kept), AS_GIVEN);
    assert_eq!(hhea_extremes(&kept), AS_GIVEN);

    let cff2 = elements(BOUNDS_TTX, &["GlyphOrder", "maxp", "post", "head", "hhea", "hmtx"]) + &elements(BOUNDS_CFF2_TTX, &["CFF2"]);
    let base = read_bounds(SfntFont::new(Flavour::Cff), &document(&cff2), true);
    assert_eq!(hhea_extremes(&base), AS_GIVEN);
    let font = read_bounds(base, &document(&elements(BOUNDS_TTX, &["GlyphOrder", "head", "hhea", "hmtx"])), false);
    assert_eq!(head_box(&font), BOUNDS);
    assert_eq!(hhea_extremes(&font), HHEA);
}
