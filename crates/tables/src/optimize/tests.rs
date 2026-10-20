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

    let mut junk = font.clone();
    junk.insert(*b"CFF ", vec![1, 0, 4, 1, 0, 0]);
    let result = optimize(&junk, &OptimizeOptions::default()).unwrap();
    assert_eq!(result.font, junk, "a table that cannot be read is left alone");
    assert!(result.notes.iter().any(|n| n.starts_with("CFF")), "{:?}", result.notes);
}

/// PaintGlyph of glyph `g` filled with PaintSolid of palette entry `p` (the solid right after).
fn paint_glyph(g: u16, p: u16) -> Vec<u8> {
    let mut v = vec![10, 0, 0, 6];
    v.extend(g.to_be_bytes());
    v.push(2);
    v.extend(p.to_be_bytes());
    v.extend(0x4000u16.to_be_bytes());
    v
}

/// COLR 1 whose glyph `g` draws layers `first..first + n` of `layers`, for each `(g, first, n)`.
fn colr_v1(bases: &[(u16, u32, u8)], layers: &[Vec<u8>]) -> Vec<u8> {
    let base_at = 34usize;
    let layer_at = base_at + 4 + 6 * bases.len();
    let mut paints: Vec<Vec<u8>> = bases
        .iter()
        .map(|(_, first, n)| {
            let mut v = vec![1, *n];
            v.extend(first.to_be_bytes());
            v
        })
        .collect();
    paints.extend(layers.iter().cloned());
    let mut at = layer_at + 4 + 4 * layers.len();
    let mut offsets = Vec::new();
    for p in &paints {
        offsets.push(at);
        at += p.len();
    }
    let mut out = vec![0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    for v in [base_at, layer_at, 0, 0, 0] {
        out.extend((v as u32).to_be_bytes());
    }
    out.extend((bases.len() as u32).to_be_bytes());
    for (i, (g, _, _)) in bases.iter().enumerate() {
        out.extend(g.to_be_bytes());
        out.extend(((offsets[i] - base_at) as u32).to_be_bytes());
    }
    out.extend((layers.len() as u32).to_be_bytes());
    for o in &offsets[bases.len()..] {
        out.extend(((o - layer_at) as u32).to_be_bytes());
    }
    for p in paints {
        out.extend(p);
    }
    out
}

/// A paint as text: `G<glyph>/<palette>` or `L<first>+<count>`.
fn describe(t: &[u8], at: usize) -> String {
    let u16_at = |i: usize| u16::from_be_bytes([t[i], t[i + 1]]);
    let u32_at = |i: usize| u32::from_be_bytes([t[i], t[i + 1], t[i + 2], t[i + 3]]);
    match t[at] {
        10 => {
            let solid = at + ((usize::from(t[at + 1]) << 16) | usize::from(u16_at(at + 2)));
            format!("G{}/{}", u16_at(at + 4), u16_at(solid + 1))
        }
        1 => format!("L{}+{}", u32_at(at + 2), t[at + 1]),
        f => format!("?{f}"),
    }
}

/// Expected values from fontTools 4.66: `colorLib.builder.buildColrV1` on the same two glyphs.
#[test]
fn colr_layer_reuse_shares_runs_as_the_reference_does() {
    let mut font = SfntFont::new(Flavour::TrueType);
    let mut maxp = vec![0u8; 6];
    maxp[1] = 1;
    maxp[4..6].copy_from_slice(&7u16.to_be_bytes());
    font.insert(*b"maxp", maxp);
    let a = [paint_glyph(3, 0), paint_glyph(4, 1), paint_glyph(5, 2)];
    let layers: Vec<Vec<u8>> = a.iter().cloned().chain(a.iter().cloned()).chain([paint_glyph(6, 3)]).collect();
    let input = colr_v1(&[(1, 0, 3), (2, 3, 4)], &layers);
    font.insert(*b"COLR", input.clone());

    let out = colr::run(&font).unwrap().unwrap();
    let u32_at = |i: usize| u32::from_be_bytes([out[i], out[i + 1], out[i + 2], out[i + 3]]) as usize;
    let list = u32_at(18);
    let layers: Vec<String> = (0..u32_at(list)).map(|i| describe(&out, list + u32_at(list + 4 + 4 * i))).collect();
    assert_eq!(layers, ["G3/0", "G4/1", "G5/2", "L0+3", "G6/3"]);
    let base = u32_at(14);
    let bases: Vec<String> = (0..u32_at(base)).map(|i| describe(&out, base + u32_at(base + 6 + 6 * i))).collect();
    assert_eq!(bases, ["L0+3", "L3+2"]);
    assert!(out.len() < input.len());

    let result = optimize(&font, &OptimizeOptions::default()).unwrap();
    assert_eq!(result.font.get(*b"COLR"), Some(out.as_slice()));
    assert!(result.steps.iter().any(|s| s.step == "colrLayerReuse" && s.applied), "{:?}", result.steps);
}
