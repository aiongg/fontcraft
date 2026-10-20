#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

//! Expected values come from fontTools 4.66 run on the same glyph: `dropImpliedOnCurvePoints`
//! given the default glyph and the glyph moved by the tuple, then `iup_delta_optimize` with a
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

fn font(instructions: &[u8], deltas: Option<&[(i16, i16)]>) -> SfntFont {
    let mut f = SfntFont::new(Flavour::TrueType);
    let mut head = vec![0u8; 54];
    head[..4].copy_from_slice(&[0, 1, 0, 0]);
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    let mut maxp = vec![0u8; 32];
    maxp[..4].copy_from_slice(&[0, 1, 0, 0]);
    maxp[4..6].copy_from_slice(&1u16.to_be_bytes());
    maxp[6..8].copy_from_slice(&12u16.to_be_bytes());
    let (glyf, loca, long) = build_glyf(&[glyph(instructions)]).unwrap();
    head[50..52].copy_from_slice(&u16::from(long).to_be_bytes());
    f.insert(*b"head", head);
    f.insert(*b"maxp", maxp);
    f.insert(*b"glyf", glyf);
    f.insert(*b"loca", loca);
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
