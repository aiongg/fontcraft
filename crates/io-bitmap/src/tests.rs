use crate::*;

/// Three glyphs: one at the origin, one wider than a byte that reaches below the baseline and
/// left of the origin, and one outside the encoding. The bitmaps the reference converter writes
/// for this file in two layouts are the expected bytes of
/// `pcf_reads_rows_laid_out_as_the_reference_converter_writes_them`.
const SAMPLE: &str = "STARTFONT 2.1
COMMENT a test font
FONT -test-mini-medium-r-normal--10-100-75-75-c-80-iso10646-1
SIZE 10 75 75
FONTBOUNDINGBOX 12 10 -1 -2
STARTPROPERTIES 5
FONT_ASCENT 8
FONT_DESCENT 2
PIXEL_SIZE 10
COPYRIGHT \"a \"\"quoted\"\" note\"
DEFAULT_CHAR 65
ENDPROPERTIES
CHARS 3
STARTCHAR A
ENCODING 65
SWIDTH 600 0
DWIDTH 6 0
BBX 5 3 0 0
BITMAP
88
F8
20
ENDCHAR
STARTCHAR wide
ENCODING 300
SWIDTH 1200 0
DWIDTH 12 0
BBX 12 2 -1 -2
BITMAP
8010
F0F0
ENDCHAR
STARTCHAR odd
ENCODING -1 1234
SWIDTH 500 0
DWIDTH 5 0
BBX 3 1 1 4
BITMAP
A0
ENDCHAR
ENDFONT
";

const A_ROWS: [&str; 3] = ["#...#", "#####", "..#.."];
const WIDE_ROWS: [&str; 2] = ["#..........#", "####....####"];

fn rows(b: &Bitmap) -> Vec<String> {
    (0..b.height).map(|y| (0..b.width).map(|x| if b.pixel(x, y) != 0 { '#' } else { '.' }).collect()).collect()
}

fn sample() -> BitmapFont {
    read_bdf(SAMPLE.as_bytes()).unwrap().font
}

#[test]
fn bdf_reads_the_strike_its_glyphs_and_properties() {
    let import = read_bdf(SAMPLE.as_bytes()).unwrap();
    assert!(import.warnings.is_empty(), "{:?}", import.warnings);
    let font = import.font;
    assert_eq!(font.name, "-test-mini-medium-r-normal--10-100-75-75-c-80-iso10646-1");
    assert_eq!((font.pixel_size, font.point_size, font.resolution, font.depth), (10, 10.0, (75, 75), 1));
    assert_eq!((font.ascent, font.descent), (8, 2));
    assert_eq!(font.text_property("COPYRIGHT"), Some("a \"quoted\" note"));
    assert_eq!(font.integer_property("DEFAULT_CHAR"), Some(65));
    assert_eq!(font.glyphs.len(), 3);

    let a = &font.glyphs[0];
    assert_eq!((a.name.as_str(), a.encoding, a.advance, a.scalable_width), ("A", Some(65), 6, Some(600)));
    assert_eq!((a.bitmap.width, a.bitmap.height, a.bitmap.left, a.bitmap.bottom), (5, 3, 0, 0));
    assert_eq!(rows(&a.bitmap), A_ROWS);

    let wide = &font.glyphs[1];
    assert_eq!(wide.encoding, Some(300));
    assert_eq!((wide.bitmap.left, wide.bitmap.bottom), (-1, -2));
    assert_eq!(rows(&wide.bitmap), WIDE_ROWS);

    let odd = &font.glyphs[2];
    assert_eq!((odd.encoding, odd.bitmap.bottom), (None, 4));
    assert_eq!(rows(&odd.bitmap), ["#.#"]);
}

