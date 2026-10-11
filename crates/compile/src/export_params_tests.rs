//! Behaviour tests of the glyph-set parameters at export, on fonts built here.

use std::sync::Arc;

use fontcraft_features::evaluate::PairEvaluator;
use fontcraft_model::plist::{Array, Number, Value};
use fontcraft_model::{Component, Font, Param, Path, Shape};
use skrifa::instance::{LocationRef, Size};
use skrifa::raw::TableProvider;
use skrifa::raw::tables::glyf::Glyph as RawGlyph;
use skrifa::raw::types::{GlyphId, Tag};
use skrifa::{FontRef, MetadataProvider};

use crate::{Compiled, Options, compile_ttf};

fn list(items: &[&str]) -> Value {
    Value::Array(Array { items: items.iter().map(|s| Value::String((*s).into())).collect(), ..Default::default() })
}

/// Sets a parameter on the master, which the compile of the first master reads.
fn set(f: &mut Font, name: &str, value: Value) {
    Arc::make_mut(&mut f.masters)[0].params.0.push(Param::new(name, value));
}

fn rect(f: &mut Font, glyph: &str, x0: f64, y0: f64, x1: f64, y1: f64) {
    f.glyph_mut(glyph).unwrap().layers[0].shapes.push(Shape::Path(Path::rect(x0, y0, x1, y1)));
}

fn component(f: &mut Font, glyph: &str, base: &str, dx: f64) {
    f.glyph_mut(glyph).unwrap().layers[0].shapes.push(Shape::Component(Component::new(base, dx, 0.0)));
}

/// `.notdef`, `space`, A–Z and a–z, plus accented, alternate and mark glyphs.
fn font() -> Font {
    let mut f = Font::new("Set Sans");
    rect(&mut f, "A", 60.0, 0.0, 560.0, 700.0);
    rect(&mut f, "a", 50.0, 0.0, 450.0, 500.0);
    f.add_glyph("acutecomb", vec![0x301], 0.0).unwrap();
    rect(&mut f, "acutecomb", -150.0, 720.0, -50.0, 820.0);
    f.add_glyph("Aacute", vec![0xC1], 620.0).unwrap();
    component(&mut f, "Aacute", "A", 0.0);
    component(&mut f, "Aacute", "acutecomb", 360.0);
    f.add_glyph("Abreveacute", vec![0x1EAE], 620.0).unwrap();
    component(&mut f, "Abreveacute", "A", 0.0);
    for name in ["A.alt", "B.alt"] {
        f.add_glyph(name, vec![], 600.0).unwrap();
        rect(&mut f, name, 50.0, 0.0, 550.0, 700.0);
    }
    f.glyph_mut("A.alt").unwrap().export = false;
    f
}

fn compile(f: &Font) -> Compiled {
    compile_ttf(f, &Options::default()).unwrap()
}

fn has(out: &Compiled, name: &str) -> bool {
    out.glyph_order.iter().any(|n| n == name)
}

fn gid(out: &Compiled, name: &str) -> GlyphId {
    GlyphId::new(out.glyph_order.iter().position(|n| n == name).unwrap() as u32)
}

/// The glyph each code point maps to, by name.
fn mapped(out: &Compiled, cp: u32) -> Option<String> {
    let font = FontRef::new(&out.bytes).unwrap();
    let gid = font.charmap().map(cp)?;
    out.glyph_order.get(gid.to_u32() as usize).cloned()
}

fn raw<'a>(font: &FontRef<'a>, gid: GlyphId) -> Option<RawGlyph<'a>> {
    font.loca(None).unwrap().get_glyf(gid, &font.glyf().unwrap()).unwrap()
}

/// The glyph ids a composite uses; `None` for a simple glyph.
fn components_of(out: &Compiled, name: &str) -> Option<Vec<u32>> {
    let font = FontRef::new(&out.bytes).unwrap();
    match raw(&font, gid(out, name)) {
        Some(RawGlyph::Composite(c)) => Some(c.components().map(|c| c.glyph.to_u32()).collect()),
        _ => None,
    }
}

/// The left edge of the glyph's drawing.
fn x_min(out: &Compiled, name: &str) -> Option<f32> {
    let font = FontRef::new(&out.bytes).unwrap();
    font.glyph_metrics(Size::unscaled(), LocationRef::default()).bounds(gid(out, name)).map(|b| b.x_min)
}

fn advance(out: &Compiled, name: &str) -> Option<f32> {
    let font = FontRef::new(&out.bytes).unwrap();
    font.glyph_metrics(Size::unscaled(), LocationRef::default()).advance_width(gid(out, name))
}

