//! The subsetter options that act on the subset font as a whole, on fonts built in code. The
//! expected values follow fontTools 4.66.1: its `Options`, `Subsetter._prune_post_subset`, the
//! `name` and `DSIG` pruning, and what `TTFont.save` recomputes.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use fontcraft_compile::{Flavour, Options, compile};
use fontcraft_model::{Font, Path, Shape};

use super::super::{DEFAULT_DROP_TABLES, SubsetOptions, SubsetResult, subset};
use crate::reorder::be::Data;
use crate::sfnt::{SfntFont, read};

/// `.notdef`, `A` (a rectangle from 10 to 210 by 0 to 700, advance 500) and `W` (from -30 to
/// 900 by -200 to 800), so the font box is W's.
fn font(flavour: Flavour) -> SfntFont {
    let mut f = Font::new("Finish Sans");
    f.glyphs.retain(|g| g.name == ".notdef");
    for (name, cp, rect) in [("A", 0x41, [10.0, 0.0, 210.0, 700.0]), ("W", 0x57, [-30.0, -200.0, 900.0, 800.0])] {
        f.add_glyph(name, vec![cp], 500.0).unwrap();
        f.glyph_mut(name).unwrap().layers[0].shapes.push(Shape::Path(Path::rect(rect[0], rect[1], rect[2], rect[3])));
    }
    let out = compile(&f, &f.default_target().unwrap(), flavour, &Options::default()).unwrap();
    let mut font = read(&out.bytes, 0).unwrap();
    for tag in [*b"GDEF", *b"GSUB", *b"GPOS", *b"kern"] {
        font.remove(tag);
    }
    font
}

fn run(font: &SfntFont, edit: impl FnOnce(&mut SubsetOptions)) -> SubsetResult {
    let mut options = SubsetOptions { text: "A".into(), ..SubsetOptions::default() };
    edit(&mut options);
    subset(font, &options).unwrap()
}

fn i16s(table: &[u8], at: usize, n: usize) -> Vec<i16> {
    (0..n).map(|i| Data::new(*b"test", table).i16(at + 2 * i).unwrap()).collect()
}

#[test]
fn recalc_bounds_takes_the_boxes_from_the_kept_glyphs() {
    for flavour in [Flavour::TrueType, Flavour::Cff] {
        let input = font(flavour);
        let kept = run(&input, |_| {});
        assert_eq!(i16s(kept.font.get(*b"head").unwrap(), 36, 4), [-30, -200, 900, 800], "{flavour:?}: the box stays by default");
        let r = run(&input, |o| o.recalc_bounds = true);
        assert_eq!(i16s(r.font.get(*b"head").unwrap(), 36, 4), [10, 0, 210, 700], "{flavour:?}: the box of A");
        let hhea = r.font.get(*b"hhea").unwrap();
        assert_eq!(i16s(hhea, 12, 3), [10, 290, 210], "{flavour:?}: smallest side bearings and largest extent");
        if flavour == Flavour::Cff {
            let before = crate::ttx::read_cff::font_bbox(kept.font.get(*b"CFF ").unwrap()).unwrap();
            assert_eq!(before, Some([-30.0, -200.0, 900.0, 800.0]));
            let after = crate::ttx::read_cff::font_bbox(r.font.get(*b"CFF ").unwrap()).unwrap();
            assert_eq!(after, Some([10.0, 0.0, 210.0, 700.0]), "the FontBBox follows");
        }
    }
}

#[test]
fn recalc_average_width_max_context_and_timestamp_rewrite_their_fields() {
    let mut input = font(Flavour::TrueType);
    let mut os2 = input.get(*b"OS/2").unwrap().to_vec();
    os2[94..96].copy_from_slice(&7u16.to_be_bytes());
    os2[2..4].copy_from_slice(&1234u16.to_be_bytes());
    input.insert(*b"OS/2", os2);
    let mut head = input.get(*b"head").unwrap().to_vec();
    head[28..36].copy_from_slice(&5i64.to_be_bytes());
    input.insert(*b"head", head);

    let kept = run(&input, |_| {});
    let os2 = kept.font.get(*b"OS/2").unwrap();
    assert_eq!((i16s(os2, 2, 1)[0], i16s(os2, 94, 1)[0]), (1234, 7), "kept by default");
    assert_eq!(kept.font.get(*b"head").unwrap()[28..36], 5i64.to_be_bytes());

    let r = run(&input, |o| {
        o.recalc_average_width = true;
        o.recalc_max_context = true;
        o.recalc_timestamp = Some(0);
    });
    let count = super::glyph_count(&r.font).unwrap();
    let widths: Vec<i64> = super::metrics(&r.font, false, count).unwrap().unwrap().into_iter().map(|m| m.0).filter(|a| *a > 0).collect();
    let expected = (widths.iter().sum::<i64>() as f64 / widths.len() as f64 + 0.5).floor() as i16;
    let os2 = r.font.get(*b"OS/2").unwrap();
    assert_eq!(i16s(os2, 2, 1)[0], expected, "the mean of the kept advances");
    assert_eq!(i16s(os2, 94, 1)[0], 0, "no layout tables: no context");
    let head = r.font.get(*b"head").unwrap();
    assert_eq!(head[28..36], 2_082_844_800i64.to_be_bytes(), "1970-01-01 in the head epoch");
}

