//! A matrix on a variable TrueType font built here. fontTools has no matrix for `gvar`, so the
//! check is an invariant: instancing, then transforming, gives the outline that transforming,
//! then instancing gives, within the rounding of the two orders.

use read_fonts::{FontData, FontRead, FontRef, TableProvider};

use super::tests::{font, gvar};
use super::{TransformOptions, transform, transform_with_report};
use crate::instance::{AxisLimit, InstanceOptions, instantiate};
use crate::sfnt::SfntFont;

/// One `wght` axis from 100 to 900, default 400.
fn fvar() -> Vec<u8> {
    let mut out = Vec::new();
    for v in [1u16, 0, 16, 2, 1, 20, 0, 8] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    out.extend_from_slice(b"wght");
    for v in [100i32, 400, 900] {
        out.extend_from_slice(&(v << 16).to_be_bytes());
    }
    out.extend_from_slice(&[0, 0, 1, 0]);
    out
}

/// A `gvar` where glyph 1 (`A`, four points) has one sparse tuple at the axis maximum: point 1
/// moves by (10, 5), point 2 by (30, -20); points 0 and 3 are inferred.
fn sparse_gvar() -> Vec<u8> {
    let data: Vec<u8> = vec![
        0x00, 0x01, // one tuple, no shared points
        0x00, 0x0A, // data after the 10-byte header
        0x00, 0x0A, // 10 bytes of data
        0xA0, 0x00, // embedded peak, private points
        0x40, 0x00, // peak 1.0
        0x02, 0x01, 1, 1, // points 1 and 2
        0x01, 10, 30, // x deltas
        0x01, 5, 0xEC, // y deltas
    ];
    let mut out = Vec::new();
    for v in [1u16, 0, 1, 0] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    out.extend_from_slice(&28u32.to_be_bytes());
    out.extend_from_slice(&3u16.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&28u32.to_be_bytes());
    for o in [0u16, 0, 10, 10] {
        out.extend_from_slice(&o.to_be_bytes());
    }
    out.extend_from_slice(&data);
    out
}

fn variable(gvar: Vec<u8>) -> SfntFont {
    let mut f = font();
    f.insert(*b"fvar", fvar());
    f.insert(*b"gvar", gvar);
    f
}

fn at(f: &SfntFont, wght: f64) -> SfntFont {
    instantiate(f, &InstanceOptions { limits: vec![(*b"wght", AxisLimit::Pin(wght))], ..InstanceOptions::default() }).unwrap()
}

fn points(f: &SfntFont, gid: u32) -> Vec<(i32, i32)> {
    let bytes = crate::write(f, &crate::WriteOptions::default()).unwrap();
    let font = FontRef::new(&bytes).unwrap();
    let glyph = font.loca(None).unwrap().get_glyf(read_fonts::types::GlyphId::new(gid), &font.glyf().unwrap()).unwrap();
    match glyph {
        Some(read_fonts::tables::glyf::Glyph::Simple(s)) => s.points().map(|p| (i32::from(p.x), i32::from(p.y))).collect(),
        _ => Vec::new(),
    }
}

fn advance(f: &SfntFont, gid: u32) -> u16 {
    let bytes = crate::write(f, &crate::WriteOptions::default()).unwrap();
    FontRef::new(&bytes).unwrap().hmtx().unwrap().advance(read_fonts::types::GlyphId::new(gid)).unwrap()
}

#[test]
fn transforming_commutes_with_instancing() {
    let input = variable(sparse_gvar());
    let matrices = [
        TransformOptions { rotate: Some(30.0), ..TransformOptions::default() },
        TransformOptions { slant: Some(12.0), ..TransformOptions::default() },
        TransformOptions { matrix: Some([1.2, 0.0, 0.0, 0.8, 5.0, -3.0]), ..TransformOptions::default() },
        TransformOptions { rotate: Some(90.0), translate: Some((0.0, 880.0)), ..TransformOptions::default() },
    ];
    for options in &matrices {
        let transformed = transform(&input, options).unwrap();
        for wght in [400.0, 650.0, 900.0] {
            let first = points(&transform(&at(&input, wght), options).unwrap(), 1);
            let second = points(&at(&transformed, wght), 1);
            assert_eq!(first.len(), 4);
            for (a, b) in first.iter().zip(&second) {
                assert!((a.0 - b.0).abs() <= 2 && (a.1 - b.1).abs() <= 2, "{options:?} at {wght}: {first:?} against {second:?}");
            }
            let widths = (advance(&transform(&at(&input, wght), options).unwrap(), 1), advance(&at(&transformed, wght), 1));
            assert!(widths.0.abs_diff(widths.1) <= 1, "{options:?} at {wght}: advances {widths:?}");
        }
    }
}

