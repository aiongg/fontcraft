// Tests may slice strings: a failing assertion is the point of a test.
#![allow(clippy::string_slice)]

//! The core tables against `ttx`. The fonts are built here table by table; the expected text in
//! `testdata/` is what fontTools 4.66.1's `ttx` writes for the same bytes, with comments taken
//! out (they hold no data, and `ttx` puts tracebacks in some).

use crate::sfnt::{Flavour, SfntFont};
use std::time::{Duration, Instant};

use crate::TablesError;
use crate::ttx::{DumpFormat, DumpOptions, dump, dump_split, dump_within, fixed_to_str, num2binary, py_float, tag_to_identifier, tag_to_xml};

/// Big-endian table builder.
#[derive(Default)]
struct Be(Vec<u8>);

impl Be {
    fn u8s(&mut self, v: &[u8]) {
        self.0.extend_from_slice(v);
    }
    fn i8s(&mut self, v: &[i8]) {
        self.0.extend(v.iter().map(|x| *x as u8));
    }
    fn u16s(&mut self, v: &[u16]) {
        v.iter().for_each(|x| self.0.extend_from_slice(&x.to_be_bytes()));
    }
    fn i16s(&mut self, v: &[i16]) {
        v.iter().for_each(|x| self.0.extend_from_slice(&x.to_be_bytes()));
    }
    fn u24s(&mut self, v: &[u32]) {
        v.iter().for_each(|x| self.0.extend_from_slice(&x.to_be_bytes()[1..]));
    }
    fn u32s(&mut self, v: &[u32]) {
        v.iter().for_each(|x| self.0.extend_from_slice(&x.to_be_bytes()));
    }
    fn i32s(&mut self, v: &[i32]) {
        v.iter().for_each(|x| self.0.extend_from_slice(&x.to_be_bytes()));
    }
    fn bytes(&mut self, v: &[u8]) {
        self.0.extend_from_slice(v);
    }
    fn utf16(&mut self, s: &str) {
        s.encode_utf16().for_each(|u| self.0.extend_from_slice(&u.to_be_bytes()));
    }
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
fn core_tables_match_ttx() {
    let text = dump(&fonts::core_font(), &DumpOptions::default()).unwrap();
    assert_same_lines(&text, include_str!("testdata/core.ttx"));
}

#[test]
fn tables_that_cannot_be_read_are_written_as_hex() {
    let font = fonts::broken_font();
    let text = dump(&font, &DumpOptions::default()).unwrap();
    assert_same_lines(&text, include_str!("testdata/broken.ttx"));
    let strict = DumpOptions { strict: true, ..DumpOptions::default() };
    assert!(dump(&font, &strict).is_err(), "strict stops at the first table it cannot read");
}

#[test]
fn chosen_tables_hex_json_and_split() {
    let font = fonts::core_font();
    let only = DumpOptions { tables: vec![*b"cvt ", *b"gasp"], ..DumpOptions::default() };
    let text = dump(&font, &only).unwrap();
    assert!(!text.contains("<GlyphOrder>"), "chosen tables leave the glyph order out");
    assert!(text.find("<cvt>").unwrap() < text.find("<gasp>").unwrap(), "chosen tables keep the order given");
    let both = DumpOptions { tables: vec![*b"cvt "], skip: vec![*b"gasp"], ..DumpOptions::default() };
    assert!(dump(&font, &both).is_err());

    let skip = DumpOptions { skip: vec![*b"glyf"], ..DumpOptions::default() };
    assert!(!dump(&font, &skip).unwrap().contains("<glyf>"));

    let raw = DumpOptions { tables: vec![*b"gasp"], hex: vec![*b"gasp"], ..DumpOptions::default() };
    assert!(dump(&font, &raw).unwrap().contains("<gasp raw=\"True\">\n    <hexdata>\n      00010002 0008000a ffff000f \n"));

    let asm = DumpOptions { tables: vec![*b"fpgm"], no_disassemble: true, ..DumpOptions::default() };
    assert!(dump(&font, &asm).unwrap().contains("<bytecode>\n      b0002cb1 01025820 5821591b 22592d"));

    let hex = dump(&font, &DumpOptions { format: DumpFormat::Hex, tables: vec![*b"gasp"], ..DumpOptions::default() }).unwrap();
    assert_eq!(hex, "'gasp' 12 bytes\n00000000  00 01 00 02 00 08 00 0a  ff ff 00 0f              |............|\n");

    let json = dump(&font, &DumpOptions { format: DumpFormat::Json, tables: vec![*b"gasp"], ..DumpOptions::default() }).unwrap();
    assert!(json.starts_with(r#"{"name":"ttFont","attributes":{"sfntVersion":"\\x00\\x01\\x00\\x00","ttLibVersion":"4.66"},"children":[{"name":"gasp","children":[{"name":"gaspRange","attributes":{"rangeMaxPPEM":"8","rangeGaspBehavior":"10"}}"#), "{json}");

    let parts = dump_split(&font, &DumpOptions::default(), "core", ".ttx").unwrap();
    assert_eq!(parts[0].0, "core.ttx");
    assert!(parts[0].1.contains("  <OS_2 src=\"core.O_S_2f_2.ttx\"/>\n"));
    assert!(parts.iter().any(|(n, t)| n == "core._g_l_y_f.ttx" && t.contains("<TTGlyph name=\"A\"")));
}

#[test]
fn every_truncation_of_every_table_is_survived() {
    let font = fonts::core_font();
    for (tag, data) in &font.tables {
        for len in 0..data.len() {
            let mut cut = font.clone();
            cut.insert(*tag, data[..len].to_vec());
            dump(&cut, &DumpOptions::default()).unwrap();
        }
    }
}

#[test]
fn numbers_are_written_as_fonttools_writes_them() {
    assert_eq!(fixed_to_str(-0.61883544921875, 14), "-0.61884");
    assert_eq!(fixed_to_str(1.5, 16), "1.5");
    assert_eq!(fixed_to_str(1.0, 16), "1.0");
    assert_eq!(py_float(-12.5), "-12.5");
    assert_eq!(py_float(1e-5), "1e-05");
    assert_eq!(py_float(3276.75), "3276.75");
    assert_eq!(num2binary(0x0B, 16), "00000000 00001011");
    assert_eq!(num2binary(0b01101, 5), "01101");
    assert_eq!(tag_to_xml(*b"cvt "), "cvt");
    assert_eq!(tag_to_xml(*b"OS/2"), "OS_2");
    assert_eq!(tag_to_identifier(*b"glyf"), "_g_l_y_f");
    assert_eq!(tag_to_identifier(*b"OS/2"), "O_S_2f_2");
}

/// A font of `head`, `maxp`, long `loca` and `glyf` holding `glyphs`.
fn glyf_font(glyphs: &[Vec<u8>]) -> SfntFont {
    let mut font = SfntFont::new(Flavour::TrueType);
    let mut head = vec![0u8; 54];
    head[50..52].copy_from_slice(&1i16.to_be_bytes());
    font.insert(*b"head", head);
    let mut maxp = Be::default();
    maxp.u32s(&[0x5000]);
    maxp.u16s(&[glyphs.len() as u16]);
    font.insert(*b"maxp", maxp.0);
    let (mut glyf, mut loca) = (Vec::new(), Be::default());
    for g in glyphs {
        loca.u32s(&[glyf.len() as u32]);
        glyf.extend_from_slice(g);
    }
    loca.u32s(&[glyf.len() as u32]);
    font.insert(*b"loca", loca.0);
    font.insert(*b"glyf", glyf);
    font
}

/// A simple glyph whose contours end at `ends`, over 65,535 on-curve points at the origin
/// (flags repeated in runs of 256, no coordinate bytes).
fn many_point_glyph(ends: &[u16]) -> Vec<u8> {
    let mut g = Be::default();
    g.i16s(&[ends.len() as i16, 0, 0, 0, 0]);
    g.u16s(ends);
    g.u16s(&[0]);
    for _ in 0..255 {
        g.u8s(&[0x39, 255]);
    }
    g.u8s(&[0x39, 254]);
    g.0
}

#[test]
fn decreasing_contour_ends_are_refused_at_once() {
    // Ends alternating 65,534 and 0 would repeat the 65,535 points in every other contour:
    // about a billion points.
    let ends: Vec<u16> = (0..32_767).map(|i| if i % 2 == 0 { 65_534 } else { 0 }).collect();
    let font = glyf_font(&[many_point_glyph(&ends)]);
    let only = DumpOptions { tables: vec![*b"glyf"], ..DumpOptions::default() };
    let started = Instant::now();
    let text = dump(&font, &only).unwrap();
    assert!(text.contains("ERROR=\"decompilation error\"") && text.contains("decrease"), "{}", text.chars().take(2000).collect::<String>());
    assert!(dump(&font, &DumpOptions { strict: true, ..only.clone() }).is_err());
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn a_dump_past_its_budget_stops_with_an_error() {
    // 1,000 glyphs of 65,535 points each: about two gigabytes of text.
    let font = glyf_font(&vec![many_point_glyph(&[65_534]); 1000]);
    let only = DumpOptions { tables: vec![*b"glyf"], ..DumpOptions::default() };
    let json = DumpOptions { format: DumpFormat::Json, ..only.clone() };
    let started = Instant::now();
    assert!(matches!(dump_within(&font, &only, 1 << 20), Err(TablesError::TooLarge { .. })));
    assert!(matches!(dump_within(&font, &json, 1 << 20), Err(TablesError::TooLarge { .. })), "the JSON tree counts too");
    assert!(started.elapsed() < Duration::from_secs(5));
    let one = glyf_font(&[many_point_glyph(&[65_534])]);
    let text = dump_within(&one, &only, 16 << 20).unwrap();
    assert_eq!(text.matches("<pt ").count(), 65_535, "within the budget the glyph is written whole");
}

#[test]
fn cmap_records_sharing_a_subtable_count_toward_one_total() {
    // 1,000 records share one format 12 subtable of 1,048,576 code points.
    let records = 1000u32;
    let mut t = Be::default();
    t.u16s(&[0, records as u16]);
    for _ in 0..records {
        t.u16s(&[3, 10]);
        t.u32s(&[4 + 8 * records]);
    }
    t.u16s(&[12, 0]);
    t.u32s(&[28, 0, 1, 0x1_0000, 0x10_FFFF, 1]);
    let mut font = glyf_font(&[]);
    font.insert(*b"cmap", t.0);
    let only = DumpOptions { tables: vec![*b"cmap"], ..DumpOptions::default() };
    let text = dump(&font, &only).unwrap();
    assert!(text.contains("<cmap ERROR=\"decompilation error\""), "{}", text.chars().take(2000).collect::<String>());
    let strict = dump(&font, &DumpOptions { strict: true, ..only });
    assert!(strict.is_err_and(|e| e.to_string().contains("'cmap' mappings")));
}

mod fonts {
    use super::*;

    /// Every core table, with the cases the writer must get right.
    pub(super) fn core_font() -> SfntFont {
        let mut font = SfntFont::new(Flavour::TrueType);
        // OS/2
        let mut t = Be::default();
        t.u16s(&[4]);
        t.i16s(&[512]);
        t.u16s(&[0x190, 5, 8]);
        t.i16s(&[650, 600, 0, 75, 650, 600, 0, 350, 50, 300, 2053]);
        t.u8s(&[2, 11, 6, 3, 0, 0, 0, 0, 0, 0]);
        t.u32s(&[0x80000003, 0x10000000, 0, 0]);
        t.bytes(b"TE\x22T");
        t.u16s(&[64, 32, 0xFFFF]);
        t.i16s(&[800, -200, 90]);
        t.u16s(&[0x3E8, 0x190]);
        t.u32s(&[1, 0]);
        t.i16s(&[500, 700]);
        t.u16s(&[0, 32, 2]);
        font.insert(*b"OS/2", t.0);
        // cmap
        let mut t = Be::default();
        t.u16s(&[0, 6, 0, 3]);
        t.u32s(&[52]);
        t.u16s(&[0, 5]);
        t.u32s(&[104]);
        t.u16s(&[1, 0]);
        t.u32s(&[162]);
        t.u16s(&[1, 1]);
        t.u32s(&[180]);
        t.u16s(&[3, 1]);
        t.u32s(&[52]);
        t.u16s(&[3, 10]);
        t.u32s(&[204]);
        t.u16s(&[4, 52, 0]);
        t.u16s(&[8, 4, 1, 0, 32, 67, 98, 0xFFFF, 0, 32, 65, 97, 0xFFFF, 0xFFE4, 0xFFC0, 0, 1, 0, 0, 4, 0, 6, 0]);
        t.u16s(&[14]);
        t.u32s(&[58, 2]);
        t.u24s(&[0xFE00]);
        t.u32s(&[32, 40]);
        t.u24s(&[0xFE01]);
        t.u32s(&[0, 49]);
        t.u32s(&[1]);
        t.u24s(&[65]);
        t.u8s(&[1]);
        t.u32s(&[1]);
        t.u24s(&[97]);
        t.u16s(&[6]);
        t.u32s(&[1]);
        t.u24s(&[65]);
        t.u16s(&[3]);
        t.u16s(&[6, 18, 0, 65, 4, 1, 2, 3, 0]);
        t.u16s(&[10, 0]);
        t.u32s(&[24, 0, 0x2000, 2]);
        t.u16s(&[1, 2]);
        t.u16s(&[12, 0]);
        t.u32s(&[52, 0, 3, 48, 49, 0, 65, 67, 1, 0x1F600, 0x1F600, 5]);
        font.insert(*b"cmap", t.0);
        // cvt
        let mut t = Be::default();
        t.i16s(&[0, -10, 700, 32767]);
        font.insert(*b"cvt ", t.0);
        // fpgm
        let mut t = Be::default();
        t.u8s(&[176, 0, 44, 177, 1, 2, 88, 32, 88, 33, 89, 27, 34, 89, 45]);
        font.insert(*b"fpgm", t.0);
        // gasp
        let mut t = Be::default();
        t.u16s(&[1, 2, 8, 10, 0xFFFF, 15]);
        font.insert(*b"gasp", t.0);
        // glyf
        let mut t = Be::default();
        t.i16s(&[1, 50, 0, 450, 700]);
        t.u16s(&[3, 5]);
        t.u8s(&[177, 1, 2, 46, 1, 51, 17, 33, 17]);
        t.u8s(&[50]);
        t.i16s(&[400]);
        t.i16s(&[700, -700]);
        t.i16s(&[2, 0, 0, 600, 700]);
        t.u16s(&[5, 9, 0]);
        t.u8s(&[113, 9, 1, 35, 11, 1, 55, 54, 50, 23]);
        t.i16s(&[300, 300]);
        t.u8s(&[100, 200, 200, 100, 50, 100, 50]);
        t.i16s(&[700, -700, 600, -600]);
        t.u8s(&[200, 60, 60]);
        t.i16s(&[-1, -20, -300, 620, 1000]);
        t.u16s(&[0x22F, 1]);
        t.i16s(&[10, -20, 8192]);
        t.u16s(&[0x942, 0]);
        t.i8s(&[-20, 100]);
        t.i16s(&[20480, -12288]);
        t.u16s(&[3]);
        t.u8s(&[24, 122, 40, 0]);
        t.i16s(&[-1, 0, 0, 600, 700]);
        t.u16s(&[0x4A2, 1]);
        t.i8s(&[5, 6]);
        t.i16s(&[11585, 11585, -11585, 11585]);
        t.u16s(&[0x1001, 0, 1, 2]);
        t.i16s(&[1, 100, 0, 400, 500]);
        t.u16s(&[3, 45]);
        t.u8s(&[
            64, 30, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 88, 176, 7, 27,
            184, 255, 156, 0, 100, 89, 237, 48, 49, 51, 17, 33, 17,
        ]);
        t.u8s(&[100]);
        t.i16s(&[300]);
        t.i16s(&[500, -500]);
        t.i16s(&[1, 0, 0, 10, 10]);
        t.u16s(&[2, 0]);
        t.u8s(&[49, 53, 51]);
        t.u8s(&[10]);
        t.u8s(&[10, 0]);
        t.i16s(&[1, -1000, -400, 0, 0]);
        t.u16s(&[2, 0]);
        t.u8s(&[49, 16, 33]);
        t.i16s(&[-1000]);
        t.i16s(&[-400]);
        t.u8s(&[0]);
        font.insert(*b"glyf", t.0);
        // hdmx
        let mut t = Be::default();
        t.u16s(&[0, 2]);
        t.i32s(&[12]);
        t.u8s(&[12, 12, 2, 14, 9, 4, 16, 11, 6, 1, 0, 0, 9, 9, 10, 2, 11, 3, 12, 4, 13, 5, 0, 0]);
        font.insert(*b"hdmx", t.0);
        // head
        let mut t = Be::default();
        t.u32s(&[0x10000, 0x18000, 0x12345678, 0x5F0F3CF5]);
        t.u16s(&[11, 0x3E8]);
        t.u32s(&[0, 0xD5B1F000, 0, 0x100]);
        t.i16s(&[-1000, -400, 600, 700]);
        t.u16s(&[1, 9]);
        t.i16s(&[2, 0, 0]);
        font.insert(*b"head", t.0);
        // hhea
        let mut t = Be::default();
        t.u32s(&[0x10000]);
        t.i16s(&[800, -200, 90]);
        t.u16s(&[0x28A]);
        t.i16s(&[-1000, -50, 600, 1, 0, 0, 0, 0, 0, 0, 0]);
        t.u16s(&[6]);
        font.insert(*b"hhea", t.0);
        // hmtx
        let mut t = Be::default();
        t.u16s(&[0x1F4]);
        t.i16s(&[50]);
        t.u16s(&[0x258]);
        t.i16s(&[0]);
        t.u16s(&[0x258]);
        t.i16s(&[-20]);
        t.u16s(&[0x258]);
        t.i16s(&[0]);
        t.u16s(&[250]);
        t.i16s(&[0]);
        t.u16s(&[0x1F4]);
        t.i16s(&[100, 0, -1000]);
        font.insert(*b"hmtx", t.0);
        // loca
        let mut t = Be::default();
        t.u16s(&[0, 15, 39, 57, 73, 73, 108, 118, 129]);
        font.insert(*b"loca", t.0);
        // maxp
        let mut t = Be::default();
        t.u32s(&[0x10000]);
        t.u16s(&[8, 10, 2, 12, 3, 2, 0, 1, 1, 0, 64, 40, 2, 1]);
        font.insert(*b"maxp", t.0);
        // name
        let mut t = Be::default();
        t.u16s(&[
            0, 10, 126, 0, 3, 0, 3, 22, 0, 1, 0, 0, 1, 8, 22, 1, 0, 0, 2, 10, 30, 3, 1, 0x409, 1, 22, 40, 3, 1, 0x409, 2, 14, 62, 3, 1, 0x409, 5, 50,
            76, 3, 1, 0x409, 0x100, 20, 126, 3, 1, 0x409, 0x101, 4, 146, 3, 1, 0x409, 0x102, 5, 150, 3, 1, 0x40C, 0x103, 8, 155,
        ]);
        t.utf16("Test:Unique");
        t.bytes(b"Test F\x8em");
        t.bytes(b"Bold \xa5 <&>");
        t.utf16("Test Family");
        t.utf16("Regular");
        t.utf16("Version 1.500; \u{22}q\u{22} & <x>\u{d}");
        t.utf16("Smile \u{1f600} \u{e9}");
        t.bytes(b"\xd8\x00\x00A");
        t.bytes(b"\x00A\x00B\x00");
        t.utf16("\u{0}A\u{0}B");
        font.insert(*b"name", t.0);
        // post
        let mut t = Be::default();
        t.u32s(&[0x20000, 0xFFF38000]);
        t.i16s(&[-100, 50]);
        t.u32s(&[0, 0, 0, 0, 0]);
        t.u16s(&[8, 0, 36, 37, 38, 3, 0x102, 0x102, 0x103]);
        t.u8s(&[1]);
        t.bytes(b"a");
        font.insert(*b"post", t.0);
        // prep
        let mut t = Be::default();
        t.u8s(&[185, 128, 0, 127, 255, 0, 192, 29, 65, 2, 1, 0, 255, 254, 229, 146]);
        font.insert(*b"prep", t.0);
        // vhea
        let mut t = Be::default();
        t.u32s(&[0x11000]);
        t.i16s(&[500, -500, 0]);
        t.u16s(&[0x3E8]);
        t.i16s(&[0, 0, 1000, 0, 1, 0, 0, 0, 0, 0, 0]);
        t.u16s(&[8]);
        font.insert(*b"vhea", t.0);
        // vmtx
        let mut t = Be::default();
        t.u16s(&[0x3E8]);
        t.i16s(&[100]);
        t.u16s(&[0x3E8]);
        t.i16s(&[101]);
        t.u16s(&[0x3E8]);
        t.i16s(&[102]);
        t.u16s(&[0x3E8]);
        t.i16s(&[103]);
        t.u16s(&[0x3E8]);
        t.i16s(&[104]);
        t.u16s(&[0x3E8]);
        t.i16s(&[105]);
        t.u16s(&[0x3E8]);
        t.i16s(&[106]);
        t.u16s(&[0x3E8]);
        t.i16s(&[107]);
        font.insert(*b"vmtx", t.0);
        // zzzz
        let mut t = Be::default();
        t.bytes(b"unknown table data, kept as hex");
        font.insert(*b"zzzz", t.0);
        font
    }

    /// A `gasp` table of an unknown version and an `hhea` table cut short.
    pub(super) fn broken_font() -> SfntFont {
        let mut font = SfntFont::new(Flavour::TrueType);
        // gasp
        let mut t = Be::default();
        t.u16s(&[2, 1, 8, 2]);
        font.insert(*b"gasp", t.0);
        // hhea
        let mut t = Be::default();
        t.u16s(&[1, 0]);
        font.insert(*b"hhea", t.0);
        // maxp
        let mut t = Be::default();
        t.u32s(&[0x10000]);
        t.u16s(&[8, 10, 2, 12, 3, 2, 0, 1, 1, 0, 64, 40, 2, 1]);
        font.insert(*b"maxp", t.0);
        font
    }
}
