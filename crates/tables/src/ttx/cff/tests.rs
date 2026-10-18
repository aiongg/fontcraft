// Tests may index and slice: a failing assertion is the point of a test.
#![allow(clippy::indexing_slicing, clippy::string_slice)]

//! `CFF `, `CFF2` and `VORG` against `ttx`. Each font is assembled here from a `.spec` file in
//! `testdata/`; the `.ttx` file beside it is what fontTools 4.66.1's `ttx` writes for the same
//! bytes, with comments taken out (they hold no data).
//!
//! A spec is lines of text. `table TAG` starts a table; every other line is a block of it,
//! `LABEL KIND PAYLOAD`, the blocks laid out one after another:
//! - `raw HEX`: bytes (spaces allowed);
//! - `head2 LABEL`: a CFF2 header whose Top DICT is block LABEL;
//! - `dict ENTRIES`; `index ITEMS` and `index2 ITEMS` (CFF2's 32-bit count).
//!
//! ITEMS are separated by ` | `: `str:TEXT` (`\n` for a line break), `dict:ENTRIES`,
//! `cs:TOKENS` or `hex:HEX`. ENTRIES are separated by `;`: operands, then an operator `oN` or
//! `o12.N`. An operand is an integer, `rTEXT` (a real), `@L` (the position of block L in the
//! table, as a 5-byte integer), `#L` (its length) or `@L-@M` (the position of L minus that of
//! M). TOKENS are charstring operands (integers, `fV` for a 16.16 number, `xHH` for a raw byte)
//! and operator names; the token after `hintmask` or `cntrmask` is the mask in bits.

use std::collections::HashMap;

use crate::sfnt::{Flavour, SfntFont, Tag};
use crate::ttx::{DumpOptions, dump};

const T2_OPERATORS: &[(&str, &[u8])] = &[
    ("hstem", &[1]),
    ("vstem", &[3]),
    ("vmoveto", &[4]),
    ("rlineto", &[5]),
    ("hlineto", &[6]),
    ("vlineto", &[7]),
    ("rrcurveto", &[8]),
    ("callsubr", &[10]),
    ("return", &[11]),
    ("endchar", &[14]),
    ("vsindex", &[15]),
    ("blend", &[16]),
    ("hstemhm", &[18]),
    ("hintmask", &[19]),
    ("cntrmask", &[20]),
    ("rmoveto", &[21]),
    ("hmoveto", &[22]),
    ("vstemhm", &[23]),
    ("rcurveline", &[24]),
    ("rlinecurve", &[25]),
    ("vvcurveto", &[26]),
    ("hhcurveto", &[27]),
    ("callgsubr", &[29]),
    ("vhcurveto", &[30]),
    ("hvcurveto", &[31]),
    ("add", &[12, 10]),
    ("hflex", &[12, 34]),
    ("flex", &[12, 35]),
    ("hflex1", &[12, 36]),
    ("flex1", &[12, 37]),
];

fn int_bytes(v: i64, dict: bool) -> Vec<u8> {
    match v {
        -107..=107 => vec![(v + 139) as u8],
        108..=1131 => vec![((v - 108) / 256 + 247) as u8, ((v - 108) % 256) as u8],
        -1131..=-108 => vec![((-v - 108) / 256 + 251) as u8, ((-v - 108) % 256) as u8],
        -32768..=32767 => [&[28u8][..], &(v as i16).to_be_bytes()].concat(),
        _ => {
            assert!(dict, "{v} does not fit a charstring integer");
            [&[29u8][..], &(v as i32).to_be_bytes()].concat()
        }
    }
}

fn real_bytes(s: &str) -> Vec<u8> {
    let chars: Vec<char> = s.chars().collect();
    let mut nibbles = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            c @ '0'..='9' => nibbles.push(c as u8 - b'0'),
            '.' => nibbles.push(0xA),
            'e' | 'E' if chars.get(i + 1) == Some(&'-') => {
                nibbles.push(0xC);
                i += 1;
            }
            'e' | 'E' => nibbles.push(0xB),
            '-' => nibbles.push(0xE),
            c => panic!("{c} in real {s}"),
        }
        i += 1;
    }
    nibbles.push(0xF);
    if nibbles.len() % 2 == 1 {
        nibbles.push(0xF);
    }
    let mut out = vec![30];
    out.extend(nibbles.chunks(2).map(|p| p[0] << 4 | p[1]));
    out
}

