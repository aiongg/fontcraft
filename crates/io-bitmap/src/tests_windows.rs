//! Gzip-compressed PCF, Windows FNT and FON: strikes built in code round trip through each, and
//! hostile headers are refused with a message.

use std::collections::BTreeSet;

use crate::*;

fn glyph(name: &str, code: Option<u32>, advance: i32, left: i32, bottom: i32, rows: &[&str]) -> BitmapGlyph {
    let width = rows.first().map_or(0, |r| r.len()) as u16;
    let mut bitmap = Bitmap::blank(width, rows.len() as u16, left, bottom, 1).unwrap();
    for (y, row) in rows.iter().enumerate() {
        for (x, c) in row.chars().enumerate() {
            if c == '#' {
                bitmap.set_pixel(x as u16, y as u16, 1);
            }
        }
    }
    BitmapGlyph { name: name.into(), encoding: code, advance, scalable_width: None, bitmap }
}

fn text(name: &str, value: &str) -> Property {
    Property { name: name.into(), value: PropertyValue::Text(value.into()) }
}

/// A 9 pixel strike in Unicode: a space, `A`, a `g` below the baseline, a 10-pixel-wide `W`
/// (two byte strips), the euro sign (0x80 in code page 1252) and `é`.
fn strike(pixel_size: u16) -> BitmapFont {
    BitmapFont {
        name: "Pixel".into(),
        pixel_size,
        point_size: 9.0,
        resolution: (96, 96),
        depth: 1,
        ascent: 7,
        descent: 2,
        properties: vec![
            text("FAMILY_NAME", "Pixel"),
            text("WEIGHT_NAME", "Bold"),
            text("SLANT", "R"),
            text("COPYRIGHT", "nobody"),
            text("CHARSET_REGISTRY", "ISO10646"),
            text("CHARSET_ENCODING", "1"),
        ],
        glyphs: vec![
            glyph("space", Some(0x20), 3, 0, 0, &[]),
            glyph("A", Some(0x41), 6, 0, 0, &[".###.", "#...#", "#####", "#...#", "#...#"]),
            glyph("g", Some(0x67), 5, 0, -2, &[".###", "#..#", ".###", "...#", "###."]),
            glyph("W", Some(0x57), 11, 0, 0, &["#........#", "#........#", "#...##...#", "#.#....#.#", "##......##"]),
            glyph("Euro", Some(0x20AC), 6, 0, 0, &["..###", ".#...", "####.", ".#...", "..###"]),
            glyph("eacute", Some(0xE9), 5, 0, 0, &["..#.", ".##.", "#..#", "###.", "#...", ".###"]),
        ],
    }
}

/// Each lit pixel of a glyph, from the origin: x to the right, y up.
fn ink(g: &BitmapGlyph) -> BTreeSet<(i32, i32)> {
    let b = &g.bitmap;
    let mut out = BTreeSet::new();
    for y in 0..b.height {
        for x in 0..b.width {
            if b.pixel(x, y) != 0 {
                out.insert((b.left + i32::from(x), b.bottom + i32::from(b.height) - 1 - i32::from(y)));
            }
        }
    }
    out
}

/// Code, advance and ink of every glyph, in code order.
fn shapes(font: &BitmapFont) -> Vec<(Option<u32>, i32, BTreeSet<(i32, i32)>)> {
    let mut out: Vec<_> = font.glyphs.iter().map(|g| (g.encoding, g.advance, ink(g))).collect();
    out.sort_by_key(|(code, _, _)| *code);
    out
}

#[test]
fn pcf_gz_reads_back_as_the_pcf_it_holds() {
    let font = strike(9);
    let packed = write_pcf_gz(&font, PcfLayout::default()).unwrap();
    assert!(is_gzip(&packed));
    let plain = read_pcf(&write_pcf(&font, PcfLayout::default()).unwrap()).unwrap();
    assert_eq!(read(&packed).unwrap(), plain);
    assert_eq!(read_pcf(&packed).unwrap(), plain, "read_pcf takes the compressed form too");
    assert_eq!(read_all(&packed).unwrap(), vec![plain]);
    // A gzip-compressed BDF reads as well.
    let bdf = fontcraft_container::gzip::gzip(write_bdf(&font).unwrap().as_bytes());
    assert_eq!(shapes(&read(&bdf).unwrap().font), shapes(&font));
}

#[test]
fn hostile_gzip_data_is_refused_with_a_message() {
    let packed = write_pcf_gz(&strike(9), PcfLayout::default()).unwrap();
    for len in 0..packed.len() {
        assert!(read(&packed[..len]).is_err(), "cut at {len}");
    }
    let header = [0x1f, 0x8b, 8, 0, 0, 0, 0, 0];
    assert!(matches!(read(&header), Err(BitmapError::Gzip(_))));
    let junk = fontcraft_container::gzip::gzip(b"not a font");
    let e = read(&junk).unwrap_err().to_string();
    assert!(e.contains("neither a PCF nor a BDF"), "{e}");
    // A trailer that declares more than the cap is refused before inflating.
    let mut big = packed.clone();
    let n = big.len();
    big[n - 4..].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(read(&big), Err(BitmapError::Limit { .. })));
}

