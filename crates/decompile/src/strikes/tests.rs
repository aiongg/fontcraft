use fontcraft_compile::{Options, compile_ttf};
use fontcraft_model::strike::{BitmapStrike, StrikeGlyph};
use fontcraft_model::{Font, Path, Shape};

fn base() -> Vec<u8> {
    let mut f = Font::new("Embedded");
    f.glyph_mut("A").unwrap().layers[0].shapes.push(Shape::Path(Path::rect(60.0, 0.0, 560.0, 700.0)));
    compile_ttf(&f, &Options::default()).unwrap().bytes
}

/// Glyph ids of `A` and `B` in the compiled font.
fn ids(bytes: &[u8]) -> (u16, u16) {
    let font = crate::decompile(bytes, None).unwrap().font;
    let at = |name: &str| font.glyphs.iter().position(|g| g.name == name).unwrap() as u16;
    (at("A"), at("B"))
}

fn with_tables(base: &[u8], tables: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut b = write_fonts::FontBuilder::new();
    for (tag, data) in tables {
        b.add_raw(write_fonts::types::Tag::new(tag), data.clone());
    }
    b.copy_missing_tables(skrifa::raw::FontRef::new(base).unwrap());
    b.build()
}

/// One 8 ppem strike of depth `depth`: `A` a bit-aligned image (format 2, index format 1), `B`
/// a composite (format 8, index format 3) of two copies of the glyph `component`.
fn tables(a: u16, b: u16, component: u16, depth: u8) -> (Vec<u8>, Vec<u8>) {
    let mut ebdt = vec![0, 2, 0, 0];
    // A: 3 × 3, a diagonal cross, as nine bits running on.
    ebdt.extend([3, 3, 0, 3, 4, 0b1010_1010, 0b1000_0000]);
    // B: 5 × 4, the component at (0, 0) and at (2, 1).
    ebdt.extend([4, 5, 1, 4, 6, 0, 0, 2]);
    for (x, y) in [(0u8, 0u8), (2, 1)] {
        ebdt.extend(component.to_be_bytes());
        ebdt.extend([x, y]);
    }
    let mut eblc = vec![0, 2, 0, 0, 0, 0, 0, 1];
    eblc.extend(56u32.to_be_bytes());
    eblc.extend(44u32.to_be_bytes());
    eblc.extend(2u32.to_be_bytes());
    eblc.extend(0u32.to_be_bytes());
    eblc.extend([7, 0xFF, 6, 1, 0, 0, 0, 0, 7, 0xFF, 0, 0]);
    eblc.extend([0; 12]);
    eblc.extend(a.to_be_bytes());
    eblc.extend(b.to_be_bytes());
    eblc.extend([8, 8, depth, 1]);
    for (gid, at) in [(a, 16u32), (b, 32)] {
        eblc.extend(gid.to_be_bytes());
        eblc.extend(gid.to_be_bytes());
        eblc.extend(at.to_be_bytes());
    }
    eblc.extend([0, 1, 0, 2]);
    eblc.extend(4u32.to_be_bytes());
    eblc.extend(0u32.to_be_bytes());
    eblc.extend(7u32.to_be_bytes());
    eblc.extend([0, 3, 0, 8]);
    eblc.extend(11u32.to_be_bytes());
    eblc.extend([0, 0, 0, 16]);
    assert_eq!(eblc.len(), 100);
    (eblc, ebdt)
}

#[test]
fn bit_aligned_and_composite_images_open_as_model_rows() {
    let base = base();
    let (a, b) = ids(&base);
    let (eblc, ebdt) = tables(a, b, a, 1);
    let d = crate::decompile(&with_tables(&base, &[(b"EBLC", eblc.clone()), (b"EBDT", ebdt.clone())]), None).unwrap();
    let glyph = |name: &str, advance, width, height, left, data: &[u8]| StrikeGlyph {
        glyph: name.into(),
        code: None,
        advance,
        scalable_width: None,
        width,
        height,
        left,
        bottom: 0,
        data: data.to_vec(),
        references: Vec::new(),
    };
    let mut want = BitmapStrike::new(8, 1);
    want.ascent = 7;
    want.descent = 1;
    want.glyphs = vec![glyph("A", 4, 3, 3, 0, &[0xA0, 0x40, 0xA0]), glyph("B", 6, 5, 4, 1, &[0xA0, 0x68, 0xB0, 0x28])];
    assert_eq!(*d.font.bitmap_strikes, vec![want.clone()]);
    assert!(!d.warnings.iter().any(|w| w.contains("EBDT") || w.contains("bitmap")), "{:?}", d.warnings);

    // The Apple tables share the layout.
    let apple = crate::decompile(&with_tables(&base, &[(b"bloc", eblc), (b"bdat", ebdt)]), None).unwrap();
    assert_eq!(*apple.font.bitmap_strikes, vec![want]);
}

