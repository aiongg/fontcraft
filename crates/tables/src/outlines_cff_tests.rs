//! Outline repairs on CFF fonts built in code.

use super::*;
use crate::convert::cff_table::{test_charstring as cs, test_table};
use crate::sfnt::Flavour;

const CFF: [u8; 4] = *b"CFF ";
const HINTS: [u16; 6] = [1, 3, 18, 23, 19, 20];

fn repair(font: &SfntFont, options: &OutlineOptions) -> OutlineReport {
    crate::outlines::repair_report(font, options).unwrap()
}

fn with(action: OutlineAction, glyphs: &[&str]) -> OutlineOptions {
    OutlineOptions { glyphs: glyphs.iter().map(|g| g.to_string()).collect(), ..OutlineOptions::new(action) }
}

/// A CFF font of these glyphs, every advance 600, with a `TEST` table riding along.
fn font_of(names: &[&str], charstrings: Vec<Vec<u8>>) -> SfntFont {
    let n = (names.len() as u16).to_be_bytes();
    let mut font = SfntFont::new(Flavour::Cff);
    let mut head = vec![0u8; 54];
    head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    let mut hhea = vec![0u8; 36];
    hhea[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    hhea[34..36].copy_from_slice(&n);
    let mut maxp = vec![0u8; 6];
    maxp[0..4].copy_from_slice(&0x0000_5000u32.to_be_bytes());
    maxp[4..6].copy_from_slice(&n);
    let hmtx: Vec<u8> = names.iter().flat_map(|_| [2u8, 88, 0, 0]).collect();
    let mut post = vec![0u8; 32];
    post[0..4].copy_from_slice(&0x0003_0000u32.to_be_bytes());
    font.insert(*b"head", head);
    font.insert(*b"hhea", hhea);
    font.insert(*b"maxp", maxp);
    font.insert(*b"hmtx", hmtx);
    font.insert(*b"post", post);
    font.insert(*b"TEST", b"rides along".to_vec());
    font.insert(CFF, test_table(names, charstrings, &[]));
    font
}

/// `.notdef`; `A` two overlapping counter-clockwise squares and `B` one, both with a stem hint;
/// `C` a clockwise square; `D` an arch from (200, 0) to (0, 0) without a point at its top extreme
/// (100, 75); `E` a square from (10.25, 20.5).
fn font() -> SfntFont {
    let square = [200.0, 0.0, 0.0, 200.0, -200.0, 0.0];
    font_of(
        &[".notdef", "A", "B", "C", "D", "E"],
        vec![
            cs(&[(&[600.0], 14)]),
            cs(&[(&[600.0, 0.0, 200.0], 1), (&[0.0, 0.0], 21), (&square, 5), (&[100.0, -100.0], 21), (&square, 5), (&[], 14)]),
            cs(&[(&[600.0, 0.0, 200.0], 1), (&[0.0, 0.0], 21), (&square, 5), (&[], 14)]),
            cs(&[(&[600.0, 0.0, 0.0], 21), (&[0.0, 200.0, 200.0, 0.0, 0.0, -200.0], 5), (&[], 14)]),
            cs(&[(&[600.0, 200.0, 0.0], 21), (&[0.0, 100.0, -200.0, 0.0, 0.0, -100.0], 8), (&[], 14)]),
            cs(&[(&[600.0, 10.25, 20.5], 21), (&[100.0, 0.0, 0.0, 100.0, -100.0, 0.0], 5), (&[], 14)]),
        ],
    )
}

fn table(font: &SfntFont) -> (&[u8], bool) {
    match font.get(CFF) {
        Some(t) => (t, false),
        None => (font.get(*b"CFF2").unwrap(), true),
    }
}

fn drawn(font: &SfntFont, gid: usize) -> Vec<Contour> {
    let (t, cff2) = table(font);
    Programs::read(t, cff2).unwrap().draw(gid).unwrap().contours
}

fn charstrings(font: &SfntFont) -> Vec<Vec<u8>> {
    let (t, cff2) = table(font);
    fontcraft_cff::subr::charstrings_of_table(t, cff2).unwrap().charstrings
}

fn has_subroutines(font: &SfntFont) -> bool {
    let (t, cff2) = table(font);
    let set = fontcraft_cff::subr::charstrings_of_table(t, cff2).unwrap();
    !set.global_subrs.is_empty() || set.local_subrs.iter().any(|l| !l.is_empty())
}

/// The operators of a charstring in order (no hint mask is expected in these).
fn operators(code: &[u8]) -> Vec<u16> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&b) = code.get(i) {
        i += match b {
            28 => 3,
            32..=246 => 1,
            247..=254 => 2,
            255 => 5,
            12 => {
                out.push(1200 + u16::from(code.get(i + 1).copied().unwrap_or(0)));
                2
            }
            _ => {
                out.push(u16::from(b));
                1
            }
        };
    }
    out
}

