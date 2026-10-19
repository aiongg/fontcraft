//! Behaviour tests of the compile extras (M12.42) on fonts built here: `meta`, `VORG`, the stub
//! `DSIG`, stored tables, the curve modes, implied on-curve points and the subset switch.

use std::sync::Arc;

use fontcraft_model::plist::{Array, Dict, Value};
use fontcraft_model::{Font, Node, NodeKind, Param, Path, Shape};
use read_fonts::tables::glyf::Glyph as RawGlyph;
use read_fonts::types::GlyphId;
use read_fonts::{FontRef, TableProvider};

use super::*;
use crate::outline::CurveMode;
use crate::{Compiled, Flavour, Options, compile, compile_ttf};

fn options() -> Options {
    Options { remove_overlap: false, skip_features: true, ..Options::default() }
}

fn set_font_param(f: &mut Font, name: &str, value: Value) {
    Arc::make_mut(&mut f.info).params.0.push(Param::new(name, value));
}

fn table<'a>(out: &'a Compiled, tag: &[u8; 4]) -> Option<&'a [u8]> {
    FontRef::new(&out.bytes).unwrap().table_data(Tag::new(tag)).map(|d| d.as_bytes())
}

fn gid(out: &Compiled, name: &str) -> GlyphId {
    GlyphId::new(out.glyph_order.iter().position(|n| n == name).unwrap() as u32)
}

/// How many points the glyph `name` has in `glyf`.
fn point_count(out: &Compiled, name: &str) -> usize {
    let font = FontRef::new(&out.bytes).unwrap();
    match font.loca(None).unwrap().get_glyf(gid(out, name), &font.glyf().unwrap()).unwrap() {
        Some(RawGlyph::Simple(s)) => s.num_points(),
        _ => panic!("{name} is not a simple glyph"),
    }
}

/// The text of each record of a `meta` table, by tag.
fn meta_records(bytes: &[u8]) -> Vec<(String, String)> {
    let u32_at = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
    (0..u32_at(12))
        .map(|i| {
            let at = 16 + 12 * i;
            let (offset, len) = (u32_at(at + 4), u32_at(at + 8));
            (String::from_utf8(bytes[at..at + 4].to_vec()).unwrap(), String::from_utf8(bytes[offset..offset + len].to_vec()).unwrap())
        })
        .collect()
}

fn meta_entry(tag: &str, data: &str) -> Value {
    let mut d = Dict::new();
    d.insert("data", Value::String(data.into()));
    d.insert("tag", Value::String(tag.into()));
    Value::Dict(d)
}

#[test]
fn meta_records_join_each_tags_entries_as_both_reference_compilers_write_them() {
    let mut f = Font::new("Meta Sans");
    let entries = vec![
        meta_entry("dlng", "en-Latn"),
        meta_entry("slng", "Latn, Cyrl"),
        meta_entry("dlng", "sr-Cyrl"),
        meta_entry("appl", "x"),
        meta_entry("slng", "not a tag"),
    ];
    set_font_param(&mut f, META_PARAM, Value::Array(Array { items: entries, ..Default::default() }));
    let out = compile_ttf(&f, &options()).unwrap();
    // fontmake joins each tag's data with commas (glyphsLib, ufo2ft); fontc splits on commas and
    // write-fonts joins the pieces with commas: the same text, spaces kept.
    assert_eq!(
        meta_records(table(&out, b"meta").unwrap()),
        [("dlng".to_string(), "en-Latn,sr-Cyrl".to_string()), ("slng".to_string(), "Latn, Cyrl,not a tag".to_string())]
    );
    assert!(out.warnings.iter().any(|w| w.contains("`appl`")), "{:?}", out.warnings);
    assert!(out.warnings.iter().any(|w| w.contains("`not a tag`")), "{:?}", out.warnings);

    let plain = compile_ttf(&Font::new("Meta Sans"), &options()).unwrap();
    assert!(table(&plain, b"meta").is_none(), "no parameter, no table");
}

#[test]
fn language_tags_are_checked_for_their_shape() {
    for good in ["Latn", "sr-Cyrl", "sr-Cyrl-BA", "zh-Hant-TW"] {
        assert!(is_script_lang_tag(good), "{good}");
    }
    for bad in ["", "Latn,", " Latn", "sr--Cyrl", "toolongsubtag", "é"] {
        assert!(!is_script_lang_tag(bad), "{bad}");
    }
}

