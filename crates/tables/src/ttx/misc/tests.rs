// Tests may slice strings: a failing assertion is the point of a test.
#![allow(clippy::string_slice)]

//! kern, DSIG, FFTM, LTSH, VDMX and prop against `ttx`. The fonts are built in `fonts.rs` table
//! by table; the expected text in `testdata/` is what fontTools 4.66.1's `ttx` writes for the
//! same bytes, with comments taken out.

use super::fonts;
use crate::sfnt::Tag;
use crate::ttx::var::tests::{assert_dump, assert_truncations_survive};
use crate::ttx::{DumpFormat, DumpOptions, dump};

const TAGS: [Tag; 6] = [*b"kern", *b"DSIG", *b"FFTM", *b"LTSH", *b"VDMX", *b"prop"];

#[test]
fn other_tables_match_ttx() {
    assert_dump(&fonts::misc_font(), &TAGS, include_str!("testdata/misc.ttx"));
    assert_dump(&fonts::apple_kern_font(), &[*b"kern"], include_str!("testdata/apple_kern.ttx"));
}

#[test]
fn every_truncation_of_the_other_tables_is_survived() {
    assert_truncations_survive(&fonts::misc_font(), &TAGS);
    assert_truncations_survive(&fonts::apple_kern_font(), &[*b"kern"]);
}

#[test]
fn the_signature_text_is_in_the_json_tree() {
    let options = DumpOptions { format: DumpFormat::Json, tables: vec![*b"DSIG"], ..DumpOptions::default() };
    let json = dump(&fonts::misc_font(), &options).unwrap();
    assert!(json.contains("\"text\":\"AAECAwQFBgcICQoL"), "{json}");
}

#[test]
fn unknown_tables_are_written_as_hex() {
    let mut font = fonts::misc_font();
    font.insert(*b"zzzz", vec![1, 2, 3]);
    let options = DumpOptions { tables: vec![*b"zzzz"], ..DumpOptions::default() };
    let text = dump(&font, &options).unwrap();
    assert!(text.contains("<zzzz raw=\"True\">\n    <hexdata>\n      010203   \n"), "{text}");
}

#[test]
fn overlapping_lookup_segments_cost_no_more_than_their_glyphs() {
    // 65,535 segments, each covering every glyph id: the later segment wins for every glyph.
    let mut prop = vec![0, 2, 0, 0, 0, 1, 0, 0];
    prop.extend_from_slice(&[0, 2, 0, 6, 0xFF, 0xFF, 0, 0, 0, 0, 0, 0]);
    for i in 0..0xFFFFu32 {
        prop.extend_from_slice(&[0xFF, 0xFE, 0, 0]);
        prop.extend_from_slice(&((i % 7) as u16).to_be_bytes());
    }
    let mut font = fonts::misc_font();
    font.insert(*b"prop", prop);
    let options = DumpOptions { tables: vec![*b"prop"], ..DumpOptions::default() };
    let start = std::time::Instant::now();
    let text = dump(&font, &options).unwrap();
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
    // The last segment (index 65,534) holds 65,534 % 7 = 0.
    assert!(text.contains("<Lookup glyph=\"A\" value=\"0\"/>"), "{}", &text[..400.min(text.len())]);
}
