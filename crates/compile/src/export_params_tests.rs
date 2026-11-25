//! Behaviour tests of the glyph-set parameters at export, on fonts built here.

use std::sync::Arc;

use fontcraft_features::evaluate::PairEvaluator;
use fontcraft_model::plist::{Array, Number, Value};
use fontcraft_model::{Component, Font, NodeKind, Param, Path, Shape};
use skrifa::instance::{LocationRef, Size};
use skrifa::raw::TableProvider;
use skrifa::raw::tables::glyf::Glyph as RawGlyph;
use skrifa::raw::types::{GlyphId, Tag};
use skrifa::{FontRef, MetadataProvider};

use crate::{Compiled, Flavour, Options, compile_ttf};

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

#[test]
fn pattern_lines_times_glyphs_are_capped_and_the_cap_is_reported() {
    let mut f = font();
    for i in 0..2_000 {
        f.add_glyph(&format!("g{i:04}"), vec![], 500.0).unwrap();
    }
    let mut lines = vec!["B.al*".to_string()];
    lines.extend((1..fontcraft_fontinfo::MAX_GLYPHSET_LINES).map(|i| format!("zz{i}*")));
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    set(&mut f, "Remove Glyphs", list(&refs));
    let id = f.masters[0].id.clone();
    let mut resolved = f.resolve(&fontcraft_model::Target::Master(id)).unwrap();
    let mut warnings = Vec::new();
    let started = std::time::Instant::now();
    let edits = super::apply(Some(&f), &mut resolved, &mut warnings).unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(20), "took {:?}", started.elapsed());
    assert!(edits.removed.contains("B.alt"), "the lines within the cap are applied");
    assert!(warnings.iter().any(|w| w.contains("limit of") && w.contains("Remove Glyphs")), "{warnings:?}");
}

// ---------- export-time parameters ----------

fn whole(v: i64) -> Value {
    Value::Number(Number::int(v))
}

fn real(v: f64) -> Value {
    Value::Number(Number::float(v).unwrap())
}

fn compile_with(f: &Font, flavour: Flavour, options: &Options) -> Compiled {
    crate::compile(f, &f.default_target().unwrap(), flavour, options).unwrap()
}

fn cff(f: &Font) -> Compiled {
    compile_with(f, Flavour::Cff, &Options::default())
}

/// The `post` version and, when it holds names, the name of the glyph after `.notdef`.
fn post(out: &Compiled) -> (u16, Option<String>) {
    let font = FontRef::new(&out.bytes).unwrap();
    let post = font.post().unwrap();
    (post.version().to_major_minor().0, post.glyph_name(skrifa::raw::types::GlyphId16::new(1)).map(str::to_string))
}

#[test]
fn post_table_type_chooses_the_post_version_and_a_cff_font_then_keeps_its_names() {
    let plain = font();
    assert_eq!(post(&compile(&plain)).0, 2);
    assert_eq!(post(&cff(&plain)).0, 3);

    let mut three = font();
    set(&mut three, "Post Table Type", whole(3));
    assert_eq!(post(&compile(&three)).0, 3);
    let named = compile_with(&three, Flavour::TrueType, &Options { post_format: Some(2), ..Options::default() });
    assert_eq!(post(&named).0, 2, "the compile's own option wins");

    let mut two = font();
    set(&mut two, "Post Table Type", whole(2));
    let out = cff(&two);
    assert_eq!(post(&out), (2, Some(out.glyph_order[1].clone())));

    let mut bad = font();
    set(&mut bad, "Post Table Type", whole(4));
    let out = compile(&bad);
    assert_eq!(post(&out).0, 2);
    assert!(out.warnings.iter().any(|w| w.contains("Post Table Type")), "{:?}", out.warnings);
}

#[test]
fn web_font_parameters_leave_out_glyph_names_and_desktop_data() {
    let web = Options { web_container: true, ..Options::default() };
    let mut f = font();
    set(&mut f, "Remove post names for webfonts", whole(1));
    assert_eq!(post(&compile(&f)).0, 2, "a plain file keeps the names");
    assert_eq!(post(&compile_with(&f, Flavour::TrueType, &web)).0, 3);

    let mut f = font();
    Arc::make_mut(&mut f.info).properties.set("copyrights", "(c) Set", false);
    Arc::make_mut(&mut f.info).properties.set("designers", "Ada", false);
    let ids = |out: &Compiled| -> Vec<u16> {
        let font = FontRef::new(&out.bytes).unwrap();
        font.name().unwrap().name_record().iter().map(|r| r.name_id().to_u16()).collect()
    };
    let fs_type = |out: &Compiled| FontRef::new(&out.bytes).unwrap().os2().unwrap().fs_type();
    let plain = compile(&f);
    assert!(ids(&plain).contains(&0) && ids(&plain).contains(&9), "{:?}", ids(&plain));
    assert_ne!(fs_type(&plain), 2);
    set(&mut f, "Webfont Only", whole(1));
    let out = compile_with(&f, Flavour::TrueType, &web);
    assert!(ids(&out).iter().all(|id| ![0, 7, 8, 9, 11, 12, 13, 14].contains(id)), "{:?}", ids(&out));
    assert!([1, 2, 4, 6].iter().all(|id| ids(&out).contains(id)));
    assert_eq!((fs_type(&out), post(&out).0), (2, 3));
    assert!(!out.warnings.iter().any(|w| w.contains("Webfont Only")), "{:?}", out.warnings);
    assert!(compile(&f).warnings.iter().any(|w| w.contains("Webfont Only")), "a plain file says that it cannot be installed");
}