#[test]
fn bdf_writer_output_reads_back_unchanged() {
    let font = sample();
    let text = write_bdf(&font).unwrap();
    assert!(text.starts_with("STARTFONT 2.1\nFONT -test-mini"), "{text}");
    assert!(text.contains("COPYRIGHT \"a \"\"quoted\"\" note\"\n"), "{text}");
    assert!(text.contains("STARTCHAR wide\nENCODING 300\nSWIDTH 1200 0\nDWIDTH 12 0\nBBX 12 2 -1 -2\nBITMAP\n8010\nF0F0\nENDCHAR\n"));
    assert!(text.contains("STARTCHAR odd\nENCODING -1\n"));
    let back = read_bdf(text.as_bytes()).unwrap();
    assert!(back.warnings.is_empty(), "{:?}", back.warnings);
    assert_eq!(back.font, font);
}

#[test]
fn bdf_reads_and_writes_grey_pixels() {
    let text = "STARTFONT 2.3\nFONT grey\nSIZE 8 72 72 2\nFONTBOUNDINGBOX 4 2 0 0\nCHARS 1\nSTARTCHAR g\nENCODING 103\nDWIDTH 5 0\nBBX 4 2 0 0\nBITMAP\n1B\nE4\nENDCHAR\nENDFONT\n";
    let font = read_bdf(text.as_bytes()).unwrap().font;
    assert_eq!((font.depth, font.pixel_size), (2, 8));
    let b = &font.glyphs[0].bitmap;
    let values: Vec<u8> = (0..2).flat_map(|y| (0..4).map(move |x| (x, y))).map(|(x, y)| b.pixel(x, y)).collect();
    assert_eq!(values, [0, 1, 2, 3, 3, 2, 1, 0]);
    let written = write_bdf(&font).unwrap();
    assert!(written.starts_with("STARTFONT 2.3\n") && written.contains("SIZE 8 72 72 2\n"), "{written}");
    let back = read_bdf(written.as_bytes()).unwrap().font;
    assert_eq!((back.glyphs, back.depth, back.ascent, back.descent), (font.glyphs.clone(), 2, 2, 0));
    assert!(matches!(write_pcf(&font, PcfLayout::default()), Err(BitmapError::Write(_))));
}

fn layouts() -> Vec<PcfLayout> {
    let mut all = Vec::new();
    for glyph_pad in [1, 2, 4, 8] {
        for scan_unit in [1, 2, 4].into_iter().filter(|u| *u <= glyph_pad) {
            for (msb_byte_first, msb_bit_first) in [(true, true), (true, false), (false, true), (false, false)] {
                for compressed_metrics in [true, false] {
                    all.push(PcfLayout { glyph_pad, scan_unit, msb_byte_first, msb_bit_first, compressed_metrics });
                }
            }
        }
    }
    all
}

#[test]
fn pcf_written_in_every_layout_reads_back_as_the_bdf() {
    let font = sample();
    for layout in layouts() {
        let bytes = write_pcf(&font, layout).unwrap();
        assert!(is_pcf(&bytes));
        let back = read_pcf(&bytes).unwrap_or_else(|e| panic!("{layout:?}: {e}"));
        assert!(back.warnings.is_empty(), "{layout:?}: {:?}", back.warnings);
        let back = back.font;
        assert_eq!(back.glyphs, font.glyphs, "{layout:?}");
        assert_eq!(back.name, font.name);
        assert_eq!((back.pixel_size, back.point_size, back.resolution), (10, 10.0, (75, 75)));
        assert_eq!((back.ascent, back.descent), (8, 2));
        assert_eq!(back.text_property("COPYRIGHT"), Some("a \"quoted\" note"));
    }
}

/// Little-endian bytes of `values`, each `size` bytes wide.
fn le(values: &[i64], size: usize) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes().into_iter().take(size)).collect()
}

