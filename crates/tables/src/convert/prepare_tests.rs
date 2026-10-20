//! The steps around a conversion ([`super::prepare`]) on fonts built in code. The glyph subset
//! and GOADB expectations follow what `tx` (AFDKO 5.0.1) wrote for `-g`, `-gf`, `-gr`, `-gs` and
//! `-go` on a corpus font.

use std::collections::BTreeSet;

use super::outline::encode_simple;
use fontcraft_cff::PathOp;
use fontcraft_cff::cid::{CidFont, CidGlyph, FontDict, Ros};

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
fn overlaps_are_merged_before_or_after_the_conversion() {
    let prepare = PrepareOptions { remove_overlaps: true, ..PrepareOptions::default() };
    assert_eq!(glyf_points(&overlapping_font(), 1).len(), 2);
    let font = converted(&overlapping_font(), ConvertTarget::TrueType { max_error: 1.0 }, &prepare);
    let merged = glyf_points(&font, 1);
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].len(), 8, "the union of the squares has eight corners");
    let font = converted(&overlapping_font(), ConvertTarget::Cff, &prepare);
    assert_eq!(ps_contours(&font, *b"CFF ", 1).len(), 1);
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

fn square(x: f64, y: f64, size: f64) -> Vec<PathOp> {
    vec![PathOp::MoveTo((x, y)), PathOp::LineTo((x + size, y)), PathOp::LineTo((x + size, y + size)), PathOp::LineTo((x, y + size))]
}

fn with_post3(mut font: SfntFont, table: Vec<u8>) -> SfntFont {
    font.insert(*b"CFF ", table);
    let (post, _) = post_table(None, PostFormat::NoNames, &[]).unwrap();
    font.insert(*b"post", post);
    font
}

/// `.notdef` (empty) and `O`, two overlapping counter-clockwise squares, in a CFF font.
fn overlapping_cff_font() -> SfntFont {
    let mut font = SfntFont::new(Flavour::Cff);
    common(&mut font, &[500, 800], 0x5000);
    let glyph = |name: &str, path: Vec<PathOp>| fontcraft_cff::Glyph { name: name.into(), advance: 800.0, path, hints: None };
    let cff = fontcraft_cff::Font {
        name: "Test-Regular".into(),
        glyphs: vec![glyph(".notdef", Vec::new()), glyph("O", [square(0.0, 0.0, 500.0), square(250.0, 250.0, 500.0)].concat())],
        ..fontcraft_cff::Font::default()
    };
    with_post3(font, fontcraft_cff::build(&cff).unwrap().table)
}

fn ros(registry: &str, ordering: &str, supplement: i64) -> Ros {
    Ros { registry: registry.into(), ordering: ordering.into(), supplement }
}

/// A CID-keyed CFF font: CID 0 and CID 1 in Font DICT 0, CID 5 in Font DICT 1.
fn two_dict_cid_font() -> SfntFont {
    let mut font = SfntFont::new(Flavour::Cff);
    common(&mut font, &[500, 500, 500], 0x5000);
    let glyph = |cid: u32, fd: usize, path: Vec<PathOp>| CidGlyph {
        cid,
        fd,
        glyph: fontcraft_cff::Glyph { name: format!("cid{cid}"), advance: 500.0, path, hints: None },
    };
    let dict = |name: &str| FontDict { name: name.into(), private: fontcraft_cff::PrivateDict::default(), widths: fontcraft_cff::Widths::Optimise };
    let cid = CidFont {
        name: "Test-CID".into(),
        top: fontcraft_cff::TopDict::default(),
        ros: ros("Adobe", "Identity", 0),
        cid_count: None,
        font_dicts: vec![dict("Test-Zero"), dict("Test-One")],
        glyphs: vec![glyph(0, 0, Vec::new()), glyph(1, 0, square(0.0, 0.0, 400.0)), glyph(5, 1, square(50.0, 0.0, 300.0))],
        round_tolerance: 0.5,
        specialise: true,
    };
    with_post3(font, fontcraft_cff::cid::build(&cid).unwrap().table)
}

