use crate::compress::{self, Mode};
use crate::sfnt::{self, Table, Tag};
use crate::woff2::{self, Options};
use crate::{Error, Extras, Kind, MAX_SFNT, sniff, to_sfnt, woff};

fn head_table() -> Vec<u8> {
    let mut h = vec![0u8; 54];
    h[0..4].copy_from_slice(&[0, 1, 0, 0]);
    h[4..8].copy_from_slice(&[0, 2, 0x80, 0]); // fontRevision 2.5
    h[8..12].copy_from_slice(&[0xde, 0xad, 0xbe, 0xef]); // a stale checkSumAdjustment
    h[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    h[16..18].copy_from_slice(&3u16.to_be_bytes());
    h[18..20].copy_from_slice(&1000u16.to_be_bytes());
    h
}

/// A font of synthetic tables: a compressible one, an incompressible one, an odd-sized one, an
/// unknown tag, `DSIG`, and for TrueType `glyf`, `loca` and `hmtx`.
fn tables(truetype: bool) -> Vec<(Tag, Vec<u8>)> {
    let mut noise = Vec::new();
    let mut x: u32 = 12345;
    for _ in 0..300 {
        x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
        noise.push((x >> 16) as u8);
    }
    let mut t: Vec<(Tag, Vec<u8>)> = vec![
        (*b"name", b"FontCraft test font, FontCraft test font, FontCraft test font".repeat(4)),
        (*b"cmap", noise),
        (*b"post", vec![0, 3, 0, 0, 1, 2, 3]),
        (*b"head", head_table()),
        (*b"zzzz", vec![7; 33]),
        (*b"DSIG", vec![0, 0, 0, 1, 0, 0, 0, 0]),
        (*b"OS/2", vec![0; 0]),
    ];
    if truetype {
        t.push((*b"glyf", vec![0, 1, 0, 0, 0, 0, 0, 0, 0, 10, 0, 10, 0, 0, 0, 0, 1, 0, 0]));
        t.push((*b"loca", vec![0, 0, 0, 10, 0, 10]));
        t.push((*b"hmtx", vec![1, 244, 0, 0, 1, 244, 0, 0]));
    } else {
        t.push((*b"CFF ", vec![1, 0, 4, 1, 0, 1, 1, 1, 65]));
    }
    t
}

fn font(truetype: bool) -> Vec<u8> {
    let owned = tables(truetype);
    let list: Vec<Table<'_>> = owned.iter().map(|(tag, data)| Table { tag: *tag, data }).collect();
    sfnt::assemble(if truetype { sfnt::TRUETYPE } else { sfnt::OTTO }, &list).unwrap()
}

fn table<'a>(sfnt_bytes: &'a [u8], tag: &[u8; 4]) -> Option<&'a [u8]> {
    sfnt::parse(sfnt_bytes).unwrap().get(*tag)
}

fn valid_checksums(sfnt_bytes: &[u8]) {
    assert_eq!(sfnt::checksum(sfnt_bytes), 0xB1B0_AFBA, "the whole font must sum to the magic number");
    let parsed = sfnt::parse(sfnt_bytes).unwrap();
    for i in 0..parsed.tables.len() {
        let at = 12 + 16 * i;
        let tag: Tag = sfnt_bytes[at..at + 4].try_into().unwrap();
        let stored = u32::from_be_bytes(sfnt_bytes[at + 4..at + 8].try_into().unwrap());
        assert_eq!(stored, sfnt::table_checksum(tag, parsed.get(tag).unwrap()));
    }
    let tags: Vec<Tag> = (0..parsed.tables.len()).map(|i| sfnt_bytes[12 + 16 * i..16 + 16 * i].try_into().unwrap()).collect();
    assert!(tags.windows(2).all(|w| w[0] < w[1]), "the directory is sorted by tag");
}

