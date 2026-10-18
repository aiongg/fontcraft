//! Curve conversion and componentizing on TrueType fonts built in code. The expected points come
//! from fontTools 4.66.1 on the same glyphs built with its FontBuilder: the `qu2cu` command line
//! (conversion error 0.001 em on a 1000-unit em) for the cubic results, and `Cu2QuPen` into
//! `TTGlyphPen(...).glyph(dropImpliedOnCurves=True)` at 1 unit for the quadratic ones.

use super::*;
use crate::curves::{ComponentizeOptions, CurveOptions, CurveTarget, ReferenceComponent, ReferenceGlyph};
use crate::sfnt::Flavour;

type Raw = Vec<(i32, i32, u8)>;

fn glyph(ends: &[usize], pts: &[(i32, i32, u8)]) -> Vec<u8> {
    let g = Points {
        points: pts.iter().map(|&(x, y, _)| (f64::from(x), f64::from(y))).collect(),
        flags: pts.iter().map(|p| p.2).collect(),
        ends: ends.to_vec(),
    };
    encode_points(&g).unwrap()
}

fn read_points(font: &SfntFont, gid: usize) -> (Vec<usize>, Raw) {
    let w = Work::read(font).unwrap();
    let p = Points::read(&w.glyphs[gid].bytes).unwrap();
    (p.ends.clone(), p.points.iter().zip(&p.flags).map(|(&(x, y), &f)| (x as i32, y as i32, f)).collect())
}

