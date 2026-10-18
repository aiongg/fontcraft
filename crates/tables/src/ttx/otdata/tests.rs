// Tests may slice strings: a failing assertion is the point of a test.
#![allow(clippy::string_slice)]

//! The structured tables against `ttx`. The fonts are described in `testdata/fonts.otasm` and
//! assembled here; the expected text next to it is what fontTools 4.66's `ttx` writes for the
//! same bytes (`testdata/oracle.py`), without comments and trailing spaces.
//!
//! The assembly: `font NAME` starts a font and `table TAG` a table (`_` stands for a space in
//! tags); `@label` marks the current position; `u8 i8 u16 i16 u24 u32 i32` write numbers
//! (decimal or `0x` hex); `fixed` and `f2` write 16.16 and 2.14 numbers; `tag` writes four-byte
//! tags; `o16 o24 o32 TARGET BASE` write the distance from label BASE to label TARGET; `pstr`
//! writes Pascal strings; `utf16 TEXT` and `ascii TEXT` write the rest of the line. `#` starts a
//! comment, except on `utf16` and `ascii` lines.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crate::sfnt::{Flavour, SfntFont, Tag};
use crate::ttx::{DumpFormat, DumpOptions, dump};

const TABLES: [&[u8; 4]; 10] = [b"GDEF", b"GSUB", b"GPOS", b"BASE", b"JSTF", b"MATH", b"STAT", b"fvar", b"avar", b"meta"];

/// The fonts of the description, by name, each a list of tables.
#[allow(clippy::type_complexity)]
fn parse(text: &str) -> BTreeMap<String, Vec<(Tag, Vec<Vec<String>>)>> {
    let mut fonts: BTreeMap<String, Vec<(Tag, Vec<Vec<String>>)>> = BTreeMap::new();
    let mut font = String::new();
    for raw in text.lines() {
        let first = raw.split_whitespace().next().unwrap_or("");
        let item: Vec<String> = if first == "utf16" || first == "ascii" {
            let rest = raw.trim_start()[first.len()..].trim_start();
            vec![first.to_string(), rest.to_string()]
        } else {
            raw.split('#').next().unwrap().split_whitespace().map(str::to_string).collect()
        };
        match item.first().map(String::as_str) {
            None => {}
            Some("font") => {
                font = item[1].clone();
                fonts.insert(font.clone(), Vec::new());
            }
            Some("table") => {
                let tag: Tag = item[1].replace('_', " ").as_bytes().try_into().unwrap();
                fonts.get_mut(&font).unwrap().push((tag, Vec::new()));
            }
            Some(_) => fonts.get_mut(&font).unwrap().last_mut().unwrap().1.push(item),
        }
    }
    fonts
}

fn number(text: &str) -> i64 {
    match text.strip_prefix("0x") {
        Some(hex) => i64::from_str_radix(hex, 16).unwrap(),
        None => text.parse().unwrap(),
    }
}

fn size(op: &str) -> usize {
    match op {
        "u8" | "i8" => 1,
        "u16" | "i16" | "f2" => 2,
        "u24" => 3,
        _ => 4,
    }
}

