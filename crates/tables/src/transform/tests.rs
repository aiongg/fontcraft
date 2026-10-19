//! Behaviour of `transform` on fonts built in code. Units-per-em expectations follow fontTools'
//! `scaleUpem` rule, otRound(value × new ÷ old), worked out by hand for 1000 → 2048.

use std::collections::BTreeSet;

use read_fonts::types::GlyphId16;
use read_fonts::{FontData, FontRead};
use write_fonts::tables::gpos::{Gpos, PositionLookup, SinglePos, ValueRecord};
use write_fonts::tables::layout::{CoverageTable, FeatureList, Lookup, LookupFlag, LookupList, ScriptList};

use super::glyf::{self, Args, Component, Composite, Form, Glyph, Point, Shape, Simple};
use super::{TransformOptions, Widths, transform, transform_with_report};
use crate::TablesError;
use crate::sfnt::{Flavour, SfntFont};

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}

fn i16_at(b: &[u8], at: usize) -> i16 {
    u16_at(b, at) as i16
}

fn changed(shape: Shape) -> Glyph {
    Glyph { shape, raw: Vec::new(), changed: true }
}

/// `A`: (0, 0) (0, 700) (333, 700) (100, 0) with two instruction bytes.
fn glyph_a() -> Vec<u8> {
    let points = [(0, 0), (0, 700), (333, 700), (100, 0)].map(|(x, y)| Point { x, y, on: true, cubic: false }).to_vec();
    let g = changed(Shape::Simple(Simple { ends: vec![3], points, instructions: vec![0xB0, 0x00], overlap: false }));
    glyf::encode(&g, Some([0, 0, 333, 700])).unwrap()
}

/// A composite of `A` at (dx, 0).
fn composite_of_a(dx: i32) -> Vec<u8> {
    let comp = Component { flags: 0, glyph: 1, args: Args::Offset(dx, 0), matrix: [1.0, 0.0, 0.0, 1.0], form: Form::None };
    let g = changed(Shape::Composite(Composite { components: vec![comp], instructions: Vec::new() }));
    glyf::encode(&g, Some([dx, 0, dx + 333, 700])).unwrap()
}

