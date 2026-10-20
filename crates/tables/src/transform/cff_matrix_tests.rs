//! A matrix on CFF and CFF2 fonts built here. The expected points, stem edges and Private DICT
//! values follow rotatefont's formulas (`rotateFont.cpp`): each absolute point `RND(a·x + c·y + e)`
//! and `RND(b·x + d·y + f)`, each kept edge `offset + RND(scale · edge)`, rounded halves up.

use super::{CharstringMemory, Item, MatrixJob, Parser, Pt, apply_matrix, apply_matrix_within};
use crate::TablesError;
use crate::dehint::cff::Table;
use crate::transform::cff::numbers;
use crate::transform::cff::tests::{CFF_GLYPHS, cff_font, cff_table, cff2_font, cs, font, hmtx};
use crate::transform::{TransformOptions, transform, transform_with_report};

fn fmt(v: &[f64]) -> String {
    let mut v: Vec<f64> = v.iter().map(|x| x + 0.0).collect();
    while v.len() > 1 && v.last() == Some(&0.0) {
        v.pop();
    }
    v.iter().map(f64::to_string).collect::<Vec<_>>().join("/")
}

fn pt(p: &Pt) -> String {
    format!("({},{})", fmt(&p[0]), fmt(&p[1]))
}

/// Each glyph of a written table as words: `w` and the absolute width (CFF), each stem
/// (`h`/`v`, its edges), then the path with absolute points, deltas after slashes.
fn read_back(table: &[u8], cff2: bool) -> Vec<String> {
    let t = Table::read(table, cff2).unwrap();
    let mut out = Vec::new();
    for (gid, code) in t.charstrings.iter().enumerate() {
        let fd = t.fd_select.as_ref().map_or(0, |s| s[gid] as usize);
        let p = &t.privates[fd];
        let get = |op: u16| p.entries.iter().find(|e| e.op == op).map_or(0.0, |e| numbers(&e.raw).unwrap().unwrap()[0]);
        let mut memory = CharstringMemory::new(1 << 24);
        let mut parser = Parser::new(cff2, &t.regions, p.vsindex, get(21), &mut memory);
        parser.run(code).unwrap();
        let g = parser.out;
        let mut words = Vec::new();
        if !cff2 {
            words.push(format!("w{}", g.width.unwrap_or(get(20))));
        }
        for s in &g.stems {
            words.push(format!("{}{}:{}", if s.vertical { "v" } else { "h" }, fmt(&s.edges[0]), fmt(&s.edges[1])));
        }
        for i in &g.items {
            words.push(match i {
                Item::Move(p) => format!("M{}", pt(p)),
                Item::Line(p) => format!("L{}", pt(p)),
                Item::Curve(c) => format!("C{}", c.iter().map(pt).collect::<String>()),
                Item::Flex(f, d) => format!("F{}d{}", f.iter().map(pt).collect::<String>(), fmt(d)),
                Item::Mask(op, b) => format!("{}{}", if *op == 19 { "H" } else { "K" }, b.iter().map(|x| format!("{x:02x}")).collect::<String>()),
            });
        }
        out.push(words.join(" "));
    }
    out
}

fn private_value(table: &[u8], cff2: bool, op: u16) -> Option<Vec<f64>> {
    let t = Table::read(table, cff2).unwrap();
    t.privates[0].entries.iter().find(|e| e.op == op).map(|e| numbers(&e.raw).unwrap().unwrap())
}

fn matrix(m: [f64; 6]) -> TransformOptions {
    TransformOptions { matrix: Some(m), ..TransformOptions::default() }
}

#[test]
fn a_scale_keeps_every_hint_and_moves_the_edges() {
    let (out, report) = transform_with_report(&cff_font(None, 1000), &matrix([1.5, 0.0, 0.0, 1.5, 10.0, 20.0])).unwrap();
    let table = out.get(*b"CFF ").unwrap();
    assert_eq!(
        read_back(table, false),
        vec![
            "w516",
            "w966 h20:170 v87:208 M(27,52) Hc0 L(178,52) L(178,203) L(27,203)",
            "w762 h20:97 v57:118 Hc0 M(12,19) F(28,19)(45,35)(61,35)(78,35)(94,19)(111,19)d50 L(111,170)",
            "w1117 M(87,20) F(118,20)(165,37)(196,37)(243,37)(274,20)(321,20)d50 L(321,4)",
        ]
    );
    assert_eq!(private_value(table, false, 6), Some(vec![4.0, 16.0, 737.0, 15.0]), "blue zones: 1.5·v + 20");
    assert_eq!(private_value(table, false, 11), Some(vec![122.0]), "StdVW: RND(1.5 · 81)");
    assert_eq!(private_value(table, false, 1212), Some(vec![62.0, 33.0]), "StemSnapH, absolute 41 and 63");
    assert_eq!(private_value(table, false, 1211), Some(vec![1.0]), "BlueFuzz stays");
    assert_eq!(private_value(table, false, 19), None, "no subroutines are left");
    assert_eq!(hmtx(&out), vec![(516, 0), (966, 27), (762, 12), (1117, 87)], "widths 1.5·w + 10, side bearings at xMin");
    assert!(report.hints_kept);
    assert_eq!(report.glyphs_changed, 4);
}