/// Tables of two fonts equal byte for byte, `head` compared without `checkSumAdjustment` and
/// with `flags` bit 11 masked when `woff2`.
fn same_tables(a: &[u8], b: &[u8], woff2: bool) {
    let (fa, fb) = (sfnt::parse(a).unwrap(), sfnt::parse(b).unwrap());
    let expected: Vec<&Table<'_>> = fa.tables.iter().filter(|t| !(woff2 && t.tag == *b"DSIG")).collect();
    assert_eq!(expected.len(), fb.tables.len());
    for t in expected {
        let other = fb.get(t.tag).unwrap();
        if t.tag == *b"head" {
            let mut x = t.data.to_vec();
            let mut y = other.to_vec();
            x[8..12].fill(0);
            y[8..12].fill(0);
            if woff2 {
                x[16] |= 0x08;
            }
            assert_eq!(x, y);
        } else {
            assert_eq!(t.data, other, "table {}", sfnt::tag_str(t.tag));
        }
    }
}

#[test]
fn checksums_and_assembly() {
    assert_eq!(sfnt::checksum(b"abcd"), 1_633_837_924);
    assert_eq!(sfnt::checksum(b"abcdxyz"), 3_655_064_932);
    for truetype in [true, false] {
        let f = font(truetype);
        valid_checksums(&f);
        assert!(read_fonts::FontRef::new(&f).is_ok());
    }
}

#[test]
fn recommended_order_follows_the_reference() {
    let owned = tables(true);
    let list: Vec<Table<'_>> = owned.iter().map(|(tag, data)| Table { tag: *tag, data }).collect();
    let order: Vec<String> = sfnt::recommended_order(&list).iter().map(|t| sfnt::tag_str(t.tag)).collect();
    assert_eq!(order, ["head", "OS/2", "hmtx", "cmap", "loca", "glyf", "name", "post", "zzzz", "DSIG"]);
    let owned = tables(false);
    let list: Vec<Table<'_>> = owned.iter().map(|(tag, data)| Table { tag: *tag, data }).collect();
    let order: Vec<String> = sfnt::recommended_order(&list).iter().map(|t| sfnt::tag_str(t.tag)).collect();
    assert_eq!(order, ["head", "OS/2", "name", "cmap", "post", "CFF ", "zzzz", "DSIG"]);
}

#[test]
fn woff_round_trip() {
    for truetype in [true, false] {
        let f = font(truetype);
        let w = woff::encode(&f, &Extras::default()).unwrap();
        assert_eq!(sniff(&w), Some(Kind::Woff));
        assert_eq!(u32::from_be_bytes(w[8..12].try_into().unwrap()) as usize, w.len());
        let back = woff::decode(&w).unwrap();
        assert_eq!(back.version, (2, 0x8000), "the version comes from head.fontRevision");
        assert_eq!((back.metadata, back.private), (None, None));
        valid_checksums(&back.sfnt);
        same_tables(&f, &back.sfnt, false);
        assert_eq!(to_sfnt(&w).unwrap().as_ref(), back.sfnt.as_slice());
        let dir = woff::directory(&w).unwrap();
        let by_tag = |tag: &[u8; 4]| dir.iter().find(|t| t.tag.to_be_bytes() == *tag).unwrap().clone();
        assert!(dir.windows(2).all(|p| p[0].tag < p[1].tag), "the directory is sorted by tag");
        let head = by_tag(b"head");
        assert_eq!(head.compressed, head.length, "head is stored raw");
        let name = by_tag(b"name");
        assert!(name.compressed < name.length, "a compressible table is compressed");
        let cmap = by_tag(b"cmap");
        assert_eq!(cmap.compressed, cmap.length, "noise is stored raw");
        assert_eq!(head.offset as usize, 44 + 20 * dir.len(), "head comes first in the data");
        // A second pass is stable: the unwrapped font wraps to the same bytes.
        assert_eq!(woff::encode(&back.sfnt, &Extras::default()).unwrap(), w);
    }
}

#[test]
fn woff_extras_round_trip() {
    let f = font(true);
    let extras = Extras {
        version: Some((3, 7)),
        metadata: Some(b"<?xml version=\"1.0\"?><metadata version=\"1.0\"/>".to_vec()),
        private: Some(vec![1, 2, 3, 4, 5]),
    };
    let w = woff::encode(&f, &extras).unwrap();
    let back = woff::decode(&w).unwrap();
    assert_eq!((back.version, back.metadata, back.private), (extras.version.unwrap(), extras.metadata.clone(), extras.private.clone()));
    let private_offset = u32::from_be_bytes(w[36..40].try_into().unwrap());
    assert_eq!(private_offset % 4, 0);
    let w2 = woff2::encode(&f, &Options { extras: extras.clone(), ..Options::untransformed() }).unwrap();
    let back = woff2::decode(&w2).unwrap();
    assert_eq!((back.version, back.metadata, back.private), (extras.version.unwrap(), extras.metadata, extras.private));
}