#[test]
fn remove_glyphs_leaves_out_names_patterns_and_queries_and_decomposes_a_removed_base() {
    let plain = compile(&font());
    assert!(has(&plain, "A") && has(&plain, "Aacute") && has(&plain, "B.alt") && !has(&plain, "A.alt"));

    // A removed base: the composites that use it keep its drawing, decomposed.
    let mut f = font();
    set(&mut f, "Remove Glyphs", list(&["A"]));
    let out = compile(&f);
    assert!(!has(&out, "A") && has(&out, "Aacute"), "{:?}", out.glyph_order);
    assert_eq!(mapped(&out, 0x41), None);
    assert_eq!(mapped(&out, 0xC1).as_deref(), Some("Aacute"));
    assert_eq!(x_min(&out, "Aacute"), Some(60.0), "Aacute keeps A's drawing");

    for (entry, gone, kept) in [
        ("*.alt", &["B.alt"][..], &["A", "Aacute"][..]),
        ("unicode=1E*", &["Abreveacute"], &["A", "Aacute"]),
        ("category=Mark", &["acutecomb"], &["A", "Aacute"]),
        ("name=A*", &["A", "Aacute", "Abreveacute"], &["B", "a"]),
    ] {
        let mut f = font();
        set(&mut f, "Remove Glyphs", list(&[entry]));
        let out = compile(&f);
        for g in gone {
            assert!(!has(&out, g), "{entry} removes {g}");
        }
        for g in kept {
            assert!(has(&out, g), "{entry} keeps {g}");
        }
    }

    // `export=0` drops the components of glyphs that do not export instead of decomposing them.
    let mut f = font();
    f.add_glyph("Adot", vec![0x1E00], 620.0).unwrap();
    component(&mut f, "Adot", "A", 0.0);
    component(&mut f, "Adot", "A.alt", 0.0);
    let out = compile(&f);
    assert_eq!(x_min(&out, "Adot"), Some(50.0), "without the query the alternate is decomposed into Adot");
    set(&mut f, "Remove Glyphs", list(&["export=0"]));
    let out = compile(&f);
    assert_eq!(components_of(&out, "Adot"), Some(vec![gid(&out, "A").to_u32()]));
    assert_eq!(x_min(&out, "Adot"), Some(60.0));
    assert!(out.warnings.iter().any(|w| w.contains("export=0")), "{:?}", out.warnings);
}

#[test]
fn keep_glyphs_keeps_only_its_glyphs_and_remove_glyphs_then_removes_from_them() {
    let mut f = font();
    Arc::make_mut(&mut f.kerning).ltr.set("m01", "A", "V", -80.0);
    set(&mut f, "Keep Glyphs", list(&["A", "a"]));
    let out = compile(&f);
    let mut order = out.glyph_order.clone();
    order.sort();
    assert_eq!(order, [".notdef", "A", "a", "space"]);
    assert!(FontRef::new(&out.bytes).unwrap().gpos().is_err(), "the pair with the dropped V is gone");

    set(&mut f, "Remove Glyphs", list(&["a"]));
    let out = compile(&f);
    let mut order = out.glyph_order.clone();
    order.sort();
    assert_eq!(order, [".notdef", "A", "space"]);
    assert!(out.warnings.iter().any(|w| w.contains("Keep Glyphs") && w.contains("Remove Glyphs")), "{:?}", out.warnings);
}

#[test]
fn export_glyphs_exports_a_glyph_set_not_to_export() {
    let mut f = font();
    set(&mut f, "Export Glyphs", list(&["A.alt"]));
    assert!(has(&compile(&f), "A.alt"));
}

/// `g` and a non-exporting `g.single` of another width, `gbreve` built on `g`, and kerning on both.
fn g_font() -> Font {
    let mut f = font();
    f.glyph_mut("g").unwrap().layers[0].width = 500.0;
    rect(&mut f, "g", 50.0, -200.0, 450.0, 500.0);
    f.add_glyph("g.single", vec![], 450.0).unwrap();
    rect(&mut f, "g.single", 40.0, -200.0, 410.0, 500.0);
    f.glyph_mut("g.single").unwrap().export = false;
    f.add_glyph("gbreve", vec![0x11F], 500.0).unwrap();
    component(&mut f, "gbreve", "g", 0.0);
    rect(&mut f, "T", 0.0, 0.0, 600.0, 700.0);
    let kerning = Arc::make_mut(&mut f.kerning);
    kerning.ltr.set("m01", "g", "T", -50.0);
    kerning.ltr.set("m01", "g.single", "T", -20.0);
    f
}

#[test]
fn rename_glyphs_swaps_names_and_flags_while_code_points_stay_and_composites_and_kerning_follow() {
    // The export flags travel with the glyphs: the drawing now called `g` does not export.
    let mut f = g_font();
    set(&mut f, "Rename Glyphs", list(&["g.single=g"]));
    let out = compile(&f);
    assert!(!has(&out, "g") && has(&out, "g.single"), "{:?}", out.glyph_order);
    assert_eq!(advance(&out, "g.single"), Some(500.0));

    set(&mut f, "Export Glyphs", list(&["g"]));
    let out = compile(&f);
    assert_eq!(mapped(&out, 0x67).as_deref(), Some("g"), "the code point stays with the name");
    assert_eq!(advance(&out, "g"), Some(450.0), "`g` has the single-storey drawing");
    assert_eq!(advance(&out, "g.single"), Some(500.0));
    assert_eq!(components_of(&out, "gbreve"), Some(vec![gid(&out, "g.single").to_u32()]), "gbreve keeps its drawing");
    let eval = PairEvaluator::new(&out.bytes).unwrap();
    let id = |n: &str| gid(&out, n).to_u32() as u16;
    let latn = Tag::new(b"latn");
    assert_eq!(eval.value(latn, id("g.single"), id("T")), -50, "the kerning follows the drawing");
    assert_eq!(eval.value(latn, id("g"), id("T")), -20);
}

