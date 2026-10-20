//! The steps around a conversion ([`super::prepare`]) on fonts built in code. The glyph subset
//! and GOADB expectations follow what `tx` (AFDKO 5.0.1) wrote for `-g`, `-gf`, `-gr`, `-gs` and
//! `-go` on a corpus font.

use std::collections::BTreeSet;

use super::outline::encode_simple;
use super::tests::{be16, cff_font, common, glyf_points, post_v2, ps_contours, truetype_font};
use super::*;
use crate::goadb;

fn names(font: &SfntFont) -> Vec<String> {
    crate::reorder::glyph_names(font).unwrap()
}

fn goadb_use(text: &str, rename: bool, subset: bool, order: bool) -> Option<GoadbUse> {
    Some(GoadbUse { goadb: goadb::parse(text).unwrap(), rename, subset, order })
}

/// `.notdef` (empty) and `O`, two overlapping clockwise squares.
fn overlapping_font() -> SfntFont {
    let mut font = SfntFont::new(Flavour::TrueType);
    common(&mut font, &[500, 800], 0x0001_0000);
    let first = vec![(0, 0, true), (0, 500, true), (500, 500, true), (500, 0, true)];
    let second = vec![(250, 250, true), (250, 750, true), (750, 750, true), (750, 250, true)];
    let (o, _) = encode_simple(&[first, second]).unwrap();
    let mut glyf = o;
    if glyf.len() % 2 == 1 {
        glyf.push(0);
    }
    let mut loca = Vec::new();
    for offset in [0u32, 0, glyf.len() as u32] {
        loca.extend_from_slice(&offset.to_be_bytes());
    }
    let mut head = font.get(*b"head").unwrap().to_vec();
    head[50..52].copy_from_slice(&be16(1));
    font.insert(*b"head", head);
    font.insert(*b"glyf", glyf);
    font.insert(*b"loca", loca);
    font.insert(*b"post", post_v2(&[".notdef", "O"]));
    font
}

fn converted(font: &SfntFont, target: ConvertTarget, prepare: &PrepareOptions) -> SfntFont {
    convert_prepared(font, &ConvertOptions::new(target), prepare).unwrap().font.unwrap()
}

#[test]
fn a_glyph_subset_keeps_notdef_and_the_selection_in_font_order() {
    let prepare = PrepareOptions { glyphs: Some(BTreeSet::from([1])), ..PrepareOptions::default() };
    let font = converted(&truetype_font(), ConvertTarget::Cff, &prepare);
    assert_eq!(names(&font), [".notdef", "A"]);
    assert!(font.get(*b"CFF ").is_some());
    // Nothing selected but .notdef still writes a font, as tx does.
    let prepare = PrepareOptions { glyphs: Some(BTreeSet::new()), ..PrepareOptions::default() };
    assert_eq!(names(&converted(&truetype_font(), ConvertTarget::TrueType { max_error: 1.0 }, &prepare)), [".notdef"]);
}

#[test]
fn a_goadb_renames_subsets_and_orders_as_tx_does() {
    let text = "Bee B\nAy A\n";
    // -gf: renamed, font order kept.
    let prepare = PrepareOptions { goadb: goadb_use(text, true, false, false), ..PrepareOptions::default() };
    assert_eq!(names(&converted(&truetype_font(), ConvertTarget::Cff, &prepare)), [".notdef", "Ay", "Bee"]);
    // -gf -go: in the file's order; B's component follows A to its new glyph id.
    let prepare = PrepareOptions { goadb: goadb_use(text, true, false, true), ..PrepareOptions::default() };
    let font = converted(&truetype_font(), ConvertTarget::TrueType { max_error: 1.0 }, &prepare);
    assert_eq!(names(&font), [".notdef", "Bee", "Ay"]);
    assert_eq!(glyf_points(&font, 1), glyf_points(&truetype_font(), 2), "Bee draws as B did");
    // -gf -gr -gs: only the listed glyphs, names kept.
    let prepare = PrepareOptions { goadb: goadb_use("Ay A\n", false, true, false), ..PrepareOptions::default() };
    assert_eq!(names(&converted(&truetype_font(), ConvertTarget::Cff, &prepare)), [".notdef", "A"]);
    // -g with -gs: the glyphs both select.
    let prepare = PrepareOptions { glyphs: Some(BTreeSet::from([2])), goadb: goadb_use("Ay A\n", true, true, false), ..PrepareOptions::default() };
    assert_eq!(names(&converted(&truetype_font(), ConvertTarget::Cff, &prepare)), [".notdef"]);
}

#[test]
fn overlaps_are_merged_before_or_after_the_conversion_and_refused_on_cff_outlines() {
    let prepare = PrepareOptions { remove_overlaps: true, ..PrepareOptions::default() };
    assert_eq!(glyf_points(&overlapping_font(), 1).len(), 2);
    let font = converted(&overlapping_font(), ConvertTarget::TrueType { max_error: 1.0 }, &prepare);
    let merged = glyf_points(&font, 1);
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].len(), 8, "the union of the squares has eight corners");
    let font = converted(&overlapping_font(), ConvertTarget::Cff, &prepare);
    assert_eq!(ps_contours(&font, *b"CFF ", 1).len(), 1);
    // CFF to CFF: no TrueType stage to merge in.
    let refused = convert_prepared(&cff_font(), &ConvertOptions::new(ConvertTarget::Cff2), &prepare);
    assert!(matches!(refused, Err(TablesError::Unsupported(_))), "{refused:?}");
    // CFF to TrueType merges after the conversion.
    assert!(convert_prepared(&cff_font(), &ConvertOptions::new(ConvertTarget::TrueType { max_error: 1.0 }), &prepare).is_ok());
}

#[test]
fn a_location_snapshots_a_variable_font_and_bad_axes_are_refused() {
    let var = crate::instance::tests::variable_font();
    let at = |location: Vec<(Tag, f64)>, clamp: bool| {
        let options = ConvertOptions::new(ConvertTarget::TrueType { max_error: 1.0 });
        convert_prepared(&var, &options, &PrepareOptions { location: Some(location), clamp, ..PrepareOptions::default() })
    };
    let font = at(vec![(*b"wght", 900.0)], true).unwrap().font.unwrap();
    for tag in [*b"fvar", *b"gvar", *b"avar"] {
        assert!(font.get(tag).is_none(), "{}", crate::sfnt::tag_str(tag));
    }
    // At wght 900 the square's left side moves -20 and its top +50 (see the font's builder).
    let square = glyf_points(&font, 1);
    assert_eq!(square, [vec![(80, 0, true), (80, 750, true), (520, 750, true), (520, 0, true)]]);
    // Clamped to the axis maximum, or refused without clamping.
    assert_eq!(at(vec![(*b"wght", 2000.0)], true).unwrap().bytes, at(vec![(*b"wght", 900.0)], true).unwrap().bytes);
    assert!(matches!(at(vec![(*b"wght", 2000.0)], false), Err(TablesError::BadOptions(_))));
    let err = at(vec![(*b"wdth", 100.0)], true).unwrap_err();
    assert!(err.to_string().contains("no `wdth` axis"), "{err}");
    // A static font has no location to go to.
    let err = convert_prepared(
        &truetype_font(),
        &ConvertOptions::new(ConvertTarget::Cff),
        &PrepareOptions { location: Some(Vec::new()), ..PrepareOptions::default() },
    );
    assert!(matches!(err, Err(TablesError::BadOptions(_))));
}