#[test]
fn vorg_takes_the_most_frequent_origin_as_default_and_lists_the_others() {
    assert_eq!(
        vorg_table(&[880, 700, 880, 700, 880, 640]).unwrap(),
        [0, 1, 0, 0, 0x03, 0x70, 0, 3, 0, 1, 0x02, 0xBC, 0, 3, 0x02, 0xBC, 0, 5, 0x02, 0x80]
    );
    // Equally frequent: the first in glyph order wins, as Counter.most_common picks it.
    assert_eq!(vorg_table(&[700, 880]).unwrap()[4..6], 700i16.to_be_bytes());
    assert!(vorg_table(&[]).is_none());
}

#[test]
fn a_cff_font_with_vertical_metrics_gets_a_vorg_table() {
    let mut f = Font::new("Vert Sans");
    f.glyph_mut("A").unwrap().layers[0].shapes.push(Shape::Path(Path::rect(50.0, 0.0, 550.0, 700.0)));
    f.glyph_mut("A").unwrap().layers[0].info.vert_origin = Some(40.0);
    let target = f.default_target().unwrap();
    let out = compile(&f, &target, Flavour::Cff, &options()).unwrap();
    let font = FontRef::new(&out.bytes).unwrap();
    let ascender = font.os2().unwrap().s_typo_ascender();
    let vorg = table(&out, b"VORG").unwrap();
    assert_eq!(vorg[4..6], ascender.to_be_bytes(), "the typographic ascender is every other glyph's origin");
    assert_eq!(vorg[6..8], 1u16.to_be_bytes());
    assert_eq!(vorg[8..10], (gid(&out, "A").to_u32() as u16).to_be_bytes());
    assert_eq!(vorg[10..12], (ascender - 40).to_be_bytes());

    let ttf = compile_ttf(&f, &options()).unwrap();
    assert!(table(&ttf, b"VORG").is_none(), "VORG is for CFF outlines");
}

#[test]
fn a_stub_dsig_is_written_only_when_asked() {
    let f = Font::new("Sig Sans");
    let out = compile_ttf(&f, &Options { stub_dsig: true, ..options() }).unwrap();
    assert_eq!(table(&out, b"DSIG").unwrap(), crate::makeotf::STUB_DSIG);
    assert!(table(&compile_ttf(&f, &options()).unwrap(), b"DSIG").is_none());
}

#[test]
fn stored_tables_pass_through_only_when_asked_and_while_the_glyph_order_holds() {
    let mut f = Font::new("Kept Sans");
    let order = compile_ttf(&f, &options()).unwrap().glyph_order;
    let ltsh: Vec<u8> = (0..20u8).collect();
    let jstf = b"JSTF table bytes".to_vec();
    let kept = store_tables(
        &mut f,
        &[(Tag::new(b"LTSH"), ltsh.as_slice()), (Tag::new(b"JSTF"), jstf.as_slice()), (Tag::new(b"cmap"), b"no".as_slice())],
        &order,
    );
    assert_eq!(kept, 2, "cmap is the compiler's own table, never stored");

    let out = compile_ttf(&f, &Options { pass_through_tables: true, ..options() }).unwrap();
    assert_eq!(table(&out, b"LTSH").unwrap(), ltsh.as_slice());
    assert_eq!(table(&out, b"JSTF").unwrap(), jstf.as_slice());
    assert!(table(&out, b"cmap").unwrap() != b"no");

    assert!(table(&compile_ttf(&f, &options()).unwrap(), b"LTSH").is_none(), "the normal export leaves them out");

    f.add_glyph("B.alt", vec![], 600.0).unwrap();
    let out = compile_ttf(&f, &Options { pass_through_tables: true, ..options() }).unwrap();
    assert!(table(&out, b"LTSH").is_none());
    assert!(out.warnings.iter().any(|w| w.contains("LTSH") && w.contains("glyph order")), "{:?}", out.warnings);
}

/// Two cubic arcs that are exact quadratics, (0,0) → (600,0) → (1200,0), closed by a line: the
/// node at (600,0) lies exactly halfway between the two quadratic control points.
fn arcs_font() -> Font {
    let mut f = Font::new("Arc Sans");
    let n = |x: f64, y: f64, kind| Node::new(x, y, kind);
    let nodes = vec![
        n(0.0, 0.0, NodeKind::Line),
        n(200.0, 200.0, NodeKind::OffCurve),
        n(400.0, 200.0, NodeKind::OffCurve),
        n(600.0, 0.0, NodeKind::Curve),
        n(800.0, -200.0, NodeKind::OffCurve),
        n(1000.0, -200.0, NodeKind::OffCurve),
        n(1200.0, 0.0, NodeKind::Curve),
    ];
    f.glyph_mut("A").unwrap().layers[0].shapes.push(Shape::Path(Path::new(true, nodes)));
    f
}

