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

#[test]
fn transforms_are_refused_for_now() {
    let tt = font(true);
    assert!(matches!(woff2::encode(&tt, &Options::default()), Err(Error::Unsupported { .. })));
    let hmtx = Options { transform_hmtx: true, ..Options::untransformed() };
    assert!(matches!(woff2::encode(&tt, &hmtx), Err(Error::Unsupported { .. })));
    // A CFF font has nothing to transform: the default options work.
    let otf = font(false);
    assert_eq!(woff2::encode(&otf, &Options::default()).unwrap(), woff2::encode(&otf, &Options::untransformed()).unwrap());
    // A file with a transformed glyf and loca: the directory reads, decoding is refused.
    let mut w = woff2::encode(&tt, &Options::untransformed()).unwrap();
    let dir = woff2::directory(&w).unwrap();
    let mut rebuilt = Vec::new();
    for t in &dir.tables {
        let index = woff2::known_tag_index(t.tag);
        match &t.tag {
            b"glyf" => {
                rebuilt.push(index);
                woff2::push_base128(&mut rebuilt, t.orig_length);
                woff2::push_base128(&mut rebuilt, t.orig_length);
            }
            b"loca" => {
                rebuilt.push(index);
                woff2::push_base128(&mut rebuilt, t.orig_length);
                woff2::push_base128(&mut rebuilt, 0);
            }
            _ => {
                rebuilt.push(index | (t.transform << 6));
                if index == 63 {
                    rebuilt.extend_from_slice(&t.tag);
                }
                woff2::push_base128(&mut rebuilt, t.orig_length);
            }
        }
    }
    let old_len = dir.compressed.0 - 48;
    w.splice(48..48 + old_len, rebuilt.iter().copied());
    let len = w.len() as u32;
    w[8..12].copy_from_slice(&len.to_be_bytes());
    // The stream is now shorter than stated (loca counts zero): the directory still reads.
    let dir = woff2::directory(&w).unwrap();
    assert!(dir.tables.iter().any(|t| t.transformed()));
    assert!(matches!(woff2::decode(&w), Err(Error::Unsupported { .. })));
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
