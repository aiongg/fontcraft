//! `CFF2` in a subset, and the subroutine steps, on a variable font built in code. The table
//! rules themselves are checked against pyftsubset in `fontcraft_cff::subr::subset_table`'s
//! tests; here the whole subset must draw every kept glyph as the input draws it, at the default
//! and at two other weights.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_cff::subr::{CharstringSet, SubrOptions, charstrings_of_table, prune_unused, subroutinize_table};
use fontcraft_compile::Options;
use fontcraft_compile::var::{VarFlavour, VariableOptions, compile_variable};
use fontcraft_model::{Axis, Font, Glyph, GlyphTable, Layer, Master, Path, Shape};
use read_fonts::ps::cff::CffFontRef;
use read_fonts::ps::cs::CommandSink;
use read_fonts::types::{F2Dot14, Fixed, GlyphId};

use super::super::{SubsetOptions, subset};
use crate::reorder::glyph_names;
use crate::sfnt::{SfntFont, read};

const NAMES: [&str; 8] = ["a", "b", "c", "d", "e", "f", "g", "h"];

/// Eight glyphs over a weight axis (Thin 100, Black 900), each three shared stems that grow with
/// the weight and one bar of its own, its `CFF2` table subroutinised so that the stems become
/// subroutines.
fn font() -> SfntFont {
    let mut f = Font::new("Cff2 Subset");
    f.axes = Arc::new(vec![Axis { name: "Weight".into(), tag: "wght".into(), ..Axis::default() }]);
    f.masters = Arc::new(vec![
        Master { id: "thin".into(), name: "Thin".into(), location: vec![100.0], ..Master::default() },
        Master { id: "black".into(), name: "Black".into(), location: vec![900.0], ..Master::default() },
    ]);
    let layer = |master: &str, stem: f64, i: usize| {
        let mut l = Layer::new(master, 400.0 + 2.0 * stem);
        l.id = master.into();
        for k in 0..3 {
            let x = 120.0 * k as f64;
            l.shapes.push(Shape::Path(Path::rect(x, 0.0, x + stem, 700.0)));
        }
        let y = 100.0 + 50.0 * i as f64;
        l.shapes.push(Shape::Path(Path::rect(0.0, y, 300.0 + stem, y + stem / 2.0)));
        l
    };
    let glyphs: Vec<Arc<Glyph>> = NAMES
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let mut g = Glyph::new(*name);
            g.unicodes = vec![0x61 + i as u32];
            g.layers = vec![layer("thin", 40.0, i), layer("black", 100.0, i)];
            Arc::new(g)
        })
        .collect();
    f.glyphs = GlyphTable::from_glyphs(glyphs);
    let var = VariableOptions { flavour: VarFlavour::Cff2, ..VariableOptions::default() };
    let built = compile_variable(&f, None, &Options::default(), &var).unwrap();
    let mut font = read(&built.compiled.bytes, 0).unwrap();
    let table = subroutinize_table(font.get(*b"CFF2").unwrap(), true, &SubrOptions::default()).unwrap();
    font.insert(*b"CFF2", table);
    font
}

#[derive(Default)]
struct Pen(Vec<(char, Vec<i32>)>);

impl CommandSink for Pen {
    fn move_to(&mut self, x: Fixed, y: Fixed) {
        self.0.push(('M', vec![x.to_bits(), y.to_bits()]));
    }
    fn line_to(&mut self, x: Fixed, y: Fixed) {
        self.0.push(('L', vec![x.to_bits(), y.to_bits()]));
    }
    fn curve_to(&mut self, a: Fixed, b: Fixed, c: Fixed, d: Fixed, e: Fixed, f: Fixed) {
        self.0.push(('C', [a, b, c, d, e, f].iter().map(|v| v.to_bits()).collect()));
    }
    fn close(&mut self) {
        self.0.push(('Z', Vec::new()));
    }
}

fn draw(font: &SfntFont, gid: u32, at: f32) -> Vec<(char, Vec<i32>)> {
    let cff = CffFontRef::new_cff2(font.get(*b"CFF2").unwrap(), None).unwrap();
    let coords = [F2Dot14::from_f32(at)];
    let gid = GlyphId::new(gid);
    let subfont = cff.subfont(cff.subfont_index(gid).unwrap_or(0), &coords).unwrap();
    let mut pen = Pen::default();
    cff.evaluate_charstring(&subfont, gid, &coords, &mut pen).unwrap();
    pen.0
}

fn set_of(font: &SfntFont) -> CharstringSet {
    charstrings_of_table(font.get(*b"CFF2").unwrap(), true).unwrap()
}

fn subr_count(set: &CharstringSet) -> usize {
    set.global_subrs.len() + set.local_subrs.iter().map(Vec::len).sum::<usize>()
}

/// Every output glyph draws as the input glyph it came from.
fn assert_draws_the_same(input: &SfntFont, output: &SfntFont, new_to_old: &[(u32, u32)]) {
    for &(new, old) in new_to_old {
        for at in [0.0, 0.5, 1.0] {
            assert_eq!(draw(output, new, at), draw(input, old, at), "glyph {old} (now {new}) at {at}");
        }
    }
}

#[test]
fn a_cff2_subset_draws_the_same_and_keeps_only_the_subroutines_it_calls() {
    let input = font();
    let names = glyph_names(&input).unwrap();
    let old = |name: &str| names.iter().position(|n| n == name).unwrap() as u32;
    let before = set_of(&input);
    assert!(subr_count(&before) > 0, "the input has subroutines");

    let result = subset(&input, &SubsetOptions { text: "bg".into(), ..SubsetOptions::default() }).unwrap();
    assert_eq!(result.retained, [".notdef", "b", "g"]);
    assert!(result.font.get(*b"HVAR").is_some() && result.font.get(*b"fvar").is_some());
    assert_draws_the_same(&input, &result.font, &[(1, old("b")), (2, old("g"))]);
    let after = set_of(&result.font);
    assert!(after.charstrings[0].is_empty(), ".notdef is emptied");
    assert!(subr_count(&after) <= subr_count(&before));
    assert_eq!(prune_unused(&after).unwrap(), after, "every subroutine left is called, in order");

    // Expanded, or without hints: no subroutine is left and the outlines stay.
    for options in [
        SubsetOptions { text: "bg".into(), desubroutinize: true, ..SubsetOptions::default() },
        SubsetOptions { text: "bg".into(), hinting: false, ..SubsetOptions::default() },
    ] {
        let flat = subset(&input, &options).unwrap();
        assert_eq!(subr_count(&set_of(&flat.font)), 0, "{options:?}");
        assert_draws_the_same(&input, &flat.font, &[(1, old("b")), (2, old("g"))]);
    }

    // Glyph ids kept: the glyphs before `c` stay in place, empty.
    let retained = subset(&input, &SubsetOptions { glyphs: vec!["c".into()], retain_gids: true, ..SubsetOptions::default() }).unwrap();
    let c = old("c");
    let set = set_of(&retained.font);
    assert_eq!(set.charstrings.len(), c as usize + 1);
    assert!(set.charstrings[..c as usize].iter().all(Vec::is_empty));
    assert_draws_the_same(&input, &retained.font, &[(c, c)]);
}

#[test]
fn a_damaged_cff2_table_fails_the_subset() {
    let mut input = font();
    let cut = input.get(*b"CFF2").unwrap()[..30].to_vec();
    input.insert(*b"CFF2", cut);
    assert!(subset(&input, &SubsetOptions { text: "a".into(), ..SubsetOptions::default() }).is_err());
}
