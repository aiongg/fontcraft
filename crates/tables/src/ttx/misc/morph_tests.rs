// Tests may slice strings: a failing assertion is the point of a test.
#![allow(clippy::string_slice)]

//! `mort` and `morx` against `ttx` (M12.7d), on tables built byte by byte in layouts fontTools
//! does not write: a format 4 lookup, spare bytes after a state array, reserved coverage bits,
//! `mort` substitution windows that overlap, and a `mort` ligature component table where only
//! some components hold the ligature table's offset. `testdata/morph_crafted.ttx` is what
//! fontTools 4.66.1's `ttx` writes for the same bytes. The tables fontTools compiles itself are
//! covered by the reader's tests (`read_misc/morph_tests.rs`), which dump them back.

use super::fonts::Be;
use super::morph;
use crate::TablesError;
use crate::sfnt::{Flavour, SfntFont};
use crate::ttx::var::tests::assert_truncations_survive;
use crate::ttx::{DumpOptions, ImportOptions, XmlWriter, dump, import};

const EXPECTED: &str = include_str!("testdata/morph_crafted.ttx");

fn subtable_header(b: &mut Be, length: usize, coverage: u8, reserved: u16, morph_type: u8, flags: u32) {
    b.u32s(&[length as u32]);
    b.u8s(&[coverage]);
    b.u16s(&[reserved]);
    b.u8s(&[morph_type]);
    b.u32s(&[flags]);
}

/// A noncontextual subtable with a format 4 lookup, and a rearrangement subtable whose classes
/// are a format 2 lookup and whose state array is followed by two spare bytes.
fn crafted_morx() -> Vec<u8> {
    let mut lookup = Be::default();
    lookup.u16s(&[4, 6, 2, 12, 1, 0, 3, 1, 24, 0xFFFF, 0xFFFF, 0, 2, 3, 4]);
    let mut a = Be::default();
    subtable_header(&mut a, 12 + lookup.0.len(), 0x10, 0, 4, 1);
    a.bytes(&lookup.0);

    let mut stx = Be::default();
    stx.u32s(&[4, 16, 40, 58]);
    stx.u16s(&[2, 6, 1, 6, 0, 0, 2, 1, 3, 0xFFFF, 0xFFFF, 0]);
    stx.u16s(&[0, 0, 0, 1, 0, 0, 0, 2, 0]);
    stx.u16s(&[0, 0, 1, 0x8000, 0, 0x2103]);
    let mut b = Be::default();
    subtable_header(&mut b, 12 + stx.0.len(), 0x63, 5, 0, 2);
    b.bytes(&stx.0);

    let mut t = Be::default();
    t.u16s(&[2, 0]);
    t.u32s(&[1, 1, (16 + a.0.len() + b.0.len()) as u32, 0, 2]);
    t.bytes(&a.0);
    t.bytes(&b.0);
    t.0
}

/// A contextual subtable whose two substitution windows overlap and run to the end of the
/// subtable, and a ligature subtable with one rebased component and fewer ligatures than its
/// actions can reach.
fn crafted_mort() -> Vec<u8> {
    let mut d = Be::default();
    d.u16s(&[6, 10, 18, 30, 54, 1, 4]);
    d.u8s(&[4, 1, 4, 5]);
    d.u8s(&[0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 2]);
    d.u16s(&[18, 0, 0, 0, 24, 0x8000, 0, 0, 18, 0, 26, 25]);
    d.u16s(&[2, 0, 4, 1]);
    let mut e = Be::default();
    e.u16s(&[6, 14, 20, 38, 56, 68, 76, 8, 2]);
    e.u8s(&[4, 5]);
    e.u8s(&[0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 3]);
    e.u16s(&[20, 0, 26, 0x8000, 32, 0x8000, 20, 0x8000 | 56, 0]);
    e.u32s(&[25, 27, 28 | 0xC000_0000]);
    e.u16s(&[0, 2, 76, 8, 6, 7]);

    let mut chain = Be::default();
    chain.u16s(&[1, 0]);
    chain.u32s(&[1, 0xFFFF_FFFE]);
    for (body, coverage, morph_type, flags) in [(&d.0, 0x80u8, 1u8, 2u32), (&e.0, 0, 2, 4)] {
        chain.u16s(&[(8 + body.len()) as u16]);
        chain.u8s(&[coverage, morph_type]);
        chain.u32s(&[flags]);
        chain.bytes(body);
    }
    let mut t = Be::default();
    t.u32s(&[0x0001_0000, 1, 1, (12 + chain.0.len()) as u32]);
    t.u16s(&[1, 2]);
    t.bytes(&chain.0);
    t.0
}

