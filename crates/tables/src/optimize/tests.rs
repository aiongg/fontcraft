use read_fonts::ps::cff::CffFontRef;
use read_fonts::ps::cs::CommandSink;
use read_fonts::types::{Fixed, GlyphId};

use super::*;
use crate::Flavour;
use fontcraft_cff::{Font, Glyph, PathOp, Widths};

const GLYPHS: usize = 40;

/// A shape with enough curves that its charstring repeats across glyphs.
fn shape(dx: f64) -> Vec<PathOp> {
    let mut p = vec![PathOp::MoveTo((50.0 + dx, 0.0))];
    for k in 0..6 {
        let y = 100.0 * f64::from(k);
        p.push(PathOp::CurveTo((80.0 + dx, y + 20.0), (90.0 + dx, y + 60.0), (50.0 + dx, y + 100.0)));
    }
    p.push(PathOp::LineTo((20.0 + dx, 600.0)));
    p.push(PathOp::LineTo((20.0 + dx, 0.0)));
    p
}

/// The `CFF ` table of [`GLYPHS`] glyphs that share one shape, with every width written out.
fn cff_table() -> Vec<u8> {
    let glyphs = (0..GLYPHS)
        .map(|i| Glyph {
            name: if i == 0 { ".notdef".into() } else { format!("g{i}") },
            advance: if i % 5 == 0 { 520.0 } else { 600.0 },
            path: if i == 0 { Vec::new() } else { shape((i % 3) as f64) },
            hints: None,
        })
        .collect();
    let font = Font { name: "OptimizeTest".into(), glyphs, widths: Widths::Fixed { default_width_x: 0, nominal_width_x: 0 }, ..Font::default() };
    fontcraft_cff::build(&font).unwrap().table
}

fn otf(cff: Vec<u8>) -> SfntFont {
    let mut head = vec![0u8; 54];
    head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    let mut maxp = vec![0u8; 6];
    maxp[0..4].copy_from_slice(&0x0000_5000u32.to_be_bytes());
    maxp[4..6].copy_from_slice(&(GLYPHS as u16).to_be_bytes());
    let mut font = SfntFont::new(Flavour::Cff);
    font.insert(*b"head", head);
    font.insert(*b"maxp", maxp);
    font.insert(*b"CFF ", cff);
    font
}

#[derive(Default)]
struct Path(Vec<String>);

impl CommandSink for Path {
    fn move_to(&mut self, x: Fixed, y: Fixed) {
        self.0.push(format!("M {x:?} {y:?}"));
    }
    fn line_to(&mut self, x: Fixed, y: Fixed) {
        self.0.push(format!("L {x:?} {y:?}"));
    }
    fn curve_to(&mut self, a: Fixed, b: Fixed, c: Fixed, d: Fixed, e: Fixed, f: Fixed) {
        self.0.push(format!("C {a:?} {b:?} {c:?} {d:?} {e:?} {f:?}"));
    }
    fn close(&mut self) {
        self.0.push("Z".into());
    }
}

/// Every glyph's outline as drawn by read-fonts.
fn outlines(table: &[u8]) -> Vec<Vec<String>> {
    let cff = CffFontRef::new_cff(table, 0, None).unwrap();
    let sub = cff.subfont(0, &[]).unwrap();
    (0..GLYPHS as u32)
        .map(|g| {
            let mut sink = Path::default();
            cff.evaluate_charstring(&sub, GlyphId::new(g), &[], &mut sink).unwrap();
            sink.0
        })
        .collect()
}

fn subroutine_count(table: &[u8]) -> usize {
    let set = charstrings_of_table(table, false).unwrap();
    set.global_subrs.len() + set.local_subrs.iter().map(Vec::len).sum::<usize>()
}

#[test]
fn optimize_makes_a_cff_font_smaller_and_keeps_every_outline() {
    let input = otf(cff_table());
    let before = input.get(*b"CFF ").unwrap().to_vec();
    let result = optimize(&input, &OptimizeOptions::default()).unwrap();
    let after = result.font.get(*b"CFF ").unwrap();
    assert!(after.len() < before.len(), "{} >= {}", after.len(), before.len());
    assert!(result.bytes_after < result.bytes_before);
    assert_eq!(outlines(after), outlines(&before), "outlines are unchanged");
    assert!(subroutine_count(after) > 0, "the repeated shape went into subroutines");
    let applied: Vec<&str> = result.steps.iter().filter(|s| s.applied).map(|s| s.step).collect();
    assert_eq!(applied, ["cffWidths", "subroutinize"], "{:?}", result.steps);
    assert_eq!(result.font.get(*b"head"), input.get(*b"head"), "other tables are copied");

    // Expanding again gives the same outlines and no subroutines.
    let options = OptimizeOptions { desubroutinize: true, subroutines: false, cff_widths: false, ..OptimizeOptions::default() };
    let flat = optimize(&result.font, &options).unwrap();
    let flat_cff = flat.font.get(*b"CFF ").unwrap();
    assert_eq!(subroutine_count(flat_cff), 0);
    assert_eq!(outlines(flat_cff), outlines(&before));
}

#[test]
fn optimize_refuses_contradicting_options_and_keeps_fonts_without_cff() {
    let font = otf(cff_table());
    let both = OptimizeOptions { desubroutinize: true, subroutines: true, ..OptimizeOptions::default() };
    assert!(matches!(optimize(&font, &both), Err(TablesError::BadOptions(_))));
    let zero = OptimizeOptions { max_subrs: Some(0), ..OptimizeOptions::default() };
    assert!(matches!(optimize(&font, &zero), Err(TablesError::BadOptions(_))));

    let mut tt = SfntFont::new(Flavour::TrueType);
    tt.insert(*b"head", font.get(*b"head").unwrap().to_vec());
    tt.insert(*b"GPOS", vec![0, 1, 0, 0, 0, 10, 0, 10, 0, 10]);
    let result = optimize(&tt, &OptimizeOptions::default()).unwrap();
    assert_eq!(result.font, tt);
    assert!(result.steps.is_empty());
    assert!(result.notes.iter().any(|n| n.contains("GPOS")), "{:?}", result.notes);

    let mut junk = font.clone();
    junk.insert(*b"CFF ", vec![1, 0, 4, 1, 0, 0]);
    let result = optimize(&junk, &OptimizeOptions::default()).unwrap();
    assert_eq!(result.font, junk, "a table that cannot be read is left alone");
    assert!(result.notes.iter().any(|n| n.starts_with("CFF")), "{:?}", result.notes);
}
