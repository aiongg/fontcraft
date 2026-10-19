//! The colour tables against `ttx`. The font is built in `fonts.rs` table by table; the expected
//! text in `testdata/` is what fontTools 4.66.1's `ttx` writes for the same bytes, with comments
//! taken out.

use super::fonts;
use crate::sfnt::Tag;
use crate::ttx::var::tests::{assert_dump, assert_truncations_survive};
use crate::ttx::{DumpOptions, dump};

const TAGS: [Tag; 4] = [*b"COLR", *b"CPAL", *b"SVG ", *b"sbix"];

#[test]
fn colour_tables_match_ttx() {
    assert_dump(&fonts::color_font(), &TAGS, include_str!("testdata/color.ttx"));
}

#[test]
fn every_truncation_of_the_colour_tables_is_survived() {
    assert_truncations_survive(&fonts::color_font(), &TAGS);
}

#[test]
fn compressed_svg_and_duplicate_strikes_are_written_as_hex() {
    let mut font = fonts::color_font();
    let mut svg = font.get(*b"SVG ").unwrap().to_vec();
    // The first document starts at 10 (the index) + 26.
    svg[36] = 0x1F;
    svg[37] = 0x8B;
    font.insert(*b"SVG ", svg);
    let mut sbix = font.get(*b"sbix").unwrap().to_vec();
    // The second strike's ppem (10) becomes the first's (20).
    let second = u32::from_be_bytes(sbix[12..16].try_into().unwrap()) as usize;
    sbix[second + 1] = 20;
    font.insert(*b"sbix", sbix);
    let options = DumpOptions { tables: vec![*b"SVG ", *b"sbix"], ..DumpOptions::default() };
    let text = dump(&font, &options).unwrap();
    assert!(text.contains("<SVG ERROR=\"decompilation error\" raw=\"True\">"), "{text}");
    assert!(text.contains("<sbix ERROR=\"decompilation error\" raw=\"True\">"), "{text}");
}

#[test]
fn colr_version_1_is_not_written_as_version_0() {
    let mut font = fonts::color_font();
    let mut colr = font.get(*b"COLR").unwrap().to_vec();
    colr[1] = 1;
    font.insert(*b"COLR", colr);
    let options = DumpOptions { tables: vec![*b"COLR"], ..DumpOptions::default() };
    let text = dump(&font, &options).unwrap();
    assert!(!text.contains("<ColorGlyph"), "{text}");
}

const BITMAP_TAGS: [Tag; 4] = [*b"EBDT", *b"EBLC", *b"bdat", *b"bloc"];

#[test]
fn colr_version_1_matches_ttx() {
    assert_dump(&fonts::colr_v1_font(), &[*b"COLR"], include_str!("testdata/colr1.ttx"));
}

#[test]
fn bitmap_tables_match_ttx() {
    assert_dump(&fonts::bitmap_font(), &BITMAP_TAGS, include_str!("testdata/bitmap.ttx"));
}

#[test]
fn colour_bitmap_tables_match_ttx() {
    assert_dump(&fonts::cbdt_font(), &[*b"CBDT", *b"CBLC"], include_str!("testdata/cbdt.ttx"));
}

#[test]
fn every_truncation_of_colr_1_and_the_bitmap_tables_is_survived() {
    assert_truncations_survive(&fonts::colr_v1_font(), &[*b"COLR"]);
    assert_truncations_survive(&fonts::bitmap_font(), &BITMAP_TAGS);
    assert_truncations_survive(&fonts::cbdt_font(), &[*b"CBDT", *b"CBLC"]);
}

#[test]
fn an_unknown_paint_format_is_written_as_hex() {
    let mut font = fonts::colr_v1_font();
    let mut colr = font.get(*b"COLR").unwrap().to_vec();
    // The PaintColrLayers of glyph A: the base glyph list at 48, its paint 89 further on.
    assert_eq!(colr[137], 1);
    colr[137] = 33;
    font.insert(*b"COLR", colr);
    let options = DumpOptions { tables: vec![*b"COLR"], ..DumpOptions::default() };
    let text = dump(&font, &options).unwrap();
    assert!(text.contains("<COLR ERROR=\"decompilation error\" raw=\"True\">"), "{text}");
}

