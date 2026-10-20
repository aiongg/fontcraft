//! `recompile_cff` on CFF fonts built in code.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use fontcraft_cff::subr::{TableRewrite, rewrite_table};
use fontcraft_cff::{Font, Glyph, Number, PathOp, PrivateDict};
use read_fonts::ps::cff::CffFontRef;
use read_fonts::ps::cs::CommandSink;
use read_fonts::types::{Fixed, GlyphId};

use super::*;
use crate::sfnt::Flavour;

/// Every command a charstring sends, paths and hints, with its width.
#[derive(Default)]
struct Record(Vec<String>);

impl CommandSink for Record {
    fn move_to(&mut self, x: Fixed, y: Fixed) {
        self.0.push(format!("M {x:?} {y:?}"));
    }
    fn line_to(&mut self, x: Fixed, y: Fixed) {
        self.0.push(format!("L {x:?} {y:?}"));
    }
    fn curve_to(&mut self, a: Fixed, b: Fixed, c: Fixed, d: Fixed, x: Fixed, y: Fixed) {
        self.0.push(format!("C {a:?} {b:?} {c:?} {d:?} {x:?} {y:?}"));
    }
    fn close(&mut self) {
        self.0.push("Z".into());
    }
    fn hstem(&mut self, y: Fixed, dy: Fixed) {
        self.0.push(format!("hstem {y:?} {dy:?}"));
    }
    fn vstem(&mut self, x: Fixed, dx: Fixed) {
        self.0.push(format!("vstem {x:?} {dx:?}"));
    }
    fn hint_mask(&mut self, mask: &[u8]) {
        self.0.push(format!("hintmask {mask:?}"));
    }
}

/// Each glyph's commands, its width first.
fn drawn(table: &[u8]) -> Vec<Vec<String>> {
    let cff = CffFontRef::new_cff(table, 0, None).unwrap();
    let sub = cff.subfont(0, &[]).unwrap();
    (0..cff.num_glyphs())
        .map(|g| {
            let mut rec = Record::default();
            let width = cff.evaluate_charstring(&sub, GlyphId::new(g), &[], &mut rec).unwrap();
            rec.0.insert(0, format!("width {width:?}"));
            rec.0
        })
        .collect()
}

fn paths_only(cmds: &[String]) -> Vec<String> {
    cmds.iter().filter(|c| matches!(c.chars().next(), Some('M' | 'L' | 'C' | 'Z' | 'w'))).cloned().collect()
}

fn square(x: f64, y: f64, size: f64) -> Vec<PathOp> {
    vec![PathOp::MoveTo((x, y)), PathOp::LineTo((x + size, y)), PathOp::LineTo((x + size, y + size)), PathOp::LineTo((x, y + size))]
}

/// `.notdef`, A (with dotsection), acute, Aacute (seac of A and acute at 100, 200), and 40
/// glyphs drawn alike; ForceBold set.
fn font() -> SfntFont {
    let shape: Vec<PathOp> = (0..12).flat_map(|i| square(f64::from(i) * 37.0, f64::from(i % 3) * 51.0, 23.0)).collect();
    let mut glyphs = vec![
        Glyph { name: ".notdef".into(), advance: 500.0, ..Glyph::default() },
        Glyph {
            name: "A".into(),
            advance: 600.0,
            path: vec![
                PathOp::MoveTo((10.0, 0.0)),
                PathOp::LineTo((590.0, 0.0)),
                PathOp::CurveTo((500.0, 300.0), (400.0, 600.0), (300.0, 700.0)),
            ],
            ..Glyph::default()
        },
        Glyph { name: "acute".into(), advance: 300.0, path: square(50.0, 600.0, 120.0), ..Glyph::default() },
        Glyph { name: "Aacute".into(), advance: 600.0, path: square(0.0, 0.0, 10.0), ..Glyph::default() },
    ];
    for i in 0..40 {
        glyphs.push(Glyph { name: format!("g{i:02}"), advance: 700.0, path: shape.clone(), ..Glyph::default() });
    }
    let built = fontcraft_cff::build(&Font {
        name: "Test".into(),
        private: PrivateDict { force_bold: Number::Int(1), ..PrivateDict::default() },
        glyphs,
        ..Font::default()
    })
    .unwrap();
    // A: dotsection before its endchar. Aacute: `100 200 65 194 endchar` (A and acute).
    let (table, _) = rewrite_table(&built.table, false, &TableRewrite::default(), &mut |set| {
        let mut set = set.clone();
        let a = &mut set.charstrings[1];
        let end = a.len() - 1;
        a.splice(end..end, [12, 0]).for_each(drop);
        set.charstrings[3] = vec![239, 247, 92, 204, 247, 86, 14];
        Ok(set)
    })
    .unwrap();
    let mut font = SfntFont::new(Flavour::Cff);
    font.insert(*b"CFF ", table);
    font
}

#[test]
fn recompile_cff_draws_seac_out_keeps_outlines_and_subroutinises() {
    let font = font();
    let before = font.get(*b"CFF ").unwrap().to_vec();
    let r = recompile_cff(&font).unwrap();
    let after = r.font.get(*b"CFF ").unwrap().to_vec();

    assert_eq!(r.glyphs, 2, "A loses its dotsection, Aacute its seac");
    assert_eq!(r.seac_glyphs, 1);
    assert_eq!(r.removed_keys, vec!["ForceBold"]);
    assert!(after.len() < before.len(), "repeated charstrings go into subroutines: {} → {}", before.len(), after.len());
    assert!(fontcraft_cff::subr::charstrings_of_table(&after, false).unwrap().local_subrs.iter().any(|l| !l.is_empty())
        || !fontcraft_cff::subr::charstrings_of_table(&after, false).unwrap().global_subrs.is_empty());

    let (old, new) = (drawn(&before), drawn(&after));
    assert_eq!(old.len(), new.len());
    for (gid, (o, n)) in old.iter().zip(&new).enumerate() {
        if gid == 3 {
            // The accent's hints go; its outline stays where seac put it.
            assert_eq!(paths_only(o), paths_only(n), "glyph {gid}");
            assert!(n.iter().any(|c| c.starts_with('M')));
        } else {
            assert_eq!(o, n, "glyph {gid}");
        }
    }

    // Nothing obsolete is left to remove.
    let again = recompile_cff(&r.font).unwrap();
    assert_eq!((again.glyphs, again.removed_keys.len()), (0, 0));
}