#[test]
fn a_variable_font_is_snapshotted_at_its_default_when_its_flavour_changes() {
    let var = crate::instance::tests::variable_font();
    let options = ConvertOptions::new(ConvertTarget::Cff);
    let plain = convert_prepared(&var, &options, &PrepareOptions::default()).unwrap();
    let at_default = convert_prepared(&var, &options, &PrepareOptions { location: Some(Vec::new()), ..PrepareOptions::default() }).unwrap();
    assert_eq!(plain.bytes, at_default.bytes, "the same font as an explicit snapshot at the default");
    assert!(plain.left_behind.first().is_some_and(|n| n.contains("variations")), "{:?}", plain.left_behind);
    let font = plain.font.unwrap();
    assert!(font.get(*b"fvar").is_none() && font.get(*b"gvar").is_none());
    assert_eq!(ps_contours(&font, *b"CFF ", 1).len(), 1);
    // Keeping the flavour keeps the variations.
    let kept = converted(&var, ConvertTarget::TrueType { max_error: 1.0 }, &PrepareOptions::default());
    assert!(kept.get(*b"fvar").is_some() && kept.get(*b"gvar").is_some());
}

#[test]
fn a_name_keyed_cff_becomes_cid_keyed_with_cid_n_for_glyph_n() {
    let keyed = |font: &SfntFont, target: ConvertTarget, collection: Ros| {
        convert_with(font, &ConvertOptions { cid_keyed: Some(collection), ..ConvertOptions::new(target) })
    };
    let result = keyed(&cff_font(), ConvertTarget::Cff, ros("Adobe", "Identity", 0)).unwrap();
    let font = result.font.unwrap();
    assert_eq!(glyph_cids(&font).unwrap(), Some(vec![0, 1]));
    let n = super::num_glyphs(&font).unwrap();
    let cff = crate::reorder::cff::parse(font.get(*b"CFF ").unwrap(), n).unwrap();
    assert_eq!(cff.ros(), Some(("Adobe".to_string(), "Identity".to_string(), 0)));
    assert_eq!(cff.font_dict_of_glyphs(), (vec![0, 0], 1));
    assert_eq!(ps_contours(&font, *b"CFF ", 1), ps_contours(&cff_font(), *b"CFF ", 1), "the charstrings are kept");
    assert!(result.left_behind.iter().any(|n| n.contains("glyph names")));
    // From TrueType, as a bare table, with another collection.
    let bare = keyed(&truetype_font(), ConvertTarget::BareCff, ros("Test", "Order", 3)).unwrap();
    let mut wrapped = SfntFont::new(Flavour::Cff);
    common(&mut wrapped, &[500, 500, 500], 0x5000);
    wrapped.insert(*b"CFF ", bare.bytes);
    assert_eq!(glyph_cids(&wrapped).unwrap(), Some(vec![0, 1, 2]));
    // Already CID-keyed: kept with the same collection, refused with another; TrueType has no CIDs.
    assert!(keyed(&two_dict_cid_font(), ConvertTarget::Cff, ros("Adobe", "Identity", 0)).is_ok());
    assert!(matches!(keyed(&two_dict_cid_font(), ConvertTarget::Cff, ros("Adobe", "Japan1", 6)), Err(TablesError::Unsupported(_))));
    assert!(matches!(keyed(&cff_font(), ConvertTarget::TrueType { max_error: 1.0 }, ros("Adobe", "Identity", 0)), Err(TablesError::BadOptions(_))));
    assert!(matches!(keyed(&cff_font(), ConvertTarget::Cff, ros("Adobe", "Iden tity", 0)), Err(TablesError::BadOptions(_))));
}

