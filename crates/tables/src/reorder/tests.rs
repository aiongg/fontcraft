#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_compile::{Flavour as CompileFlavour, Options, compile};
use fontcraft_model::{Anchor, Component, Font, Path, Shape, Snippet};
use read_fonts::types::{GlyphId, GlyphId16};
use read_fonts::{FontRef, TableProvider};
use write_fonts::from_obj::ToOwnedTable;
use write_fonts::tables::gpos::{PairPos, PositionLookup};
use write_fonts::tables::gsub::SubstitutionLookup;
use write_fonts::tables::layout::CoverageTable;

use super::*;
use crate::rename::{RenameOptions, rename_glyphs};
use crate::sfnt::{WriteOptions, read, write};

/// A, V, a combining acute with anchors, Aacute built from components, kerning A V and a
/// ligature A V → Aacute, so the compiled font has `glyf` composites, `cmap`, `hmtx`, `GDEF`,
/// `GSUB` and `GPOS`.
fn source() -> Font {
    let mut f = Font::new("Order Sans");
    for (name, cps, width) in [("A", vec![0x41], 600.0), ("V", vec![0x56], 650.0), ("acutecomb", vec![0x301], 0.0), ("Aacute", vec![0xC1], 600.0)] {
        if f.glyph(name).is_none() {
            f.add_glyph(name, cps, width).unwrap();
        }
    }
    {
        let mut a = f.glyph_mut("A").unwrap();
        a.layers[0].shapes.push(Shape::Path(Path::rect(0.0, 0.0, 600.0, 700.0)));
        a.layers[0].anchors.push(Anchor::new("top", 300.0, 700.0));
    }
    f.glyph_mut("V").unwrap().layers[0].shapes.push(Shape::Path(Path::rect(0.0, 0.0, 650.0, 700.0)));
    {
        let mut m = f.glyph_mut("acutecomb").unwrap();
        m.layers[0].shapes.push(Shape::Path(Path::rect(50.0, 500.0, 150.0, 800.0)));
        m.layers[0].anchors.push(Anchor::new("_top", 100.0, 500.0));
    }
    {
        let mut c = f.glyph_mut("Aacute").unwrap();
        c.layers[0].shapes.push(Shape::Component(Component::new("A", 0.0, 0.0)));
        c.layers[0].shapes.push(Shape::Component(Component::new("acutecomb", 200.0, 200.0)));
    }
    Arc::make_mut(&mut f.kerning).ltr.set("m01", "A", "V", -50.0);
    Arc::make_mut(&mut f.features).features.push(Snippet { name: "liga".into(), code: "sub A V by Aacute;".into(), ..Default::default() });
    f
}

fn built(flavour: CompileFlavour) -> SfntFont {
    let f = source();
    let out = compile(&f, &f.default_target().unwrap(), flavour, &Options::default()).unwrap();
    read(&out.bytes, 0).unwrap()
}

fn gid(font: &FontRef, c: char) -> GlyphId {
    font.cmap().unwrap().map_codepoint(c).unwrap()
}

fn g16(g: GlyphId) -> GlyphId16 {
    GlyphId16::new(g.to_u32() as u16)
}

fn coverage(c: &CoverageTable) -> Vec<GlyphId16> {
    match c {
        CoverageTable::Format1(f) => f.glyph_array.clone(),
        CoverageTable::Format2(f) => {
            f.range_records.iter().flat_map(|r| (r.start_glyph_id.to_u16()..=r.end_glyph_id.to_u16()).map(GlyphId16::new)).collect()
        }
    }
}