/// The on-curve points of a contour, sorted, in whole units.
fn corners(c: &Contour) -> Vec<(i64, i64)> {
    let mut out: Vec<(i64, i64)> = c.segs.iter().map(seg_end).map(|p| (p.0 as i64, p.1 as i64)).collect();
    out.sort_unstable();
    out
}

#[test]
fn removing_overlaps_on_cff_merges_the_glyph_and_keeps_everything_else() {
    let input = font();
    let report = repair(&input, &with(OutlineAction::RemoveOverlaps, &["A", "B"]));
    assert_eq!(report.result.changed, vec!["A"]);
    let out = &report.result.font;
    for tag in input.tags().into_iter().filter(|t| *t != CFF) {
        assert_eq!(input.get(tag), out.get(tag), "{} keeps its bytes", crate::sfnt::tag_str(tag));
    }
    let (before, after) = (charstrings(&input), charstrings(out));
    for gid in [0, 2, 3, 4, 5] {
        assert_eq!(before[gid], after[gid], "glyph {gid} is unchanged and keeps its bytes");
    }
    assert!(operators(&after[2]).contains(&1), "B keeps its stem hint");
    assert!(operators(&before[1]).contains(&1));
    assert!(!operators(&after[1]).iter().any(|op| HINTS.contains(op)), "the merged glyph loses its hints");

    let a = drawn(out, 1);
    assert_eq!(a.len(), 1, "the squares are merged");
    let mut union = vec![(0, 0), (200, 0), (200, 100), (300, 100), (300, 300), (100, 300), (100, 200), (0, 200)];
    union.sort_unstable();
    assert_eq!(corners(&a[0]), union);
    assert!(signed_area(&a) > 0.0, "the drawn counter-clockwise winding stays");
    let (t, _) = table(out);
    assert_eq!(Programs::read(t, false).unwrap().width(1), Some(600 << 16), "the width operand stays");

    // Nothing to merge: the font comes back as it was.
    let report = repair(&input, &with(OutlineAction::RemoveOverlaps, &["B"]));
    assert!(report.result.changed.is_empty());
    assert_eq!(report.result.font, input);
}