#[test]
fn fnt_round_trips_a_strike() {
    let font = strike(9);
    let written = write_fnt(&font).unwrap();
    assert!(written.warnings.is_empty(), "{:?}", written.warnings);
    assert!(is_fnt(&written.bytes));
    let import = read(&written.bytes).unwrap();
    assert!(import.warnings.is_empty(), "{:?}", import.warnings);
    let back = import.font;
    assert_eq!((back.pixel_size, back.ascent, back.descent, back.depth), (9, 7, 2, 1));
    assert_eq!((back.point_size, back.resolution), (9.0, (96, 96)));
    assert_eq!(back.name, "Pixel");
    assert_eq!(back.text_property("FAMILY_NAME"), Some("Pixel"));
    assert_eq!(back.text_property("WEIGHT_NAME"), Some("Bold"));
    assert_eq!(back.text_property("COPYRIGHT"), Some("nobody"));
    assert!(back.codes_are_unicode());
    // Codes between the first and last with no glyph are not read back.
    assert_eq!(back.glyphs.len(), font.glyphs.len());
    assert_eq!(shapes(&back), shapes(&font));
    // Every glyph is a cell as wide as its advance.
    let w = back.glyphs.iter().find(|g| g.encoding == Some(0x57)).unwrap();
    assert_eq!((w.name.as_str(), w.bitmap.width, w.bitmap.height, w.bitmap.bottom), ("uni0057", 11, 9, -2));
    // The writer's output reads back to the same bytes.
    assert_eq!(write_fnt(&back).unwrap().bytes, written.bytes);
}

/// The version 2 form of a version 3 file: the shorter header and 16-bit table offsets.
fn to_version_2(v3: &[u8]) -> Vec<u8> {
    let count = usize::from(v3[96] - v3[95]) + 2;
    let shrink = (148 - 118) + 2 * count;
    let mut out = v3[..118].to_vec();
    out[0..2].copy_from_slice(&0x0200u16.to_le_bytes());
    for i in 0..count {
        let at = 148 + 6 * i;
        out.extend_from_slice(&v3[at..at + 2]);
        let offset = u32::from_le_bytes(v3[at + 2..at + 6].try_into().unwrap()) as usize - shrink;
        out.extend_from_slice(&(offset as u16).to_le_bytes());
    }
    out.extend_from_slice(&v3[148 + 6 * count..]);
    let size = out.len() as u32;
    out[2..6].copy_from_slice(&size.to_le_bytes());
    for field in [105, 113] {
        let v = u32::from_le_bytes(out[field..field + 4].try_into().unwrap()) - shrink as u32;
        out[field..field + 4].copy_from_slice(&v.to_le_bytes());
    }
    out
}

#[test]
fn fnt_version_2_reads_as_version_3() {
    let v3 = write_fnt(&strike(9)).unwrap().bytes;
    let v2 = to_version_2(&v3);
    assert!(is_fnt(&v2));
    assert_eq!(read_fnt(&v2).unwrap(), read_fnt(&v3).unwrap());
}

#[test]
fn fnt_writer_lists_what_it_leaves_out_and_refuses_what_it_cannot_hold() {
    let mut font = strike(9);
    font.glyphs.push(glyph("uni4E00", Some(0x4E00), 9, 0, 0, &["#########"]));
    font.glyphs.push(glyph("wide", Some(0x42), 2, -1, 0, &["####"]));
    font.glyphs.push(glyph("again", Some(0x41), 6, 0, 0, &["#"]));
    let written = write_fnt(&font).unwrap();
    let all = written.warnings.join(" | ");
    assert!(all.contains("1 glyphs have no code") && all.contains("uni4E00"), "{all}");
    assert!(all.contains("1 glyphs have a code an earlier glyph has"), "{all}");
    assert!(all.contains("1 glyphs have pixels outside their advance"), "{all}");
    let back = read_fnt(&written.bytes).unwrap().font;
    let b = back.glyphs.iter().find(|g| g.encoding == Some(0x42)).unwrap();
    assert_eq!(ink(b), BTreeSet::from([(0, 0), (1, 0)]), "the pixels left of the origin and past the advance are cut");

    let mut grey = strike(9);
    grey.depth = 4;
    assert!(matches!(write_fnt(&grey), Err(BitmapError::Write(_))));
    let mut none = strike(9);
    for g in &mut none.glyphs {
        g.encoding = Some(0x3000);
    }
    assert!(matches!(write_fnt(&none), Err(BitmapError::Write(_))));
    let mut tall = strike(9);
    tall.ascent = 2_000;
    assert!(matches!(write_fnt(&tall), Err(BitmapError::Write(_))));
}