#[test]
fn woff2_round_trip_untransformed() {
    for truetype in [true, false] {
        let f = font(truetype);
        let w = woff2::encode(&f, &Options::untransformed()).unwrap();
        assert_eq!(sniff(&w), Some(Kind::Woff2));
        assert_eq!(w.len() % 4, 0);
        let back = woff2::decode(&w).unwrap();
        valid_checksums(&back.sfnt);
        same_tables(&f, &back.sfnt, true);
        assert_eq!(table(&back.sfnt, b"head").unwrap()[16] & 0x08, 0x08, "head.flags bit 11 is set");
        assert!(table(&back.sfnt, b"DSIG").is_none(), "DSIG is dropped");
        let dir = woff2::directory(&w).unwrap();
        assert!(dir.tables.iter().all(|t| !t.transformed()));
        assert!(dir.tables.windows(2).all(|p| p[0].tag < p[1].tag), "tables are sorted by tag");
        // The directory bytes: flags with the known-tag index, glyf and loca with transform 3.
        let first = w[48];
        assert_eq!(first & 0x3f, woff2::known_tag_index(dir.tables[0].tag));
        if truetype {
            let glyf = dir.tables.iter().find(|t| t.tag == *b"glyf").unwrap();
            assert_eq!(glyf.transform, 3);
        }
        assert_eq!(woff2::encode(&back.sfnt, &Options::untransformed()).unwrap(), w, "a second pass is stable");
    }
}

/// A TrueType font with real outlines: a box with instructions, an empty glyph, a glyph whose
/// moves use every triplet size (with off-curve points and the overlap flag), a glyph whose stored
/// bounding box is not the computed one, and two composites (offsets, scales, a 2×2 matrix, word
/// arguments, instructions). Glyphs are written as fontTools writes them and padded to two bytes,
/// as fontTools' WOFF2 reader lays them out. `lsb` gives the side bearing of glyph 2.
fn outline_font(lsb: i16) -> Vec<u8> {
    use crate::woff2::glyf::test_glyphs::{composite, simple};
    let square: &[(i32, i32, bool)] = &[(0, 0, true), (0, 700, true), (500, 700, true), (500, 0, true)];
    let moves: &[(i32, i32, bool)] = &[
        (10, 10, true),
        (10, 300, false),
        (1500, 300, true),
        (1530, 340, false),
        (2230, 1000, true),
        (5000, -2000, true),
        (-27000, 2000, false),
        (-27000, 2000, true),
        (-26990, 2000, true),
    ];
    let glyphs = [
        simple(&[square], &[0xb0, 0x01], false, None),
        Vec::new(),
        simple(&[moves, &[(1, 1, true), (2, 2, true), (2, 3, true)]], &[], true, None),
        simple(&[square], &[], false, Some([-5, 0, 600, 700])),
        composite(&[(0, 10, 20, None), (2, -300, 900, Some([8192, 0, 0, 8192]))], [0, 0, 900, 1600], Some(&[0x2c])),
        composite(&[(3, 1, 2, Some([16384, 4096, -4096, 16384])), (0, 0, 0, Some([16384, 0, 0, -16384]))], [-5, -700, 600, 700], None),
    ];
    let mut glyf = Vec::new();
    let mut loca = Vec::new();
    let mut x_mins = Vec::new();
    for g in &glyphs {
        loca.extend_from_slice(&u16::try_from(glyf.len() / 2).unwrap().to_be_bytes());
        glyf.extend_from_slice(g);
        if glyf.len() % 2 == 1 {
            glyf.push(0);
        }
        x_mins.push(if g.is_empty() { 0 } else { i16::from_be_bytes([g[2], g[3]]) });
    }
    loca.extend_from_slice(&u16::try_from(glyf.len() / 2).unwrap().to_be_bytes());
    // Four long metrics, then two side bearings; every side bearing is the glyph's xMin but glyph 2's.
    let mut hmtx = Vec::new();
    for (i, x) in x_mins.iter().enumerate() {
        let bearing = if i == 2 { lsb } else { *x };
        if i < 4 {
            hmtx.extend_from_slice(&(500 + 10 * i as u16).to_be_bytes());
        }
        hmtx.extend_from_slice(&bearing.to_be_bytes());
    }
    let mut hhea = vec![0u8; 36];
    hhea[34..36].copy_from_slice(&4u16.to_be_bytes());
    let maxp = [0, 0, 0x50, 0, 0, glyphs.len() as u8].to_vec();
    let owned: Vec<(Tag, Vec<u8>)> = vec![
        (*b"head", head_table()),
        (*b"hhea", hhea),
        (*b"maxp", maxp),
        (*b"hmtx", hmtx),
        (*b"loca", loca),
        (*b"glyf", glyf),
        (*b"name", b"FontCraft".to_vec()),
    ];
    let list: Vec<Table<'_>> = owned.iter().map(|(tag, data)| Table { tag: *tag, data }).collect();
    sfnt::assemble(sfnt::TRUETYPE, &list).unwrap()
}