#[test]
fn a_quarter_turn_swaps_the_stem_directions() {
    let (out, report) =
        transform_with_report(&cff_font(None, 1000), &TransformOptions { rotate: Some(90.0), ..TransformOptions::default() }).unwrap();
    let table = out.get(*b"CFF ").unwrap();
    let glyphs = read_back(table, false);
    assert_eq!(glyphs[1], "w1000 h-132:-51 v0:100 M(21,-11) Hc0 L(21,-112) L(122,-112) L(122,-11)");
    assert_eq!(glyphs[2], "w1000 h-72:-31 v0:51 Hc0 M(-1,-1) F(-1,-12)(10,-23)(10,-34)(10,-45)(-1,-56)(-1,-67)d50 L(100,-67)");
    assert_eq!(private_value(table, false, 6), Some(vec![-250.0, 0.0, 1350.0, 0.0]), "rotatefont's fixed zones");
    assert_eq!(private_value(table, false, 10), Some(vec![81.0]), "StdHW from StdVW");
    assert_eq!(private_value(table, false, 11), Some(vec![81.0]), "StdVW stays when StdHW is unset");
    assert_eq!(private_value(table, false, 1213), Some(vec![41.0, 22.0]), "StemSnapV from StemSnapH");
    assert_eq!(private_value(table, false, 1212), None);
    assert!(hmtx(&out).iter().all(|(w, _)| *w == 1000), "the auto widths of a turn are the em");
    assert!(report.hints_kept);
}

#[test]
fn a_half_turn_reverses_the_blue_zones() {
    let out = transform(&cff_font(None, 1000), &TransformOptions { rotate: Some(180.0), ..TransformOptions::default() }).unwrap();
    let table = out.get(*b"CFF ").unwrap();
    assert_eq!(read_back(table, false)[1], "w0 h-100:0 v-132:-51 M(-11,-21) Hc0 L(-112,-21) L(-112,-122) L(-11,-122)");
    assert_eq!(private_value(table, false, 6), Some(vec![-501.0, 10.0, 491.0, 11.0]));
    assert_eq!(private_value(table, false, 11), Some(vec![81.0]));
    assert_eq!(hmtx(&out)[1].0, 0, "a negative width is set to 0, in hmtx and in the charstring");
}

#[test]
fn another_angle_drops_every_hint() {
    let (out, report) =
        transform_with_report(&cff_font(None, 1000), &TransformOptions { rotate: Some(10.0), ..TransformOptions::default() }).unwrap();
    let table = out.get(*b"CFF ").unwrap();
    let glyphs = read_back(table, false);
    assert_eq!(glyphs[1], "w1000 M(14,19) L(114,1) L(131,101) L(32,118)");
    assert!(glyphs[2].contains(" C(") && !glyphs[2].contains('F') && !glyphs[2].contains('H'), "flex becomes curves: {}", glyphs[2]);
    assert_eq!(private_value(table, false, 6), Some(vec![-250.0, 0.0, 1350.0, 0.0]));
    for op in [10, 11, 1212, 1213] {
        assert_eq!(private_value(table, false, op), None, "{op} goes");
    }
    assert!(!report.hints_kept);
    assert!(report.warnings.iter().any(|w| w.contains("every stem hint is dropped")));
}

