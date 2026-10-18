// Tests may slice strings: a failing assertion is the point of a test.
#![allow(clippy::string_slice)]

//! The variation tables against `ttx`. The font is built in `fonts.rs` table by table; the
//! expected text in `testdata/` is what fontTools 4.66.1's `ttx` writes for the same bytes, with
//! comments taken out (they hold no data). The comparison helpers are shared with the colour and
//! miscellaneous tables' tests.

use super::fonts;
use crate::sfnt::{SfntFont, Tag};
use crate::ttx::{DumpOptions, dump};

/// The text without comments; a line that held only a comment goes away.
pub(in crate::ttx) fn without_comments(text: &str) -> String {
    let mut out = Vec::new();
    let mut in_comment = false;
    for line in text.split('\n') {
        let mut kept = String::new();
        let mut touched = in_comment;
        let mut rest = line;
        loop {
            if in_comment {
                match rest.find("-->") {
                    Some(j) => {
                        in_comment = false;
                        rest = &rest[j + 3..];
                    }
                    None => break,
                }
            } else {
                match rest.find("<!--") {
                    Some(j) => {
                        kept.push_str(&rest[..j]);
                        in_comment = true;
                        touched = true;
                        rest = &rest[j + 4..];
                    }
                    None => {
                        kept.push_str(rest);
                        break;
                    }
                }
            }
        }
        if touched && kept.trim().is_empty() {
            continue;
        }
        out.push(kept);
    }
    out.join("\n")
}

/// Dumps `tags` of `font` and compares the text, comments left out, with `expected`.
pub(in crate::ttx) fn assert_dump(font: &SfntFont, tags: &[Tag], expected: &str) {
    let options = DumpOptions { tables: tags.to_vec(), ..DumpOptions::default() };
    let ours = without_comments(&dump(font, &options).unwrap());
    for (i, (a, b)) in ours.lines().zip(expected.lines()).enumerate() {
        assert_eq!(a, b, "line {} differs", i + 1);
    }
    assert_eq!(ours.lines().count(), expected.lines().count(), "line counts differ");
}

/// Every prefix of every table in `tags` is dumped without a panic, and each table that cannot
/// be read comes out as hex.
pub(in crate::ttx) fn assert_truncations_survive(font: &SfntFont, tags: &[Tag]) {
    let options = DumpOptions { tables: tags.to_vec(), ..DumpOptions::default() };
    for tag in tags {
        let data = font.get(*tag).unwrap().to_vec();
        for len in 0..data.len() {
            let mut cut = font.clone();
            cut.insert(*tag, data[..len].to_vec());
            dump(&cut, &options).unwrap();
        }
    }
}

const TAGS: [Tag; 5] = [*b"gvar", *b"cvar", *b"HVAR", *b"VVAR", *b"MVAR"];

#[test]
fn variation_tables_match_ttx() {
    assert_dump(&fonts::var_font(), &TAGS, include_str!("testdata/var.ttx"));
}

#[test]
fn every_truncation_of_the_variation_tables_is_survived() {
    assert_truncations_survive(&fonts::var_font(), &TAGS);
}

#[test]
fn gvar_without_fvar_numbers_its_axes() {
    let mut font = fonts::var_font();
    font.remove(*b"fvar");
    let options = DumpOptions { tables: vec![*b"gvar", *b"cvar"], ..DumpOptions::default() };
    let text = dump(&font, &options).unwrap();
    assert!(text.contains("<axisCount value=\"2\"/>"), "{text}");
    assert!(text.contains("<coord axis=\"0\" value=\"-1.0\"/>\n        <coord axis=\"1\" value=\"0.5\"/>"), "{text}");
    // cvar cannot be read without the axes of fvar.
    assert!(text.contains("<cvar ERROR=\"decompilation error\" raw=\"True\">"), "{text}");
}

#[test]
fn gvar_whose_glyph_count_differs_is_written_as_hex() {
    let mut font = fonts::var_font();
    let mut gvar = font.get(*b"gvar").unwrap().to_vec();
    gvar[13] = 5;
    font.insert(*b"gvar", gvar);
    let options = DumpOptions { tables: vec![*b"gvar"], ..DumpOptions::default() };
    let text = dump(&font, &options).unwrap();
    assert!(text.contains("<gvar ERROR=\"decompilation error\" raw=\"True\">"), "{text}");
}

#[test]
fn a_composite_that_never_ends_is_refused() {
    let mut font = fonts::var_font();
    // Every component of C says another follows; the glyph runs out of bytes.
    let mut glyf = font.get(*b"glyf").unwrap().to_vec();
    let c = glyf.len() - 26;
    glyf[c + 10..c + 12].copy_from_slice(&0x0023u16.to_be_bytes());
    glyf[c + 18..c + 20].copy_from_slice(&0x0023u16.to_be_bytes());
    font.insert(*b"glyf", glyf);
    let options = DumpOptions { tables: vec![*b"gvar"], ..DumpOptions::default() };
    assert!(dump(&font, &options).unwrap().contains("ERROR=\"decompilation error\""));
}
