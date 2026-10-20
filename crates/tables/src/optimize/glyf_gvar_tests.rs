#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

//! Expected values come from fontTools 4.66 run on the same glyph: `dropImpliedOnCurvePoints`
//! given the default glyph and the glyph at each tuple peak, then `iup_delta_optimize` with a
//! tolerance of 0.5.

use read_fonts::tables::gvar::Gvar as ReadGvar;
use read_fonts::types::{F2Dot14, GlyphId};
use read_fonts::{FontData, FontRead};
use write_fonts::tables::gvar::{GlyphDelta, GlyphDeltas, GlyphVariations, Gvar as WriteGvar};
use write_fonts::tables::variations::Tent;

use super::*;
use crate::Flavour;

/// Contour 0 alternates off- and on-curve points; its on-curve points 1 and 4 lie halfway.
/// Contour 1 is six on-curve points.
const POINTS: [(i32, i32); 12] =
    [(0, 0), (50, 0), (100, 0), (100, 100), (50, 100), (0, 100), (200, 0), (250, 0), (300, 0), (300, 100), (250, 100), (200, 100)];
const FLAGS: [u8; 12] = [0, 1, 0, 0, 1, 0, 1, 1, 1, 1, 1, 1];
/// One tuple's deltas: points 1 and 4 move halfway too, contour 1 stretches, the advance grows.
const DELTAS: [(i16, i16); 16] =
    [(0, 0), (10, 0), (20, 0), (20, 10), (10, 10), (0, 10), (0, 0), (15, 0), (30, 0), (30, 0), (15, 0), (0, 0), (0, 0), (30, 0), (0, 0), (0, 0)];

fn glyph(instructions: &[u8]) -> Vec<u8> {
    let s = Simple {
        bbox: [0, 0, 0, 0, 1, 44, 0, 100],
        ends: vec![5, 11],
        instructions: instructions.to_vec(),
        flags: FLAGS.to_vec(),
        points: POINTS.to_vec(),
    };
    encode_simple(&s).unwrap()
}

/// A TrueType font of `glyphs` with `head`, `maxp`, `glyf` and `loca`.
fn sfnt(glyphs: &[Vec<u8>]) -> SfntFont {
    let mut f = SfntFont::new(Flavour::TrueType);
    let mut head = vec![0u8; 54];
    head[..4].copy_from_slice(&[0, 1, 0, 0]);
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    let mut maxp = vec![0u8; 32];
    maxp[..4].copy_from_slice(&[0, 1, 0, 0]);
    maxp[4..6].copy_from_slice(&(glyphs.len() as u16).to_be_bytes());
    maxp[6..8].copy_from_slice(&12u16.to_be_bytes());
    let (glyf, loca, long) = build_glyf(glyphs).unwrap();
    head[50..52].copy_from_slice(&u16::from(long).to_be_bytes());
    f.insert(*b"head", head);
    f.insert(*b"maxp", maxp);
    f.insert(*b"glyf", glyf);
    f.insert(*b"loca", loca);
    f
}

fn font(instructions: &[u8], deltas: Option<&[(i16, i16)]>) -> SfntFont {
    let mut f = sfnt(&[glyph(instructions)]);
    if let Some(d) = deltas {
        let tuple = GlyphDeltas::new(vec![Tent::new(F2Dot14::from_f32(1.0), None)], d.iter().map(|&(x, y)| GlyphDelta::required(x, y)).collect());
        let gvar = WriteGvar::new(vec![GlyphVariations::new(GlyphId::new(0), vec![tuple])], 1).unwrap();
        f.insert(*b"gvar", write_fonts::dump_table(&gvar).unwrap());
    }
    f
}

fn table<'a>(r: &'a GlyfGvarResult, tag: &[u8; 4]) -> Option<&'a [u8]> {
    r.tables.iter().find(|(t, _)| t == tag).map(|(_, b)| b.as_slice())
}

/// Each tuple's listed points and their deltas.
fn tuples(gvar: &[u8]) -> Vec<Vec<(u16, i32, i32)>> {
    let g = ReadGvar::read(FontData::new(gvar)).unwrap();
    let data = g.glyph_variation_data(GlyphId::new(0)).unwrap();
    data.iter().flat_map(|d| d.tuples()).map(|t| t.deltas().map(|d| (d.position, d.x_delta, d.y_delta)).collect()).collect()
}