#[test]
fn hostile_fnt_headers_are_refused_with_a_message() {
    let good = write_fnt(&strike(9)).unwrap().bytes;
    for len in 0..good.len() {
        let _ = read_fnt(&good[..len]);
    }
    assert!(matches!(read_fnt(&good[..100]), Err(BitmapError::Fnt(_))));
    // The blank glyph after the last code is not read; cutting one byte more cuts the last bitmap.
    let blank = usize::from(u16::from_le_bytes([good[91], good[92]])).div_ceil(8) * 9;
    let e = read_fnt(&good[..good.len() - blank - 1]).unwrap_err().to_string();
    assert!(e.contains("outside the file"), "{e}");
    let patched = |at: usize, value: &[u8]| {
        let mut b = good.clone();
        b[at..at + value.len()].copy_from_slice(value);
        b
    };
    let cases: [(Vec<u8>, &str); 7] = [
        (patched(0, &0x0100u16.to_le_bytes()), "version"),
        (patched(66, &1u16.to_le_bytes()), "vector"),
        (patched(88, &5_000u16.to_le_bytes()), "limit"),
        (patched(88, &0u16.to_le_bytes()), "height is 0"),
        (patched(118, &0x0004u32.to_le_bytes()), "ABC"),
        (patched(148, &5_000u16.to_le_bytes()), "limit"),
        (patched(150, &u32::MAX.to_le_bytes()), "outside the file"),
    ];
    for (bytes, words) in &cases {
        let e = read_fnt(bytes).unwrap_err().to_string();
        assert!(e.contains(words), "{words}: {e}");
    }
    let mut backwards = good.clone();
    backwards[95] = 0x90;
    backwards[96] = 0x20;
    assert!(!is_fnt(&backwards));
    assert!(read_fnt(&backwards).unwrap_err().to_string().contains("before its first"));
}

#[test]
fn fon_holds_every_strike_and_reads_back() {
    let mut big = strike(12);
    big.ascent = 10;
    big.descent = 2;
    let fonts = [strike(9), big];
    let written = write_fon(&fonts).unwrap();
    assert!(written.warnings.is_empty(), "{:?}", written.warnings);
    assert!(is_fon(&written.bytes));
    let all = read_all(&written.bytes).unwrap();
    assert_eq!(all.len(), 2);
    for (import, font) in all.iter().zip(&fonts) {
        assert!(import.warnings.is_empty(), "{:?}", import.warnings);
        assert_eq!(import.font, read_fnt(&write_fnt(font).unwrap().bytes).unwrap().font);
    }
    assert_eq!((all[0].font.pixel_size, all[1].font.pixel_size), (9, 12));
    // `read` gives the first font and says the others were left out.
    let first = read(&written.bytes).unwrap();
    assert_eq!(first.font, all[0].font);
    assert!(first.warnings.iter().any(|w| w.contains("1 more fonts")), "{:?}", first.warnings);

    assert!(matches!(write_fon(&[]), Err(BitmapError::Write(_))));
    let mut grey = strike(9);
    grey.depth = 2;
    let e = write_fon(&[strike(9), grey]).unwrap_err().to_string();
    assert!(e.contains("9 pixel strike"), "{e}");
}

#[test]
fn hostile_fon_files_are_refused_with_a_message() {
    let good = write_fon(&[strike(9), strike(10)]).unwrap().bytes;
    for len in 0..good.len() {
        let _ = read_fon(&good[..len]);
    }
    let ne = 64;
    let table = ne + usize::from(u16::from_le_bytes([good[ne + 0x24], good[ne + 0x25]]));
    let patched = |at: usize, value: &[u8]| {
        let mut b = good.clone();
        b[at..at + value.len()].copy_from_slice(value);
        b
    };
    let mut both_out = good.clone();
    for entry in [table + 2 + 20 + 8, table + 2 + 20 + 8 + 12] {
        both_out[entry..entry + 2].copy_from_slice(&u16::MAX.to_le_bytes());
    }
    let cases: [(Vec<u8>, &str); 6] = [
        (patched(0x3C, &u32::MAX.to_le_bytes()), "no NE header"),
        (patched(ne, b"PE"), "32-bit"),
        (patched(table, &40u16.to_le_bytes()), "shift"),
        // The font directory claims 65,535 entries.
        (patched(table + 4, &u16::MAX.to_le_bytes()), "ends inside"),
        // Both fonts point past the end of the file.
        (both_out, "none of its fonts"),
        // No font type: the font list is renamed to another type.
        (patched(table + 2 + 20, &0x8003u16.to_le_bytes()), "no font"),
    ];
    for (bytes, words) in &cases {
        let e = read_fon(bytes).unwrap_err().to_string();
        assert!(e.contains(words), "{words}: {e}");
    }
    // One font out of reach: the other is read, with a warning.
    let mut fonts_moved = good.clone();
    let second = table + 2 + 20 + 8 + 12;
    fonts_moved[second..second + 2].copy_from_slice(&u16::MAX.to_le_bytes());
    let read_back = read_fon(&fonts_moved).unwrap();
    assert_eq!(read_back.len(), 1);
    assert!(read_back[0].warnings.iter().any(|w| w.contains("font 2")), "{:?}", read_back[0].warnings);
    // A resource table without its end is bounded.
    let mut endless = good[..table + 2].to_vec();
    for _ in 0..300 {
        endless.extend_from_slice(&[1, 0x80, 0, 0, 0, 0, 0, 0]);
    }
    assert!(matches!(read_fon(&endless), Err(BitmapError::Limit { .. })));
}