#[test]
fn hostile_strikes_are_left_out_and_the_font_still_opens() {
    let base = base();
    let (a, b) = ids(&base);

    // A composite made of itself: only `A` is kept.
    let (eblc, ebdt) = tables(a, b, b, 1);
    let d = crate::decompile(&with_tables(&base, &[(b"EBLC", eblc), (b"EBDT", ebdt)]), None).unwrap();
    let names: Vec<&str> = d.font.bitmap_strikes[0].glyphs.iter().map(|g| g.glyph.as_str()).collect();
    assert_eq!(names, ["A"]);
    assert!(d.warnings.iter().any(|w| w.contains("1 images could not be read")), "{:?}", d.warnings);

    // A depth the model has no place for.
    let (eblc, ebdt) = tables(a, b, a, 3);
    let d = crate::decompile(&with_tables(&base, &[(b"EBLC", eblc), (b"EBDT", ebdt)]), None).unwrap();
    assert!(d.font.bitmap_strikes.is_empty());
    assert!(d.warnings.iter().any(|w| w.contains("3 bits per pixel")), "{:?}", d.warnings);

    // A location table that claims a million strikes, and one cut short.
    let (mut eblc, ebdt) = tables(a, b, a, 1);
    eblc[4..8].copy_from_slice(&1_000_000u32.to_be_bytes());
    let d = crate::decompile(&with_tables(&base, &[(b"EBLC", eblc.clone()), (b"EBDT", ebdt.clone())]), None).unwrap();
    assert!(d.font.bitmap_strikes.len() <= 1);
    let d = crate::decompile(&with_tables(&base, &[(b"EBLC", eblc[..60].to_vec()), (b"EBDT", ebdt)]), None).unwrap();
    assert!(d.font.bitmap_strikes.is_empty());
    assert!(d.font.glyph("A").is_some());

    // A data table without its location table.
    let d = crate::decompile(&with_tables(&base, &[(b"EBDT", vec![0, 2, 0, 0])]), None).unwrap();
    assert!(d.warnings.iter().any(|w| w.contains("no `EBLC`")), "{:?}", d.warnings);
}

#[test]
fn strikes_sharing_one_large_image_stop_at_the_decode_budget() {
    use skrifa::raw::FontData;

    // One 255 × 255 bit-aligned image (format 2), which every other glyph of an index subtable
    // (format 1) points at, and 200 strikes that all use that subtable. Each strike decodes past
    // the model's size for one strike and is left out; before the font-wide budget, every one of
    // them decoded the image thousands of times.
    let glyphs: u16 = 16_600;
    let image_len = 5 + (255usize * 255).div_ceil(8);
    let mut ebdt = vec![0, 2, 0, 0, 255, 255, 0, 127, 255];
    ebdt.resize(4 + image_len, 0xAA);

    let sizes = 200u32;
    let array = 8 + 48 * sizes;
    let mut eblc = vec![0, 2, 0, 0];
    eblc.extend(sizes.to_be_bytes());
    let subtable_len = 8 + 4 * (u32::from(glyphs) + 1);
    for ppem in 1..=sizes as u8 {
        eblc.extend(array.to_be_bytes());
        eblc.extend((8 + subtable_len).to_be_bytes());
        eblc.extend(1u32.to_be_bytes());
        eblc.extend(0u32.to_be_bytes());
        eblc.extend([127, 0x81, 255, 0, 0, 0, 0, 0, 127, 0x81, 0, 0]);
        eblc.extend([0; 12]);
        eblc.extend(0u16.to_be_bytes());
        eblc.extend((glyphs - 1).to_be_bytes());
        eblc.extend([ppem, ppem, 1, 1]);
    }
    eblc.extend(0u16.to_be_bytes());
    eblc.extend((glyphs - 1).to_be_bytes());
    eblc.extend(8u32.to_be_bytes());
    eblc.extend([0, 1, 0, 2]);
    eblc.extend(4u32.to_be_bytes());
    for i in 0..=u32::from(glyphs) {
        eblc.extend(if i % 2 == 0 { 0u32 } else { image_len as u32 }.to_be_bytes());
    }

    let names: Vec<String> = (0..glyphs).map(|i| format!("g{i}")).collect();
    let mut warnings = Vec::new();
    let read = super::read(FontData::new(&eblc), FontData::new(&ebdt), &names, &mut warnings);
    let Err(message) = read else { panic!("the strikes were read: {warnings:?}") };
    assert!(message.contains("decode to more than"), "{message}");
}