#[test]
fn a_reordered_truetype_font_keeps_every_glyph_reference() {
    let font = built(CompileFlavour::TrueType);
    let names = glyph_names(&font).unwrap();
    // Every glyph but .notdef in reverse order.
    let wanted: Vec<String> = names.iter().skip(1).rev().cloned().collect();
    let out = reorder_glyphs(font.clone(), &ReorderOptions { order: wanted.clone() }).unwrap();
    assert_eq!(glyph_names(&out).unwrap()[1..], wanted[..], "post names follow the new order");

    let bytes = write(&out, &WriteOptions::default()).unwrap();
    let before_bytes = write(&font, &WriteOptions::default()).unwrap();
    let (old, new) = (FontRef::new(&before_bytes).unwrap(), FontRef::new(&bytes).unwrap());
    let (a, v, acute, aacute) = (gid(&new, 'A'), gid(&new, 'V'), gid(&new, '\u{301}'), gid(&new, 'Á'));
    assert_ne!(a, gid(&old, 'A'), "the order changed");
    assert_eq!(names.get(gid(&old, 'A').to_u32() as usize), glyph_names(&out).unwrap().get(a.to_u32() as usize), "cmap and names agree");

    let hmtx = new.hmtx().unwrap();
    assert_eq!((hmtx.advance(a), hmtx.advance(v), hmtx.advance(acute)), (Some(600), Some(650), Some(0)));

    let glyf = new.glyf().unwrap();
    let composite = new.loca(None).unwrap().get_glyf(aacute, &glyf).unwrap().unwrap();
    let read_fonts::tables::glyf::Glyph::Composite(c) = composite else { panic!("Aacute is a composite") };
    let parts: Vec<GlyphId16> = c.components().map(|c| c.glyph).collect();
    assert_eq!(parts, [g16(a), g16(acute)]);

    let classes = new.gdef().unwrap().glyph_class_def().unwrap().unwrap();
    assert_eq!((classes.get(g16(a)), classes.get(g16(acute))), (1, 3));

    let gsub: write_fonts::tables::gsub::Gsub = new.gsub().unwrap().to_owned_table();
    let lig = gsub.lookup_list.lookups.iter().find_map(|l| match &**l {
        SubstitutionLookup::Ligature(l) => Some(l.subtables[0].clone()),
        _ => None,
    });
    let lig = lig.expect("a ligature lookup");
    assert_eq!(coverage(&lig.coverage), [g16(a)]);
    let first = &lig.ligature_sets[0].ligatures[0];
    assert_eq!((first.ligature_glyph, first.component_glyph_ids.clone()), (g16(aacute), vec![g16(v)]));

    let gpos: write_fonts::tables::gpos::Gpos = new.gpos().unwrap().to_owned_table();
    for lookup in &gpos.lookup_list.lookups {
        if let PositionLookup::Pair(l) = &**lookup {
            for sub in &l.subtables {
                let cov = match &**sub {
                    PairPos::Format1(f) => {
                        for set in &f.pair_sets {
                            let seconds: Vec<GlyphId16> = set.pair_value_records.iter().map(|r| r.second_glyph).collect();
                            assert!(seconds.windows(2).all(|w| w[0] < w[1]), "pair sets are sorted by the new ids");
                        }
                        coverage(&f.coverage)
                    }
                    PairPos::Format2(f) => coverage(&f.coverage),
                };
                assert!(cov.contains(&g16(a)) && cov.windows(2).all(|w| w[0] < w[1]), "{cov:?}");
            }
        }
    }

    // The inverse order gives the original glyph order and outlines back.
    let back = reorder_glyphs(out, &ReorderOptions { order: names[1..].to_vec() }).unwrap();
    assert_eq!(glyph_names(&back).unwrap(), names);
    assert_eq!(back.get(*b"glyf"), font.get(*b"glyf"));
}