/// A PCF file holding `tables` (type, format, body without the format word).
fn pcf_file(tables: &[(u32, u32, Vec<u8>)]) -> Vec<u8> {
    let mut file = b"\x01fcp".to_vec();
    file.extend(le(&[tables.len() as i64], 4));
    let mut offset = 8 + 16 * tables.len();
    let mut body = Vec::new();
    for (kind, format, data) in tables {
        let mut table = le(&[i64::from(*format)], 4);
        table.extend(data);
        table.resize(table.len().div_ceil(4) * 4, 0);
        file.extend(le(&[i64::from(*kind), i64::from(*format), table.len() as i64, offset as i64], 4));
        offset += table.len();
        body.extend(table);
    }
    file.extend(body);
    file
}

/// Uncompressed little-endian metrics for (left, right, advance, ascent, descent) glyphs.
fn metrics_body(glyphs: &[[i64; 5]]) -> Vec<u8> {
    let mut body = le(&[glyphs.len() as i64], 4);
    for g in glyphs {
        body.extend(le(g, 2));
        body.extend(le(&[0], 2));
    }
    body
}

#[test]
fn pcf_reads_rows_laid_out_as_the_reference_converter_writes_them() {
    // Bitmaps tables of the sample font as the reference converter wrote them with rows padded
    // to 4 bytes: least significant byte first with most significant bit first and 2-byte scan
    // units (format 0x1a), and least significant byte and bit first with 4-byte units (0x22).
    let cases: [(u32, [u8; 20]); 2] = [
        (0x1a, [0, 0x88, 0, 0, 0, 0xf8, 0, 0, 0, 0x20, 0, 0, 0x10, 0x80, 0, 0, 0xf0, 0xf0, 0, 0]),
        (0x22, [0x11, 0, 0, 0, 0x1f, 0, 0, 0, 0x04, 0, 0, 0, 0x01, 0x08, 0, 0, 0x0f, 0x0f, 0, 0]),
    ];
    for (format, data) in cases {
        let metrics = metrics_body(&[[0, 5, 6, 3, 0], [-1, 11, 12, 0, 2]]);
        let mut bitmaps = le(&[2, 0, 12, 8, 12, 20, 40], 4);
        bitmaps.extend(data);
        let file = pcf_file(&[(4, format & !0x100, metrics), (8, format, bitmaps)]);
        let font = read_pcf(&file).unwrap().font;
        assert_eq!(rows(&font.glyphs[0].bitmap), A_ROWS, "format {format:#x}");
        assert_eq!(rows(&font.glyphs[1].bitmap), WIDE_ROWS, "format {format:#x}");
        assert_eq!((font.glyphs[0].name.as_str(), font.glyphs[0].encoding), ("glyph0", None));
        assert_eq!((font.glyphs[1].bitmap.left, font.glyphs[1].bitmap.bottom, font.glyphs[1].advance), (-1, -2, 12));
        assert_eq!((font.ascent, font.descent), (3, 2));
    }
}

fn limit(result: Result<Import>) -> &'static str {
    match result {
        Err(BitmapError::Limit { what, .. }) => what,
        other => panic!("expected a limit error, got {other:?}"),
    }
}