#[test]
fn a_font_dict_selection_keeps_the_glyphs_of_those_dicts_as_tx_fd_does() {
    let filter = |keep: Option<&[u16]>, exclude: &[u16]| PrepareOptions {
        font_dicts: Some(FontDictFilter { keep: keep.map(|k| k.iter().copied().collect()), exclude: exclude.iter().copied().collect() }),
        ..PrepareOptions::default()
    };
    let font = converted(&two_dict_cid_font(), ConvertTarget::Cff, &filter(Some(&[1]), &[]));
    assert_eq!(glyph_cids(&font).unwrap(), Some(vec![0, 5]));
    let font = converted(&two_dict_cid_font(), ConvertTarget::Cff, &filter(None, &[1]));
    assert_eq!(glyph_cids(&font).unwrap(), Some(vec![0, 1]));
    // CFF2 keeps its Font DICTs: the same selection after a conversion to CFF2.
    let cff2 = converted(&two_dict_cid_font(), ConvertTarget::Cff2, &PrepareOptions::default());
    assert_eq!(names(&converted(&cff2, ConvertTarget::Cff2, &filter(Some(&[1]), &[]))).len(), 2);
    // A name-keyed font, TrueType, and a Font DICT the font lacks are refused.
    for (font, f) in
        [(cff_font(), filter(Some(&[0]), &[])), (truetype_font(), filter(Some(&[0]), &[])), (two_dict_cid_font(), filter(Some(&[2]), &[]))]
    {
        let refused = convert_prepared(&font, &ConvertOptions::new(ConvertTarget::Cff), &f);
        assert!(matches!(refused, Err(TablesError::BadOptions(_))), "{refused:?}");
    }
}

#[test]
fn a_goadb_orders_the_glyphs_of_a_cff2_font() {
    let cff2 = converted(&truetype_font(), ConvertTarget::Cff2, &PrepareOptions::default());
    assert_eq!(names(&cff2), [".notdef", "A", "B"]);
    let prepare = PrepareOptions { goadb: goadb_use("B B\nA A\n", false, false, true), ..PrepareOptions::default() };
    let reordered = converted(&cff2, ConvertTarget::Cff2, &prepare);
    assert_eq!(names(&reordered), [".notdef", "B", "A"]);
    assert_eq!(ps_contours(&reordered, *b"CFF2", 1), ps_contours(&cff2, *b"CFF2", 2), "B's charstring moved with it");
    assert_eq!(ps_contours(&reordered, *b"CFF2", 2), ps_contours(&cff2, *b"CFF2", 1));
}

#[test]
fn overlaps_of_cff_and_cff2_outlines_are_merged_without_a_truetype_stage() {
    let prepare = PrepareOptions { remove_overlaps: true, ..PrepareOptions::default() };
    assert_eq!(ps_contours(&overlapping_cff_font(), *b"CFF ", 1).len(), 2);
    let merged = convert_prepared(&overlapping_cff_font(), &ConvertOptions::new(ConvertTarget::Cff), &prepare).unwrap();
    let contours = ps_contours(merged.font.as_ref().unwrap(), *b"CFF ", 1);
    assert_eq!(contours.len(), 1);
    assert_eq!(contours[0].segs.len(), 8, "the union of the squares has eight corners");
    assert!(merged.left_behind.iter().any(|n| n.ends_with("whose overlaps were merged (1)")), "{:?}", merged.left_behind);
    // The width is kept: hmtx and the charstring still agree after a conversion to TrueType.
    let back = converted(merged.font.as_ref().unwrap(), ConvertTarget::TrueType { max_error: 1.0 }, &PrepareOptions::default());
    assert_eq!(glyf_points(&back, 1).len(), 1);
    // CFF2 in, CFF2 out.
    let cff2 = converted(&overlapping_cff_font(), ConvertTarget::Cff2, &PrepareOptions::default());
    assert_eq!(ps_contours(&cff2, *b"CFF2", 1).len(), 2);
    let merged = converted(&cff2, ConvertTarget::Cff2, &prepare);
    assert_eq!(ps_contours(&merged, *b"CFF2", 1).len(), 1);
    // A font without overlaps comes back as it was.
    let plain = convert_prepared(&cff_font(), &ConvertOptions::new(ConvertTarget::Cff), &prepare).unwrap();
    assert_eq!(plain.font.unwrap().get(*b"CFF "), cff_font().get(*b"CFF "));
}
