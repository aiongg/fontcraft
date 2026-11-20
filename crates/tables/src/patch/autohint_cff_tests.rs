//! `autohint_cff` on CFF fonts built in code, hinted by the autohinter as an export runs it.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use fontcraft_cff::subr::{TableRewrite, rewrite_table};
use fontcraft_cff::{CharstringHints, Font, Glyph, Number, PathOp, PrivateDict, StemHint};
use fontcraft_compile::cff_hints::{autohint_dict, autohinted};
use read_fonts::ps::cff::CffFontRef;
use read_fonts::ps::cs::CommandSink;
use read_fonts::types::{Fixed, GlyphId};

use super::*;
use crate::sfnt::Flavour;

/// Every command a charstring sends, paths and stems, with its width first.
#[derive(Default)]
struct Record(Vec<String>);

impl CommandSink for Record {
    fn move_to(&mut self, x: Fixed, y: Fixed) {
        self.0.push(format!("M {} {}", x.to_f64(), y.to_f64()));
    }
    fn line_to(&mut self, x: Fixed, y: Fixed) {
        self.0.push(format!("L {} {}", x.to_f64(), y.to_f64()));
    }
    fn curve_to(&mut self, a: Fixed, b: Fixed, c: Fixed, d: Fixed, x: Fixed, y: Fixed) {
        self.0.push(format!("C {} {} {} {} {} {}", a.to_f64(), b.to_f64(), c.to_f64(), d.to_f64(), x.to_f64(), y.to_f64()));
    }
    fn close(&mut self) {}
    fn hstem(&mut self, y: Fixed, dy: Fixed) {
        self.0.push(format!("hstem {} {}", y.to_f64(), dy.to_f64()));
    }
    fn vstem(&mut self, x: Fixed, dx: Fixed) {
        self.0.push(format!("vstem {} {}", x.to_f64(), dx.to_f64()));
    }
}

fn drawn(font: &SfntFont) -> Vec<Vec<String>> {
    let table = font.get(*b"CFF ").unwrap();
    let cff = CffFontRef::new_cff(table, 0, None).unwrap();
    let sub = cff.subfont(0, &[]).unwrap();
    (0..cff.num_glyphs())
        .map(|g| {
            let mut rec = Record::default();
            // The `seac` glyph names components the font lacks, so the reader refuses it.
            match cff.evaluate_charstring(&sub, GlyphId::new(g), &[], &mut rec) {
                Ok(width) => rec.0.insert(0, format!("width {width:?}")),
                Err(e) => return vec![format!("error {e:?}")],
            }
            rec.0
        })
        .collect()
}

fn outline(cmds: &[String]) -> Vec<String> {
    cmds.iter().filter(|c| !c.contains("stem")).cloned().collect()
}

fn path_text(path: &[PathOp]) -> Vec<String> {
    path.iter()
        .map(|op| match *op {
            PathOp::MoveTo((x, y)) => format!("M {x} {y}"),
            PathOp::LineTo((x, y)) => format!("L {x} {y}"),
            PathOp::CurveTo((a, b), (c, d), (x, y)) => format!("C {a} {b} {c} {d} {x} {y}"),
        })
        .collect()
}

fn stem(x: f64, y: f64, w: f64, h: f64) -> Vec<PathOp> {
    vec![PathOp::MoveTo((x, y)), PathOp::LineTo((x + w, y)), PathOp::LineTo((x + w, y + h)), PathOp::LineTo((x, y + h))]
}

fn cff_font(glyphs: Vec<Glyph>, private: PrivateDict) -> SfntFont {
    let built = fontcraft_cff::build(&Font { name: "Test".into(), private, glyphs, ..Font::default() }).unwrap();
    let mut font = SfntFont::new(Flavour::Cff);
    font.insert(*b"CFF ", built.table);
    font
}

/// Zones at the baseline and at 700, stems of 80.
fn private() -> PrivateDict {
    PrivateDict {
        blue_values: [-10, 0, 700, 710].map(Number::Int).to_vec(),
        std_hw: Some(Number::Int(80)),
        std_vw: Some(Number::Int(80)),
        stem_snap_h: vec![Number::Int(80)],
        stem_snap_v: vec![Number::Int(80)],
        ..PrivateDict::default()
    }
}