#[test]
fn direction_extremes_and_rounding_on_cff() {
    let input = font();
    let r = repair(&input, &with(OutlineAction::CorrectDirection, &["B", "C"]));
    assert_eq!(r.result.changed, vec!["C"], "the PostScript convention by default");
    assert!(signed_area(&drawn(&r.result.font, 3)) > 0.0);
    let truetype = OutlineOptions { winding: Winding::TrueType, ..with(OutlineAction::CorrectDirection, &["C"]) };
    assert!(repair(&input, &truetype).result.changed.is_empty(), "C already runs as TrueType wants");
    let reverse = OutlineOptions { winding: Winding::Reverse, ..with(OutlineAction::CorrectDirection, &["B"]) };
    let r = repair(&input, &reverse);
    assert_eq!(r.result.changed, vec!["B"]);
    assert!(signed_area(&drawn(&r.result.font, 2)) < 0.0);

    let r = repair(&input, &with(OutlineAction::AddExtremes, &["B", "D"]));
    assert_eq!(r.result.changed, vec!["D"]);
    assert_eq!(r.points_added.get("D"), Some(&1));
    let d = drawn(&r.result.font, 4);
    assert!(d[0].segs.iter().any(|s| seg_end(s) == (100.0, 75.0)), "a point at the top extreme: {d:?}");

    let r = repair(&input, &OutlineOptions::new(OutlineAction::Round));
    assert_eq!(r.result.changed, vec!["E"]);
    assert_eq!(r.values_changed, 8);
    assert_eq!(drawn(&r.result.font, 5)[0].start, (10.0, 21.0), "halves go up");
    let near = OutlineOptions { tolerance: Some(0.3), ..OutlineOptions::new(OutlineAction::Round) };
    let r = repair(&input, &near);
    assert_eq!(r.values_changed, 4, "only the x values are within 0.3 of the grid");
    assert_eq!(drawn(&r.result.font, 5)[0].start, (10.0, 20.5));

    let decompose = crate::outlines::repair_report(&input, &OutlineOptions::new(OutlineAction::Decompose));
    assert!(matches!(decompose, Err(TablesError::Unsupported(_))));
}

#[test]
fn a_subroutinised_cff_stays_subroutinised() {
    // F to I share one staircase outline, which the subroutiniser moves into a subroutine.
    let mut stairs = Vec::new();
    for _ in 0..6 {
        stairs.extend([30.0, 0.0, 0.0, 30.0]);
    }
    stairs.extend([-180.0, 0.0]);
    let square = [200.0, 0.0, 0.0, 200.0, -200.0, 0.0];
    let mut glyphs = vec![cs(&[(&[600.0], 14)]), cs(&[(&[600.0, 0.0, 0.0], 21), (&square, 5), (&[100.0, -100.0], 21), (&square, 5), (&[], 14)])];
    for x in [0.0, 10.0, 20.0, 30.0] {
        glyphs.push(cs(&[(&[600.0, x, 0.0], 21), (stairs.as_slice(), 5), (&[], 14)]));
    }
    let mut input = font_of(&[".notdef", "A", "F", "G", "H", "I"], glyphs);
    let options = fontcraft_cff::subr::SubrOptions::default();
    let table = fontcraft_cff::subr::subroutinize_table(input.get(CFF).unwrap(), false, &options).unwrap();
    input.insert(CFF, table);
    assert!(has_subroutines(&input), "the input is subroutinised");

    for keep in [false, true] {
        let options = OutlineOptions { keep_unused_subroutines: keep, ..OutlineOptions::new(OutlineAction::RemoveOverlaps) };
        let r = repair(&input, &options);
        assert_eq!(r.result.changed, vec!["A"]);
        assert!(has_subroutines(&r.result.font), "subroutines stay (keep {keep})");
        for gid in 2..6 {
            assert_eq!(drawn(&r.result.font, gid), drawn(&input, gid), "glyph {gid} draws as before");
        }
    }
}

#[test]
fn removing_overlaps_on_static_cff2() {
    let mut to2 = crate::convert::ConvertOptions::new(crate::convert::ConvertTarget::Cff2);
    to2.subroutinize = Some(false);
    let input = crate::convert::convert_with(&font(), &to2).unwrap().font.unwrap();
    let r = repair(&input, &with(OutlineAction::RemoveOverlaps, &["A", "B"]));
    assert_eq!(r.result.changed, vec!["A"]);
    assert_eq!(drawn(&r.result.font, 1).len(), 1);
    assert_eq!(charstrings(&r.result.font)[2], charstrings(&input)[2], "B keeps its charstring");
    for tag in input.tags().into_iter().filter(|t| *t != *b"CFF2") {
        assert_eq!(input.get(tag), r.result.font.get(tag));
    }
}