fn transform_of(w: &[u8], tag: &[u8; 4]) -> u8 {
    woff2::directory(w).unwrap().tables.iter().find(|t| &t.tag == tag).unwrap().transform
}

#[test]
fn woff2_transforms_round_trip() {
    let with_hmtx = Options::all_transforms();
    for lsb in [0, -7] {
        let f = outline_font(lsb);
        for options in [Options::default(), with_hmtx.clone()] {
            let w = woff2::encode(&f, &options).unwrap();
            assert_eq!((transform_of(&w, b"glyf"), transform_of(&w, b"loca")), (0, 0), "glyf and loca are transformed");
            let dir = woff2::directory(&w).unwrap();
            assert_eq!(dir.tables.iter().find(|t| t.tag == *b"loca").unwrap().transform_length, Some(0));
            // Glyph 2's side bearing is not its xMin when lsb is -7: the proportional side
            // bearings are kept, the transform still applies.
            assert_eq!(transform_of(&w, b"hmtx"), u8::from(options.transform_hmtx));
            let back = woff2::decode(&w).unwrap();
            valid_checksums(&back.sfnt);
            same_tables(&f, &back.sfnt, true);
            assert!(read_fonts::FontRef::new(&back.sfnt).is_ok());
            assert_eq!(woff2::encode(&back.sfnt, &options).unwrap(), w, "a second pass is stable");
        }
    }
    // A CFF font has nothing to transform: the options change nothing.
    let otf = font(false);
    assert_eq!(woff2::encode(&otf, &with_hmtx).unwrap(), woff2::encode(&otf, &Options::untransformed()).unwrap());
}

#[test]
fn woff2_glyf_is_normalised_before_the_transform() {
    // The same outlines with long loca offsets and no padding: the writer lays glyf out with
    // 4-byte padding and short offsets, and head.indexToLocFormat follows.
    let f = outline_font(0);
    let glyf = table(&f, b"glyf").unwrap();
    let loca = table(&f, b"loca").unwrap();
    let mut long_loca = Vec::new();
    for w in loca.chunks(2) {
        long_loca.extend_from_slice(&(2 * u32::from(u16::from_be_bytes([w[0], w[1]]))).to_be_bytes());
    }
    let mut head = table(&f, b"head").unwrap().to_vec();
    head[50..52].copy_from_slice(&1u16.to_be_bytes());
    let mut owned: Vec<(Tag, Vec<u8>)> = sfnt::parse(&f).unwrap().tables.iter().map(|t| (t.tag, t.data.to_vec())).collect();
    for (tag, data) in &mut owned {
        match &*tag {
            b"loca" => *data = long_loca.clone(),
            b"head" => *data = head.clone(),
            _ => {}
        }
    }
    let list: Vec<Table<'_>> = owned.iter().map(|(tag, data)| Table { tag: *tag, data }).collect();
    let long = sfnt::assemble(sfnt::TRUETYPE, &list).unwrap();
    let back = woff2::decode(&woff2::encode(&long, &Options::default()).unwrap()).unwrap();
    assert_eq!(table(&back.sfnt, b"glyf").unwrap(), glyf);
    assert_eq!(table(&back.sfnt, b"loca").unwrap(), loca);
    assert_eq!(&table(&back.sfnt, b"head").unwrap()[50..52], &[0, 0]);
    let w = woff2::encode(&long, &Options::untransformed()).unwrap();
    assert_eq!(transform_of(&w, b"glyf"), 3);
    assert_eq!(table(&woff2::decode(&w).unwrap().sfnt, b"loca").unwrap(), long_loca, "untransformed tables are kept as they are");

    // A cubic point cannot be carried: glyf and loca stay untransformed, but normalised.
    let mut owned: Vec<(Tag, Vec<u8>)> = sfnt::parse(&f).unwrap().tables.iter().map(|t| (t.tag, t.data.to_vec())).collect();
    let glyf_slot = owned.iter_mut().find(|t| &t.0 == b"glyf").unwrap();
    glyf_slot.1[17] |= 0x80; // the box's second point: the flags follow the header, one end point and two instructions
    let list: Vec<Table<'_>> = owned.iter().map(|(tag, data)| Table { tag: *tag, data }).collect();
    let cubic = sfnt::assemble(sfnt::TRUETYPE, &list).unwrap();
    let w = woff2::encode(&cubic, &Options::all_transforms()).unwrap();
    assert_eq!((transform_of(&w, b"glyf"), transform_of(&w, b"loca"), transform_of(&w, b"hmtx")), (3, 3, 1));
    let back = woff2::decode(&w).unwrap();
    let glyf = table(&back.sfnt, b"glyf").unwrap();
    assert_eq!(glyf.len() % 4, 0);
    assert_eq!(table(&back.sfnt, b"hmtx"), table(&cubic, b"hmtx"), "hmtx is rebuilt from the untransformed glyf");
}