/// `.notdef` (empty), `l` (a stem from x 100 to 180), `hinted` (the same with a stem hint of its
/// own), `seac` (an accented glyph drawn with seac) and `width` (`l` with a width of its own).
fn font() -> SfntFont {
    let l = stem(100.0, 0.0, 80.0, 700.0);
    let glyphs = vec![
        Glyph { name: ".notdef".into(), advance: 500.0, ..Glyph::default() },
        Glyph { name: "l".into(), advance: 280.0, path: l.clone(), ..Glyph::default() },
        Glyph {
            name: "hinted".into(),
            advance: 280.0,
            path: l.clone(),
            hints: Some(CharstringHints { v_stems: vec![StemHint { edge: 100.0, width: 80.0 }], ..CharstringHints::default() }),
        },
        Glyph { name: "seac".into(), advance: 600.0, path: stem(0.0, 0.0, 10.0, 10.0), ..Glyph::default() },
        Glyph { name: "width".into(), advance: 413.0, path: l, ..Glyph::default() },
    ];
    let font = cff_font(glyphs, private());
    // `seac`: `100 200 65 194 endchar`.
    let (table, _) = rewrite_table(font.get(*b"CFF ").unwrap(), false, &TableRewrite::default(), &mut |set| {
        let mut set = set.clone();
        set.charstrings[3] = vec![239, 247, 92, 204, 247, 86, 14];
        Ok(set)
    })
    .unwrap();
    let mut out = font.clone();
    out.insert(*b"CFF ", table);
    out
}

fn run(font: &SfntFont, hint_all: bool) -> CffAutohint {
    let setup = cff_hint_setup(font).unwrap();
    let dict = autohint_dict(&setup.privates[0], "Test", &mut Vec::new()).unwrap();
    let options = CffAutohintOptions { hint_all, round_stems: true };
    autohint_cff(font, &setup, &options, &|name| name != "skipme", &mut |g| {
        Ok(autohinted(g.name, g.path, &dict, &Default::default(), &mut Vec::new()))
    })
    .unwrap()
}

#[test]
fn autohint_cff_hints_unhinted_glyphs_and_keeps_every_outline_and_width() {
    let font = font();
    let setup = cff_hint_setup(&font).unwrap();
    assert_eq!(setup.names, [".notdef", "l", "hinted", "seac", "width"]);
    assert!(!setup.cid);
    assert_eq!(setup.privates[0].stem_snap_v, vec![Number::Int(80)]);

    let before = drawn(&font);
    let r = run(&font, false);
    let hinted: Vec<(&str, usize)> = r.hinted.iter().map(|h| (h.glyph.as_str(), h.vertical)).collect();
    assert_eq!(hinted, [("l", 1), ("width", 1)], "{:?}", r.skipped);
    let reason = |glyph: &str| r.skipped.iter().find(|s| s.glyph == glyph).map(|s| s.reason.clone()).unwrap_or_default();
    assert!(reason(".notdef").contains("draws nothing"), "{:?}", r.skipped);
    assert!(reason("hinted").contains("already has hints"), "{:?}", r.skipped);
    assert!(reason("seac").contains("seac"), "{:?}", r.skipped);

    let after = drawn(&r.font);
    for (g, (a, b)) in before.iter().zip(&after).enumerate() {
        assert_eq!(outline(a), outline(b), "glyph {g} draws as it did, with its width");
    }
    assert!(after[1].iter().any(|c| c == "vstem 100 180"), "{:?}", after[1]);
    assert!(after[4].iter().any(|c| c == "vstem 100 180"), "{:?}", after[4]);
    assert_eq!(after[2], before[2], "a hinted glyph keeps its hints");
    let charstrings = |f: &SfntFont| subr::charstrings_of_table(f.get(*b"CFF ").unwrap(), false).unwrap().charstrings;
    assert_eq!(charstrings(&r.font)[3], charstrings(&font)[3], "the seac glyph keeps its charstring");

    // Hinting the result again changes nothing unless hintAll.
    let again = run(&r.font, false);
    assert!(again.hinted.is_empty());
    assert_eq!(again.font.get(*b"CFF "), r.font.get(*b"CFF "));
    let all = run(&r.font, true);
    let names: Vec<&str> = all.hinted.iter().map(|h| h.glyph.as_str()).collect();
    assert_eq!(names, ["l", "hinted", "width"]);
}