#[test]
fn a_cff2_turn_transforms_the_deltas() {
    let out = transform(&cff2_font(), &TransformOptions { rotate: Some(90.0), ..TransformOptions::default() }).unwrap();
    let table = out.get(*b"CFF2").unwrap();
    let t = Table::read(table, true).unwrap();
    assert_eq!(t.charstrings[1], cs("0 101 50 51 2 blend vstem 21 -11 rmoveto -101 -3 1 blend vlineto 101 hlineto 101 vlineto").as_slice());
    let glyphs = read_back(table, true);
    assert!(glyphs[2].starts_with("h-72/5/-7:-31 v0:51 Hc0 M(-7,-5)"), "{}", glyphs[2]);
    assert_eq!(hmtx(&out), vec![(1000, 0), (1000, 21), (1000, -7)]);
}

#[test]
fn only_the_selected_glyphs_change() {
    let options = TransformOptions { glyphs: Some([1u16].into()), ..matrix([1.0, 0.0, 0.0, 1.0, 100.0, 0.0]) };
    let (out, report) = transform_with_report(&cff_font(None, 1000), &options).unwrap();
    let glyphs = read_back(out.get(*b"CFF ").unwrap(), false);
    assert_eq!(glyphs[1], "w737 h0:100 v151:232 M(111,21) Hc0 L(212,21) L(212,122) L(111,122)");
    assert_eq!(glyphs[3], "w738 M(51,0) F(72,0)(103,11)(124,11)(155,11)(176,0)(207,0)d50 L(207,-11)", "another glyph keeps its outline and width");
    assert_eq!(report.glyphs_changed, 1);
    assert_eq!(hmtx(&out)[1], (737, 111));
}

#[test]
fn hostile_matrices_and_tables_are_refused() {
    let input = cff_font(None, 1000);
    assert!(matches!(transform(&input, &matrix([1000.0, 0.0, 0.0, 1000.0, 0.0, 0.0])), Err(TablesError::Invalid(_))), "points past ±32,767");
    for m in [[f64::NAN, 0.0, 0.0, 1.0, 0.0, 0.0], [1.0, 0.0, 0.0, 1.0, f64::INFINITY, 0.0], [1.0e7, 0.0, 0.0, 1.0, 0.0, 0.0]] {
        assert!(matches!(transform(&input, &matrix(m)), Err(TablesError::BadOptions(_))), "{m:?}");
    }

    let table = cff_table(&CFF_GLYPHS, None);
    let m = [2.0, 0.0, 0.0, 2.0, 0.0, 0.0];
    let glyphs = [Some(m); 4];
    let widths = [None; 4];
    let job = MatrixJob { matrix: m, glyphs: &glyphs, widths: &widths, em_factor: None };
    assert!(apply_matrix(&table, false, &job).is_ok());
    assert!(matches!(apply_matrix_within(&table, false, &job, 64), Err(TablesError::TooLarge { .. })), "the memory budget holds");
    let nan = [Some([f64::NAN, 0.0, 0.0, 1.0, 0.0, 0.0]); 4];
    assert!(matches!(apply_matrix(&table, false, &MatrixJob { glyphs: &nan, ..job }), Err(TablesError::BadOptions(_))));
    assert!(apply_matrix(&table, false, &MatrixJob { glyphs: &glyphs[..3], ..job }).is_err(), "a glyph count that differs");
    for cut in (0..table.len()).step_by(7) {
        assert!(apply_matrix(&table[..cut], false, &job).is_err(), "a table cut at {cut} is refused");
    }

    // An accented character built with seac needs its parts to take the same matrix.
    let seac = font(
        *b"CFF ",
        cff_table(&["endchar", "0 0 rmoveto 10 0 rlineto endchar", "0 0 rmoveto 0 10 rlineto endchar", "10 20 65 97 endchar"], None),
        1000,
        &[(500, 0); 4],
    );
    let options = TransformOptions { glyphs: Some([3u16].into()), ..matrix([2.0, 0.0, 0.0, 2.0, 0.0, 0.0]) };
    assert!(matches!(transform(&seac, &options), Err(TablesError::Unsupported(_))));
    let out = transform(&seac, &matrix([0.0, -1.0, 1.0, 0.0, 0.0, 0.0])).unwrap();
    assert!(read_back(out.get(*b"CFF ").unwrap(), false)[3].starts_with("w1000"));
    let t = Table::read(out.get(*b"CFF ").unwrap(), false).unwrap();
    assert!(t.charstrings[3].ends_with(&cs("20 -10 65 97 endchar")), "the accent offset turns with the glyphs");

    for program in
        ["1 2 3 rlineto endchar", "1 2 3 4 5 6 7 rrcurveto endchar", "1 2 3 hvcurveto endchar", "1 2 3 4 5 6 flex endchar", "1 2 3 endchar"]
    {
        let f = font(*b"CFF ", cff_table(&[program; 4], None), 1000, &[(500, 0); 4]);
        assert!(transform(&f, &matrix(m)).is_err(), "{program} is refused");
    }
}

