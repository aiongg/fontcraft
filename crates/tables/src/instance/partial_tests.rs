//! Partial instances of a two-axis font built in code. The values of the pinning test come from
//! fontTools 4.66.1 `varLib.instancer.instantiateVariableFont` run on the same font built with
//! `fontBuilder` (same glyphs, deltas and axes). The other tests check the instancer's own
//! invariant: inside the new ranges, the partial instance pinned at a location gives the
//! original font pinned there, to within one unit of rounding.

#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic, clippy::expect_used)]

use read_fonts::{FontData, FontRead};
use write_fonts::tables::avar::{Avar, AxisValueMap, SegmentMaps};
use write_fonts::tables::fvar::{AxisInstanceArrays, Fvar, InstanceRecord, VariationAxisRecord};
use write_fonts::tables::gvar::{GlyphDelta, GlyphDeltas, GlyphVariations, Gvar};
use write_fonts::tables::hvar::Hvar;
use write_fonts::tables::mvar::{Mvar, ValueRecord};
use write_fonts::tables::variations::ivs_builder::VariationStoreBuilder;
use write_fonts::tables::variations::{RegionAxisCoordinates, Tent as WTent, VariationRegion};
use write_fonts::types::{F2Dot14, Fixed, GlyphId, MajorMinor, NameId, Tag as WTag};

use super::super::glyf::{self, Component, Composite, Glyph, Simple};
use super::super::{AxisLimit, InstanceOptions, full, instantiate, static_tables};
use super::*;
use crate::sfnt::Flavour;

const WGHT: Tag = *b"wght";
const WDTH: Tag = *b"wdth";

fn be16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_be_bytes());
}

