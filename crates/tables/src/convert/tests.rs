//! Conversions between outline flavours on fonts built in code. The expected TrueType points of
//! `cff_to_truetype_matches_otf2ttf` come from AFDKO 5.0.1's otf2ttf on the same outline built
//! with fontTools' FontBuilder and `T2CharStringPen`.

use super::outline::{TtContour, encode_simple};
use super::*;
use fontcraft_cff::PathOp;

fn be16(v: u16) -> [u8; 2] {
    v.to_be_bytes()
}

/// `head`, `hhea`, `maxp` and `hmtx` for `count` glyphs with these advances.
fn common(font: &mut SfntFont, advances: &[u16], maxp_version: u32) {
    let count = advances.len() as u16;
    let mut head = vec![0u8; 54];
    head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    head[4..8].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    head[18..20].copy_from_slice(&be16(1000));
    let mut hhea = vec![0u8; 36];
    hhea[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    hhea[34..36].copy_from_slice(&be16(count));
    let mut maxp = vec![0u8; if maxp_version == 0x5000 { 6 } else { 32 }];
    maxp[0..4].copy_from_slice(&maxp_version.to_be_bytes());
    maxp[4..6].copy_from_slice(&be16(count));
    let mut hmtx = Vec::new();
    for a in advances {
        hmtx.extend_from_slice(&be16(*a));
        hmtx.extend_from_slice(&be16(0));
    }
    font.insert(*b"head", head);
    font.insert(*b"hhea", hhea);
    font.insert(*b"maxp", maxp);
    font.insert(*b"hmtx", hmtx);
}

fn post_v2(names: &[&str]) -> Vec<u8> {
    let (post, _) = post_table(None, PostFormat::Names, &names.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap();
    post
}

/// A composite of one component moved by `(dx, dy)`.
fn composite(gid: u16, dx: i16, dy: i16, bbox: [i16; 4]) -> Vec<u8> {
    let mut g = Vec::new();
    g.extend_from_slice(&(-1i16).to_be_bytes());
    for v in bbox {
        g.extend_from_slice(&v.to_be_bytes());
    }
    g.extend_from_slice(&be16(0x0003)); // ARG_1_AND_2_ARE_WORDS | ARGS_ARE_XY_VALUES
    g.extend_from_slice(&be16(gid));
    g.extend_from_slice(&dx.to_be_bytes());
    g.extend_from_slice(&dy.to_be_bytes());
    g
}

const A: [(i32, i32, bool); 5] = [(0, 0, true), (0, 700, true), (300, 900, false), (600, 700, true), (600, 0, true)];

/// `.notdef` (empty), `A` (a clockwise arch) and `B` (A moved right by 50, as a component).
fn truetype_font() -> SfntFont {
    let mut font = SfntFont::new(Flavour::TrueType);
    common(&mut font, &[500, 600, 650], 0x0001_0000);
    let (a, _) = encode_simple(&[A.to_vec()]).unwrap();
    let glyphs = [Vec::new(), a, composite(1, 50, 0, [50, 0, 650, 900])];
    let mut glyf = Vec::new();
    let mut loca = Vec::new();
    for g in &glyphs {
        loca.extend_from_slice(&(glyf.len() as u32).to_be_bytes());
        glyf.extend_from_slice(g);
        if glyf.len() % 2 == 1 {
            glyf.push(0);
        }
    }
    loca.extend_from_slice(&(glyf.len() as u32).to_be_bytes());
    let mut head = font.get(*b"head").unwrap().to_vec();
    head[50..52].copy_from_slice(&be16(1));
    font.insert(*b"head", head);
    font.insert(*b"glyf", glyf);
    font.insert(*b"loca", loca);
    font.insert(*b"post", post_v2(&[".notdef", "A", "B"]));
    font.insert(*b"fpgm", vec![0xB0, 0x00]);
    font
}

/// A CFF-flavoured font of `.notdef` (empty) and `O` (the oracle's outline).
fn cff_font() -> SfntFont {
    let mut font = SfntFont::new(Flavour::Cff);
    common(&mut font, &[500, 500], 0x5000);
    let o = vec![
        PathOp::MoveTo((100.0, 0.0)),
        PathOp::CurveTo((300.0, -10.0), (400.0, 150.0), (400.0, 350.0)),
        PathOp::CurveTo((400.0, 550.0), (300.0, 700.0), (100.0, 700.0)),
        PathOp::LineTo((100.0, 600.0)),
        PathOp::CurveTo((250.0, 600.0), (300.0, 500.0), (300.0, 350.0)),
        PathOp::CurveTo((300.0, 200.0), (250.0, 100.0), (100.0, 100.0)),
    ];
    let cff = fontcraft_cff::Font {
        name: "Test-Regular".into(),
        glyphs: vec![
            fontcraft_cff::Glyph { name: ".notdef".into(), advance: 500.0, path: Vec::new(), hints: None },
            fontcraft_cff::Glyph { name: "O".into(), advance: 500.0, path: o, hints: None },
        ],
        ..fontcraft_cff::Font::default()
    };
    font.insert(*b"CFF ", fontcraft_cff::build(&cff).unwrap().table);
    let (post, _) = post_table(None, PostFormat::NoNames, &[]).unwrap();
    font.insert(*b"post", post);
    font
}

fn glyf_points(font: &SfntFont, gid: usize) -> Vec<TtContour> {
    let long = u16_at(font.get(*b"head").unwrap(), 50).unwrap() != 0;
    let n = num_glyphs(font).unwrap();
    let outlines = GlyfOutlines::new(font.get(*b"glyf").unwrap(), font.get(*b"loca").unwrap(), long, n).unwrap();
    let mut instructions = false;
    let contours = outlines.contours(gid, &mut instructions).unwrap();
    outline::truetype_points(&contours).unwrap()
}

fn ps_contours(font: &SfntFont, tag: Tag, gid: usize) -> Vec<Contour> {
    let mut ps = PsOutlines::new(font.get(tag).unwrap(), 1000).unwrap();
    ps.contours(gid).unwrap()
}

/// Twice the signed area of a contour's on-curve polygon: positive counter-clockwise.
fn area(points: &[(i32, i32, bool)]) -> i64 {
    let on: Vec<_> = points.iter().filter(|p| p.2).collect();
    (0..on.len())
        .map(|i| {
            let (a, b) = (on[i], on[(i + 1) % on.len()]);
            i64::from(a.0) * i64::from(b.1) - i64::from(b.0) * i64::from(a.1)
        })
        .sum()
}

#[test]
fn truetype_to_cff_and_back_keeps_the_points() {
    let ttf = truetype_font();
    let mut options = ConvertOptions::new(ConvertTarget::Cff);
    options.quad_to_cubic = QuadToCubic::Exact;
    options.subroutinize = Some(false);
    let otf = convert_with(&ttf, &options).unwrap();
    let font = otf.font.clone().unwrap();
    assert_eq!(font.flavour, Flavour::Cff);
    for tag in [*b"glyf", *b"loca", *b"fpgm"] {
        assert!(font.get(tag).is_none(), "{tag:?} is removed");
    }
    assert_eq!(font.get(*b"maxp").unwrap().len(), 6, "maxp 0.5");
    assert_eq!(u32_at(font.get(*b"post").unwrap(), 0), Some(0x0003_0000));
    assert!(otf.left_behind.iter().any(|l| l.contains("instructions")));
    // B is A decomposed and moved; the contour runs counter-clockwise now.
    let a = ps_contours(&font, CFF, 1);
    let b = ps_contours(&font, CFF, 2);
    assert_eq!(a.len(), 1);
    assert_eq!(b.len(), 1);
    assert_eq!(b[0].start, (a[0].start.0 + 50.0, a[0].start.1));
    let quadratic: Vec<Contour> = a.iter().map(|c| outline::to_quadratic(c, 1.0).unwrap()).collect();
    assert!(area(&outline::truetype_points(&quadratic).unwrap()[0]) > 0, "cubic outer contours run counter-clockwise");
    // The left side bearing of B is its xMin.
    let hmtx = font.get(*b"hmtx").unwrap();
    assert_eq!(u16_at(hmtx, 10), Some(50));

    let back = convert_with(&font, &ConvertOptions::new(ConvertTarget::TrueType { max_error: 1.0 })).unwrap().font.unwrap();
    assert_eq!(back.flavour, Flavour::TrueType);
    assert!(back.get(*b"CFF ").is_none());
    assert_eq!(glyf_points(&back, 1), vec![A.to_vec()], "a raised quadratic curve comes back as itself");
    assert_eq!(u32_at(back.get(*b"post").unwrap(), 0), Some(0x0002_0000));
    let names = post_names(&back, 3).unwrap();
    assert_eq!(names, vec![".notdef", "A", "B"]);
}

#[test]
fn cff_to_truetype_matches_otf2ttf() {
    let expected: Vec<(i32, i32, bool)> = [
        (100, 0, 1),
        (100, 100, 1),
        (175, 100, 0),
        (262, 163, 0),
        (300, 275, 0),
        (300, 350, 1),
        (300, 425, 0),
        (262, 538, 0),
        (175, 600, 0),
        (100, 600, 1),
        (100, 700, 1),
        (200, 700, 0),
        (333, 608, 0),
        (400, 450, 0),
        (400, 350, 1),
        (400, 275, 0),
        (363, 145, 0),
        (288, 48, 0),
        (175, -4, 0),
    ]
    .iter()
    .map(|&(x, y, on)| (x, y, on == 1))
    .collect();
    let converted = convert_with(&cff_font(), &ConvertOptions::new(ConvertTarget::TrueType { max_error: 1.0 })).unwrap();
    let ttf = converted.font.unwrap();
    let got = glyf_points(&ttf, 1);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].len(), expected.len(), "{got:?}");
    for (g, e) in got[0].iter().zip(&expected) {
        assert_eq!(g.2, e.2);
        assert!((g.0 - e.0).abs() <= 1 && (g.1 - e.1).abs() <= 1, "{g:?} vs {e:?}");
    }
    let maxp = ttf.get(*b"maxp").unwrap();
    assert_eq!((u16_at(maxp, 6), u16_at(maxp, 8)), (Some(19), Some(1)));
    assert_eq!(u16_at(ttf.get(*b"hmtx").unwrap(), 6), Some(100), "lsb is xMin");
    assert_eq!(post_names(&ttf, 2).unwrap(), vec![".notdef", "O"]);
}