#[test]
fn a_quarter_turn_turns_dense_deltas_and_fixes_the_advance() {
    let input = variable(gvar());
    let (out, report) = transform_with_report(&input, &TransformOptions { rotate: Some(90.0), ..TransformOptions::default() }).unwrap();
    assert!(report.warnings.iter().all(|w| !w.contains("HVAR")));
    let table = read_fonts::tables::gvar::Gvar::read(FontData::new(out.get(*b"gvar").unwrap())).unwrap();
    let data = table.glyph_variation_data(read_fonts::types::GlyphId::new(1)).unwrap().unwrap();
    let tuple = data.tuples().next().unwrap();
    let xs: Vec<i32> = tuple.deltas().map(|d| d.x_delta).collect();
    let ys: Vec<i32> = tuple.deltas().map(|d| d.y_delta).collect();
    assert_eq!(xs, vec![0; 8], "x' = y: the y deltas were 0; the advance of a turn is the em at every location");
    assert_eq!(ys, vec![-10, 0, -100, 1, 0, 0, 0, 0], "y' = -x");
}

#[test]
fn a_damaged_gvar_is_refused_under_a_matrix() {
    let options = TransformOptions { rotate: Some(30.0), ..TransformOptions::default() };
    let good = sparse_gvar();
    for cut in (21..good.len()).step_by(3) {
        let f = variable(good[..cut].to_vec());
        assert!(transform(&f, &options).is_err(), "a gvar cut at {cut} is refused");
    }
    let mut far = good.clone();
    let last = far.len() - 7;
    far[last] = 200; // point 201, past the glyph's eight
    assert!(transform(&variable(far), &options).is_err());
}

/// A `gvar` where glyph 1 has one tuple at the axis maximum with `points` as its private point
/// numbers, x deltas 1, 2, 3, 4 and four zeros, and y deltas all zero.
fn one_tuple_gvar(points: &[u8]) -> Vec<u8> {
    let mut data: Vec<u8> = points.to_vec();
    data.extend_from_slice(&[0x07, 1, 2, 3, 4, 0, 0, 0, 0, 0x87]);
    let mut glyph: Vec<u8> = vec![0x00, 0x01, 0x00, 0x0A];
    glyph.extend_from_slice(&(data.len() as u16).to_be_bytes());
    glyph.extend_from_slice(&[0xA0, 0x00, 0x40, 0x00]);
    glyph.extend_from_slice(&data);
    if glyph.len() % 2 == 1 {
        glyph.push(0);
    }
    let mut out = Vec::new();
    for v in [1u16, 0, 1, 0] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    out.extend_from_slice(&28u32.to_be_bytes());
    out.extend_from_slice(&3u16.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&28u32.to_be_bytes());
    let half = (glyph.len() / 2) as u16;
    for o in [0u16, 0, half, half] {
        out.extend_from_slice(&o.to_be_bytes());
    }
    out.extend_from_slice(&glyph);
    out
}

#[test]
fn a_zero_point_count_in_two_bytes_means_every_point() {
    let words = variable(one_tuple_gvar(&[0x80, 0x00]));
    let byte = variable(one_tuple_gvar(&[0x00]));
    for options in [
        TransformOptions { rotate: Some(30.0), ..TransformOptions::default() },
        TransformOptions { units_per_em: Some(2000), ..TransformOptions::default() },
    ] {
        let (a, b) = (transform(&words, &options).unwrap(), transform(&byte, &options).unwrap());
        assert_eq!(points(&at(&a, 900.0), 1), points(&at(&b, 900.0), 1), "{options:?}");
    }
}

#[test]
fn filling_in_too_many_points_is_refused_before_the_work() {
    // One glyph of 65,535 points with 4,095 tuples that each name one point.
    let tuples = 4095usize;
    let headers = 4 + 6 * tuples;
    let mut glyph: Vec<u8> = Vec::new();
    glyph.extend_from_slice(&(tuples as u16).to_be_bytes());
    glyph.extend_from_slice(&(headers as u16).to_be_bytes());
    for _ in 0..tuples {
        glyph.extend_from_slice(&[0x00, 0x05, 0xA0, 0x00, 0x40, 0x00]);
    }
    for _ in 0..tuples {
        glyph.extend_from_slice(&[0x01, 0x00, 0x00, 0x80, 0x80]);
    }
    if glyph.len() % 2 == 1 {
        glyph.push(0);
    }
    let mut table = Vec::new();
    for v in [1u16, 0, 1, 0] {
        table.extend_from_slice(&v.to_be_bytes());
    }
    table.extend_from_slice(&24u32.to_be_bytes());
    table.extend_from_slice(&1u16.to_be_bytes());
    table.extend_from_slice(&0u16.to_be_bytes());
    table.extend_from_slice(&24u32.to_be_bytes());
    table.extend_from_slice(&0u16.to_be_bytes());
    table.extend_from_slice(&((glyph.len() / 2) as u16).to_be_bytes());
    table.extend_from_slice(&glyph);
    let all = 65_535 + 4;
    let deltas = super::gvar_matrix::GlyphDeltas {
        coords: vec![(0.0, 0.0); all],
        ends: vec![65_534],
        linear: Some(vec![[0.0, 1.0, -1.0, 0.0]; all - 4]),
        densify: true,
        advance_factor: None,
    };
    let result = super::gvar_matrix::transform_gvar(&table, &[deltas]);
    assert!(matches!(result, Err(crate::TablesError::TooLarge { .. })), "{result:?}");
}