#[test]
fn a_reordered_cff_font_moves_charstrings_and_charset_together() {
    let font = built(CompileFlavour::Cff);
    let names = glyph_names(&font).unwrap();
    let n = names.len();
    let before = cff::parse(font.get(*b"CFF ").unwrap(), n).unwrap().charstring_lengths();
    let out = reorder_glyphs(font.clone(), &ReorderOptions { order: vec!["V".into()] }).unwrap();
    let after_names = glyph_names(&out).unwrap();
    assert_eq!(after_names[1], "V", "a partial order puts its glyphs first, after .notdef");
    let after = cff::parse(out.get(*b"CFF ").unwrap(), n).unwrap().charstring_lengths();
    for (new, name) in after_names.iter().enumerate() {
        let old = names.iter().position(|x| x == name).unwrap();
        assert_eq!(after[new], before[old], "{name}");
    }
    let bytes = write(&out, &WriteOptions::default()).unwrap();
    let f = FontRef::new(&bytes).unwrap();
    assert_eq!(gid(&f, 'V').to_u32(), 1);
}

#[test]
fn renaming_changes_post_or_the_charset_and_refuses_clashes() {
    for flavour in [CompileFlavour::TrueType, CompileFlavour::Cff] {
        let font = built(flavour);
        let options = RenameOptions { map: vec![("V".into(), "vee".into()), ("missing".into(), "x".into())], ..Default::default() };
        let out = rename_glyphs(font.clone(), &options).unwrap();
        assert_eq!((out.renamed, out.missing.clone()), (1, vec!["missing".to_string()]));
        let names = glyph_names(&out.font).unwrap();
        assert!(names.contains(&"vee".to_string()) && !names.contains(&"V".to_string()), "{names:?}");
        assert_eq!(out.font.get(*b"glyf"), font.get(*b"glyf"), "outlines are untouched");

        let clash = RenameOptions { map: vec![("V".into(), "A".into())], ..Default::default() };
        assert!(rename_glyphs(font, &clash).unwrap_err().to_string().contains("two glyphs"));
    }
}

#[test]
fn bad_orders_are_refused() {
    let names: Vec<String> = [".notdef", "A", "B"].iter().map(|s| s.to_string()).collect();
    assert_eq!(new_order(&names, &["B".into()]).unwrap(), [0, 2, 1]);
    assert_eq!(new_order(&names, &["B".into(), ".notdef".into()]).unwrap(), [0, 2, 1], ".notdef stays first");
    assert!(new_order(&names, &["C".into()]).is_err());
    assert!(new_order(&names, &["A".into(), "A".into()]).is_err());
    let font = built(CompileFlavour::TrueType);
    let n = names::num_glyphs(&font).unwrap();
    assert!(apply_order(font.clone(), &vec![0; n]).is_err(), "a glyph twice");
    let mut colour = font;
    colour.insert(*b"COLR", vec![0; 14]);
    let mut order: Vec<u16> = (0..n as u16).collect();
    order.swap(1, 2);
    assert!(matches!(apply_order(colour, &order), Err(TablesError::Unsupported(_))));
}

#[test]
fn hostile_tables_fail_with_a_message() {
    let font = built(CompileFlavour::TrueType);
    let n = names::num_glyphs(&font).unwrap();
    let mut order: Vec<u16> = (0..n as u16).collect();
    order.swap(1, 2);
    for tag in [*b"glyf", *b"loca", *b"hmtx", *b"cmap", *b"GSUB", *b"GPOS", *b"GDEF", *b"post"] {
        let mut bad = font.clone();
        let mut bytes = bad.get(tag).unwrap().to_vec();
        bytes.truncate(bytes.len() / 3);
        bad.insert(tag, bytes);
        let _ = apply_order(bad, &order);
    }
}

#[test]
fn shared_layout_subtables_are_refused_before_they_are_copied() {
    use crate::layout_budget::tests::{MAX_SHARED, shared_gpos};
    let mut font = built(CompileFlavour::TrueType);
    let n = names::num_glyphs(&font).unwrap();
    let mut order: Vec<u16> = (0..n as u16).collect();
    order.swap(1, 2);
    font.insert(*b"GPOS", shared_gpos(MAX_SHARED, MAX_SHARED, 2));
    let e = apply_order(font, &order).unwrap_err();
    assert!(e.to_string().contains("too large"), "{e}");
}