#[test]
fn implied_on_curve_points_are_dropped_by_default_and_nodes_kept_when_asked() {
    let f = arcs_font();
    // fontc (and fontmake --drop-implied-oncurves): (0,0), (300,300), (900,-300), (1200,0).
    assert_eq!(point_count(&compile_ttf(&f, &options()).unwrap(), "A"), 4);
    // fontmake's default keeps the node at (600,0).
    assert_eq!(point_count(&compile_ttf(&f, &Options { drop_implied_on_curves: false, ..options() }).unwrap(), "A"), 5);
}

#[test]
fn keeping_nodes_still_leaves_out_the_points_a_quadratic_spline_implies() {
    let mut f = Font::new("Quad Sans");
    let n = |x: f64, y: f64, kind| Node::new(x, y, kind);
    // One spline of three off-curve points from (0,0) to (800,0): two implied on-curve points.
    let nodes = vec![
        n(0.0, 0.0, NodeKind::Line),
        n(100.0, 300.0, NodeKind::OffCurve),
        n(400.0, 400.0, NodeKind::OffCurve),
        n(700.0, 300.0, NodeKind::OffCurve),
        n(800.0, 0.0, NodeKind::QCurve),
    ];
    f.glyph_mut("A").unwrap().layers[0].shapes.push(Shape::Path(Path::new(true, nodes)));
    for drop in [true, false] {
        let out = compile_ttf(&f, &Options { drop_implied_on_curves: drop, ..options() }).unwrap();
        assert_eq!(point_count(&out, "A"), 5, "drop {drop}");
    }
}

#[test]
fn keep_quad_writes_quadratic_outlines_unconverted_and_refuses_cubic_ones() {
    let mut f = Font::new("Quad Sans");
    let n = |x: f64, y: f64, kind| Node::new(x, y, kind);
    let nodes = vec![n(0.0, 0.0, NodeKind::Line), n(400.0, 600.0, NodeKind::OffCurve), n(800.0, 0.0, NodeKind::QCurve)];
    f.glyph_mut("A").unwrap().layers[0].shapes.push(Shape::Path(Path::new(true, nodes)));
    let keep = Options { curves: CurveMode::KeepQuad, ..options() };
    assert_eq!(point_count(&compile_ttf(&f, &keep).unwrap(), "A"), 3);

    let err = compile_ttf(&arcs_font(), &keep).unwrap_err().to_string();
    assert!(err.contains("`A`") && err.contains("cubic"), "{err}");
    // CFF keeps cubic curves whatever the mode.
    let f = arcs_font();
    assert!(compile(&f, &f.default_target().unwrap(), Flavour::Cff, &keep).is_ok());
}

#[test]
fn curve_modes_have_the_reference_compilers_names() {
    for mode in [CurveMode::Cu2Qu, CurveMode::KeepQuad] {
        assert_eq!(CurveMode::parse(mode.name()), Some(mode));
    }
    assert_eq!(CurveMode::parse("mixed"), None);
    assert_eq!(CurveMode::default(), CurveMode::Cu2Qu);
}

#[test]
fn without_subsetting_keep_and_remove_glyphs_are_ignored() {
    let mut f = Font::new("Set Sans");
    let list = |items: &[&str]| Value::Array(Array { items: items.iter().map(|s| Value::String((*s).into())).collect(), ..Default::default() });
    Arc::make_mut(&mut f.masters)[0].params.0.push(Param::new("Keep Glyphs", list(&["A", "B"])));
    Arc::make_mut(&mut f.masters)[0].params.0.push(Param::new("Remove Glyphs", list(&["B"])));
    f.glyph_mut("C").unwrap().export = false;
    let subset = compile_ttf(&f, &options()).unwrap();
    assert!(subset.glyph_order.iter().any(|n| n == "A") && !subset.glyph_order.iter().any(|n| n == "B" || n == "D"));

    let all = compile_ttf(&f, &Options { subset: false, ..options() }).unwrap();
    for name in ["A", "B", "D"] {
        assert!(all.glyph_order.iter().any(|n| n == name), "{name}");
    }
    assert!(!all.glyph_order.iter().any(|n| n == "C"), "the export flag still applies");
}