/// The bytes of one table, laid out in two passes so offsets can point forward.
fn assemble(lines: &[Vec<String>]) -> Vec<u8> {
    let mut labels: BTreeMap<&str, i64> = BTreeMap::new();
    let mut pos = 0usize;
    for item in lines {
        let (op, args) = (item[0].as_str(), &item[1..]);
        if let Some(label) = op.strip_prefix('@') {
            labels.insert(label, pos as i64);
            continue;
        }
        pos += match op {
            "u8" | "i8" | "u16" | "i16" | "u24" | "u32" | "i32" | "fixed" | "f2" => size(op) * args.len(),
            "tag" => 4 * args.len(),
            "o16" => 2,
            "o24" => 3,
            "o32" => 4,
            "pstr" => args.iter().map(|s| 1 + s.len()).sum(),
            "utf16" => args[0].encode_utf16().count() * 2,
            "ascii" => args[0].len(),
            other => panic!("unknown directive {other}"),
        };
    }
    let mut out = Vec::new();
    for item in lines {
        let (op, args) = (item[0].as_str(), &item[1..]);
        match op {
            "u8" | "i8" | "u16" | "i16" | "u24" | "u32" | "i32" => {
                for a in args {
                    let bytes = (number(a) as u64).to_be_bytes();
                    out.extend_from_slice(&bytes[8 - size(op)..]);
                }
            }
            "fixed" => args.iter().for_each(|a| out.extend_from_slice(&((a.parse::<f64>().unwrap() * 65536.0).round() as i32).to_be_bytes())),
            "f2" => args.iter().for_each(|a| out.extend_from_slice(&((a.parse::<f64>().unwrap() * 16384.0).round() as i16).to_be_bytes())),
            "tag" => args.iter().for_each(|a| out.extend(a.replace('_', " ").bytes())),
            "o16" | "o24" | "o32" => {
                let value = (labels[args[0].as_str()] - labels[args[1].as_str()]) as u32;
                let n = if op == "o16" {
                    2
                } else if op == "o24" {
                    3
                } else {
                    4
                };
                out.extend_from_slice(&value.to_be_bytes()[4 - n..]);
            }
            "pstr" => args.iter().for_each(|s| {
                out.push(s.len() as u8);
                out.extend(s.bytes());
            }),
            "utf16" => args[0].encode_utf16().for_each(|u| out.extend_from_slice(&u.to_be_bytes())),
            "ascii" => out.extend(args[0].bytes()),
            _ => {}
        }
    }
    out
}

fn font(name: &str) -> SfntFont {
    let fonts = parse(include_str!("testdata/fonts.otasm"));
    let mut font = SfntFont::new(Flavour::TrueType);
    for (tag, lines) in &fonts[name] {
        font.insert(*tag, assemble(lines));
    }
    font
}

fn chosen(font: &SfntFont) -> DumpOptions {
    DumpOptions { tables: TABLES.iter().map(|t| **t).filter(|t| font.get(*t).is_some()).collect(), ..DumpOptions::default() }
}