fn cs_bytes(text: &str) -> Vec<u8> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let t = tokens[i];
        i += 1;
        if let Some((name, code)) = T2_OPERATORS.iter().find(|(n, _)| *n == t) {
            out.extend_from_slice(code);
            if *name == "hintmask" || *name == "cntrmask" {
                let bits = tokens[i];
                i += 1;
                for chunk in bits.as_bytes().chunks(8) {
                    out.push(u8::from_str_radix(std::str::from_utf8(chunk).unwrap(), 2).unwrap());
                }
            }
        } else if let Some(hex) = t.strip_prefix('x') {
            out.push(u8::from_str_radix(hex, 16).unwrap());
        } else if let Some(v) = t.strip_prefix('f') {
            let v: f64 = v.parse().unwrap();
            out.push(255);
            out.extend(((v * 65536.0).round() as i32).to_be_bytes());
        } else {
            out.extend(int_bytes(t.parse().unwrap(), false));
        }
    }
    out
}

type Places = HashMap<String, i64>;

fn dict_bytes(entries: &str, pos: &Places, len: &Places) -> Vec<u8> {
    let long = |v: i64| [&[29u8][..], &(v as i32).to_be_bytes()].concat();
    let at = |l: &str| pos.get(l).copied().unwrap_or(0);
    let mut out = Vec::new();
    for entry in entries.split(';') {
        for t in entry.split_whitespace() {
            if let Some(op) = t.strip_prefix('o') {
                out.extend(op.split('.').map(|b| b.parse::<u8>().unwrap()));
            } else if let Some((a, b)) = t.strip_prefix('@').and_then(|r| r.split_once("-@")) {
                out.extend(long(at(a) - at(b)));
            } else if let Some(l) = t.strip_prefix('@') {
                out.extend(long(at(l)));
            } else if let Some(l) = t.strip_prefix('#') {
                out.extend(long(len.get(l).copied().unwrap_or(0)));
            } else if let Some(r) = t.strip_prefix('r') {
                out.extend(real_bytes(r));
            } else {
                out.extend(int_bytes(t.parse().unwrap(), true));
            }
        }
    }
    out
}

fn hex_bytes(text: &str) -> Vec<u8> {
    let digits: String = text.split_whitespace().collect();
    (0..digits.len()).step_by(2).map(|i| u8::from_str_radix(&digits[i..i + 2], 16).unwrap()).collect()
}

fn item_bytes(item: &str, pos: &Places, len: &Places) -> Vec<u8> {
    let (kind, rest) = item.split_once(':').unwrap();
    match kind {
        "str" => rest.replace("\\n", "\n").into_bytes(),
        "dict" => dict_bytes(rest, pos, len),
        "cs" => cs_bytes(rest),
        "hex" => hex_bytes(rest),
        _ => panic!("item kind {kind}"),
    }
}

fn index_bytes(items: &[Vec<u8>], cff2: bool) -> Vec<u8> {
    let count = items.len() as u32;
    let mut out = if cff2 { count.to_be_bytes().to_vec() } else { (count as u16).to_be_bytes().to_vec() };
    if items.is_empty() {
        return out;
    }
    let mut offsets = vec![1u32];
    for item in items {
        offsets.push(offsets.last().unwrap() + item.len() as u32);
    }
    let last = *offsets.last().unwrap();
    let size = if last < 0x100 {
        1
    } else if last < 0x10000 {
        2
    } else if last < 0x100_0000 {
        3
    } else {
        4
    };
    out.push(size as u8);
    for o in offsets {
        out.extend_from_slice(&o.to_be_bytes()[4 - size..]);
    }
    for item in items {
        out.extend_from_slice(item);
    }
    out
}

fn block_bytes(kind: &str, payload: &str, pos: &Places, len: &Places) -> Vec<u8> {
    match kind {
        "raw" => hex_bytes(payload),
        "head2" => [&[2u8, 0, 5][..], &(len.get(payload).copied().unwrap_or(0) as u16).to_be_bytes()].concat(),
        "dict" => dict_bytes(payload, pos, len),
        "index" | "index2" => {
            let items: Vec<Vec<u8>> =
                if payload.trim().is_empty() { Vec::new() } else { payload.split(" | ").map(|i| item_bytes(i.trim(), pos, len)).collect() };
            index_bytes(&items, kind == "index2")
        }
        _ => panic!("block kind {kind}"),
    }
}