#[test]
fn damaged_transformed_tables_never_panic() {
    let f = outline_font(-7);
    let w = woff2::encode(&f, &Options::all_transforms()).unwrap();
    let dir = woff2::directory(&w).unwrap();
    let data = woff2::stream(&w, &dir).unwrap();
    assert!(woff2::tables(&dir, &data).is_ok());
    for i in 0..data.len() {
        for delta in [1u8, 0x80, 0xff] {
            let mut bad = data.clone();
            bad[i] = bad[i].wrapping_add(delta);
            let _ = woff2::tables(&dir, &bad);
        }
    }
    let part = |tag: &[u8; 4]| {
        let t = dir.tables.iter().find(|t| &t.tag == tag).unwrap();
        data[t.stream_offset..t.stream_offset + t.stream_length() as usize].to_vec()
    };
    let (glyf, hmtx) = (part(b"glyf"), part(b"hmtx"));
    for cut in 0..glyf.len() {
        assert!(woff2::glyf::reconstruct(&glyf[..cut]).is_err());
    }
    let x_mins = woff2::glyf::reconstruct(&glyf).unwrap().x_mins;
    for cut in 0..hmtx.len() {
        assert!(woff2::hmtx::reconstruct(&hmtx[..cut], 4, &x_mins).is_err());
    }
    // hhea naming more metrics than there are glyphs, or a rebuilt hmtx of another size than the
    // directory states, is damage.
    assert_eq!(woff2::hmtx::reconstruct(&[3, 1, 244, 1, 244], 2, &[0, 0]).unwrap().len(), 8);
    assert!(woff2::hmtx::reconstruct(&[3, 1, 244, 1, 244], 3, &[0, 0]).is_err());
    let mut longer = dir.clone();
    longer.tables.iter_mut().find(|t| &t.tag == b"hmtx").unwrap().orig_length += 2;
    assert!(woff2::tables(&longer, &data).is_err());
    // Damaged glyphs on the way in leave glyf untransformed rather than fail.
    let glyf_at = {
        let parsed = sfnt::parse(&f).unwrap();
        let g = parsed.get(*b"glyf").unwrap();
        g.as_ptr() as usize - f.as_ptr() as usize
    };
    for i in glyf_at..glyf_at + table(&f, b"glyf").unwrap().len() {
        for delta in [1u8, 0x80, 0xff] {
            let mut bad = f.clone();
            bad[i] = bad[i].wrapping_add(delta);
            let w = woff2::encode(&bad, &Options::all_transforms()).unwrap();
            let _ = woff2::decode(&w);
        }
    }
}