/// The text without comments and trailing spaces; a line that held only a comment goes away.
fn without_comments(text: &str) -> String {
    let mut marked = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("<!--") {
        marked.push_str(&rest[..start]);
        marked.push('\0');
        match rest[start..].find("-->") {
            Some(end) => rest = &rest[start + end + 3..],
            None => rest = "",
        }
    }
    marked.push_str(rest);
    marked
        .split('\n')
        .filter(|line| !(line.contains('\0') && line.replace('\0', "").trim().is_empty()))
        .map(|line| line.replace('\0', "").trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_same_lines(ours: &str, expected: &str) {
    let ours = without_comments(ours);
    for (i, (a, b)) in ours.lines().zip(expected.lines()).enumerate() {
        assert_eq!(a, b, "line {} differs", i + 1);
    }
    assert_eq!(ours.lines().count(), expected.lines().count(), "line counts differ");
}

#[test]
fn layout_tables_match_ttx() {
    let font = font("layout");
    let text = dump(&font, &chosen(&font)).unwrap();
    assert_same_lines(&text, include_str!("testdata/layout.ttx"));
}

#[test]
fn variation_tables_match_ttx() {
    let font = font("variable");
    let text = dump(&font, &chosen(&font)).unwrap();
    assert_same_lines(&text, include_str!("testdata/variable.ttx"));
}

#[test]
fn comments_name_the_flags_and_the_names() {
    let font = font("layout");
    let text = dump(&font, &DumpOptions { tables: vec![*b"GSUB"], ..DumpOptions::default() }).unwrap();
    assert!(text.contains("<LookupFlag value=\"792\"/><!-- ignoreMarks useMarkFilteringSet markAttachmentType[3] -->"), "{text}");
    assert!(text.contains("<UINameID value=\"256\"/><!-- Alt a -->"));
    assert!(text.contains("<!-- LookupCount=10 -->"));
    let variable = self::font("variable");
    let text = dump(&variable, &DumpOptions { tables: vec![*b"fvar", *b"STAT"], ..DumpOptions::default() }).unwrap();
    assert!(text.contains("\n    <!-- Weight -->\n    <Axis>"), "{text}");
    assert!(text.contains("<!-- PostScript: Bold-PS -->"));
    assert!(text.contains("<ElidedFallbackNameID value=\"2\"/><!-- missing from name table -->"));
    assert!(text.contains("<Flags value=\"2\"/><!-- ElidableAxisValueName -->"));
}

#[test]
fn structured_tables_in_json() {
    let font = font("layout");
    let options = DumpOptions { format: DumpFormat::Json, tables: vec![*b"GSUB", *b"meta"], ..DumpOptions::default() };
    let json = dump(&font, &options).unwrap();
    assert!(json.contains(r#"{"name":"Substitution","attributes":{"in":"a","out":"a.sc"}}"#), "{json}");
    assert!(json.contains(r#"{"name":"text","attributes":{"tag":"dlng"},"text":"Latn,Grek"}"#), "{json}");
}

#[test]
fn every_truncation_of_every_structured_table_is_survived() {
    for name in ["layout", "variable"] {
        let font = font(name);
        for tag in TABLES {
            let Some(data) = font.get(*tag) else { continue };
            for len in 0..data.len() {
                let mut cut = font.clone();
                cut.insert(*tag, data[..len].to_vec());
                dump(&cut, &DumpOptions { tables: vec![*tag], ..DumpOptions::default() }).unwrap();
            }
        }
    }
}

/// A GSUB table with one lookup of `lookup_type` whose subtable is `subtable`.
fn gsub_with(lookup_type: u16, subtable: &[u8]) -> Vec<u8> {
    let mut t: Vec<u8> = Vec::new();
    for v in [1u16, 0, 10, 12, 14, 0, 0, 1, 4, lookup_type, 0, 1, 8] {
        t.extend_from_slice(&v.to_be_bytes());
    }
    t.extend_from_slice(subtable);
    t
}

#[test]
fn hostile_layout_tables_are_written_as_hex() {
    let base = font("layout");
    let error = "<GSUB ERROR=\"decompilation error\" raw=\"True\">";
    let gsub_only = DumpOptions { tables: vec![*b"GSUB"], ..DumpOptions::default() };

    // Extension lookups pointing at extension lookups, deeper than any reader follows.
    let mut chain = Vec::new();
    for _ in 0..5_000 {
        chain.extend_from_slice(&[0, 1, 0, 7, 0, 0, 0, 8]);
    }
    let mut font = base.clone();
    font.insert(*b"GSUB", gsub_with(7, &chain));
    assert!(dump(&font, &gsub_only).unwrap().contains(error));

    // A lookup type GSUB does not define.
    let mut font = base.clone();
    font.insert(*b"GSUB", gsub_with(9, &[0, 1]));
    assert!(dump(&font, &gsub_only).unwrap().contains(error));

    // A single substitution whose substitutes and coverage differ in length.
    let mut font = base.clone();
    font.insert(*b"GSUB", gsub_with(1, &[0, 2, 0, 8, 0, 2, 0, 1, 0, 1, 0, 1, 0, 1]));
    assert!(dump(&font, &gsub_only).unwrap().contains(error));

    // Coverage ranges that list far more glyphs than a font holds, refused quickly.
    let mut gdef: Vec<u8> = Vec::new();
    for v in [1u16, 0, 0, 12, 0, 0, 4, 0, 2, 200] {
        gdef.extend_from_slice(&v.to_be_bytes());
    }
    for _ in 0..200 {
        gdef.extend_from_slice(&[0, 0, 0xFF, 0xFF, 0, 0]);
    }
    let mut font = base.clone();
    font.insert(*b"GDEF", gdef);
    let start = Instant::now();
    let text = dump(&font, &DumpOptions { tables: vec![*b"GDEF"], ..DumpOptions::default() }).unwrap();
    assert!(text.contains("<GDEF ERROR=\"decompilation error\" raw=\"True\">"), "{}", &text[..text.len().min(2000)]);
    assert!(start.elapsed() < Duration::from_secs(10));

    // avar needs fvar.
    let mut font = self::font("variable");
    font.remove(*b"fvar");
    let text = dump(&font, &DumpOptions { tables: vec![*b"avar"], ..DumpOptions::default() }).unwrap();
    assert!(text.contains("<avar ERROR=\"decompilation error\" raw=\"True\">"));

    let strict = DumpOptions { tables: vec![*b"avar"], strict: true, ..DumpOptions::default() };
    assert!(dump(&font, &strict).is_err());
}
