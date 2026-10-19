//! The core tables against `ttx`'s compiler. The input is the dump the writer tests compare
//! with (`../core/testdata/core.ttx`, written by fontTools 4.66.1 for a font built in code);
//! `testdata/core.digest` holds the length and SHA-256 of each table fontTools 4.66.1 writes
//! from it (`TTFont(recalcTimestamp=False)`, `importXML`, `save`, bounds recalculated), one table
//! a line. `head.checkSumAdjustment` is zeroed before hashing: the font writer fills it. On a
//! mismatch the test prints our bytes as hex, to compare with what `ttx` compiles.
#![allow(clippy::string_slice)]

use super::program::assemble;
use crate::sfnt::{Flavour, SfntFont};
use crate::ttx::oracle_digest::{assert_table, expected};
use crate::ttx::{DumpOptions, ImportOptions, dump, import};

const CORE_TTX: &str = include_str!("../core/testdata/core.ttx");
const CORE_DIGEST: &str = include_str!("testdata/core.digest");

/// The dump without the one record fontTools cannot read back (`&#0;` is not XML).
fn core_ttx() -> String {
    let start = CORE_TTX.find("    <namerecord nameID=\"257\"").unwrap();
    let end = start + CORE_TTX[start..].find("</namerecord>\n").unwrap() + "</namerecord>\n".len();
    format!("{}{}", &CORE_TTX[..start], &CORE_TTX[end..])
}

fn assert_tables(font: &SfntFont, digest: &str) {
    let want = expected(digest);
    for (tag, len, sha) in &want {
        let mut got = font.get(*tag).unwrap_or_else(|| panic!("no '{}' table", String::from_utf8_lossy(tag))).to_vec();
        if tag == b"head" {
            got[8..12].fill(0);
        }
        assert_table(tag, &got, *len, sha);
    }
    assert_eq!(font.tables.len(), want.len(), "no other tables");
}

#[test]
fn core_dump_compiles_as_ttx_compiles() {
    let font = import(SfntFont::new(Flavour::TrueType), &core_ttx(), &ImportOptions::default()).unwrap();
    assert_eq!(font.flavour, Flavour::TrueType);
    assert_tables(&font, CORE_DIGEST);
}

#[test]
fn our_core_dump_reads_back_to_the_same_tables() {
    let font = import(SfntFont::new(Flavour::TrueType), &core_ttx(), &ImportOptions::default()).unwrap();
    let text = dump(&font, &DumpOptions::default()).unwrap();
    let again = import(SfntFont::new(Flavour::TrueType), &text, &ImportOptions::default()).unwrap();
    for (tag, data) in &font.tables {
        assert_eq!(again.get(*tag), Some(&data[..]), "table '{}'", String::from_utf8_lossy(tag));
    }
}

#[test]
fn one_table_merges_into_a_font_and_keeps_the_rest() {
    let font = import(SfntFont::new(Flavour::TrueType), &core_ttx(), &ImportOptions::default()).unwrap();
    let gasp = "<ttFont><gasp><gaspRange rangeMaxPPEM=\"65535\" rangeGaspBehavior=\"3\"/></gasp></ttFont>";
    let merged = import(font.clone(), gasp, &ImportOptions::default()).unwrap();
    assert_eq!(merged.get(*b"gasp"), Some(&[0, 0, 0, 1, 0xFF, 0xFF, 0, 3][..]));
    for (tag, data) in font.tables.iter().filter(|(t, _)| *t != b"gasp") {
        assert_eq!(merged.get(*tag), Some(&data[..]), "table '{}' is untouched", String::from_utf8_lossy(tag));
    }
}