#[test]
fn collections_are_refused() {
    let mut ttc = b"ttcf\0\x01\0\0\0\0\0\x01\0\0\0\x10".to_vec();
    ttc.extend_from_slice(&font(true));
    assert!(matches!(woff::encode(&ttc, &Extras::default()), Err(Error::Unsupported { .. })));
    assert!(matches!(woff2::encode(&ttc, &Options::untransformed()), Err(Error::Unsupported { .. })));
    let mut w = woff2::encode(&font(true), &Options::untransformed()).unwrap();
    w[4..8].copy_from_slice(b"ttcf");
    assert!(matches!(woff2::decode(&w), Err(Error::Unsupported { .. })));
    assert!(matches!(woff2::directory(&w), Err(Error::Unsupported { .. })));
}

#[test]
fn variable_length_numbers() {
    let enc = |n| {
        let mut v = Vec::new();
        woff2::push_base128(&mut v, n);
        v
    };
    assert_eq!(enc(63), [0x3f]);
    assert_eq!(enc(0), [0]);
    assert_eq!(enc(u32::MAX), [0x8f, 0xff, 0xff, 0xff, 0x7f]);
    for n in [0, 1, 127, 128, 24567, 1 << 21, u32::MAX] {
        let mut at = 0;
        assert_eq!(woff2::read_base128(&enc(n), &mut at).unwrap(), n);
        assert_eq!(at, enc(n).len());
    }
    for bad in [&[0x80u8, 0x80, 0x3f][..], &[0x8f, 0xff, 0xff, 0xff, 0xff, 0x7f], &[0x90, 0x80, 0x80, 0x80, 0], &[0x81], &[]] {
        assert!(woff2::read_base128(bad, &mut 0).is_err(), "{bad:?}");
    }
    let enc16 = |n| {
        let mut v = Vec::new();
        woff2::push_255u16(&mut v, n);
        v
    };
    assert_eq!(enc16(252), [252]);
    assert_eq!(enc16(253), [255, 0]);
    assert_eq!(enc16(506), [254, 0]);
    assert_eq!(enc16(762), [253, 2, 250]);
    for n in [0, 252, 253, 505, 506, 761, 762, u16::MAX] {
        let mut at = 0;
        assert_eq!(woff2::read_255u16(&enc16(n), &mut at).unwrap(), n);
    }
    for alt in [&[254u8, 0][..], &[255, 253], &[253, 1, 250]] {
        assert_eq!(woff2::read_255u16(alt, &mut 0).unwrap(), 506);
    }
    assert!(woff2::read_255u16(&[253, 1], &mut 0).is_err());
}

#[test]
fn truncated_and_damaged_input_never_panics() {
    let f = font(true);
    let files = [f.clone(), woff::encode(&f, &Extras::default()).unwrap(), woff2::encode(&f, &Options::untransformed()).unwrap()];
    for good in &files {
        for cut in 0..good.len() {
            let part = &good[..cut];
            let _ = to_sfnt(part);
            let _ = woff::encode(part, &Extras::default());
            let _ = woff2::encode(part, &Options::untransformed());
            let _ = woff::directory(part);
        }
        for i in 0..good.len().min(400) {
            for delta in [1u8, 0x80, 0xff] {
                let mut bad = good.clone();
                bad[i] = bad[i].wrapping_add(delta);
                let _ = to_sfnt(&bad);
                let _ = woff::encode(&bad, &Extras::default());
                let _ = woff2::encode(&bad, &Options::untransformed());
            }
        }
    }
    for junk in [&b""[..], b"wOFF", b"wOF2", b"wOFF\0\0\0\0\0\0\0\0\xff\xff", b"wOF2\0\x01\0\0\0\0\0\x30\xff\xff", b"ttcf\0\0\0\0", b"OTTO"] {
        assert!(woff::decode(junk).is_err());
        assert!(woff2::decode(junk).is_err());
    }
}

/// A WOFF2 file with one table, `name`, claiming `orig` bytes, and `stream` as its compressed data.
fn one_table_woff2(orig: u32, compressed: &[u8], total_sfnt: u32) -> Vec<u8> {
    let mut w = vec![0u8; 48];
    w.push(woff2::known_tag_index(*b"name"));
    woff2::push_base128(&mut w, orig);
    w.extend_from_slice(compressed);
    w.resize(w.len().next_multiple_of(4), 0);
    w[0..4].copy_from_slice(b"wOF2");
    w[4..8].copy_from_slice(&[0, 1, 0, 0]);
    let len = w.len() as u32;
    w[8..12].copy_from_slice(&len.to_be_bytes());
    w[12..14].copy_from_slice(&1u16.to_be_bytes());
    w[16..20].copy_from_slice(&total_sfnt.to_be_bytes());
    w[20..24].copy_from_slice(&(compressed.len() as u32).to_be_bytes());
    w
}

