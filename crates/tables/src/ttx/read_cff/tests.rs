//! `CFF `, `CFF2` and `VORG` against `ttx`'s compiler. The inputs are the dumps the writer tests
//! compare with (`../cff/testdata/`, written by fontTools 4.66.1 for fonts built in code); the
//! expected bytes in `testdata/*.compiled` are what fontTools 4.66.1 compiles from them, one
//! table a line. fontTools refuses `cff1.ttx` as it is (two charstrings end with operands, and a
//! `VORG` record names a glyph past the glyph order), so that dump is first edited as
//! [`cff1_ttx`] does.
#![allow(clippy::string_slice)]

use super::{encode_float, format_g8};
use crate::sfnt::{Flavour, SfntFont};
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

fn expected(compiled: &str) -> Vec<([u8; 4], Vec<u8>)> {
    compiled
        .lines()
        .map(|line| {
            let (tag, hex) = line.split_once(' ').unwrap();
            let tag: [u8; 4] = tag.replace('_', " ").as_bytes().try_into().unwrap();
            let bytes = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
            (tag, bytes)
        })
        .collect()
}

fn check(text: &str, compiled: &str) -> SfntFont {
    let font = import(SfntFont::new(Flavour::Cff), text, &ImportOptions::default()).unwrap();
    for (tag, bytes) in expected(compiled) {
        let got = font.get(tag).unwrap_or_else(|| panic!("no '{}' table", String::from_utf8_lossy(&tag)));
        assert_eq!(got, &bytes[..], "table '{}'", String::from_utf8_lossy(&tag));
    }
    font
}

#[test]
fn cff_dumps_compile_as_ttx_compiles() {
    check(&cff1_ttx(), include_str!("testdata/cff1.compiled"));
    check(CID_TTX, include_str!("testdata/cid.compiled"));
    check(CFF2_TTX, include_str!("testdata/cff2.compiled"));
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