#[test]
fn decode_reads_every_operator_form_as_the_reference_reader_draws_it() {
    // Curves and lines in every direction, so that the specialiser uses hlineto, vlineto,
    // hvcurveto, vhcurveto, hhcurveto, vvcurveto, rcurveline and rlinecurve; flex on `flex`.
    let curvy = vec![
        PathOp::MoveTo((50.0, 0.0)),
        PathOp::LineTo((300.0, 0.0)),
        PathOp::LineTo((300.0, 200.0)),
        PathOp::CurveTo((400.0, 200.0), (500.0, 300.0), (500.0, 400.0)),
        PathOp::CurveTo((500.0, 500.0), (400.0, 600.0), (300.0, 600.0)),
        PathOp::CurveTo((250.0, 600.0), (200.0, 600.0), (150.0, 600.0)),
        PathOp::CurveTo((150.0, 550.0), (150.0, 500.0), (150.0, 450.0)),
        PathOp::CurveTo((120.0, 440.0), (90.0, 420.0), (70.0, 380.0)),
        PathOp::LineTo((60.0, 300.0)),
        PathOp::LineTo((55.0, 200.0)),
        PathOp::CurveTo((52.0, 120.0), (51.0, 60.0), (50.0, 30.0)),
        PathOp::MoveTo((600.0, 0.0)),
        PathOp::LineTo((700.0, 0.0)),
        PathOp::LineTo((700.0, 100.0)),
    ];
    let flex_path = vec![
        PathOp::MoveTo((0.0, 0.0)),
        PathOp::LineTo((1000.0, 0.0)),
        PathOp::LineTo((1000.0, 500.0)),
        PathOp::CurveTo((800.0, 500.0), (600.0, 500.0), (500.0, 505.0)),
        PathOp::CurveTo((400.0, 500.0), (200.0, 500.0), (0.0, 500.0)),
    ];
    let flex_hints = CharstringHints { flex: vec![3], ..CharstringHints::default() };
    let glyphs = vec![
        Glyph { name: ".notdef".into(), advance: 500.0, ..Glyph::default() },
        Glyph { name: "curvy".into(), advance: 800.0, path: curvy, ..Glyph::default() },
        Glyph { name: "flex".into(), advance: 1000.0, path: flex_path, hints: Some(flex_hints) },
    ];
    let font = cff_font(glyphs, private());
    let reference = drawn(&font);
    let set = subr::charstrings_of_table(font.get(*b"CFF ").unwrap(), false).unwrap();
    let flat = subr::desubroutinize_within(&set, MAX_CHARSTRING_MEMORY).unwrap();
    for (g, code) in flat.charstrings.iter().enumerate() {
        let decoded = decode(code).unwrap();
        let expected: Vec<String> = outline(&reference[g]).into_iter().skip(1).collect();
        assert_eq!(path_text(&decoded.path), expected, "glyph {g}");
        // The encoder writes the outline back as it was.
        let again = decode(&encode_charstring("g", &decoded.path, None, decoded.width).unwrap()).unwrap();
        assert_eq!((again.path, again.width), (decoded.path, decoded.width), "glyph {g}");
    }
}

#[test]
fn autohint_cff_refuses_a_cff2_table() {
    let mut font = SfntFont::new(Flavour::Cff);
    font.insert(*b"CFF2", vec![2, 0, 5, 0, 0]);
    assert!(matches!(cff_hint_setup(&font), Err(TablesError::Unsupported(_))));
    let mut font = SfntFont::new(Flavour::Cff);
    font.insert(*b"glyf", Vec::new());
    assert!(matches!(cff_hint_setup(&font), Err(TablesError::MissingTable(_))));
}