#[test]
fn cff_to_cff2_and_back_draws_the_same_outlines() {
    let otf = cff_font();
    let mut to2 = ConvertOptions::new(ConvertTarget::Cff2);
    to2.subroutinize = Some(false);
    let cff2 = convert_with(&otf, &to2).unwrap().font.unwrap();
    assert!(cff2.get(*b"CFF ").is_none());
    assert_eq!(cff2.get(*b"CFF2").unwrap().first(), Some(&2));
    assert_eq!(post_names(&cff2, 2).unwrap(), vec![".notdef", "O"], "the names move to post");
    assert_eq!(ps_contours(&cff2, CFF2, 1), ps_contours(&otf, CFF, 1));

    // The names in post make the CFF result name-keyed.
    let back = convert_with(&cff2, &ConvertOptions::new(ConvertTarget::Cff)).unwrap().font.unwrap();
    assert_eq!(ps_contours(&back, CFF, 1), ps_contours(&otf, CFF, 1));
    let ps = PsOutlines::new(back.get(*b"CFF ").unwrap(), 1000).unwrap();
    assert!(!ps.is_cid());
    assert_eq!(ps.name(1).as_deref(), Some("O"));
    assert_eq!(u32_at(back.get(*b"post").unwrap(), 0), Some(0x0003_0000));

    // Without names the result is CID-keyed, as the reference writes it.
    let mut unnamed = cff2.clone();
    unnamed.insert(*b"post", post_table(None, PostFormat::NoNames, &[]).unwrap().0);
    let converted = convert_with(&unnamed, &ConvertOptions::new(ConvertTarget::Cff)).unwrap();
    let back = converted.font.unwrap();
    let ps = PsOutlines::new(back.get(*b"CFF ").unwrap(), 1000).unwrap();
    assert!(ps.is_cid());
    assert_eq!(ps.name(1).as_deref(), Some("cid00001"));
    assert_eq!(ps_contours(&back, CFF, 1), ps_contours(&otf, CFF, 1));
    assert!(converted.left_behind.iter().any(|l| l.contains("CID-keyed")));
}

