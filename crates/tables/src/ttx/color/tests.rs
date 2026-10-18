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