#[test]
fn bdf_refuses_hostile_sizes_and_counts() {
    let head = "STARTFONT 2.1\nFONT x\nSIZE 10 72 72\nFONTBOUNDINGBOX 1 1 0 0\n";
    let huge_glyph = format!("{head}CHARS 1\nSTARTCHAR a\nBBX 100000 1 0 0\nBITMAP\n00\nENDCHAR\nENDFONT\n");
    assert_eq!(limit(read_bdf(huge_glyph.as_bytes())), "bitmap width or height in pixels");
    let many_glyphs = format!("{head}CHARS 1000000\nENDFONT\n");
    assert_eq!(limit(read_bdf(many_glyphs.as_bytes())), "glyphs in one font");
    let many_properties = format!("{head}STARTPROPERTIES 1000000\nENDPROPERTIES\nCHARS 0\nENDFONT\n");
    assert_eq!(limit(read_bdf(many_properties.as_bytes())), "font properties");

    // Glyphs without bitmap rows still take their box's pixels.
    let mut blanks = format!("{head}CHARS 600\n");
    for _ in 0..600 {
        blanks.push_str("STARTCHAR g\nBBX 1024 1024 0 0\nENDCHAR\n");
    }
    assert_eq!(limit(read_bdf(blanks.as_bytes())), "decoded bitmap bytes");

    let mut big = SAMPLE.as_bytes().to_vec();
    big.resize(MAX_FILE_BYTES + 1, b'\n');
    assert!(matches!(read_bdf(&big), Err(BitmapError::TooLarge(_))));

    let short = format!("{head}CHARS 1\nSTARTCHAR a\nBBX 8 2 0 0\nBITMAP\nFF\nENDCHAR\nENDFONT\n");
    assert!(matches!(read_bdf(short.as_bytes()), Err(BitmapError::Bdf { line: 10, .. })));
    let bad_hex = format!("{head}CHARS 1\nSTARTCHAR a\nBBX 8 1 0 0\nBITMAP\nZZ\nENDCHAR\nENDFONT\n");
    assert!(matches!(read_bdf(bad_hex.as_bytes()), Err(BitmapError::Bdf { line: 9, .. })));
    let unended = format!("{head}CHARS 1\nSTARTCHAR a\nBBX 8 1 0 0\n");
    assert!(matches!(read_bdf(unended.as_bytes()), Err(BitmapError::Bdf { .. })));
    let nan = format!("{head}CHARS 1\nSTARTCHAR a\nDWIDTH NaN 0\nENDCHAR\nENDFONT\n");
    assert!(matches!(read_bdf(nan.as_bytes()), Err(BitmapError::Bdf { .. })));
}

#[test]
fn pcf_refuses_hostile_tables() {
    let one_glyph = metrics_body(&[[0, 8, 8, 1, 0]]);
    let bitmaps = le(&[1, 0, 1, 2, 4, 8], 4).into_iter().chain([0xff]).collect::<Vec<u8>>();

    let mut past_end = pcf_file(&[(4, 0, one_glyph.clone()), (8, 0, bitmaps.clone())]);
    past_end.truncate(past_end.len() - 4);
    assert!(matches!(read_pcf(&past_end), Err(BitmapError::Pcf { .. })));

    let mut many_tables = b"\x01fcp".to_vec();
    many_tables.extend(le(&[1_000_000], 4));
    assert_eq!(limit(read_pcf(&many_tables)), "PCF tables");

    let gzip = [0x1f, 0x8b, 8, 0, 0, 0, 0, 0];
    assert!(matches!(read(&gzip), Err(BitmapError::NotPcf(m)) if m.contains("gzip")));

    let many_glyphs = pcf_file(&[(4, 0, le(&[1_000_000], 4)), (8, 0, bitmaps.clone())]);
    assert_eq!(limit(read_pcf(&many_glyphs)), "glyphs in one font");

    let too_wide = pcf_file(&[(4, 0, metrics_body(&[[0, 2000, 8, 1, 0]])), (8, 0, bitmaps.clone())]);
    assert_eq!(limit(read_pcf(&too_wide)), "bitmap width or height in pixels");

    // 600 glyphs of 1,024 × 1,024 pixels all pointing at the same 128 KiB of data.
    let glyphs = [[0i64, 1024, 0, 1024, 0]; 600];
    let mut shared = le(&[600], 4);
    shared.extend(le(&[0; 600], 4));
    shared.extend(le(&[131_072, 131_072, 131_072, 131_072], 4));
    shared.resize(shared.len() + 131_072, 0);
    let file = pcf_file(&[(4, 0, metrics_body(&glyphs)), (8, 0, shared)]);
    assert_eq!(limit(read_pcf(&file)), "decoded bitmap bytes");

    let offset_past_data = le(&[1, 1, 1, 2, 4, 8], 4).into_iter().chain([0xff]).collect::<Vec<u8>>();
    let file = pcf_file(&[(4, 0, one_glyph.clone()), (8, 0, offset_past_data)]);
    assert!(matches!(read_pcf(&file), Err(BitmapError::Pcf { table: "bitmaps", .. })));

    // A property whose name offset is past the string pool.
    let mut properties = le(&[1, 50], 4);
    properties.push(0);
    properties.extend(le(&[0], 4));
    properties.extend([0, 0, 0]);
    properties.extend(le(&[2], 4));
    properties.extend(b"a\0");
    let file = pcf_file(&[(1, 0, properties), (4, 0, one_glyph), (8, 0, bitmaps)]);
    assert!(matches!(read_pcf(&file), Err(BitmapError::Pcf { table: "properties", .. })));
}

