use crate::sfnt::{Flavour, SfntFont, TableOrder, WriteOptions, read, write};
use fontcraft_container::sfnt as csfnt;

/// A `head` table long enough to hold `checkSumAdjustment`, with the magic number set.
fn head() -> Vec<u8> {
    let mut h = vec![0u8; 54];
    h[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    h
}

#[test]
fn round_trip_keeps_unknown_tables_and_fixes_checksums() {
    let mut font = SfntFont::new(Flavour::TrueType);
    font.insert(*b"zzzz", vec![1, 2, 3]);
    font.insert(*b"name", vec![0; 6]);
    font.insert(*b"head", head());
    let bytes = write(&font, &WriteOptions::default()).unwrap();

    let back = read(&bytes, 0).unwrap();
    assert_eq!(back.get(*b"zzzz"), Some([1u8, 2, 3].as_slice()), "an unknown table passes through");
    assert_eq!(back.order, vec![*b"head", *b"name", *b"zzzz"], "recommended order: head first, unknown tags last");
    let parsed = csfnt::parse(&bytes).unwrap();
    let whole = csfnt::checksum(&bytes);
    assert_eq!(whole, 0xB1B0_AFBA, "head.checkSumAdjustment makes the file sum to the magic number");
    for t in &parsed.tables {
        assert!(t.data.len() <= bytes.len());
    }

    let kept = write(&back, &WriteOptions { order: TableOrder::Keep }).unwrap();
    assert_eq!(kept, bytes, "writing what was read gives the same bytes");
}

#[test]
fn hostile_directories_are_refused() {
    let mut bytes = vec![0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    bytes.extend_from_slice(b"head");
    bytes.extend_from_slice(&[0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0x00, 0, 0, 0, 16]);
    assert!(read(&bytes, 0).is_err(), "a table past the end of the file");
    assert!(read(&bytes[..6], 0).is_err(), "a cut directory");
    assert!(read(&bytes, 1).is_err(), "a face that does not exist");
}

/// A table directory of `count` tables that all point at the same `size` bytes at `data_at`.
pub(crate) fn overlapping_directory(count: usize, data_at: usize, size: usize) -> Vec<u8> {
    let mut d = Vec::with_capacity(12 + 16 * count);
    d.extend_from_slice(&[0, 1, 0, 0]);
    d.extend_from_slice(&(count as u16).to_be_bytes());
    d.extend_from_slice(&[0; 6]);
    for i in 0..count {
        d.extend_from_slice(&[b'a' + (i / 26 % 26) as u8, b'a' + (i % 26) as u8, b'0' + (i / 676) as u8, b' ']);
        d.extend_from_slice(&[0; 4]);
        d.extend_from_slice(&(data_at as u32).to_be_bytes());
        d.extend_from_slice(&(size as u32).to_be_bytes());
    }
    d
}

#[test]
fn overlapping_tables_past_the_size_limit_are_refused_before_copying() {
    // 300 tables over the same MiB would copy 300 MiB.
    let (count, size) = (300, 1 << 20);
    let data_at = 12 + 16 * count;
    let mut bytes = overlapping_directory(count, data_at, size);
    bytes.resize(data_at + size, 7);
    let started = std::time::Instant::now();
    assert!(matches!(read(&bytes, 0), Err(crate::TablesError::TooLarge { .. })));
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    // Within the limit the same layout reads.
    let mut small = overlapping_directory(3, 12 + 16 * 3, 64);
    small.resize(12 + 16 * 3 + 64, 7);
    assert_eq!(read(&small, 0).unwrap().tables.len(), 3);
}

/// A `CFF2` INDEX with four-byte offsets.
fn cff2_index(items: &[Vec<u8>]) -> Vec<u8> {
    let mut out = (items.len() as u32).to_be_bytes().to_vec();
    out.push(4);
    let mut at = 1u32;
    out.extend_from_slice(&at.to_be_bytes());
    for item in items {
        at += item.len() as u32;
        out.extend_from_slice(&at.to_be_bytes());
    }
    for item in items {
        out.extend_from_slice(item);
    }
    out
}

/// A static `CFF2` table of `glyphs` glyphs, each calling global subroutine `depth − 1`.
/// Subroutine k > 0 calls subroutine k − 1 `fanout` times and subroutine 0 is `leaf`, so every
/// glyph runs `leaf` fanout^(depth − 1) times. `depth` is at most 10.
pub(crate) fn nested_subr_cff2(glyphs: usize, depth: usize, fanout: usize, leaf: &[u8]) -> Vec<u8> {
    // Subroutine numbers are biased by 107 below 1240 subroutines; k − 107 is the byte k + 32.
    let call = |k: usize| [k as u8 + 32, 29];
    let mut subrs = vec![leaf.to_vec()];
    for k in 1..depth {
        subrs.push((0..fanout).flat_map(|_| call(k - 1)).collect());
    }
    let gsubrs = cff2_index(&subrs);
    let charstrings = cff2_index(&vec![call(depth - 1).to_vec(); glyphs]);
    let int = |v: usize| {
        let mut b = vec![29];
        b.extend_from_slice(&(v as i32).to_be_bytes());
        b
    };
    // StdHW 50.
    let private = vec![189, 10];
    let top_len = 13;
    let at_cs = 5 + top_len + gsubrs.len();
    let fd_len = 11;
    let at_fd = at_cs + charstrings.len();
    let at_private = at_fd + cff2_index(&[vec![0; fd_len]]).len();
    let mut fd = int(private.len());
    fd.extend(int(at_private));
    fd.push(18);
    let mut top = int(at_cs);
    top.push(17);
    top.extend(int(at_fd));
    top.extend([12, 36]);
    assert_eq!((top.len(), fd.len()), (top_len, fd_len));
    let mut out = vec![2, 0, 5];
    out.extend_from_slice(&(top_len as u16).to_be_bytes());
    for part in [top, gsubrs, charstrings, cff2_index(&[fd]), private] {
        out.extend(part);
    }
    out
}
