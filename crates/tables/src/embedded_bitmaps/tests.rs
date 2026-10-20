use fontcraft_model::strike::{BitmapStrike, StrikeGlyph};

use super::*;
use crate::ttx::{DumpOptions, ImportOptions, dump, import};

/// fontTools 4.66's dump of the two strikes below (`EBLC` and `EBDT` of a font whose only other
/// tables are a version 0.5 `maxp` and a version 3 `post`), and its bytes for them.
const TTX: &str = include_str!("testdata/strikes.ttx");
const EBDT_HEX: &str = "00020000070500070620508888f888880704010706e09090e09090e00203000004a040000000000402020105050080ff40";
const EBLC_HEX: &str = concat!(
    "00020000000000020000006800000030000000020000000009fd06010000000107fe00000000000000000000000000000001",
    "00140c0c01010000009800000014000000010000000005000501000001020503000000000000000000000000000000010001",
    "101008010001000500000010001400140000002400030001000000040000000c001800180018001f00030001000000230000",
    "00050001000100000008000300010000002800000009",
);

fn hex(text: &str) -> Vec<u8> {
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap()).collect()
}

fn glyph(name: &str, (width, height): (u16, u16), (left, bottom): (i32, i32), advance: i32, data: &[u8]) -> StrikeGlyph {
    StrikeGlyph { glyph: name.into(), code: None, advance, scalable_width: None, width, height, left, bottom, data: data.to_vec() }
}

fn strikes() -> (BitmapStrike, BitmapStrike) {
    let mut small = BitmapStrike::new(12, 1);
    small.ascent = 9;
    small.descent = 3;
    small.glyphs = vec![
        glyph("g20", (0, 0), (0, 0), 4, &[]),
        glyph("g1", (5, 7), (0, 0), 6, &[0x20, 0x50, 0x88, 0x88, 0xf8, 0x88, 0x88]),
        glyph("missing", (1, 1), (0, 0), 2, &[0x80]),
        glyph("g5", (3, 2), (0, -2), 4, &[0xa0, 0x40]),
        glyph("g2", (4, 7), (1, 0), 6, &[0xe0, 0x90, 0x90, 0xe0, 0x90, 0x90, 0xe0]),
    ];
    let mut grey = BitmapStrike::new(16, 8);
    grey.glyphs = vec![glyph("g1", (2, 2), (1, 3), 5, &[0x00, 0x80, 0xff, 0x40])];
    (small, grey)
}

fn ids(g: &StrikeGlyph) -> Option<u16> {
    g.glyph.strip_prefix('g').and_then(|n| n.parse().ok())
}

/// Comments left out, trailing spaces trimmed, blank lines dropped.
fn normalised(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find("<!--") {
        out.push_str(&rest[..i]);
        rest = rest[i..].find("-->").map_or("", |j| &rest[i + j + 3..]);
    }
    out.push_str(rest);
    out.lines().map(str::trim_end).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}

#[test]
fn strikes_compile_to_the_bytes_and_ttx_fonttools_gives() {
    let (small, grey) = strikes();
    let tables = build(&[&small, &grey], &ids).unwrap();
    assert_eq!(tables.ebdt, hex(EBDT_HEX));
    assert_eq!(tables.eblc, hex(EBLC_HEX));
    assert_eq!(tables.strikes, vec![WrittenStrike { label: "12".into(), glyphs: 4 }, WrittenStrike { label: "16@8".into(), glyphs: 1 }]);
    assert_eq!((tables.left_out.clone(), tables.left_out_count), (vec!["missing".to_string()], 1));

    // maxp 0.5 with 21 glyphs, post 3.0: the glyphs are named glyph00001… as in fontTools.
    let mut font = SfntFont::new(Flavour::TrueType);
    font.insert(*b"maxp", vec![0, 0, 0x50, 0, 0, 21]);
    let mut post = vec![0u8; 32];
    post[1] = 3;
    font.insert(*b"post", post);
    embed(&mut font, &tables);
    let options = DumpOptions { tables: vec![*b"EBLC", *b"EBDT"], no_version: true, strict: true, ..DumpOptions::default() };
    let ours = dump(&font, &options).unwrap();
    assert_eq!(normalised(&ours), normalised(TTX));

    // The TTX reader compiles the dump back to the same bytes.
    let again = import(font.clone(), &ours, &ImportOptions::default()).unwrap();
    assert_eq!(again.get(*b"EBDT"), font.get(*b"EBDT"));
    assert_eq!(again.get(*b"EBLC"), font.get(*b"EBLC"));
}

#[test]
fn strikes_beyond_the_embedded_formats_are_refused() {
    let (small, _) = strikes();
    let mut big = small.clone();
    big.ppem = 256;
    assert!(matches!(build(&[&big], &ids), Err(TablesError::Unsupported(_))));

    let mut tall = small.clone();
    tall.glyphs = vec![glyph("g1", (1, 1), (0, 200), 2, &[0x80])];
    let err = build(&[&tall], &ids).unwrap_err().to_string();
    assert!(err.contains("`g1`"), "{err}");

    let mut none = small;
    none.glyphs.retain(|g| g.glyph == "missing");
    assert!(build(&[&none], &ids).is_err());
}

#[test]
fn a_bitmap_only_font_keeps_the_metadata_tables_and_empties_the_outlines() {
    let (small, _) = strikes();
    let tables = build(&[&small], &ids).unwrap();
    let mut compiled = SfntFont::new(Flavour::TrueType);
    let mut head = vec![0u8; 54];
    head[51] = 1;
    compiled.insert(*b"head", head);
    compiled.insert(*b"maxp", [&[0u8, 1, 0, 0, 0, 21][..], &[7; 26]].concat());
    for tag in [b"hhea", b"hmtx", b"cmap", b"name", b"OS/2", b"post", b"GSUB", b"fpgm", b"gasp"] {
        compiled.insert(*tag, vec![1, 2, 3, 4]);
    }
    compiled.insert(*b"glyf", vec![9; 40]);
    compiled.insert(*b"loca", vec![9; 44]);
    let otb = bitmap_only(&compiled, &tables).unwrap();
    let mut tags: Vec<String> = otb.tables.keys().map(|t| String::from_utf8_lossy(t).into_owned()).collect();
    tags.sort();
    assert_eq!(tags, ["EBDT", "EBLC", "OS/2", "cmap", "glyf", "head", "hhea", "hmtx", "loca", "maxp", "name", "post"]);
    assert_eq!(otb.get(*b"glyf"), Some(&[][..]));
    assert_eq!(otb.get(*b"loca"), Some(&[0u8; 44][..]));
    assert_eq!(otb.get(*b"head").unwrap()[50..54], [0, 0, 0, 0]);
    let maxp = otb.get(*b"maxp").unwrap();
    assert_eq!(maxp.len(), 32);
    assert_eq!(maxp[..6], [0, 1, 0, 0, 0, 21]);
    assert_eq!(maxp[14..16], [0, 1]);
    assert!(maxp[6..14].iter().chain(&maxp[16..]).all(|b| *b == 0));
    let bytes = crate::write(&otb, &crate::WriteOptions::default()).unwrap();
    assert_eq!(crate::read(&bytes, 0).unwrap().get(*b"EBLC"), Some(tables.eblc.as_slice()));
}