fn head(long: bool) -> Vec<u8> {
    let mut h = vec![0u8; 54];
    h[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    h[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    h[18..20].copy_from_slice(&1000u16.to_be_bytes());
    h[50..52].copy_from_slice(&u16::from(long).to_be_bytes());
    h
}

fn maxp(glyphs: u16) -> Vec<u8> {
    let mut m = vec![0u8; 32];
    m[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    m[4..6].copy_from_slice(&glyphs.to_be_bytes());
    m
}

fn hhea(long: u16) -> Vec<u8> {
    let mut h = vec![0u8; 36];
    h[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    h[4..6].copy_from_slice(&800i16.to_be_bytes());
    h[6..8].copy_from_slice(&(-200i16).to_be_bytes());
    h[34..36].copy_from_slice(&long.to_be_bytes());
    h
}

/// `OS/2` version 4 with weight 400, width 5 and an x-height of 500.
fn os2() -> Vec<u8> {
    let mut o = vec![0u8; 96];
    o[0..2].copy_from_slice(&4u16.to_be_bytes());
    o[4..6].copy_from_slice(&400u16.to_be_bytes());
    o[6..8].copy_from_slice(&5u16.to_be_bytes());
    o[86..88].copy_from_slice(&500i16.to_be_bytes());
    o
}

fn tents(wght: (f32, f32, f32), wdth: (f32, f32, f32)) -> Vec<WTent> {
    let t = |(s, p, e): (f32, f32, f32)| WTent::new(F2Dot14::from_f32(p), Some((F2Dot14::from_f32(s), F2Dot14::from_f32(e))));
    vec![t(wght), t(wdth)]
}

const UP: (f32, f32, f32) = (0.0, 1.0, 1.0);
const DOWN: (f32, f32, f32) = (-1.0, -1.0, 0.0);
const OFF: (f32, f32, f32) = (0.0, 0.0, 0.0);

fn region(wght: (f32, f32, f32), wdth: (f32, f32, f32)) -> VariationRegion {
    let c = |(s, p, e): (f32, f32, f32)| RegionAxisCoordinates::new(F2Dot14::from_f32(s), F2Dot14::from_f32(p), F2Dot14::from_f32(e));
    VariationRegion::new(vec![c(wght), c(wdth)])
}

/// `.notdef` (empty), `A` (a square 100,0–500,700, advance 600) and `B` (A offset by 50,
/// advance 700) on wght 100–400–900 and wdth 75–100–125. A has deltas at each axis end, at the
/// corner wght+1 wdth+1, and an intermediate wght region (0, 0.5, 1); its wght+1 tuple leaves
/// two points to inference. B's component moves at wght+1. `MVAR` varies the x-height, `HVAR`
/// the advances; `avar`, when asked, maps wght 0.5 to 0.25. One named instance at wght 650,
/// wdth 100.
fn variable_font(with_avar: bool) -> SfntFont {
    let square = Glyph::Simple(Simple {
        bbox: [100, 0, 500, 700],
        end_pts: vec![3],
        instructions: Vec::new(),
        flags: vec![1; 4],
        points: vec![(100.0, 0.0), (100.0, 700.0), (500.0, 700.0), (500.0, 0.0)],
    });
    let composite = Glyph::Composite(Composite {
        bbox: [150, 0, 550, 700],
        components: vec![Component { flags: 0x0006, gid: 1, dx: 50.0, dy: 0.0, points: (0, 0), transform_raw: Vec::new(), transform: None }],
        instructions: None,
    });
    let (glyf_bytes, loca, long) = glyf::write_glyf(&[Glyph::Empty, square, composite]).unwrap();
    let mut hmtx = Vec::new();
    for (adv, lsb) in [(500u16, 0i16), (600, 100), (700, 150)] {
        be16(&mut hmtx, adv);
        be16(&mut hmtx, lsb as u16);
    }
    let r = GlyphDelta::required;
    let o = GlyphDelta::optional;
    let all = |d: [(i16, i16); 8]| d.iter().map(|&(x, y)| r(x, y)).collect::<Vec<_>>();
    let gvar = Gvar::new(
        vec![
            GlyphVariations::new(GlyphId::new(0), vec![]),
            GlyphVariations::new(
                GlyphId::new(1),
                vec![
                    GlyphDeltas::new(tents(UP, OFF), vec![r(-20, 0), o(-20, 50), r(20, 50), o(20, 0), o(0, 0), r(40, 0), o(0, 0), o(0, 0)]),
                    GlyphDeltas::new(tents(DOWN, OFF), all([(10, 0), (10, -30), (-10, -30), (-10, 0), (0, 0), (-20, 0), (0, 0), (0, 0)])),
                    GlyphDeltas::new(tents(OFF, UP), all([(0, 0), (0, 0), (100, 0), (100, 0), (0, 0), (100, 0), (0, 0), (0, 0)])),
                    GlyphDeltas::new(tents(OFF, DOWN), all([(0, 0), (0, 0), (-50, 0), (-50, 0), (0, 0), (-50, 0), (0, 0), (0, 0)])),
                    GlyphDeltas::new(tents(UP, UP), all([(0, 0), (0, 10), (30, 10), (30, 0), (0, 0), (30, 0), (0, 0), (0, 0)])),
                    GlyphDeltas::new(tents((0.0, 0.5, 1.0), OFF), all([(0, 0), (0, 20), (0, 20), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0)])),
                ],
            ),
            GlyphVariations::new(GlyphId::new(2), vec![GlyphDeltas::new(tents(UP, OFF), vec![r(30, 10), r(0, 0), r(60, 0), r(0, 0), r(0, 0)])]),
        ],
        2,
    )
    .unwrap();
    let axis = |tag: &[u8; 4], min: f64, def: f64, max: f64, name: u16| {
        VariationAxisRecord::new(WTag::new(tag), Fixed::from_f64(min), Fixed::from_f64(def), Fixed::from_f64(max), 0, NameId::new(name))
    };
    let instance = InstanceRecord {
        subfamily_name_id: NameId::new(258),
        flags: 0,
        coordinates: vec![Fixed::from_f64(650.0), Fixed::from_f64(100.0)],
        post_script_name_id: None,
    };
    let fvar =
        Fvar::new(AxisInstanceArrays::new(vec![axis(b"wght", 100.0, 400.0, 900.0, 256), axis(b"wdth", 75.0, 100.0, 125.0, 257)], vec![instance]));

    let mut builder = VariationStoreBuilder::new(2);
    let temp = builder.add_deltas(vec![(region(UP, OFF), 40), (region(OFF, UP), 10)]);
    let (store, remap) = builder.build();
    let index = remap.get(temp).unwrap();
    let mvar = Mvar::new(
        MajorMinor::new(1, 0),
        Some(store),
        vec![ValueRecord::new(WTag::new(b"xhgt"), index.delta_set_outer_index, index.delta_set_inner_index)],
    );
    let mut builder = VariationStoreBuilder::new_with_implicit_indices(2);
    builder.add_deltas::<i32>(Vec::new());
    builder.add_deltas(vec![(region(UP, OFF), 40), (region(DOWN, OFF), -20), (region(OFF, UP), 100), (region(OFF, DOWN), -50)]);
    builder.add_deltas(vec![(region(UP, OFF), 60)]);
    let (hstore, _) = builder.build();
    let hvar = Hvar::new(hstore, None, None, None);

    let mut post = vec![0u8; 32];
    post[0..4].copy_from_slice(&0x0003_0000u32.to_be_bytes());
    let mut font = SfntFont::new(Flavour::TrueType);
    font.insert(*b"head", head(long));
    font.insert(*b"maxp", maxp(3));
    font.insert(*b"hhea", hhea(3));
    font.insert(*b"hmtx", hmtx);
    font.insert(*b"OS/2", os2());
    font.insert(*b"post", post);
    font.insert(*b"loca", loca);
    font.insert(*b"glyf", glyf_bytes);
    font.insert(*b"gvar", write_fonts::dump_table(&gvar).unwrap());
    font.insert(*b"fvar", write_fonts::dump_table(&fvar).unwrap());
    font.insert(*b"MVAR", write_fonts::dump_table(&mvar).unwrap());
    font.insert(*b"HVAR", write_fonts::dump_table(&hvar).unwrap());
    if with_avar {
        let map = |a: i16, b: i16| AxisValueMap::new(F2Dot14::from_bits(a), F2Dot14::from_bits(b));
        let avar =
            Avar::new(vec![SegmentMaps::new(vec![map(-16384, -16384), map(0, 0), map(8192, 4096), map(16384, 16384)]), SegmentMaps::new(vec![])]);
        font.insert(*b"avar", write_fonts::dump_table(&avar).unwrap());
    }
    font
}

/// A's points, B's component offset, every advance and the x-height.
#[derive(Debug, PartialEq)]
struct Outline {
    a: Vec<(f64, f64)>,
    b: (f64, f64),
    advances: Vec<u16>,
    x_height: i16,
}

fn outline(font: &SfntFont) -> Outline {
    let long = static_tables::u16_at(font.get(*b"head").unwrap(), 50).unwrap() != 0;
    let glyphs = glyf::read_glyf(font.get(*b"glyf").unwrap(), font.get(*b"loca").unwrap(), long, 3).unwrap();
    let Glyph::Simple(a) = &glyphs[1] else { panic!("A stays simple") };
    let Glyph::Composite(b) = &glyphs[2] else { panic!("B stays composite") };
    let hmtx = glyf::read_metrics(font.get(*b"hmtx").unwrap(), font.get(*b"hhea").unwrap(), 3).unwrap();
    Outline {
        a: a.points.clone(),
        b: (b.components[0].dx, b.components[0].dy),
        advances: hmtx.advances,
        x_height: static_tables::u16_at(font.get(*b"OS/2").unwrap(), 86).unwrap() as i16,
    }
}

fn close(got: &Outline, want: &Outline, what: &str) {
    let near = |a: f64, b: f64| (a - b).abs() <= 1.0;
    assert_eq!(got.a.len(), want.a.len(), "{what}");
    for (g, w) in got.a.iter().zip(&want.a) {
        assert!(near(g.0, w.0) && near(g.1, w.1), "{what}: {got:?} != {want:?}");
    }
    assert!(near(got.b.0, want.b.0) && near(got.b.1, want.b.1), "{what}: {got:?} != {want:?}");
    for (g, w) in got.advances.iter().zip(&want.advances) {
        assert!(near(f64::from(*g), f64::from(*w)), "{what}: {got:?} != {want:?}");
    }
    assert!((got.x_height - want.x_height).abs() <= 1, "{what}: {got:?} != {want:?}");
}

fn options(limits: Vec<(Tag, AxisLimit)>) -> InstanceOptions {
    InstanceOptions { limits, ..InstanceOptions::default() }
}

fn axes_of(font: &SfntFont) -> Vec<(Tag, f64, f64, f64)> {
    crate::variations::fvar_axes(font).unwrap().into_iter().map(|a| (a.tag, a.minimum, a.default, a.maximum)).collect()
}

#[test]
fn pinning_one_of_two_axes_matches_the_instancer() {
    let font = variable_font(false);
    let partial = instantiate(&font, &options(vec![(WGHT, AxisLimit::Pin(650.0))])).unwrap();
    assert_eq!(axes_of(&partial), vec![(WDTH, 75.0, 100.0, 125.0)]);
    let gvar = read_fonts::tables::gvar::Gvar::read(FontData::new(partial.get(*b"gvar").unwrap())).unwrap();
    assert_eq!(gvar.axis_count(), 1);
    let fvar = read_fonts::tables::fvar::Fvar::read(FontData::new(partial.get(*b"fvar").unwrap())).unwrap();
    assert_eq!(fvar.instance_count(), 1, "the named instance sits at the pinned weight");
    // fontTools: A [(90, 0), (90, 745), (510, 745), (510, 0)], B at (65, 5), advances 620 and 730.
    let at_default = outline(&partial);
    assert_eq!(at_default.a, vec![(90.0, 0.0), (90.0, 745.0), (510.0, 745.0), (510.0, 0.0)]);
    assert_eq!(at_default.b, (65.0, 5.0));
    assert_eq!(at_default.advances, vec![500, 620, 730]);
    assert_eq!(at_default.x_height, 520, "half of MVAR's wght delta of 40");
    let hmtx = glyf::read_metrics(partial.get(*b"hmtx").unwrap(), partial.get(*b"hhea").unwrap(), 3).unwrap();
    assert_eq!(hmtx.bearings, vec![0, 90, 155]);
    // HVAR and MVAR keep only their wdth regions.
    for tag in [*b"HVAR", *b"MVAR"] {
        let data = partial.get(tag).unwrap();
        let offset = if tag == *b"MVAR" {
            usize::from(static_tables::u16_at(data, 10).unwrap())
        } else {
            usize::from(static_tables::u16_at(data, 6).unwrap())
        };
        let store = read_fonts::tables::variations::ItemVariationStore::read(FontData::new(&data[offset..])).unwrap();
        let regions = store.variation_region_list().unwrap();
        assert_eq!(regions.axis_count(), 1, "{}", tag_str(tag));
        assert!(regions.region_count() >= 1, "{}", tag_str(tag));
    }

    // fontTools, the partial font pinned again: wdth 125 and 87.5.
    let wide = outline(&full::full_instance(&partial, &[(WDTH, 125.0)]).unwrap());
    assert_eq!(wide.a, vec![(90.0, 0.0), (90.0, 750.0), (625.0, 750.0), (625.0, 0.0)]);
    assert_eq!(wide.advances, vec![500, 735, 730]);
    assert_eq!(wide.x_height, 530);
    let narrow = outline(&full::full_instance(&partial, &[(WDTH, 87.5)]).unwrap());
    assert_eq!(narrow.a, vec![(90.0, 0.0), (90.0, 745.0), (485.0, 745.0), (485.0, 0.0)]);
    assert_eq!(narrow.advances, vec![500, 595, 730]);
}

/// Pins the partial instance at each location and compares with the original pinned there.
fn check_invariant(font: &SfntFont, partial: &SfntFont, weights: &[f64], widths: &[f64]) {
    for &w in weights {
        for &d in widths {
            let mut location = Vec::new();
            if crate::variations::fvar_axes(partial).unwrap().iter().any(|a| a.tag == WGHT) {
                location.push((WGHT, w));
            }
            if crate::variations::fvar_axes(partial).unwrap().iter().any(|a| a.tag == WDTH) {
                location.push((WDTH, d));
            }
            let got = outline(&full::full_instance(partial, &location).unwrap());
            let want = outline(&full::full_instance(font, &[(WGHT, w), (WDTH, d)]).unwrap());
            close(&got, &want, &format!("wght {w}, wdth {d}"));
        }
    }
}

#[test]
fn limiting_a_range_keeps_the_font_inside_it() {
    let font = variable_font(false);
    let limits = vec![(WGHT, AxisLimit::Range { min: f64::NEG_INFINITY, default: None, max: 700.0 })];
    let partial = instantiate(&font, &options(limits)).unwrap();
    assert_eq!(axes_of(&partial), vec![(WGHT, 100.0, 400.0, 700.0), (WDTH, 75.0, 100.0, 125.0)]);
    assert_eq!(outline(&partial), outline(&font), "the default does not move");
    check_invariant(&font, &partial, &[100.0, 250.0, 400.0, 520.0, 650.0, 700.0], &[75.0, 100.0, 112.5, 125.0]);
}

#[test]
fn a_range_with_a_new_default_moves_the_default_and_rebases_avar() {
    let font = variable_font(true);
    let limits = vec![(WGHT, AxisLimit::Range { min: 300.0, default: Some(600.0), max: 900.0 })];
    let partial = instantiate(&font, &options(limits)).unwrap();
    assert_eq!(axes_of(&partial), vec![(WGHT, 300.0, 600.0, 900.0), (WDTH, 75.0, 100.0, 125.0)]);
    close(&outline(&partial), &outline(&full::full_instance(&font, &[(WGHT, 600.0)]).unwrap()), "the new default");
    assert_eq!(static_tables::u16_at(partial.get(*b"OS/2").unwrap(), 4), Some(600), "usWeightClass follows the new default");
    check_invariant(&font, &partial, &[300.0, 350.0, 400.0, 500.0, 600.0, 650.0, 800.0, 900.0], &[75.0, 100.0, 125.0]);
    let fvar = read_fonts::tables::fvar::Fvar::read(FontData::new(partial.get(*b"fvar").unwrap())).unwrap();
    assert_eq!(fvar.instance_count(), 1, "650 lies inside the new range");
}

#[test]
fn dropping_an_axis_pins_it_at_its_default() {
    let font = variable_font(false);
    let partial = instantiate(&font, &options(vec![(WDTH, AxisLimit::Drop)])).unwrap();
    assert_eq!(axes_of(&partial), vec![(WGHT, 100.0, 400.0, 900.0)]);
    assert_eq!(outline(&partial), outline(&font));
    check_invariant(&font, &partial, &[100.0, 300.0, 400.0, 650.0, 900.0], &[100.0]);
}

#[test]
fn a_range_clamped_to_one_value_gives_a_static_font() {
    let font = variable_font(false);
    let limits = vec![(WGHT, AxisLimit::Range { min: 950.0, default: None, max: 1000.0 }), (WDTH, AxisLimit::Drop)];
    let out = instantiate(&font, &options(limits)).unwrap();
    assert!(out.get(*b"fvar").is_none(), "both axes end up pinned");
    close(&outline(&out), &outline(&full::full_instance(&font, &[(WGHT, 900.0)]).unwrap()), "pinned at the maximum");
}

#[test]
fn bad_ranges_are_refused_and_change_nothing() {
    let font = variable_font(false);
    let before = font.clone();
    for limit in [
        AxisLimit::Range { min: 700.0, default: None, max: 300.0 },
        AxisLimit::Range { min: 300.0, default: Some(800.0), max: 700.0 },
        AxisLimit::Range { min: f64::NAN, default: None, max: 700.0 },
    ] {
        let err = instantiate(&font, &options(vec![(WGHT, limit)])).unwrap_err();
        assert!(matches!(err, TablesError::BadOptions(_)), "{err}");
    }
    let err = instantiate(&font, &options(vec![(*b"opsz", AxisLimit::Range { min: 8.0, default: None, max: 12.0 })])).unwrap_err();
    assert!(matches!(err, TablesError::BadOptions(_)), "{err}");
    let mut v2 = font.clone();
    let mut avar = vec![0u8, 2, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0];
    avar.extend_from_slice(&[0; 8]);
    v2.insert(*b"avar", avar);
    let err = instantiate(&v2, &options(vec![(WGHT, AxisLimit::Pin(500.0))])).unwrap_err();
    assert!(matches!(err, TablesError::Unsupported(_)), "{err}");
    assert_eq!(font, before);
}