#[test]
fn a_bitmap_data_table_without_its_location_table_is_written_as_hex() {
    let mut font = fonts::bitmap_font();
    font.remove(*b"EBLC");
    let options = DumpOptions { tables: vec![*b"EBDT"], ..DumpOptions::default() };
    let text = dump(&font, &options).unwrap();
    assert!(text.contains("<EBDT ERROR=\"decompilation error\" raw=\"True\">"), "{text}");
}

/// `ttx` 4.66 cannot write component bitmaps (it stops with an error), so these lines follow
/// what its code means to write: each component's glyph name and offsets.
#[test]
fn component_bitmaps_are_written_with_their_offsets() {
    let mut font = fonts::cbdt_font();
    let mut ebdt = fonts::Be::default();
    ebdt.u32s(&[0x20000]);
    ebdt.u8s(&[5, 5, 0, 5, 6, 0xFE, 0, 6]);
    ebdt.u16s(&[1, 2]);
    ebdt.i8s(&[4, -4]);
    let mut eblc = fonts::Be::default();
    eblc.u32s(&[0x20000, 1]);
    eblc.u32s(&[56, 24, 1, 0]);
    eblc.u8s(&[0; 24]);
    eblc.u16s(&[1, 1]);
    eblc.u8s(&[12, 12, 1, 1]);
    eblc.u16s(&[1, 1]);
    eblc.u32s(&[8]);
    eblc.u16s(&[1, 9]);
    eblc.u32s(&[4, 0, 14]);
    font.insert(*b"EBDT", ebdt.0);
    font.insert(*b"EBLC", eblc.0);
    let options = DumpOptions { tables: vec![*b"EBDT"], ..DumpOptions::default() };
    let text = dump(&font, &options).unwrap();
    let expected = [
        "      <ebdt_bitmap_format_9 name=\"A\">",
        "        <BigGlyphMetrics>",
        "          <vertBearingX value=\"-2\"/>",
        "        </BigGlyphMetrics>",
        "        <components>",
        "          <ebdtComponent name=\"B\">",
        "            <xOffset value=\"4\"/>",
        "            <yOffset value=\"-4\"/>",
        "          </ebdtComponent>",
        "        </components>",
    ];
    for line in expected {
        assert!(text.lines().any(|l| l == line), "{line}\n{text}");
    }
}

#[test]
fn strikes_sharing_one_large_subtable_array_are_refused_quickly() {
    // 20,000 strikes, each listing the same 131,072 subtables of no glyph: 2.6e9 subtables.
    const STRIKES: u32 = 20_000;
    const SUBTABLES: u32 = 131_072;
    let array = 8 + 48 * STRIKES;
    let mut eblc = fonts::Be::default();
    eblc.u32s(&[0x20000, STRIKES]);
    for _ in 0..STRIKES {
        eblc.u32s(&[array, 0, SUBTABLES]);
        eblc.u8s(&[0; 36]);
    }
    for _ in 0..SUBTABLES {
        eblc.u16s(&[1, 0]);
        eblc.u32s(&[8 * SUBTABLES]);
    }
    eblc.u16s(&[2, 1]);
    eblc.u32s(&[0, 0]);
    eblc.u8s(&[0; 8]);
    let mut font = fonts::bitmap_font();
    font.insert(*b"EBLC", eblc.0);
    let options = DumpOptions { tables: vec![*b"EBLC"], ..DumpOptions::default() };
    match dump(&font, &options) {
        Ok(text) => assert!(text.contains("<EBLC ERROR=\"decompilation error\" raw=\"True\">"), "{}", text.chars().take(400).collect::<String>()),
        Err(e) => assert!(matches!(e, crate::TablesError::TooLarge { .. }), "{e}"),
    }
}