#[test]
fn truncated_files_fail_with_a_message() {
    let font = sample();
    let pcf = write_pcf(&font, PcfLayout::default()).unwrap();
    for len in 0..pcf.len() {
        let _ = read_pcf(pcf.get(..len).unwrap());
    }
    let bdf = write_bdf(&font).unwrap();
    for (len, _) in bdf.char_indices() {
        let _ = read_bdf(bdf.get(..len).unwrap().as_bytes());
    }
    assert!(read_pcf(pcf.get(..pcf.len() / 2).unwrap()).is_err());
}

#[test]
fn read_tells_the_formats_apart() {
    let font = sample();
    let pcf = write_pcf(&font, PcfLayout::default()).unwrap();
    assert_eq!(read(&pcf).unwrap().font.glyphs, font.glyphs);
    assert_eq!(read(SAMPLE.as_bytes()).unwrap().font, font);
    assert!(matches!(read(b"hello"), Err(BitmapError::NotBdf(_))));
}

#[test]
fn codes_are_unicode_only_in_unicode_registries() {
    let mut font = sample();
    assert!(font.codes_are_unicode());
    let text = |v: &str| PropertyValue::Text(v.into());
    font.properties.push(Property { name: "CHARSET_REGISTRY".into(), value: text("ISO8859") });
    font.properties.push(Property { name: "CHARSET_ENCODING".into(), value: text("2") });
    assert!(!font.codes_are_unicode());
    font.properties.last_mut().unwrap().value = text("1");
    assert!(font.codes_are_unicode());
}

#[test]
fn writers_refuse_what_the_readers_could_not_take_back() {
    let mut font = sample();
    font.glyphs[0].name = "two words".into();
    assert!(matches!(write_bdf(&font), Err(BitmapError::Write(_))));
    let mut font = sample();
    font.glyphs[0].bitmap.data.pop();
    assert!(matches!(write_pcf(&font, PcfLayout::default()), Err(BitmapError::Write(_))));
    let mut font = sample();
    font.glyphs[0].bitmap.left = i32::MAX;
    assert!(matches!(write_bdf(&font), Err(BitmapError::Write(_))));
    let font = sample();
    let layout = PcfLayout { glyph_pad: 1, scan_unit: 4, ..PcfLayout::default() };
    assert!(matches!(write_pcf(&font, layout), Err(BitmapError::Write(_))));
}

#[test]
fn set_pixel_and_pixel_agree_at_every_depth() {
    for depth in [1, 2, 4, 8] {
        let mut b = Bitmap::blank(5, 2, 0, 0, depth).unwrap();
        let max = ((1u16 << depth) - 1) as u8;
        b.set_pixel(4, 1, 255);
        b.set_pixel(0, 0, 1);
        assert_eq!((b.pixel(4, 1), b.pixel(0, 0), b.pixel(3, 1), b.pixel(9, 9)), (max, 1, 0, 0), "depth {depth}");
    }
    assert!(Bitmap::blank(MAX_BITMAP_SIDE + 1, 1, 0, 0, 1).is_none());
    assert!(Bitmap::blank(1, 1, 0, 0, 3).is_none());
}