#[test]
fn truetype_curve_error_is_in_units_and_gives_way_to_the_compile_option() {
    // Four curves in an oval.
    let mut f = font();
    let n = fontcraft_model::Node::new;
    let (cx, cy, rx, ry, k) = (350.0, 350.0, 300.0, 360.0, 0.5523);
    let curve = [(1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0)];
    let mut nodes = Vec::new();
    for (i, (x, y)) in curve.iter().enumerate() {
        let (nx, ny) = curve[(i + 1) % 4];
        nodes.push(n(cx + rx * x, cy + ry * y, NodeKind::Curve));
        nodes.push(n(cx + rx * (x + k * nx), cy + ry * (y + k * ny), NodeKind::OffCurve));
        nodes.push(n(cx + rx * (nx + k * x), cy + ry * (ny + k * y), NodeKind::OffCurve));
    }
    f.glyph_mut("O").unwrap().layers[0].shapes.push(Shape::Path(Path { nodes, closed: true, ..Path::rect(0.0, 0.0, 1.0, 1.0) }));
    f.units_per_em = 2000;
    let points = |out: &Compiled| {
        let font = FontRef::new(&out.bytes).unwrap();
        match raw(&font, gid(out, "O")) {
            Some(RawGlyph::Simple(g)) => g.num_points(),
            _ => 0,
        }
    };
    let plain = compile(&f);
    let with = |v: f64| {
        let mut f = f.clone();
        set(&mut f, "TrueType Curve Error", real(v));
        f
    };
    assert_eq!(compile(&with(2.0)).bytes, plain.bytes, "2 units of 2,000 are the default of 0.001 em");
    let (fine, coarse) = (compile(&with(0.05)), compile(&with(20.0)));
    assert!(points(&fine) > points(&plain) && points(&plain) > points(&coarse), "{} {} {}", points(&fine), points(&plain), points(&coarse));
    let option = Options { conversion_error: Some(0.001), ..Options::default() };
    assert_eq!(compile_with(&with(0.05), Flavour::TrueType, &option).bytes, plain.bytes);
}

#[test]
fn truetype_keep_glyph_order_fixes_the_first_two_slots_only() {
    let slot = |out: &Compiled| out.glyph_order.iter().position(|n| n == "space");
    let mut f = font();
    Arc::make_mut(&mut f.info).params.0.push(Param::new("glyphOrder", list(&["A", "space", "a"])));
    assert_eq!(compile(&f).glyph_order[..4], [".notdef", "A", "space", "a"], "the source's order");
    set(&mut f, "TrueType Keep GlyphOrder", whole(1));
    assert_eq!(compile(&f).glyph_order[..4], [".notdef", "space", "A", "a"]);
    assert_eq!(slot(&cff(&f)), Some(2), "TrueType only");
    set(&mut f, "DisableAllAutomaticBehaviour", whole(1));
    assert_eq!(compile(&f).glyph_order[..4], [".notdef", "A", "space", "a"], "no glyph is moved but .notdef");
}

#[test]
fn export_vmtx_table_forces_and_suppresses_the_vertical_tables() {
    let has = |out: &Compiled, tag: &[u8; 4]| FontRef::new(&out.bytes).unwrap().table_data(Tag::new(tag)).is_some();
    let mut f = font();
    assert!(!has(&compile(&f), b"vmtx"));
    set(&mut f, "Export vmtx Table", whole(1));
    for out in [compile(&f), cff(&f)] {
        assert!(has(&out, b"vmtx") && has(&out, b"vhea"));
    }
    assert!(has(&cff(&f), b"VORG"));

    let mut f = font();
    f.glyph_mut("A").unwrap().layers[0].info.vert_width = Some(900.0);
    assert!(has(&compile(&f), b"vmtx"));
    set(&mut f, "Export vmtx Table", whole(0));
    for (out, flavour) in [(compile(&f), Flavour::TrueType), (cff(&f), Flavour::Cff)] {
        assert!(!has(&out, b"vmtx") && !has(&out, b"vhea") && !has(&out, b"VORG"));
        let plan = crate::plan::plan(&f, &f.default_target().unwrap(), flavour, &Options::default()).unwrap();
        assert!(!plan.tables.iter().any(|t| ["vmtx", "vhea", "VORG"].contains(&t.as_str())), "{:?}", plan.tables);
    }
}

