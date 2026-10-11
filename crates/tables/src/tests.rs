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