/// `.notdef` empty, `A`, `B` = `A` at (50, 0); upem 1000, every advance 600, side bearings 0,
/// 0 and 50; `OS/2` typo ascender 800, `post` underline -100 and 50, a `kern` pair of -37, a
/// `fpgm` and a `TEST` table.
fn font() -> SfntFont {
    let glyphs = [Vec::new(), glyph_a(), composite_of_a(50)];
    let mut f = SfntFont::new(Flavour::TrueType);
    let mut head = vec![0u8; 54];
    head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    head[50..52].copy_from_slice(&1u16.to_be_bytes());
    let mut glyf = Vec::new();
    let mut loca = Vec::new();
    for g in &glyphs {
        loca.extend_from_slice(&(glyf.len() as u32).to_be_bytes());
        glyf.extend_from_slice(g);
    }
    loca.extend_from_slice(&(glyf.len() as u32).to_be_bytes());
    let mut maxp = vec![0u8; 32];
    maxp[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    maxp[4..6].copy_from_slice(&3u16.to_be_bytes());
    maxp[26..28].copy_from_slice(&2u16.to_be_bytes());
    let mut hhea = vec![0u8; 36];
    hhea[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    hhea[4..6].copy_from_slice(&800i16.to_be_bytes());
    hhea[6..8].copy_from_slice(&(-200i16).to_be_bytes());
    hhea[34..36].copy_from_slice(&3u16.to_be_bytes());
    let mut hmtx = Vec::new();
    for lsb in [0i16, 0, 50] {
        hmtx.extend_from_slice(&600u16.to_be_bytes());
        hmtx.extend_from_slice(&lsb.to_be_bytes());
    }
    let mut os2 = vec![0u8; 96];
    os2[68..70].copy_from_slice(&800i16.to_be_bytes());
    os2[74..76].copy_from_slice(&1000u16.to_be_bytes());
    let mut post = vec![0u8; 32];
    post[0..4].copy_from_slice(&0x0003_0000u32.to_be_bytes());
    post[8..10].copy_from_slice(&(-100i16).to_be_bytes());
    post[10..12].copy_from_slice(&50i16.to_be_bytes());
    // kern version 0, one format 0 subtable with the pair (A, B) = -37.
    let mut kern = Vec::new();
    for v in [0u16, 1, 0, 20, 0x0001, 1, 6, 0, 0, 1, 2] {
        kern.extend_from_slice(&v.to_be_bytes());
    }
    kern.extend_from_slice(&(-37i16).to_be_bytes());
    for (tag, data) in [
        (*b"head", head),
        (*b"hhea", hhea),
        (*b"maxp", maxp),
        (*b"OS/2", os2),
        (*b"hmtx", hmtx),
        (*b"fpgm", vec![0xB0, 0x00]),
        (*b"loca", loca),
        (*b"glyf", glyf),
        (*b"kern", kern),
        (*b"post", post),
        (*b"TEST", b"kept as is".to_vec()),
    ] {
        f.insert(tag, data);
    }
    f
}

/// The decoded glyphs of a font.
fn glyphs_of(f: &SfntFont) -> Vec<Glyph> {
    let long = i16_at(f.get(*b"head").unwrap(), 50) != 0;
    let glyf = f.get(*b"glyf").unwrap();
    let count = usize::from(u16_at(f.get(*b"maxp").unwrap(), 4));
    let offsets = glyf::read_loca(f.get(*b"loca").unwrap(), long, count, glyf.len()).unwrap();
    glyf::read_glyphs(glyf, &offsets).unwrap()
}

fn points(g: &Glyph) -> Vec<(i32, i32)> {
    match &g.shape {
        Shape::Simple(s) => s.points.iter().map(|p| (p.x, p.y)).collect(),
        _ => panic!("not a simple glyph"),
    }
}

fn metric(f: &SfntFont, gid: usize) -> (u16, i16) {
    let hmtx = f.get(*b"hmtx").unwrap();
    let long = usize::from(u16_at(f.get(*b"hhea").unwrap(), 34));
    let adv = u16_at(hmtx, 4 * gid.min(long - 1));
    let lsb = if gid < long { i16_at(hmtx, 4 * gid + 2) } else { i16_at(hmtx, 4 * long + 2 * (gid - long)) };
    (adv, lsb)
}

fn upem(target: u16) -> TransformOptions {
    TransformOptions { units_per_em: Some(target), ..TransformOptions::default() }
}

#[test]
fn units_per_em_scales_outlines_metrics_and_tables_as_scale_upem() {
    let input = font();
    let (out, report) = transform_with_report(&input, &upem(2048)).unwrap();
    assert_eq!(report.units_per_em, Some((1000, 2048)));
    let glyphs = glyphs_of(&out);
    assert_eq!(points(&glyphs[1]), vec![(0, 0), (0, 1434), (682, 1434), (205, 0)]);
    assert_eq!(glyphs[1].instructions(), &[0xB0, 0x00], "instructions are kept");
    match &glyphs[2].shape {
        Shape::Composite(c) => assert_eq!(c.components[0].args, Args::Offset(102, 0)),
        _ => panic!("B stays a composite"),
    }
    let head = out.get(*b"head").unwrap();
    assert_eq!(u16_at(head, 18), 2048);
    assert_eq!([i16_at(head, 36), i16_at(head, 38), i16_at(head, 40), i16_at(head, 42)], [0, 0, 784, 1434]);
    assert_eq!(metric(&out, 1), (1229, 0));
    assert_eq!(metric(&out, 2), (1229, 102));
    let hhea = out.get(*b"hhea").unwrap();
    assert_eq!(i16_at(hhea, 4), 1638);
    assert_eq!(i16_at(hhea, 6), -410);
    assert_eq!(u16_at(hhea, 10), 1229, "advanceWidthMax");
    assert_eq!([i16_at(hhea, 12), i16_at(hhea, 14), i16_at(hhea, 16)], [0, 445, 784], "minLSB, minRSB, xMaxExtent worked out again");
    let os2 = out.get(*b"OS/2").unwrap();
    assert_eq!(i16_at(os2, 68), 1638);
    assert_eq!(u16_at(os2, 74), 2048);
    let post = out.get(*b"post").unwrap();
    assert_eq!((i16_at(post, 8), i16_at(post, 10)), (-205, 102));
    assert_eq!(i16_at(out.get(*b"kern").unwrap(), 4 + 6 + 8 + 4), -76);
    assert_eq!(out.get(*b"TEST"), input.get(*b"TEST"), "unknown tables pass through");
    assert_eq!(out.get(*b"fpgm"), input.get(*b"fpgm"));
    assert!(report.warnings.iter().any(|w| w.contains("hint the font again")));
    // The written font reads back.
    let bytes = crate::write(&out, &crate::WriteOptions::default()).unwrap();
    assert!(crate::read(&bytes, 0).is_ok());
    // Back to 1000: within one unit of the input.
    let back = transform(&out, &upem(1000)).unwrap();
    let a = points(&glyphs_of(&back)[1]);
    for (p, q) in a.iter().zip([(0, 0), (0, 700), (333, 700), (100, 0)]) {
        assert!((p.0 - q.0).abs() <= 1 && (p.1 - q.1).abs() <= 1, "{p:?} against {q:?}");
    }
}

#[test]
fn units_per_em_scales_gpos_values() {
    let record = ValueRecord::new().with_x_advance(-50);
    let coverage: CoverageTable = [GlyphId16::new(1)].into_iter().collect();
    let lookup = PositionLookup::Single(Lookup::new(LookupFlag::empty(), vec![SinglePos::format_1(coverage, record)]));
    let gpos = Gpos::new(ScriptList::default(), FeatureList::default(), LookupList::new(vec![lookup]));
    let mut input = font();
    input.insert(*b"GPOS", write_fonts::dump_table(&gpos).unwrap());
    let out = transform(&input, &upem(2048)).unwrap();
    let read = read_fonts::tables::gpos::Gpos::read(FontData::new(out.get(*b"GPOS").unwrap())).unwrap();
    let lookup = read.lookup_list().unwrap().lookups().get(0).unwrap();
    let read_fonts::tables::gpos::PositionLookup::Single(single) = lookup else { panic!("a single adjustment lookup") };
    let read_fonts::tables::gpos::SinglePos::Format1(t) = single.subtables().get(0).unwrap() else { panic!("format 1") };
    assert_eq!(t.value_record().x_advance(), Some(-102));
}

/// A `gvar` with one axis where only glyph 1 (`A`) varies: one tuple over all of its eight
/// points, x deltas 10, 0, 100, -1 and four zeros, y deltas all zero.
fn gvar() -> Vec<u8> {
    let data: Vec<u8> = vec![
        0x00, 0x01, // one tuple, no shared points
        0x00, 0x0A, // the data starts after the 10-byte header
        0x00, 0x07, // 7 bytes of data
        0x80, 0x00, // embedded peak
        0x40, 0x00, // peak 1.0
        0x03, 10, 0, 100, 0xFF, // four byte deltas
        0x83, // four zeros
        0x87, // eight zero y deltas
        0x00, // padding
    ];
    let mut out = Vec::new();
    for v in [1u16, 0, 1, 0] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    let offsets_len = 4 * 2;
    out.extend_from_slice(&((20 + offsets_len) as u32).to_be_bytes());
    out.extend_from_slice(&3u16.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&((20 + offsets_len) as u32).to_be_bytes());
    for o in [0u16, 0, data.len() as u16 / 2, data.len() as u16 / 2] {
        out.extend_from_slice(&o.to_be_bytes());
    }
    out.extend_from_slice(&data);
    out
}

#[test]
fn units_per_em_scales_gvar_deltas() {
    let mut input = font();
    input.insert(*b"gvar", gvar());
    let out = transform(&input, &upem(2048)).unwrap();
    let table = read_fonts::tables::gvar::Gvar::read(FontData::new(out.get(*b"gvar").unwrap())).unwrap();
    let data = table.glyph_variation_data(read_fonts::types::GlyphId::new(1)).unwrap().unwrap();
    let tuple = data.tuples().next().unwrap();
    let xs: Vec<i32> = tuple.deltas().map(|d| d.x_delta).collect();
    assert_eq!(xs, vec![20, 0, 205, -2, 0, 0, 0, 0]);
    assert!(table.glyph_variation_data(read_fonts::types::GlyphId::new(2)).unwrap().is_none());
}

#[test]
fn quarter_turn_rotates_points_moves_components_and_sets_widths_to_the_em() {
    let input = font();
    let options = TransformOptions { rotate: Some(90.0), ..TransformOptions::default() };
    let (out, report) = transform_with_report(&input, &options).unwrap();
    assert_eq!(report.matrix, Some([0.0, -1.0, 1.0, 0.0, 0.0, 0.0]), "clockwise, snapped to whole numbers");
    let glyphs = glyphs_of(&out);
    assert_eq!(points(&glyphs[1]), vec![(0, 0), (700, 0), (700, -333), (0, -100)]);
    assert!(glyphs[1].instructions().is_empty(), "a transformed glyph loses its instructions");
    match &glyphs[2].shape {
        Shape::Composite(c) => {
            assert_eq!(c.components[0].args, Args::Offset(0, -50));
            assert_eq!(c.components[0].matrix, [1.0, 0.0, 0.0, 1.0]);
        }
        _ => panic!("B stays a composite"),
    }
    assert_eq!(metric(&out, 1), (1000, 0), "a quarter turn sets the advance to the em");
    assert!(out.get(*b"fpgm").is_none(), "no glyph keeps instructions, so the font program goes");
    assert!(!report.hints_kept);
    assert_eq!(out.get(*b"kern"), input.get(*b"kern"));
    assert_eq!(u16_at(out.get(*b"head").unwrap(), 18), 1000);
}

#[test]
fn transforming_some_glyphs_keeps_the_composites_that_use_them_in_place() {
    let input = font();
    let options = TransformOptions { rotate: Some(90.0), glyphs: Some(BTreeSet::from([1u16])), ..TransformOptions::default() };
    let out = transform(&input, &options).unwrap();
    let glyphs = glyphs_of(&out);
    let bounds = glyf::all_bounds(&glyphs).unwrap();
    assert_eq!(bounds[2], Some([50, 0, 383, 700]), "B looks as before");
    assert_eq!(metric(&out, 2), (600, 50), "B keeps its advance");
    assert_eq!(metric(&out, 1).0, 1000);
}

#[test]
fn scale_then_shift_transforms_widths_when_nothing_rotates() {
    let input = font();
    let options = TransformOptions { scale: Some((2.0, 1.0)), translate: Some((10.0, 0.0)), ..TransformOptions::default() };
    let out = transform(&input, &options).unwrap();
    assert_eq!(points(&glyphs_of(&out)[1]), vec![(10, 0), (10, 700), (676, 700), (210, 0)]);
    assert_eq!(metric(&out, 1), (1210, 10));
    let keep = TransformOptions { widths: Widths::Keep, ..options.clone() };
    assert_eq!(metric(&transform(&input, &keep).unwrap(), 1).0, 600);
    let em = TransformOptions { widths: Widths::Em, ..options };
    assert_eq!(metric(&transform(&input, &em).unwrap(), 1).0, 1000);
}

#[test]
fn bad_options_and_inputs_are_refused() {
    let input = font();
    let both = TransformOptions { matrix: Some([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]), rotate: Some(10.0), ..TransformOptions::default() };
    assert!(matches!(transform(&input, &both), Err(TablesError::BadOptions(_))));
    let flat = TransformOptions { matrix: Some([1.0, 0.0, 1.0, 0.0, 0.0, 0.0]), ..TransformOptions::default() };
    assert!(matches!(transform(&input, &flat), Err(TablesError::BadOptions(_))));
    assert!(matches!(transform(&input, &TransformOptions::default()), Err(TablesError::BadOptions(_))));
    assert!(matches!(transform(&input, &upem(10)), Err(TablesError::BadOptions(_))));
    let nan = TransformOptions { slant: Some(f64::NAN), ..TransformOptions::default() };
    assert!(transform(&input, &nan).is_err());
    let huge = TransformOptions { scale: Some((100.0, 100.0)), ..TransformOptions::default() };
    assert!(matches!(transform(&input, &huge), Err(TablesError::Invalid(_))), "coordinates past ±32,767 refuse the run");
    assert!(transform(&input, &upem(16384)).is_ok());

    let mut cff = input.clone();
    cff.insert(*b"CFF ", vec![1, 0, 4, 1]);
    assert!(matches!(transform(&cff, &upem(2048)), Err(TablesError::Unsupported(_))));
    let mut variable = input.clone();
    variable.insert(*b"gvar", gvar());
    assert!(matches!(transform(&variable, &TransformOptions { rotate: Some(5.0), ..TransformOptions::default() }), Err(TablesError::Unsupported(_))));

    let mut zero = input.clone();
    zero.tables.get_mut(b"head").unwrap()[18..20].copy_from_slice(&0u16.to_be_bytes());
    assert!(transform(&zero, &upem(2048)).is_err(), "units per em 0 is refused, not divided by");
    let mut cut = input.clone();
    let glyf = cut.tables.get_mut(b"glyf").unwrap();
    let half = glyf.len() / 2;
    glyf.truncate(half);
    assert!(transform(&cut, &upem(2048)).is_err());
    let mut cut_gvar = input;
    let mut g = gvar();
    g.truncate(g.len() - 6);
    cut_gvar.insert(*b"gvar", g);
    assert!(transform(&cut_gvar, &upem(2048)).is_err());
}

#[test]
fn cubic_point_flags_survive_a_rewritten_glyph() {
    // One contour: on, cubic off, cubic off, on; coordinates as words.
    let mut raw: Vec<u8> = Vec::new();
    for v in [1i16, 0, 0, 100, 100, 3, 0] {
        raw.extend_from_slice(&v.to_be_bytes());
    }
    raw.extend_from_slice(&[0x01, 0x80, 0x80, 0x01]);
    for v in [0i16, 0, 100, 0, 0, 100, 0, -100] {
        raw.extend_from_slice(&v.to_be_bytes());
    }
    let mut glyphs = glyf::read_glyphs(&raw, &[0, raw.len()]).unwrap();
    let mut g = glyphs.remove(0);
    g.changed = true;
    let out = glyf::encode(&g, Some([0, 0, 100, 100])).unwrap();
    let back = glyf::read_glyphs(&out, &[0, out.len()]).unwrap();
    let Shape::Simple(s) = &back[0].shape else { panic!("a simple glyph") };
    assert_eq!(s.points.iter().map(|p| p.cubic).collect::<Vec<_>>(), [false, true, true, false]);
}

#[test]
fn units_per_em_recalculates_maxp_and_head_flags_and_stores_standard_post_names_by_index() {
    let mut input = font();
    // Version 2 names: glyph 1 is `.null` and glyph 2 `Bcomp`, both stored as strings.
    let mut post = input.get(*b"post").unwrap()[..32].to_vec();
    post[0..4].copy_from_slice(&0x0002_0000u32.to_be_bytes());
    for v in [3u16, 0, 258, 259] {
        post.extend_from_slice(&v.to_be_bytes());
    }
    for s in [".null", "Bcomp"] {
        post.push(s.len() as u8);
        post.extend_from_slice(s.as_bytes());
    }
    input.insert(*b"post", post);
    // A stored size larger than any glyph program, as fonts whose size counts the font programs carry.
    let mut maxp = input.get(*b"maxp").unwrap().to_vec();
    maxp[26..28].copy_from_slice(&3596u16.to_be_bytes());
    input.insert(*b"maxp", maxp);
    let out = transform(&input, &upem(2048)).unwrap();

    let maxp = out.get(*b"maxp").unwrap();
    let maxima: Vec<u16> = [6, 8, 10, 12, 28, 30].iter().map(|at| u16_at(maxp, *at)).collect();
    assert_eq!(maxima, [4, 1, 4, 1, 1, 1], "points, contours, composite points and contours, component elements and depth");
    assert_eq!(u16_at(maxp, 26), 3596, "the stored instruction size is kept");
    assert_eq!(u16_at(out.get(*b"head").unwrap(), 16) & 2, 2, "every left side bearing equals xMin");

    let post = out.get(*b"post").unwrap();
    assert_eq!((i16_at(post, 8), i16_at(post, 10)), (-205, 102), "the underline still scales");
    let mut expected = post[..32].to_vec();
    for v in [3u16, 0, 1, 258] {
        expected.extend_from_slice(&v.to_be_bytes());
    }
    expected.push(5);
    expected.extend_from_slice(b"Bcomp");
    assert_eq!(post, &expected[..], "`.null` takes its standard index and leaves the stored strings");
}