#[test]
fn table_lists_replace_and_edit_the_defaults() {
    let mut input = font(Flavour::TrueType);
    input.insert(*b"DSIG", vec![0, 0, 0, 1, 0, 1, 0, 1, 9, 9, 9, 9]);
    input.insert(*b"PCLT", vec![1, 2, 3, 4]);
    input.insert(*b"gasp", vec![0, 1, 0, 1, 0xFF, 0xFF, 0, 15]);

    let r = run(&input, |_| {});
    for tag in [*b"DSIG", *b"PCLT"] {
        assert!(r.font.get(tag).is_none() && r.dropped.iter().any(|d| d.tag == tag && d.reason == "dropped by the options"));
    }
    assert!(r.font.get(*b"gasp").is_some());

    // `--drop-tables=`: DSIG is then copied without its signatures; PCLT is unknown and dropped.
    let r = run(&input, |o| o.drop_tables.clear());
    assert_eq!(r.font.get(*b"DSIG").unwrap(), [0, 0, 0, 1, 0, 0, 0, 1]);
    assert!(r.dropped.iter().any(|d| d.tag == *b"PCLT" && d.reason == "the subsetter does not know this table"));

    // `--drop-tables-=PCLT --no-subset-tables+=PCLT --no-subset-tables-=gasp`
    let r = run(&input, |o| {
        o.drop_tables.retain(|t| t != b"PCLT");
        o.no_subset_tables.push(*b"PCLT");
        o.no_subset_tables.retain(|t| t != b"gasp");
    });
    assert_eq!(r.font.get(*b"PCLT").unwrap(), [1, 2, 3, 4], "listed: copied as it is");
    assert!(r.font.get(*b"gasp").is_none(), "left out of the list: dropped");
    assert!(r.font.get(*b"DSIG").is_none());

    // A table that indexes glyphs is subset even when listed.
    let r = run(&input, |o| o.no_subset_tables.push(*b"hmtx"));
    assert!(r.font.get(*b"hmtx").unwrap().len() < input.get(*b"hmtx").unwrap().len());
    assert!(r.warnings.iter().any(|w| w.contains("'hmtx'")), "{:?}", r.warnings);

    // `--drop-tables+=gasp`
    let r = run(&input, |o| o.drop_tables.push(*b"gasp"));
    assert!(r.font.get(*b"gasp").is_none());
    assert_eq!(SubsetOptions::default().drop_tables, DEFAULT_DROP_TABLES);
}

/// A `name` table of Windows Unicode English records.
fn name(records: &[(u16, &str)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut strings = Vec::new();
    for v in [0u16, records.len() as u16, 6 + 12 * records.len() as u16] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    for (id, text) in records {
        let utf16: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
        for v in [3u16, 1, 0x409, *id, utf16.len() as u16, strings.len() as u16] {
            out.extend_from_slice(&v.to_be_bytes());
        }
        strings.extend(utf16);
    }
    out.extend(strings);
    out
}

/// The records of a `name` table as (id, bytes).
fn records(table: &[u8]) -> Vec<(u16, Vec<u8>)> {
    let d = Data::new(*b"name", table);
    let storage = usize::from(d.u16(4).unwrap());
    (0..usize::from(d.u16(2).unwrap()))
        .map(|i| {
            let at = 6 + 12 * i;
            let text = d.slice(storage + usize::from(d.u16(at + 10).unwrap()), usize::from(d.u16(at + 8).unwrap())).unwrap();
            (d.u16(at + 6).unwrap(), text.to_vec())
        })
        .collect()
}

#[test]
fn obfuscate_names_scrambles_the_install_names_and_keeps_copies_for_stat() {
    let mut input = font(Flavour::TrueType);
    input.insert(*b"name", name(&[(1, "Fam"), (2, "Bold"), (3, "Uniq"), (4, "Fam Bold"), (5, "Version 1"), (6, "Fam-Bold"), (16, "Typo")]));
    // STAT 1.1: one design axis named by ID 5, no axis values, elided fallback name ID 2.
    let mut stat = Vec::new();
    for v in [1u16, 1, 8, 1] {
        stat.extend_from_slice(&v.to_be_bytes());
    }
    stat.extend_from_slice(&20u32.to_be_bytes());
    stat.extend_from_slice(&0u16.to_be_bytes());
    stat.extend_from_slice(&0u32.to_be_bytes());
    stat.extend_from_slice(&2u16.to_be_bytes());
    stat.extend_from_slice(b"wght");
    stat.extend_from_slice(&5u16.to_be_bytes());
    stat.extend_from_slice(&0u16.to_be_bytes());
    input.insert(*b"STAT", stat);

    let r = run(&input, |o| {
        o.obfuscate_names = true;
        o.name_ids = Some(vec![1, 2, 3, 4, 5, 6, 16]);
    });
    let utf16 = |s: &str| -> Vec<u8> { s.encode_utf16().flat_map(u16::to_be_bytes).collect() };
    assert_eq!(
        records(r.font.get(*b"name").unwrap()),
        [
            (1, utf16(".\u{7f}")),
            (2, utf16("\u{7f}")),
            (3, Vec::new()),
            (4, utf16(".\u{7f}")),
            (5, utf16("Version 1")),
            (6, utf16("\u{7f}")),
            (256, utf16("Bold")),
        ],
        "ID 16 dropped; the subfamily name STAT uses is kept under the first free ID"
    );
    let stat = r.font.get(*b"STAT").unwrap();
    assert_eq!(Data::new(*b"STAT", stat).u16(18).unwrap(), 256, "STAT points at the copy");
    assert_eq!(Data::new(*b"STAT", stat).u16(24).unwrap(), 5, "names that stay are not moved");
}