#[test]
fn implied_on_curve_points_are_dropped_as_the_reference_drops_them() {
    let r = run(&font(&[], None), true, DeltaMode::Keep).unwrap();
    assert_eq!(r.dropped_points, 2);
    let mut out = font(&[], None);
    for (t, b) in &r.tables {
        out.insert(*t, b.clone());
    }
    let Shape::Simple(s) = parse_glyph(glyph_slices(&out, 1).unwrap()[0]).unwrap() else { panic!("a simple glyph") };
    assert_eq!(s.ends, [3, 9]);
    assert_eq!(s.points, [(0, 0), (100, 0), (100, 100), (0, 100), (200, 0), (250, 0), (300, 0), (300, 100), (250, 100), (200, 100)]);
    assert_eq!(&table(&r, b"maxp").unwrap()[6..8], &10u16.to_be_bytes(), "maxPoints follows");

    let hinted = run(&font(&[0xB0, 1], None), true, DeltaMode::Keep).unwrap();
    assert_eq!((hinted.dropped_points, hinted.hinted_kept), (0, 1), "instructions number the points");
    assert!(hinted.tables.is_empty());
}

#[test]
fn gvar_deltas_are_optimised_as_the_reference_optimises_them() {
    // Points dropped first, then the tuple optimised over the 14 points left.
    let r = run(&font(&[], Some(&DELTAS)), true, DeltaMode::Optimize).unwrap();
    assert_eq!(r.dropped_points, 2);
    let listed: Vec<u16> = tuples(table(&r, b"gvar").unwrap())[0].iter().map(|d| d.0).collect();
    assert_eq!(listed, [1, 3, 4, 7, 11]);
    assert_eq!(tuples(table(&r, b"gvar").unwrap())[0][0], (1, 20, 0));

    // Nothing dropped: the 16 points optimised.
    let r = run(&font(&[], Some(&DELTAS)), false, DeltaMode::Optimize).unwrap();
    assert!(table(&r, b"glyf").is_none());
    let listed: Vec<u16> = tuples(table(&r, b"gvar").unwrap())[0].iter().map(|d| d.0).collect();
    assert_eq!(listed, [2, 5, 6, 9, 13]);

    // Every delta written; a sparse input expands back to the same deltas.
    let sparse = table(&r, b"gvar").unwrap().to_vec();
    let mut f = font(&[], None);
    f.insert(*b"gvar", sparse);
    let all = run(&f, false, DeltaMode::All).unwrap();
    let got: Vec<(i16, i16)> = tuples(table(&all, b"gvar").unwrap())[0].iter().map(|d| (d.1 as i16, d.2 as i16)).collect();
    assert_eq!(got, DELTAS);
}

#[test]
fn a_point_that_one_location_needs_stays() {
    let mut deltas = DELTAS;
    deltas[1] = (11, 0);
    let r = run(&font(&[], Some(&deltas)), true, DeltaMode::Keep).unwrap();
    assert_eq!(r.dropped_points, 1, "only point 4 is halfway at both locations");
    // The glyph lost a point, so its tuple lists every point left.
    assert_eq!(tuples(table(&r, b"gvar").unwrap())[0].len(), 15);
}

#[test]
fn composite_points_are_counted_through_their_components() {
    let shapes = vec![(12, vec![]), (0, vec![0, 0]), (0, vec![1, 0]), (0, vec![3])];
    let maxp = recalc_maxp(
        &{
            let mut m = vec![0u8; 32];
            m[1] = 1;
            m
        },
        &shapes,
    );
    assert_eq!(&maxp[6..8], &12u16.to_be_bytes());
    assert_eq!(&maxp[10..12], &36u16.to_be_bytes(), "a cycle counts nothing");
}

/// A composite of glyph 0 twice; the second placed by point numbers unless `xy`.
fn composite(xy: bool) -> Vec<u8> {
    let mut c = vec![0xFF, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0];
    c.extend([0x00, 0x22, 0, 0, 0, 0]); // ARGS_ARE_XY_VALUES, MORE_COMPONENTS, glyph 0 at (0, 0)
    c.extend([0x00, if xy { 0x02 } else { 0x00 }, 0, 0, 4, 1]);
    c
}

/// GPOS with one mark-to-base lookup: mark glyph 1, base glyph 0 with an anchor of `format`.
fn mark_to_base(format: u16) -> Vec<u8> {
    let words: [u16; 36] = [
        1, 0, 0, 0, 10, // header, lookup list at 10
        1, 4, // one lookup, at 14
        4, 0, 1, 8, // mark-to-base, one subtable at 22
        1, 12, 18, 1, 24, 36, // coverages, one class, mark and base arrays
        1, 1, 1, // mark coverage: glyph 1
        1, 1, 0, // base coverage: glyph 0
        1, 0, 6, 1, 0, 0, // mark array, its anchor (format 1)
        1, 4, format, 0, 0, 1, 0, // base array, its anchor (point 1 when format 2)
    ];
    words.iter().flat_map(|w| w.to_be_bytes()).collect()
}