#[test]
fn a_grid_finer_than_a_unit_keeps_hundredths_in_cff_outlines() {
    let mut base = font();
    rect(&mut base, "B", 60.256, 0.5, 560.0, 700.75);
    let draw = |out: &Compiled| {
        let font = FontRef::new(&out.bytes).unwrap();
        let mut pen = points_pen::Points::default();
        // skrifa draws CFF outlines in whole units, so those are read from the table.
        if let Some(table) = font.table_data(skrifa::Tag::new(b"CFF ")) {
            let cff = read_fonts::ps::cff::CffFontRef::new_cff(table.as_bytes(), 0, None).unwrap();
            cff.evaluate_charstring(&cff.subfont(0, &[]).unwrap(), gid(out, "B"), &[], &mut pen).unwrap();
        } else {
            let glyph = font.outline_glyphs().get(gid(out, "B")).unwrap();
            glyph.draw(skrifa::outline::DrawSettings::unhinted(Size::unscaled(), LocationRef::default()), &mut pen).unwrap();
        }
        pen.0
    };
    let left = |out: &Compiled| draw(out).iter().map(|p| p.0).fold(f32::MAX, f32::min);
    let top = |out: &Compiled| draw(out).iter().map(|p| p.1).fold(f32::MIN, f32::max);
    assert_eq!(left(&cff(&base)), 60.0, "an integer grid rounds");

    // Grid spacing 0, and 1 over a subdivision of 10.
    for (spacing, subdivision) in [(Some(0.0), None), (None, Some(10.0))] {
        let mut f = base.clone();
        let s = &mut Arc::make_mut(&mut f.info).settings;
        (s.grid_length, s.grid_sub_division) = (spacing, subdivision);
        let out = cff(&f);
        assert!((left(&out) - 60.26).abs() < 0.001 && (top(&out) - 700.75).abs() < 0.001, "{} {}", left(&out), top(&out));
        assert_eq!(left(&compile(&f)), 60.0, "TrueType coordinates are whole");
        let whole_units = Options { cff: crate::CffOptions { round_tolerance: Some(0.5), ..Default::default() }, ..Options::default() };
        assert_eq!(left(&compile_with(&f, Flavour::Cff, &whole_units)), 60.0, "the compile's own tolerance wins");
        set(&mut f, "Grid Spacing", whole(1));
        assert_eq!(left(&cff(&f)), 60.0, "the parameter replaces the grid");
    }
    set(&mut base, "Grid Spacing", real(0.5));
    assert!((left(&cff(&base)) - 60.26).abs() < 0.001);
}

mod points_pen {
    /// Every point a pen is given.
    #[derive(Default)]
    pub struct Points(pub Vec<(f32, f32)>);

    impl skrifa::outline::OutlinePen for Points {
        fn move_to(&mut self, x: f32, y: f32) {
            self.0.push((x, y));
        }
        fn line_to(&mut self, x: f32, y: f32) {
            self.0.push((x, y));
        }
        fn quad_to(&mut self, _: f32, _: f32, x: f32, y: f32) {
            self.0.push((x, y));
        }
        fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, x: f32, y: f32) {
            self.0.push((x, y));
        }
        fn close(&mut self) {}
    }
}

#[test]
fn disable_all_automatic_behaviour_keeps_mark_widths_stored_code_and_own_anchors() {
    let mut f = font();
    f.glyph_mut("acutecomb").unwrap().layers[0].width = 300.0;
    let mut features = fontcraft_model::Features::default();
    features.classes.push(fontcraft_model::Snippet { name: "Uppercase".into(), code: "A B".into(), automatic: true, ..Default::default() });
    let code = "sub A by B;".to_string();
    features.features.push(fontcraft_model::Snippet { name: "ss01".into(), code, ..Default::default() });
    f.features = Arc::new(features);
    f.glyph_mut("A").unwrap().layers[0].anchors.push(fontcraft_model::Anchor::new("top", 310.0, 700.0));
    f.glyph_mut("acutecomb").unwrap().layers[0].anchors.push(fontcraft_model::Anchor::new("_top", -100.0, 700.0));
    let attached = |out: &Compiled, name: &str| {
        let font = FontRef::new(&out.bytes).unwrap();
        let list = font.gpos().unwrap().lookup_list().unwrap();
        list.lookups().iter().any(|l| match l.unwrap().subtables().unwrap() {
            skrifa::raw::tables::gpos::PositionSubtables::MarkToBase(tables) => {
                tables.iter().any(|t| t.unwrap().base_coverage().unwrap().iter().any(|g| GlyphId::from(g) == gid(out, name)))
            }
            _ => false,
        })
    };
    let out = compile(&f);
    assert_eq!(advance(&out, "acutecomb"), Some(0.0));
    assert!(out.feature_code.as_deref().unwrap().contains("Aacute"), "{:?}", out.feature_code);
    assert!(attached(&out, "A") && attached(&out, "Abreveacute"));

    set(&mut f, "DisableAllAutomaticBehaviour", whole(1));
    let out = compile(&f);
    assert_eq!(advance(&out, "acutecomb"), Some(300.0));
    let code = out.feature_code.as_deref().unwrap();
    assert!(!code.contains("Aacute") && code.contains("A B"), "{code}");
    assert!(attached(&out, "A") && !attached(&out, "Abreveacute"));
}