#[test]
fn rename_glyphs_to_a_missing_name_renames_alone_and_skips_a_missing_old_name() {
    let mut f = font();
    set(&mut f, "Rename Glyphs", list(&["a=nonexistent", "zzz=b"]));
    let out = compile(&f);
    assert!(!has(&out, "a") && has(&out, "nonexistent") && has(&out, "b"), "{:?}", out.glyph_order);
    assert_eq!(mapped(&out, 0x61).as_deref(), Some("nonexistent"));
    assert!(out.warnings.iter().any(|w| w.contains("without a swap") && w.contains("nonexistent")), "{:?}", out.warnings);
    assert!(out.warnings.iter().any(|w| w.contains("`zzz`")), "{:?}", out.warnings);
}

#[test]
fn reencode_glyphs_sets_code_points_and_takes_them_from_other_glyphs() {
    let mut f = font();
    set(&mut f, "Reencode Glyphs", list(&["a=0061,0251", "space=", "b=0041", "missing=0042"]));
    let out = compile(&f);
    assert_eq!(mapped(&out, 0x61).as_deref(), Some("a"));
    assert_eq!(mapped(&out, 0x251).as_deref(), Some("a"));
    assert_eq!(mapped(&out, 0x20), None);
    assert_eq!(mapped(&out, 0x41).as_deref(), Some("b"), "U+0041 was taken from A");
    assert_eq!(mapped(&out, 0x42).as_deref(), Some("B"));
    assert!(has(&out, "A") && has(&out, "space"));
    assert!(out.warnings.iter().any(|w| w.contains("`missing`")), "{:?}", out.warnings);
}

#[test]
fn arabic_presentation_forms_encode_unencoded_positional_forms_and_ligatures() {
    let mut f = font();
    for (name, cps) in [
        ("beh-ar", vec![0x628]),
        ("beh-ar.init", vec![]),
        ("beh-ar.medi", vec![]),
        ("beh-ar.isol", vec![]),
        ("uniFE8F", vec![0xFE8F]),
        ("beh-ar.fina", vec![0xE000]),
        ("lam_alef-ar", vec![]),
    ] {
        f.add_glyph(name, cps, 400.0).unwrap();
    }
    assert_eq!(mapped(&compile(&f), 0xFE91), None);
    set(&mut f, "Use Arabic Presentation Form Unicodes", Value::Number(Number::int(1)));
    let out = compile(&f);
    assert_eq!(mapped(&out, 0xFE91).as_deref(), Some("beh-ar.init"));
    assert_eq!(mapped(&out, 0xFE92).as_deref(), Some("beh-ar.medi"));
    assert_eq!(mapped(&out, 0xFEFB).as_deref(), Some("lam_alef-ar"));
    assert_eq!(mapped(&out, 0x628).as_deref(), Some("beh-ar"));
    assert_eq!(mapped(&out, 0xFE8F).as_deref(), Some("uniFE8F"), "a code point another glyph has is not given again");
    assert_eq!(mapped(&out, 0xFE90), None, "a glyph with a code point keeps it");
    assert_eq!(mapped(&out, 0xE000).as_deref(), Some("beh-ar.fina"));
}

#[test]
fn hostile_and_unknown_entries_are_reported_and_the_font_is_still_written() {
    let plain = compile(&font());
    let many = vec!["A"; 100_000];
    let stars = "*".repeat(10_000);
    for value in [list(&many), list(&[stars.as_str()]), Value::Number(Number::int(3))] {
        let mut f = font();
        set(&mut f, "Remove Glyphs", value);
        let out = compile(&f);
        assert_eq!(out.glyph_order, plain.glyph_order);
        assert!(out.warnings.iter().any(|w| w.contains("`Remove Glyphs` was not applied")), "{:?}", out.warnings);
    }

    let mut f = font();
    set(&mut f, "Remove Glyphs", list(&["weight=bold", "colorIndex=2", "B.alt"]));
    let out = compile(&f);
    assert!(!has(&out, "B.alt") && has(&out, "A"));
    assert!(out.warnings.iter().any(|w| w.contains("unknown query keys") && w.contains("weight")), "{:?}", out.warnings);
    assert!(out.warnings.iter().any(|w| w.contains("colorIndex=2")), "{:?}", out.warnings);
}