#[test]
fn kept_bounds_are_not_recalculated() {
    let text = core_ttx().replace(
        "<TTGlyph name=\"a.1\" xMin=\"0\" yMin=\"0\" xMax=\"10\" yMax=\"10\">",
        "<TTGlyph name=\"a.1\" xMin=\"1\" yMin=\"2\" xMax=\"3\" yMax=\"4\">",
    );
    let kept = import(SfntFont::new(Flavour::TrueType), &text, &ImportOptions { keep_bounds: true, ..ImportOptions::default() }).unwrap();
    let recalculated = import(SfntFont::new(Flavour::TrueType), &text, &ImportOptions::default()).unwrap();
    let header = |f: &SfntFont| {
        let glyf = f.get(*b"glyf").unwrap();
        let loca = f.get(*b"loca").unwrap();
        let at = usize::from(u16::from_be_bytes([loca[12], loca[13]])) * 2;
        glyf[at + 2..at + 10].to_vec()
    };
    assert_eq!(header(&kept), [0, 1, 0, 2, 0, 3, 0, 4]);
    assert_eq!(header(&recalculated), [0, 0, 0, 0, 0, 10, 0, 10]);
}

#[test]
fn assembly_push_forms_follow_fonttools() {
    assert_eq!(assemble("PUSH[ ] 1 2 300 4 5").unwrap(), [0xb1, 1, 2, 0xb8, 0x01, 0x2c, 0xb1, 4, 5]);
    assert_eq!(assemble("PUSH[ ] 300 4 500").unwrap(), [0xba, 0x01, 0x2c, 0x00, 0x04, 0x01, 0xf4], "a lone byte between words is a word");
    assert_eq!(assemble("NPUSHB[ ]  /* 2 values pushed */\n 7 8\nMDRP[01101]\nINSTR146[ ]").unwrap(), [0x40, 2, 7, 8, 0xcd, 146]);
    assert!(assemble("PUSHB[ ] 300").is_err());
    assert!(assemble("NOSUCH[ ]").is_err());
    assert!(assemble("MDRP[01]").is_err(), "the argument bits must match the instruction");
}

/// A font whose glyphs `c1`…`c64` each use the previous glyph twice at offset `x`, down to
/// `leaf`: 2^64 component visits for code that follows every reference.
fn doubling_chain(leaf: &str, x: &str) -> String {
    let core = core_ttx();
    let tables = &core[core.find("  <head>").unwrap()..core.find("  <hmtx>").unwrap()];
    let names: Vec<String> = [".notdef".to_string(), "leaf".to_string()].into_iter().chain((1..=64).map(|i| format!("c{i}"))).collect();
    let order: String = names.iter().enumerate().map(|(i, n)| format!("<GlyphID id=\"{i}\" name=\"{n}\"/>")).collect();
    let hmtx: String = names.iter().map(|n| format!("<mtx name=\"{n}\" width=\"500\" lsb=\"0\"/>")).collect();
    let mut glyf = format!("<TTGlyph name=\".notdef\"/>{leaf}");
    for i in 1..=64 {
        let prev = if i == 1 { "leaf".to_string() } else { format!("c{}", i - 1) };
        let comp = format!("<component glyphName=\"{prev}\" x=\"{x}\" y=\"0\" flags=\"0x0\"/>");
        glyf.push_str(&format!("<TTGlyph name=\"c{i}\">{comp}{comp}</TTGlyph>"));
    }
    format!(
        "<ttFont sfntVersion=\"\\x00\\x01\\x00\\x00\"><GlyphOrder>{order}</GlyphOrder>{tables}<hmtx>{hmtx}</hmtx><loca/><glyf>{glyf}</glyf></ttFont>"
    )
}

#[test]
fn a_doubling_chain_of_composites_compiles_without_following_every_reference() {
    let empty = "<TTGlyph name=\"leaf\"/>";
    let font = import(SfntFont::new(Flavour::TrueType), &doubling_chain(empty, "0"), &ImportOptions::default()).unwrap();
    let maxp = font.get(*b"maxp").unwrap();
    assert_eq!(&maxp[28..32], [0, 2, 0, 64], "maxComponentElements and maxComponentDepth");
    // Fractional offsets make the bounds come from the flattened points; an empty leaf still
    // costs a visit.
    let err = import(SfntFont::new(Flavour::TrueType), &doubling_chain(empty, "0.5"), &ImportOptions::default()).unwrap_err();
    assert!(err.to_string().contains("flattened"), "{err}");
}
