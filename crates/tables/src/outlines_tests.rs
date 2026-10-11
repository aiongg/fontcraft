//! Outline repairs on a TrueType font built in code. Expected outlines come from fontTools 4.66
//! `ttLib.removeOverlaps` and AFDKO 5.0.1 `ttfdecomponentizer` run on the same font built with
//! fontTools' FontBuilder (glyphs `.notdef`, `A` to `F` below).

use super::*;
use crate::sfnt::Flavour;

fn square(x: f64, y: f64, size: f64) -> Vec<Pt> {
    // Clockwise, as TrueType outlines run.
    [(x, y), (x, y + size), (x + size, y + size), (x + size, y)].iter().map(|&(x, y)| Pt { x, y, on: true }).collect()
}

fn offset(gid: u16, x: f64, y: f64) -> Comp {
    Comp { flags: 0, gid, anchor: Anchor::Offset(x, y), m: IDENTITY }
}

fn composite(comps: &[Comp], bbox: [i16; 4]) -> Vec<u8> {
    encode_composite(comps, &[], bbox).unwrap()
}

/// `.notdef` empty; `A` two overlapping squares; `B` a square with two instruction bytes; `C` two
/// overlapping `B`s; `D` two `B`s apart; `E` `B` scaled by half at (10, 20); `F` `E` at (100, 0)
/// and `B` at (400, 0). Every advance 600, every side bearing 0; a `TEST` table rides along.
fn font_with(glyphs: Vec<(&str, Vec<u8>)>) -> SfntFont {
    let mut font = SfntFont::new(Flavour::TrueType);
    let count = glyphs.len() as u16;
    let mut head = vec![0u8; 54];
    head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    head[50..52].copy_from_slice(&1u16.to_be_bytes());
    let mut glyf = Vec::new();
    let mut loca = Vec::new();
    for (_, bytes) in &glyphs {
        loca.extend_from_slice(&(glyf.len() as u32).to_be_bytes());
        glyf.extend_from_slice(bytes);
    }
    loca.extend_from_slice(&(glyf.len() as u32).to_be_bytes());
    let mut maxp = vec![0u8; 32];
    maxp[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    maxp[4..6].copy_from_slice(&count.to_be_bytes());
    let mut hhea = vec![0u8; 36];
    hhea[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    hhea[34..36].copy_from_slice(&count.to_be_bytes());
    let mut hmtx = Vec::new();
    for _ in 0..count {
        hmtx.extend_from_slice(&600u16.to_be_bytes());
        hmtx.extend_from_slice(&0i16.to_be_bytes());
    }
    let mut post = vec![0u8; 32];
    post[0..4].copy_from_slice(&0x0002_0000u32.to_be_bytes());
    post.extend_from_slice(&count.to_be_bytes());
    let mut strings = Vec::new();
    // `.notdef` is standard name 0; the others are the font's own names, 258 on, in order.
    let mut own = 0u16;
    for (name, _) in &glyphs {
        if *name == ".notdef" {
            post.extend_from_slice(&0u16.to_be_bytes());
        } else {
            post.extend_from_slice(&(258 + own).to_be_bytes());
            own += 1;
            strings.push(name.len() as u8);
            strings.extend_from_slice(name.as_bytes());
        }
    }
    post.extend_from_slice(&strings);
    for (tag, data) in [
        (*b"head", head),
        (*b"hhea", hhea),
        (*b"maxp", maxp),
        (*b"hmtx", hmtx),
        (*b"loca", loca),
        (*b"glyf", glyf),
        (*b"post", post),
        (*b"TEST", b"kept as is".to_vec()),
    ] {
        font.insert(tag, data);
    }
    font
}

fn oracle_font() -> SfntFont {
    let b = encode_simple(&[square(0.0, 0.0, 100.0)], &[0xb0, 0x01]).unwrap();
    font_with(vec![
        (".notdef", Vec::new()),
        ("A", encode_simple(&[square(0.0, 0.0, 100.0), square(50.0, 50.0, 100.0)], &[]).unwrap()),
        ("B", b),
        ("C", composite(&[offset(2, 0.0, 0.0), offset(2, 50.0, 50.0)], [0, 0, 150, 150])),
        ("D", composite(&[offset(2, 0.0, 0.0), offset(2, 300.0, 0.0)], [0, 0, 400, 100])),
        ("E", composite(&[Comp { m: [0.5, 0.0, 0.0, 0.5], ..offset(2, 10.0, 20.0) }], [10, 20, 60, 70])),
        ("F", composite(&[offset(5, 100.0, 0.0), offset(2, 400.0, 0.0)], [110, 0, 500, 100])),
    ])
}

/// The glyphs of a result, read back: `(name, outline as whole-unit points)` or the components.
fn read_back(font: &SfntFont) -> Work {
    Work::read(font).unwrap()
}

fn points(w: &Work, gid: usize) -> Vec<Vec<(i64, i64, bool)>> {
    match &w.glyphs[gid].parsed {
        Parsed::Simple { outline, .. } => outline.iter().map(|c| key(c)).collect(),
        other => panic!("glyph {gid} is not simple: {other:?}"),
    }
}

fn pts(list: &[(i64, i64)]) -> Vec<(i64, i64, bool)> {
    list.iter().map(|&(x, y)| (x, y, true)).collect()
}

fn as_set(c: &[(i64, i64, bool)]) -> BTreeSet<(i64, i64, bool)> {
    c.iter().copied().collect()
}

#[test]
fn remove_overlaps_matches_the_oracle() {
    let input = oracle_font();
    let report = repair_report(&input, &OutlineOptions::new(OutlineAction::RemoveOverlaps)).unwrap();
    assert_eq!(report.result.changed, vec!["A".to_string(), "C".to_string()]);
    let w = read_back(&report.result.font);
    // The oracle: one contour, clockwise, from (0, 0).
    let expected = pts(&[(0, 0), (0, 100), (50, 100), (50, 150), (150, 150), (150, 50), (100, 50), (100, 0)]);
    for gid in [1, 3] {
        let got = points(&w, gid);
        assert_eq!(got.len(), 1, "glyph {gid}");
        assert_eq!(as_set(&got[0]), as_set(&expected), "glyph {gid}");
        assert!(signed_area(&[got[0].iter().map(|&(x, y, on)| Pt { x: x as f64, y: y as f64, on }).collect()]) < 0.0, "the winding stays clockwise");
    }
    // B loses its instructions; D (no overlap) and E, F stay composites.
    assert!(matches!(&w.glyphs[2].parsed, Parsed::Simple { instructions, .. } if instructions.is_empty()));
    for gid in [4, 5, 6] {
        assert!(matches!(w.glyphs[gid].parsed, Parsed::Composite { .. }), "glyph {gid}");
    }
    assert_eq!(report.result.font.get(*b"TEST"), input.get(*b"TEST"));
    assert_eq!(report.result.font.get(*b"post"), input.get(*b"post"), "untouched tables keep their bytes");
    // maxp as the oracle recalculates it: 8 points, 1 contour; composites 8, 2, 2 elements, depth 2.
    let maxp = report.result.font.get(*b"maxp").unwrap();
    let at = |o: usize| u16_at(maxp, o).unwrap();
    assert_eq!([at(6), at(8), at(10), at(12), at(28), at(30)], [8, 1, 8, 2, 2, 2]);

    let keep = OutlineOptions { keep_hinting: true, ..OutlineOptions::new(OutlineAction::RemoveOverlaps) };
    let w = read_back(&repair(&input, &keep).unwrap().font);
    assert!(matches!(&w.glyphs[2].parsed, Parsed::Simple { instructions, .. } if instructions == &[0xb0, 0x01]));
}

#[test]
fn decompose_matches_the_oracle() {
    let input = oracle_font();
    let result = repair(&input, &OutlineOptions::new(OutlineAction::Decompose)).unwrap();
    assert_eq!(result.changed, ["C", "D", "E", "F"]);
    let w = read_back(&result.font);
    assert_eq!(points(&w, 5), vec![pts(&[(10, 20), (10, 70), (60, 70), (60, 20)])]);
    assert_eq!(points(&w, 6), vec![pts(&[(110, 20), (110, 70), (160, 70), (160, 20)]), pts(&[(400, 0), (400, 100), (500, 100), (500, 0)])]);
    assert_eq!(points(&w, 4), vec![pts(&[(0, 0), (0, 100), (100, 100), (100, 0)]), pts(&[(300, 0), (300, 100), (400, 100), (400, 0)])]);
    // B keeps its instructions; the decomposed glyphs have none.
    assert!(matches!(&w.glyphs[2].parsed, Parsed::Simple { instructions, .. } if instructions.len() == 2));
    let maxp = result.font.get(*b"maxp").unwrap();
    assert_eq!([u16_at(maxp, 10), u16_at(maxp, 28), u16_at(maxp, 30)], [Some(0), Some(0), Some(0)]);

    // Only F nests a composite: it becomes E's component placed directly.
    let nested = OutlineOptions { nested_only: true, ..OutlineOptions::new(OutlineAction::Decompose) };
    let result = repair(&input, &nested).unwrap();
    assert_eq!(result.changed, ["F"]);
    let w = read_back(&result.font);
    let Parsed::Composite { comps, .. } = &w.glyphs[6].parsed else { panic!("F stays a composite") };
    assert_eq!(comps[0].gid, 2);
    assert_eq!(comps[0].anchor, Anchor::Offset(110.0, 20.0));
    assert_eq!(comps[0].m, [0.5, 0.0, 0.0, 0.5]);

    let transformed = OutlineOptions { transformed_only: true, ..OutlineOptions::new(OutlineAction::Decompose) };
    assert_eq!(repair(&input, &transformed).unwrap().changed, ["E"]);
}

#[test]
fn direction_extremes_and_rounding() {
    let input = oracle_font();
    // A and B already run the TrueType way and start bottom left: nothing to do.
    let auto = repair(&input, &OutlineOptions::new(OutlineAction::CorrectDirection)).unwrap();
    assert!(auto.changed.is_empty());
    assert_eq!(auto.font, input, "with nothing changed the font comes back as it was");
    let ps = OutlineOptions { winding: Winding::PostScript, ..OutlineOptions::new(OutlineAction::CorrectDirection) };
    let w = read_back(&repair(&input, &ps).unwrap().font);
    assert_eq!(points(&w, 2), vec![pts(&[(0, 0), (100, 0), (100, 100), (0, 100)])]);
    let reverse = OutlineOptions { winding: Winding::Reverse, glyphs: vec!["B".into()], ..OutlineOptions::new(OutlineAction::CorrectDirection) };
    let w = read_back(&repair(&input, &reverse).unwrap().font);
    assert_eq!(points(&w, 2), vec![pts(&[(0, 0), (100, 0), (100, 100), (0, 100)])]);

    // A quadratic arch whose top is not a point: the extreme goes in, the off-curve point at 200
    // goes, so the stored bounds drop to the curve's top at 100.
    let arch = vec![Pt { x: 0.0, y: 0.0, on: true }, Pt { x: 100.0, y: 200.0, on: false }, Pt { x: 200.0, y: 0.0, on: true }];
    let font = font_with(vec![(".notdef", Vec::new()), ("arch", encode_simple(&[arch], &[]).unwrap())]);
    let report = repair_report(&font, &OutlineOptions::new(OutlineAction::AddExtremes)).unwrap();
    assert_eq!(report.result.changed, ["arch"]);
    assert!(report.points_added.get("arch").is_some_and(|n| *n >= 1));
    let w = read_back(&report.result.font);
    assert_eq!(bbox_in_header(&w.glyphs[1].bytes), Some([0, 0, 200, 100]));

    let grid = OutlineOptions { grid: 40.0, glyphs: vec!["A".into(), "E".into()], ..OutlineOptions::new(OutlineAction::Round) };
    let report = repair_report(&input, &grid).unwrap();
    let w = read_back(&report.result.font);
    assert_eq!(points(&w, 1)[1], pts(&[(40, 40), (40, 160), (160, 160), (160, 40)]));
    let Parsed::Composite { comps, .. } = &w.glyphs[5].parsed else { panic!("E stays a composite") };
    assert_eq!(comps[0].anchor, Anchor::Offset(0.0, 40.0));
    assert_eq!(w.metrics[1].0, 600, "600 is on the 40-unit grid");
    assert!(report.values_changed > 0);
}

#[test]
fn refusals_and_damaged_glyphs() {
    let input = oracle_font();
    let mut cff = input.clone();
    cff.remove(*b"glyf");
    cff.insert(*b"CFF ", vec![1, 0, 4, 1]);
    assert!(matches!(repair(&cff, &OutlineOptions::new(OutlineAction::Round)), Err(TablesError::Unsupported(_))));
    let mut variable = input.clone();
    variable.insert(*b"gvar", vec![0; 20]);
    assert!(matches!(repair(&variable, &OutlineOptions::new(OutlineAction::RemoveOverlaps)), Err(TablesError::Unsupported(_))));
    let unknown = OutlineOptions { glyphs: vec!["Z".into()], ..OutlineOptions::new(OutlineAction::Round) };
    assert!(matches!(repair(&input, &unknown), Err(TablesError::BadOptions(_))));
    let bad_grid = OutlineOptions { grid: f64::NAN, ..OutlineOptions::new(OutlineAction::Round) };
    assert!(repair(&input, &bad_grid).is_err());

    // Two composites that use each other, and a glyph of junk bytes.
    let cycle = font_with(vec![
        (".notdef", Vec::new()),
        ("P", composite(&[offset(2, 0.0, 0.0)], [0; 4])),
        ("Q", composite(&[offset(1, 0.0, 0.0)], [0; 4])),
        ("J", vec![0, 1, 0xff, 0xff, 0, 0, 0, 0, 0, 0, 0, 9]),
    ]);
    for action in
        [OutlineAction::RemoveOverlaps, OutlineAction::CorrectDirection, OutlineAction::AddExtremes, OutlineAction::Round, OutlineAction::Decompose]
    {
        let strict = repair(&cycle, &OutlineOptions::new(action));
        let lenient = repair_report(&cycle, &OutlineOptions { ignore_errors: true, ..OutlineOptions::new(action) });
        if matches!(action, OutlineAction::RemoveOverlaps | OutlineAction::Decompose) {
            // J cannot be read; P and Q, drawn, never end.
            assert!(strict.is_err(), "{action:?}");
            let report = lenient.unwrap();
            let failed: Vec<&str> = report.failed.iter().map(|f| f.glyph.as_str()).collect();
            let expected: &[&str] = if action == OutlineAction::Decompose { &["P", "Q"] } else { &["J"] };
            assert_eq!(failed, expected, "{action:?}");
        } else {
            let _ = (strict, lenient);
        }
    }
    let truncated = font_with(vec![(".notdef", Vec::new()), ("C", vec![0xff, 0xff, 0, 0])]);
    for action in [OutlineAction::RemoveOverlaps, OutlineAction::Decompose, OutlineAction::Round] {
        let _ = repair_report(&truncated, &OutlineOptions { ignore_errors: true, ..OutlineOptions::new(action) });
    }
}

#[test]
fn encoded_glyphs_read_back() {
    let outline = vec![vec![
        Pt { x: 0.0, y: 0.0, on: true },
        Pt { x: 300.0, y: -2.0, on: false },
        Pt { x: 300.0, y: -2.0, on: false },
        Pt { x: -1000.0, y: 5.0, on: true },
    ]];
    let bytes = encode_simple(&outline, &[1, 2, 3]).unwrap();
    let Parsed::Simple { outline: back, instructions, .. } = parse(&bytes) else { panic!("simple") };
    assert_eq!(back, outline);
    assert_eq!(instructions, [1, 2, 3]);
    let comps = [
        Comp { flags: USE_MY_METRICS, gid: 300, anchor: Anchor::Offset(-200.0, 5.0), m: [0.5, 0.25, -0.25, 1.5] },
        Comp { flags: 0, gid: 1, anchor: Anchor::Points(3, 400), m: IDENTITY },
    ];
    let bytes = encode_composite(&comps, &[7], [1, 2, 3, 4]).unwrap();
    let Parsed::Composite { comps: back, instructions, .. } = parse(&bytes) else { panic!("composite") };
    assert_eq!(back[0].flags & USE_MY_METRICS, USE_MY_METRICS);
    assert_eq!((back[0].gid, back[0].anchor, back[0].m), (300, Anchor::Offset(-200.0, 5.0), [0.5, 0.25, -0.25, 1.5]));
    assert_eq!((back[1].gid, back[1].anchor), (1, Anchor::Points(3, 400)));
    assert_eq!(instructions, [7]);
    let g = Glyph { parsed: parse(&bytes), bytes, moved: false };
    let stripped = strip_instructions(&g).unwrap();
    assert!(matches!(parse(&stripped), Parsed::Composite { instructions, .. } if instructions.is_empty()));
}