/// The font of the expected text (its glyph order, `maxp` and `post`), with the crafted tables.
fn crafted_font() -> SfntFont {
    let options = ImportOptions { tables: vec![*b"maxp", *b"post"], ..ImportOptions::default() };
    let mut font = import(SfntFont::new(Flavour::TrueType), EXPECTED, &options).unwrap();
    font.insert(*b"morx", crafted_morx());
    font.insert(*b"mort", crafted_mort());
    font
}

/// The lines of `text` from `  <tag>` to `  </tag>`.
fn block(text: &str, tag: &str) -> String {
    let start = text.find(&format!("  <{tag}>")).unwrap_or_else(|| panic!("no <{tag}> in\n{text}"));
    let end = text[start..].find(&format!("  </{tag}>")).unwrap() + start;
    text[start..end].to_string()
}

#[test]
fn hand_built_layouts_dump_as_ttx_dumps_them() {
    let font = crafted_font();
    let ours = dump(&font, &DumpOptions { tables: vec![*b"morx", *b"mort"], ..DumpOptions::default() }).unwrap();
    for tag in ["morx", "mort"] {
        let (a, b) = (block(&ours, tag), block(EXPECTED, tag));
        for (i, (x, y)) in a.lines().zip(b.lines()).enumerate() {
            assert_eq!(x, y, "'{tag}' line {} differs", i + 1);
        }
        assert_eq!(a.lines().count(), b.lines().count(), "'{tag}' line counts differ");
    }
}

#[test]
fn every_truncation_is_an_error_and_written_as_hex() {
    let font = crafted_font();
    assert_truncations_survive(&font, &[*b"morx", *b"mort"]);
    for (data, legacy) in [(crafted_morx(), false), (crafted_mort(), true)] {
        for len in 0..data.len() {
            let mut w = XmlWriter::new();
            assert!(morph::write(&data[..len], &font, &mut w, legacy).is_err(), "{len} bytes of {}", if legacy { "mort" } else { "morx" });
        }
    }
}

#[test]
fn a_state_table_with_too_many_glyph_classes_is_refused() {
    // One state of 65,537 glyph classes: the class lookup reads, the state array is not walked.
    let classes = 65_537u32;
    let mut stx = Be::default();
    stx.u32s(&[classes, 16, 24, 24 + classes * 2]);
    stx.u16s(&[8, 1, 1, 4]);
    let mut t = Be::default();
    t.u16s(&[2, 0]);
    t.u32s(&[1, 1, (16 + 12 + stx.0.len()) as u32, 0, 1]);
    subtable_header(&mut t, 12 + stx.0.len(), 0, 0, 0, 1);
    t.bytes(&stx.0);
    let font = crafted_font();
    let err = morph::write(&t.0, &font, &mut XmlWriter::new(), false).unwrap_err();
    assert!(matches!(err, TablesError::TooLarge { .. }), "{err}");
}

#[test]
fn a_ligature_action_chain_without_an_end_is_an_error() {
    // The crafted mort ligature table with its last action's end flag cleared: the actions run
    // into the component table and on to the end of the subtable.
    let mut data = crafted_mort();
    let at = data.windows(4).position(|w| w == [0xC0, 0, 0, 28]).unwrap();
    data[at] = 0x40;
    let font = crafted_font();
    assert!(morph::write(&data, &font, &mut XmlWriter::new(), true).is_err());
}

#[test]
fn a_mort_ligature_entry_named_by_every_cell_is_charged_per_cell() {
    // 16,000 state-array cells all name one entry with 12,000 ligature actions: 64 KB of data
    // that would expand to gigabytes of transitions if the cells' copies were not counted.
    // Every 16-bit word of the body is even, because the component table starts at offset 0
    // and the ligature table at the end, so the whole body is read as components.
    let (classes, states, actions) = (16u16, 1000u16, 12_000usize);
    let entry_off = 20 + classes * states;
    let actions_at = entry_off + 4;
    let len = usize::from(actions_at) + 4 * actions;
    let mut e = Be::default();
    e.u16s(&[classes, 14, 20, entry_off, 0, 0, len as u16, 0, 2]);
    e.u8s(&[4, 4]);
    e.bytes(&vec![0; usize::from(classes * states)]);
    e.u16s(&[20, actions_at]);
    e.bytes(&vec![0; 4 * (actions - 1)]);
    e.u32s(&[0x8000_0000]);
    assert_eq!(e.0.len(), len);
    let mut chain = Be::default();
    chain.u16s(&[(8 + len) as u16]);
    chain.u8s(&[0, 2]);
    chain.u32s(&[1]);
    chain.bytes(&e.0);
    let mut t = Be::default();
    t.u32s(&[0x0001_0000, 1, 1, (12 + chain.0.len()) as u32]);
    t.u16s(&[0, 1]);
    t.bytes(&chain.0);
    let font = crafted_font();
    let err = morph::write(&t.0, &font, &mut XmlWriter::new(), true).unwrap_err();
    assert!(matches!(err, TablesError::TooLarge { .. }), "{err}");
}
