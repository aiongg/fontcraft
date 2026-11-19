use super::*;
use crate::{Font, Glyph, Number, PathOp, PrivateDict, TopDict, build};

fn square(advance: f64) -> Glyph {
    Glyph {
        name: "square".into(),
        advance,
        path: vec![
            PathOp::MoveTo((100.0, 0.0)),
            PathOp::LineTo((400.0, 0.0)),
            PathOp::LineTo((400.0, 300.0)),
            PathOp::CurveTo((400.0, 400.0), (300.0, 500.0), (100.0, 500.0)),
            PathOp::LineTo((100.0, 0.0)),
        ],
        hints: None,
    }
}

fn font() -> Font {
    let blues = vec![Number::Int(-10), Number::Int(0), Number::Int(500), Number::Int(510)];
    Font {
        name: "Switches".into(),
        top: TopDict { notice: Some("A notice".into()), full_name: Some("Switches Regular".into()), ..TopDict::default() },
        private: PrivateDict { blue_values: blues.clone(), family_blues: blues, ..PrivateDict::default() },
        glyphs: vec![
            Glyph { name: ".notdef".into(), advance: 500.0, ..Glyph::default() },
            square(600.0),
            Glyph { name: "space".into(), advance: 250.0, ..Glyph::default() },
        ],
        ..Font::default()
    }
}

fn operators(code: &[u8]) -> Vec<u16> {
    tokens(code).unwrap().into_iter().filter_map(|t| if let Tok::Op(op, _) = t { Some(op) } else { None }).collect()
}

#[test]
fn general_charstrings_use_only_the_generic_operators_and_specialise_back() {
    let built = build(&font()).unwrap().table;
    let general = apply(&built, false, &CffWriteOptions { charstrings: Some(CharstringForm::General), ..CffWriteOptions::none() }).unwrap();
    assert!(general.expanded);
    let set = charstrings_of_table(&general.bytes, false).unwrap();
    let ops = operators(&set.charstrings[1]);
    assert!(ops.iter().all(|op| matches!(op, 21 | 5 | 8 | 14)), "{ops:?}");
    let original = operators(&charstrings_of_table(&built, false).unwrap().charstrings[1]);
    assert!(original.iter().any(|op| matches!(op, 6 | 7 | 30 | 31)), "the writer specialises: {original:?}");
    let back = apply(&general.bytes, false, &CffWriteOptions { charstrings: Some(CharstringForm::Specialize), ..CffWriteOptions::none() }).unwrap();
    assert_eq!(charstrings_of_table(&back.bytes, false).unwrap().charstrings, charstrings_of_table(&built, false).unwrap().charstrings);
}

#[test]
fn dict_switches_remove_what_the_reference_documents() {
    let built = build(&font()).unwrap().table;
    let untouched = apply(&built, false, &CffWriteOptions::none()).unwrap();
    assert_eq!(untouched.bytes, built, "no switch changes no byte");
    let defaults = apply(&built, false, &CffWriteOptions::default()).unwrap();
    assert_eq!(defaults.removed, [8], "FamilyBlues equal to BlueValues goes by default");
    let embed = apply(&built, false, &CffWriteOptions { embed_minimal: true, dedupe_family_blues: false, ..CffWriteOptions::none() }).unwrap();
    assert!(embed.removed.contains(&1) && embed.removed.contains(&2), "Notice and FullName: {:?}", embed.removed);
    assert_eq!(charstrings_of_table(&embed.bytes, false).unwrap().charstrings, charstrings_of_table(&built, false).unwrap().charstrings);
}

#[test]
fn hint_warnings_name_glyphs_that_draw_without_stems() {
    let built = build(&font()).unwrap().table;
    let report = apply(&built, false, &CffWriteOptions { warn_hints: true, ..CffWriteOptions::none() }).unwrap();
    assert_eq!(report.warnings, ["glyphs that draw without stem hints: 1 (glyph ids 1)"], "an empty glyph draws nothing");
    assert_eq!(report.bytes, built);
}

#[test]
fn dotsection_goes_and_the_width_stays_first() {
    // width 50, 10 20 rmoveto, dotsection, 30 hlineto, endchar
    let code = [189, 149, 159, 21, 12, 0, 169, 6, 14];
    let out = rewrite_charstring(&code, false, Some(CharstringForm::General), true, CFF_STACK).unwrap();
    assert_eq!(out, [189, 149, 159, 21, 169, 139, 5, 14]);
}

#[test]
fn a_width_moves_between_width_pairs_without_changing_the_advance() {
    // width 50 relative to nominal 500: advance 550.
    let code = [189, 149, 159, 21, 14];
    assert_eq!(reencode_width(&code, (0.0, 500.0), (550.0, 0.0)).unwrap(), [149, 159, 21, 14], "the default width needs no operand");
    let moved = reencode_width(&code, (0.0, 500.0), (0.0, 100.0)).unwrap();
    assert_eq!(read_t2_number(&moved, 0).map(|(v, _)| v), Some(450.0));
    // No width: the advance is the old default.
    assert_eq!(read_t2_number(&reencode_width(&[149, 159, 21, 14], (300.0, 0.0), (0.0, 0.0)).unwrap(), 0).map(|(v, _)| v), Some(300.0));
}