/// A TrueType font of the glyphs given, each with its advance; side bearings 0.
fn font_with(glyphs: Vec<(&str, Vec<u8>, u16)>) -> SfntFont {
    let mut font = SfntFont::new(Flavour::TrueType);
    let count = glyphs.len() as u16;
    let mut head = vec![0u8; 54];
    head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    head[50..52].copy_from_slice(&1u16.to_be_bytes());
    let mut glyf = Vec::new();
    let mut loca = Vec::new();
    for (_, bytes, _) in &glyphs {
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
    for (_, _, advance) in &glyphs {
        hmtx.extend_from_slice(&advance.to_be_bytes());
        hmtx.extend_from_slice(&0i16.to_be_bytes());
    }
    let mut post = vec![0u8; 32];
    post[0..4].copy_from_slice(&0x0002_0000u32.to_be_bytes());
    post.extend_from_slice(&count.to_be_bytes());
    let mut strings = Vec::new();
    let mut own = 0u16;
    for (name, _, _) in &glyphs {
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
    for (tag, data) in [(*b"head", head), (*b"hhea", hhea), (*b"maxp", maxp), (*b"hmtx", hmtx), (*b"loca", loca), (*b"glyf", glyf), (*b"post", post)]
    {
        font.insert(tag, data);
    }
    font
}

const C: u8 = CUBIC;

fn quadratic_o() -> Vec<u8> {
    glyph(
        &[19],
        &[
            (300, 50, 1),
            (248, 50, 0),
            (158, 89, 0),
            (89, 158, 0),
            (50, 248, 0),
            (50, 300, 1),
            (50, 352, 0),
            (89, 442, 0),
            (158, 511, 0),
            (248, 550, 0),
            (300, 550, 1),
            (352, 550, 0),
            (442, 511, 0),
            (511, 442, 0),
            (550, 352, 0),
            (550, 300, 1),
            (550, 248, 0),
            (511, 158, 0),
            (442, 89, 0),
            (352, 50, 0),
        ],
    )
}

fn quadratic_d() -> Vec<u8> {
    glyph(
        &[12],
        &[
            (0, 0, 1),
            (75, 0, 0),
            (200, 33, 0),
            (298, 92, 0),
            (365, 168, 0),
            (400, 255, 0),
            (400, 300, 1),
            (400, 345, 0),
            (365, 432, 0),
            (298, 508, 0),
            (200, 567, 0),
            (75, 600, 0),
            (0, 600, 1),
        ],
    )
}

fn rect_l() -> Vec<u8> {
    glyph(&[3], &[(0, 0, 1), (0, 500, 1), (100, 500, 1), (100, 0, 1)])
}

fn short_s() -> Vec<u8> {
    glyph(&[4], &[(0, 0, 1), (0, 100, 0), (100, 200, 0), (200, 200, 1), (200, 0, 1)])
}

fn cubic_o() -> Raw {
    vec![(161, 50, C), (50, 161, C), (50, 439, C), (161, 550, C), (439, 550, C), (550, 439, C), (550, 161, C), (439, 50, C)]
}

fn cubic_d() -> Raw {
    vec![(0, 0, 1), (250, 0, C), (400, 150, C), (400, 450, C), (250, 600, C), (0, 600, 1)]
}

fn quadratic_font() -> SfntFont {
    let composite = encode_composite(&[Comp { flags: 0, gid: 1, anchor: Anchor::Offset(10.0, 0.0), m: IDENTITY }], &[], [0; 4]).unwrap();
    font_with(vec![
        (".notdef", Vec::new(), 500),
        ("O", quadratic_o(), 600),
        ("D", quadratic_d(), 600),
        ("L", rect_l(), 600),
        ("S", short_s(), 600),
        ("Oshift", composite, 600),
    ])
}

fn head_format(font: &SfntFont) -> i16 {
    i16_at(font.get(*b"head").unwrap(), 52).unwrap()
}

#[test]
fn to_cubic_matches_qu2cu() {
    let font = quadratic_font();
    let r = convert(&font, &CurveOptions::new(CurveTarget::Cubic { max_error: 1.0 })).unwrap();
    assert_eq!(r.result.changed, vec!["O".to_string(), "D".to_string()]);
    assert_eq!(r.segments, 6, "four quarter circles in O and two arcs in D");
    let out = &r.result.font;
    assert_eq!(read_points(out, 1), (vec![7], cubic_o()));
    assert_eq!(read_points(out, 2), (vec![5], cubic_d()));
    let before = Work::read(&font).unwrap();
    let after = Work::read(out).unwrap();
    for gid in [3, 4] {
        assert_eq!(before.glyphs[gid].bytes, after.glyphs[gid].bytes, "glyph {gid} keeps its bytes");
    }
    assert_eq!(head_format(out), 1);

    let all = CurveOptions { all_cubic: true, ..CurveOptions::new(CurveTarget::Cubic { max_error: 1.0 }) };
    let r = convert(&font, &all).unwrap();
    assert_eq!(read_points(&r.result.font, 4), (vec![4], vec![(0, 0, 1), (0, 133, C), (67, 200, C), (200, 200, 1), (200, 0, 1)]));
    assert_eq!(read_points(&r.result.font, 3).1, vec![(0, 0, 1), (0, 500, 1), (100, 500, 1), (100, 0, 1)]);
}

#[test]
fn to_quadratic_matches_cu2qu() {
    let encode = |pts: &Raw| glyph(&[pts.len() - 1], pts);
    let font = font_with(vec![(".notdef", Vec::new(), 500), ("O", encode(&cubic_o()), 600), ("D", encode(&cubic_d()), 600), ("L", rect_l(), 600)]);
    let o = CurveOptions::new(CurveTarget::Quadratic { max_error: 1.0, reverse_direction: false });
    let r = convert(&font, &o).unwrap();
    assert_eq!(r.result.changed, vec!["O".to_string(), "D".to_string()]);
    assert_eq!(
        read_points(&r.result.font, 1),
        (
            vec![11],
            vec![
                (231, 50, 0),
                (117, 117, 0),
                (50, 231, 0),
                (50, 370, 0),
                (117, 483, 0),
                (231, 550, 0),
                (370, 550, 0),
                (483, 483, 0),
                (550, 370, 0),
                (550, 231, 0),
                (483, 117, 0),
                (370, 50, 0),
            ]
        )
    );
    assert_eq!(
        read_points(&r.result.font, 2),
        (
            vec![9],
            vec![
                (0, 0, 1),
                (94, 0, 0),
                (242, 51, 0),
                (346, 137, 0),
                (400, 244, 0),
                (400, 356, 0),
                (346, 463, 0),
                (242, 549, 0),
                (94, 600, 0),
                (0, 600, 1)
            ]
        )
    );
    assert_eq!(head_format(&r.result.font), 0, "no cubic curve is left");

    // Each curve needs two quadratic pieces, so with `mixed` they stay cubic.
    let mixed = CurveOptions { mixed: true, ..o };
    let r = convert(&font, &mixed).unwrap();
    assert!(r.result.changed.is_empty());
    assert_eq!(r.result.font, font, "nothing changed, nothing rewritten");
}

#[test]
fn conversion_refusals_and_failures() {
    let cubic = CurveOptions::new(CurveTarget::Cubic { max_error: 1.0 });
    let mut cff = SfntFont::new(Flavour::Cff);
    cff.insert(*b"CFF ", vec![1, 0, 4, 4]);
    assert!(matches!(convert(&cff, &cubic), Err(TablesError::Unsupported(_))));
    let mut variable = quadratic_font();
    variable.insert(*b"gvar", vec![0; 20]);
    assert!(matches!(convert(&variable, &cubic), Err(TablesError::Unsupported(_))));
    assert!(matches!(componentize(&variable, &ComponentizeOptions::default()), Err(TablesError::Unsupported(_))));

    // A contour of off-curve points only cannot be made all cubic (the reference stops there).
    let ring = glyph(&[3], &[(0, 0, 0), (0, 100, 0), (100, 100, 0), (100, 0, 0)]);
    let font = font_with(vec![(".notdef", Vec::new(), 500), ("ring", ring, 600), ("O", quadratic_o(), 600)]);
    let all = CurveOptions { all_cubic: true, ..cubic.clone() };
    assert!(matches!(convert(&font, &all), Err(TablesError::Invalid(_))));
    let r = convert(&font, &CurveOptions { ignore_errors: true, ..all }).unwrap();
    assert_eq!(r.failed.len(), 1);
    assert_eq!(r.failed[0].glyph, "ring");
    assert_eq!(r.result.changed, vec!["O".to_string()]);
    // Without all_cubic the ring is left as it is.
    let r = convert(&font, &cubic).unwrap();
    assert_eq!(r.result.changed, vec!["O".to_string()]);

    let unknown = CurveOptions { glyphs: vec!["nope".into()], ..cubic };
    assert!(matches!(convert(&font, &unknown), Err(TablesError::BadOptions(_))));
}

fn square(x: i32, y: i32) -> Raw {
    vec![(x, y, 1), (x, y + 100, 1), (x + 100, y + 100, 1), (x + 100, y, 1)]
}

fn dots_font() -> SfntFont {
    let mut colon = square(0, 0);
    colon.extend(square(0, 300));
    let mut odd = square(0, 0);
    odd.extend(square(0, 301).into_iter().take(3));
    odd.push((101, 301, 1));
    font_with(vec![
        (".notdef", Vec::new(), 500),
        ("period", glyph(&[3], &square(0, 0)), 300),
        ("colon", glyph(&[3, 7], &colon), 300),
        ("bar", glyph(&[3], &[(0, 0, 1), (0, 700, 1), (80, 700, 1), (80, 0, 1)]), 300),
        ("odd", glyph(&[3, 7], &odd), 400),
    ])
}

fn comps_of(font: &SfntFont, gid: usize) -> Vec<(u16, u16, Anchor)> {
    let w = Work::read(font).unwrap();
    match &w.glyphs[gid].parsed {
        Parsed::Composite { comps, .. } => comps.iter().map(|c| (c.gid, c.flags & (ROUND_XY | USE_MY_METRICS), c.anchor)).collect(),
        other => panic!("glyph {gid} is not a composite: {other:?}"),
    }
}

#[test]
fn componentize_finds_glyphs_made_of_other_glyphs() {
    let font = dots_font();
    let r = componentize(&font, &ComponentizeOptions::default()).unwrap();
    assert_eq!(r.result.changed, vec!["colon".to_string()]);
    assert_eq!(
        comps_of(&r.result.font, 2),
        vec![(1, ROUND_XY | USE_MY_METRICS, Anchor::Offset(0.0, 0.0)), (1, ROUND_XY, Anchor::Offset(0.0, 300.0))]
    );
    // The composite draws the same contours, and its bounds follow them.
    let w = Work::read(&r.result.font).unwrap();
    assert_eq!(w.draw(2).unwrap().len(), 2);
    assert_eq!(bbox_in_header(&w.glyphs[2].bytes), Some([0, 0, 100, 400]));
    assert!(componentize(&font, &ComponentizeOptions { allow_transforms: true, ..Default::default() }).is_err());
}

#[test]
fn componentize_from_a_reference() {
    let font = dots_font();
    let comp = |glyph: &str, dx: f64, dy: f64| ReferenceComponent { glyph: glyph.into(), transform: [1.0, 0.0, 0.0, 1.0, dx, dy] };
    let mut reference = BTreeMap::new();
    reference.insert(
        "colon".to_string(),
        ReferenceGlyph { components: vec![comp("period", 0.0, 0.0), comp("period", 0.0, 300.0)], has_contours: false, same_advance_as_first: true },
    );
    // One unit off: the components would not give the glyph's points.
    reference.insert(
        "odd".to_string(),
        ReferenceGlyph { components: vec![comp("period", 0.0, 0.0), comp("period", 0.0, 300.0)], has_contours: false, same_advance_as_first: false },
    );
    reference.insert(
        "bar".to_string(),
        ReferenceGlyph {
            components: vec![ReferenceComponent { glyph: "period".into(), transform: [0.8, 0.0, 0.0, 7.0, 0.0, 0.0] }],
            has_contours: false,
            same_advance_as_first: false,
        },
    );
    let o = ComponentizeOptions { reference: Some(reference.clone()), ..Default::default() };
    let r = componentize(&font, &o).unwrap();
    assert_eq!(r.result.changed, vec!["colon".to_string()]);
    assert_eq!(
        comps_of(&r.result.font, 2),
        vec![(1, ROUND_XY | USE_MY_METRICS, Anchor::Offset(0.0, 0.0)), (1, ROUND_XY, Anchor::Offset(0.0, 300.0))]
    );
    let skipped: Vec<(&str, &str)> = r.skipped.iter().map(|s| (s.glyph.as_str(), s.reason.as_str())).collect();
    assert_eq!(skipped, vec![("bar", "a component is scaled, rotated or flipped"), ("odd", "its components do not give the glyph's outline")]);
    // A scale above 2 cannot be written in a glyph either, so allowing transforms still skips it.
    let o = ComponentizeOptions { reference: Some(reference), allow_transforms: true, ..Default::default() };
    let r = componentize(&font, &o).unwrap();
    assert_eq!(r.skipped.len(), 2);
}

#[test]
fn components_that_lead_back_are_refused() {
    let font = font_with(vec![(".notdef", Vec::new(), 500), ("a", glyph(&[3], &square(0, 0)), 300), ("b", glyph(&[3], &square(0, 0)), 300)]);
    let one = |glyph: &str| ReferenceGlyph {
        components: vec![ReferenceComponent { glyph: glyph.into(), transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0] }],
        has_contours: false,
        same_advance_as_first: true,
    };
    let reference: BTreeMap<String, ReferenceGlyph> = [("a".to_string(), one("b")), ("b".to_string(), one("a"))].into_iter().collect();
    let r = componentize(&font, &ComponentizeOptions { reference: Some(reference), ..Default::default() }).unwrap();
    assert_eq!(r.result.changed, vec!["a".to_string()]);
    assert_eq!(r.skipped.len(), 1);
    assert_eq!(r.skipped[0].glyph, "b");
    // Matching contours, the later of two equal glyphs refers to the earlier.
    let r = componentize(&font, &ComponentizeOptions::default()).unwrap();
    assert_eq!(r.result.changed, vec!["b".to_string()]);
}