#[test]
fn decompression_bombs_are_stopped() {
    // Brotli: 8 MiB of zeros compress to a few bytes; the directory says 100.
    let zeros = vec![0u8; 8 << 20];
    let bomb = compress::brotli(&zeros, Mode::Font).unwrap();
    assert!(bomb.len() < 1000);
    let w = one_table_woff2(100, &bomb, 128);
    assert!(matches!(woff2::decode(&w), Err(Error::Invalid { .. })));
    // The same stream claiming a size above the limit is refused before decompressing.
    let w = one_table_woff2(MAX_SFNT as u32 + 1, &bomb, 128);
    assert!(woff2::directory(&w).is_err());
    let w = one_table_woff2(100, &bomb, u32::MAX);
    assert!(woff2::directory(&w).is_err());
    // A well-formed one-table file decodes.
    let name = b"plain name table".to_vec();
    let ok = one_table_woff2(name.len() as u32, &compress::brotli(&name, Mode::Font).unwrap(), 64);
    assert_eq!(table(&woff2::decode(&ok).unwrap().sfnt, b"name").unwrap(), name.as_slice());

    // zlib in WOFF 1: a table that inflates past its stated length.
    let f = font(true);
    let mut w = woff::encode(&f, &Extras::default()).unwrap();
    let dir = woff::directory(&w).unwrap();
    let i = dir.iter().position(|t| t.compressed < t.length).unwrap();
    let packed = compress::zlib(&zeros);
    let offset = w.len();
    w.extend_from_slice(&packed);
    w.resize(w.len().next_multiple_of(4), 0);
    let at = 44 + 20 * i;
    w[at + 4..at + 8].copy_from_slice(&(offset as u32).to_be_bytes());
    w[at + 8..at + 12].copy_from_slice(&(packed.len() as u32).to_be_bytes());
    w[at + 12..at + 16].copy_from_slice(&(packed.len() as u32 + 1).to_be_bytes());
    let len = w.len() as u32;
    w[8..12].copy_from_slice(&len.to_be_bytes());
    w[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(woff::decode(&w).is_err(), "a header font size above the limit");
    w[16..20].copy_from_slice(&(MAX_SFNT as u32).to_be_bytes());
    assert!(matches!(woff::decode(&w), Err(Error::Invalid { .. })));
}

#[test]
fn misplaced_tables_are_refused() {
    let f = font(true);
    let good = woff::encode(&f, &Extras::default()).unwrap();
    assert!(woff::decode(&good).is_ok());
    let dir = woff::directory(&good).unwrap();
    let field = |w: &mut Vec<u8>, i: usize, k: usize, v: u32| {
        let at = 44 + 20 * i + 4 * k;
        w[at..at + 4].copy_from_slice(&v.to_be_bytes());
    };
    // Two tables at the same place.
    let mut w = good.clone();
    field(&mut w, 1, 1, dir[0].offset);
    field(&mut w, 1, 2, dir[0].compressed);
    field(&mut w, 1, 3, dir[0].length.max(dir[0].compressed));
    assert!(woff::decode(&w).is_err());
    // A table at an odd offset.
    let mut w = good.clone();
    field(&mut w, 2, 1, dir[2].offset + 1);
    assert!(woff::decode(&w).is_err());
    // A table inside the header.
    let mut w = good.clone();
    field(&mut w, 2, 1, 8);
    assert!(woff::decode(&w).is_err());
    // A tag listed twice.
    let mut w = good.clone();
    field(&mut w, 1, 0, dir[0].tag);
    assert!(woff::decode(&w).is_err());
    // A length that does not match the file.
    let mut w = good.clone();
    w.extend_from_slice(&[0; 4]);
    assert!(woff::decode(&w).is_err());
    let mut w2 = woff2::encode(&f, &Options::untransformed()).unwrap();
    w2.extend_from_slice(&[0; 4]);
    assert!(woff2::decode(&w2).is_err());
}