#[test]
fn bare_tables_and_refusals() {
    let bare = convert_with(&truetype_font(), &ConvertOptions::new(ConvertTarget::BareCff)).unwrap();
    assert!(bare.font.is_none());
    assert_eq!(bare.bytes.get(..3), Some([1u8, 0, 4].as_slice()));
    let bare2 = convert(&cff_font(), ConvertTarget::BareCff2).unwrap();
    assert_eq!(bare2.first(), Some(&2));

    let mut variable = truetype_font();
    variable.insert(*b"gvar", vec![0; 20]);
    assert!(matches!(convert(&variable, ConvertTarget::Cff), Err(TablesError::Unsupported(_))));
    assert!(matches!(convert(&truetype_font(), ConvertTarget::TrueType { max_error: 0.0 }), Err(TablesError::BadOptions(_))));
    assert!(matches!(convert(&cff_font(), ConvertTarget::TrueType { max_error: f64::NAN }), Err(TablesError::BadOptions(_))));

    // A composite that uses itself fails with a message, never a crash.
    let mut cyclic = truetype_font();
    let mut glyf = cyclic.get(*b"glyf").unwrap().to_vec();
    // B's component glyph id is its last four bytes but two.
    let at = glyf.len() - 6;
    glyf[at..at + 2].copy_from_slice(&be16(2));
    cyclic.insert(*b"glyf", glyf);
    assert!(matches!(convert(&cyclic, ConvertTarget::Cff), Err(TablesError::Invalid(_))));
    assert!(matches!(convert(&SfntFont::new(Flavour::TrueType), ConvertTarget::Cff), Err(TablesError::MissingTable(_))));
}