#[test]
fn a_glyph_table_gives_each_listed_glyph_its_shift_and_advance() {
    use crate::transform::{GlyphSettings, IDENTITY};
    let per_glyph = [(0u16, GlyphSettings::default()), (3, GlyphSettings { offset: (5.0, 6.0), advance: Some(900.0) })].into();
    let options = TransformOptions { per_glyph: Some(per_glyph), ..TransformOptions::default() };
    let (out, report) = transform_with_report(&cff_font(None, 1000), &options).unwrap();
    assert_eq!(report.matrix, Some(IDENTITY), "a glyph table alone takes the identity");
    let glyphs = read_back(out.get(*b"CFF ").unwrap(), false);
    assert_eq!(glyphs[3], "w900 M(56,6) F(77,6)(108,17)(129,17)(160,17)(181,6)(212,6)d50 L(212,-5)");
    assert_eq!(glyphs[1], "w637 h0:100 v51:132 M(11,21) Hc0 L(112,21) L(112,122) L(11,122)", "an unlisted glyph stays");
    assert_eq!(hmtx(&out)[3], (900, 56));
    assert_eq!(report.glyphs_changed, 2);
}

#[test]
fn glyph_tables_are_read_with_limits() {
    use crate::transform::{GlyphKey, MAX_GLYPH_TABLE_LINES, parse_glyph_table};
    let entries = parse_glyph_table("# comment\n\n  .notdef .notdef None 0 0\nA A.vert 1000 120.5 -880\n").unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[1].output, GlyphKey::Name("A.vert".into()));
    assert_eq!((entries[1].advance, entries[1].offset), (Some(1000.0), (120.5, -880.0)));
    let cids = parse_glyph_table("0 0 None 0 0\n1200 1200 1000 0 0\n").unwrap();
    assert_eq!(cids[1].source, GlyphKey::Cid(1200));
    for bad in [
        "A A 500 0",
        "A A 500 0 0 extra",
        "A A NaN 0 0",
        "A A 500 inf 0",
        "A A -1 0 0",
        "A A 70000 0 0",
        "A A 500 2e6 0",
        "A A None 0 0\n12 12 None 0 0",
        "A A None 0 0\nA B None 0 0",
        "70000 70000 None 0 0",
    ] {
        assert!(matches!(parse_glyph_table(bad), Err(TablesError::BadOptions(_))), "{bad:?} is refused");
    }
    let long = "# x\n".repeat(MAX_GLYPH_TABLE_LINES + 1);
    assert!(matches!(parse_glyph_table(&long), Err(TablesError::TooLarge { .. })));
}

#[test]
fn font_dict_selectors_narrow_the_transformed_glyphs() {
    let shift = matrix([1.0, 0.0, 0.0, 1.0, 100.0, 0.0]);
    let (_, report) = transform_with_report(&cff2_font(), &TransformOptions { fds: Some([0u16].into()), ..shift.clone() }).unwrap();
    assert_eq!(report.glyphs_changed, 3, "every glyph of Font DICT 0");
    let options = TransformOptions { exclude_fds: Some([0u16].into()), ..shift.clone() };
    let (out, report) = transform_with_report(&cff_font(None, 1000), &options).unwrap();
    assert_eq!(report.glyphs_changed, 0, "the only Font DICT is excluded");
    assert_eq!(hmtx(&out), hmtx(&cff_font(None, 1000)));
    let narrowed = TransformOptions { fds: Some([0u16].into()), glyphs: Some([2u16].into()), ..shift.clone() };
    assert_eq!(transform_with_report(&cff_font(None, 1000), &narrowed).unwrap().1.glyphs_changed, 1, "with a glyph list, both apply");
    assert!(matches!(
        transform(&cff_font(None, 1000), &TransformOptions { fds: Some([1u16].into()), ..shift.clone() }),
        Err(TablesError::BadOptions(_))
    ));
    let mut truetype = cff_font(None, 1000);
    truetype.remove(*b"CFF ");
    assert!(matches!(transform(&truetype, &TransformOptions { fds: Some([0u16].into()), ..shift }), Err(TablesError::BadOptions(_))));
}