/// Lays the blocks out until every position and length is settled.
fn assemble(blocks: &[(&str, &str, &str)]) -> Vec<u8> {
    let (mut pos, mut len) = (Places::new(), Places::new());
    let mut out = Vec::new();
    for _ in 0..3 {
        out.clear();
        for (label, kind, payload) in blocks {
            let b = block_bytes(kind, payload, &pos, &len);
            pos.insert(label.to_string(), out.len() as i64);
            len.insert(label.to_string(), b.len() as i64);
            out.extend(b);
        }
    }
    out
}

fn font_from_spec(spec: &str) -> SfntFont {
    #[allow(clippy::type_complexity)]
    let mut tables: Vec<(Tag, Vec<(&str, &str, &str)>)> = Vec::new();
    for line in spec.lines() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if let Some(tag) = line.strip_prefix("table ") {
            let mut t = [b' '; 4];
            t[..tag.len()].copy_from_slice(tag.as_bytes());
            tables.push((t, Vec::new()));
            continue;
        }
        let mut parts = line.splitn(3, ' ');
        let label = parts.next().unwrap();
        let kind = parts.next().unwrap();
        let payload = parts.next().unwrap_or("");
        tables.last_mut().unwrap().1.push((label, kind, payload));
    }
    let mut font = SfntFont::new(Flavour::Cff);
    for (tag, blocks) in tables {
        font.insert(tag, assemble(&blocks));
    }
    font
}

/// The text without comments; a line that held only a comment goes away.
fn without_comments(text: &str) -> String {
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

fn assert_same_lines(ours: &str, expected: &str) {
    let ours = without_comments(ours);
    for (i, (a, b)) in ours.lines().zip(expected.lines()).enumerate() {
        assert_eq!(a, b, "line {} differs", i + 1);
    }
    assert_eq!(ours.lines().count(), expected.lines().count(), "line counts differ");
}

#[test]
fn name_keyed_cff_and_vorg_match_ttx() {
    let font = font_from_spec(include_str!("testdata/cff1.spec"));
    let text = dump(&font, &DumpOptions::default()).unwrap();
    assert_same_lines(&text, include_str!("testdata/cff1.ttx"));
}

#[test]
fn cid_keyed_cff_matches_ttx() {
    let font = font_from_spec(include_str!("testdata/cid.spec"));
    let text = dump(&font, &DumpOptions::default()).unwrap();
    assert_same_lines(&text, include_str!("testdata/cid.ttx"));
}

#[test]
fn cff2_with_blends_and_variation_store_matches_ttx() {
    let font = font_from_spec(include_str!("testdata/cff2.spec"));
    let text = dump(&font, &DumpOptions::default()).unwrap();
    assert_same_lines(&text, include_str!("testdata/cff2.ttx"));
}

#[test]
fn predefined_charset_and_defaults_match_ttx() {
    let font = font_from_spec(include_str!("testdata/iso.spec"));
    let text = dump(&font, &DumpOptions::default()).unwrap();
    assert_same_lines(&text, include_str!("testdata/iso.ttx"));
}

#[test]
fn charstrings_fonttools_refuses_make_the_table_hex() {
    let spec = include_str!("testdata/cff1.spec");
    let cases = [
        // An arithmetic operator.
        ("cs:-100 endchar", "cs:1 2 add endchar"),
        // A subroutine that calls itself: nesting stops at the limit instead of overflowing.
        ("subrs index cs:100 0 rlineto return", "subrs index cs:-107 callsubr return"),
        // A subroutine's hint mask read with no stems, then called where masks take a byte.
        ("cs:0 100 rlineto return", "cs:hintmask 0 100 rlineto return"),
    ];
    for (from, to) in cases {
        assert!(spec.contains(from), "{from}");
        let font = font_from_spec(&spec.replacen(from, to, 1));
        let text = dump(&font, &DumpOptions::default()).unwrap();
        assert!(text.contains(r#"<CFF ERROR="decompilation error" raw="True">"#), "{to}: the table is hex");
        assert!(text.contains("<VORG>"), "{to}: the other tables are still written");
        let strict = DumpOptions { strict: true, ..DumpOptions::default() };
        assert!(dump(&font, &strict).is_err(), "{to}: strict stops");
    }
}