#[test]
fn points_numbered_elsewhere_are_kept() {
    let outcome = |f: &SfntFont| {
        let r = run(f, true, DeltaMode::Keep).unwrap();
        (r.dropped_points, r.hinted_kept)
    };
    assert_eq!(outcome(&sfnt(&[glyph(&[]), composite(true)])), (2, 0), "offsets number nothing");
    assert_eq!(outcome(&sfnt(&[glyph(&[]), composite(false)])), (0, 1), "point matching numbers the component's points");
    // Through a second level: glyph 2 places glyph 1 twice, the second by points; glyph 1
    // holds glyph 0.
    let mut nested = composite(false);
    nested[13] = 0x01;
    nested[19] = 0x01;
    assert_eq!(outcome(&sfnt(&[glyph(&[]), composite(true), nested])), (0, 1));
    let mut instructed = composite(true);
    instructed[17] = 0x02 | 0x01; // the last component: offsets as words, then instructions
    instructed[16] = 0x01;
    instructed.truncate(instructed.len() - 2);
    instructed.extend([0, 0, 0, 0, 0, 2, 0xB0, 0]);
    assert_eq!(outcome(&sfnt(&[glyph(&[]), instructed])), (0, 1), "a composite's instructions number its points");

    let mut f = sfnt(&[glyph(&[]), glyph(&[])]);
    f.insert(*b"GPOS", mark_to_base(1));
    assert_eq!(outcome(&f), (4, 0));
    f.insert(*b"GPOS", mark_to_base(2));
    assert_eq!(outcome(&f), (2, 1), "an anchor on a point of glyph 0");
    f.insert(*b"GPOS", vec![0, 1, 0, 0, 0, 0, 0, 0, 0, 10, 0]);
    assert_eq!(outcome(&f), (0, 2), "a GPOS that cannot be walked keeps every point");
}

/// One contour A B C D E: C lies halfway between the off-curve points B and D.
fn skewed_glyph() -> Vec<u8> {
    let s = Simple {
        bbox: [0, 0, 0, 0, 0, 90, 0, 100],
        ends: vec![4],
        instructions: Vec::new(),
        flags: vec![1, 0, 1, 0, 1],
        points: vec![(0, 0), (10, 100), (50, 100), (90, 100), (80, 0)],
    };
    encode_simple(&s).unwrap()
}

#[test]
fn implied_points_are_tested_at_every_peak_of_the_font() {
    // Each tuple lists A and E only; IUP gives B, C and D 0.25, 1.25 and 2 (clamped). Alone,
    // a tuple leaves C halfway after rounding (10 + 92 = 2 × 51). At (1, 1) all three apply:
    // B 10.75, C 53.75 and D 96 round to 11 + 96 ≠ 2 × 54.
    // The unlisted points get large values only so that write-fonts picks the sparse encoding.
    let tuple = |a: f32, b: f32| {
        let deltas = (0..9).map(|i| match i {
            0 => GlyphDelta::new(0, 0, true),
            4 => GlyphDelta::new(2, 0, true),
            _ => GlyphDelta::new(1000 + 100 * i, -1000, false),
        });
        GlyphDeltas::new(vec![Tent::new(F2Dot14::from_f32(a), None), Tent::new(F2Dot14::from_f32(b), None)], deltas.collect())
    };
    let with = |tuples: Vec<GlyphDeltas>| {
        let mut f = sfnt(&[skewed_glyph()]);
        let gvar = WriteGvar::new(vec![GlyphVariations::new(GlyphId::new(0), tuples)], 2).unwrap();
        f.insert(*b"gvar", write_fonts::dump_table(&gvar).unwrap());
        run(&f, true, DeltaMode::Keep).unwrap().dropped_points
    };
    assert_eq!(with(vec![tuple(1.0, 0.0), tuple(0.0, 1.0)]), 1);
    assert_eq!(with(vec![tuple(1.0, 0.0), tuple(0.0, 1.0), tuple(1.0, 1.0)]), 0);
    assert_eq!(support_scalar(&[0.5, 1.0], &[(0.0, 1.0, 1.0), (0.0, 0.0, 0.0)]), 0.5);
}
