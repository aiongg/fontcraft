#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_model::plist::{Dict, Value};
use fontcraft_model::{Anchor, Font, Path, Shape};
use skrifa::FontRef;
use skrifa::raw::TableProvider;
use skrifa::raw::types::GlyphId16;

use super::*;
use crate::{Compiled, Flavour, Options, compile, compile_ttf};

/// An italic `f` with math metrics, and an integral that is an extended shape with a larger size
/// and a three-part assembly whose top part has an italic correction.
fn math_font() -> Font {
    let mut f = Font::new("Math Sans");
    for (name, cps, width, (x0, y0, x1, y1)) in [
        ("f", vec![0x66], 500.0, (50.0, 0.0, 450.0, 700.0)),
        ("integral", vec![0x222B], 500.0, (100.0, -200.0, 400.0, 800.0)),
        ("integral.size1", vec![], 500.0, (100.0, -400.0, 400.0, 1200.0)),
        ("integral.bot", vec![], 400.0, (0.0, 0.0, 300.0, 500.0)),
        ("integral.ext", vec![], 400.0, (0.0, 0.0, 300.0, 200.0)),
        ("integral.top", vec![], 400.0, (0.0, 0.0, 300.0, 500.0)),
    ] {
        f.add_glyph(name, cps, width).unwrap();
        f.glyph_mut(name).unwrap().layers[0].shapes.push(Shape::Path(Path::rect(x0, y0, x1, y1)));
    }
    {
        let mut g = f.glyph_mut("f").unwrap();
        let anchors = &mut g.layers[0].anchors;
        anchors.push(Anchor::new(ITALIC_ANCHOR, 550.0, 700.0));
        anchors.push(Anchor::new(TOP_ACCENT_ANCHOR, 250.0, 700.0));
        anchors.push(Anchor::new("math.tr2", 520.0, 500.0));
        anchors.push(Anchor::new("math.tr1", 510.0, 100.0));
        anchors.push(Anchor::new("math.bl1", 30.0, -50.0));
    }
    f.glyph_mut("integral.top").unwrap().layers[0].anchors.push(Anchor::new(ITALIC_ANCHOR, 430.0, 500.0));
    {
        let mut g = f.glyph_mut("integral").unwrap();
        g.info.user_data.0.insert(EXTENDED_SHAPE_KEY, Value::Bool(true));
        let v = GlyphVariants {
            v_variants: vec!["integral".into(), "integral.size1".into(), "integral.missing".into()],
            v_assembly: vec![
                Part { glyph: "integral.bot".into(), flags: 0, start: 0.0, end: 100.0 },
                Part { glyph: "integral.ext".into(), flags: 1, start: 100.0, end: 100.0 },
                Part { glyph: "integral.top".into(), flags: 0, start: 100.0, end: 0.0 },
            ],
            ..Default::default()
        };
        g.info.user_data.0.insert(VARIANTS_KEY, v.to_plist());
    }
    let mut constants = Dict::new();
    for (name, v) in [("ScriptPercentScaleDown", 70.0), ("AxisHeight", 250.4), ("DisplayOperatorMinHeight", 1300.0), (MIN_CONNECTOR_OVERLAP, 20.0)] {
        constants.insert(name, number_value(v));
    }
    Arc::make_mut(&mut f.masters)[0].user_data.0.insert(CONSTANTS_KEY, Value::Dict(constants));
    f
}

fn gid(out: &Compiled, name: &str) -> GlyphId16 {
    GlyphId16::new(u16::try_from(out.glyph_order.iter().position(|n| n == name).unwrap()).unwrap())
}

#[test]
fn stored_variants_read_back_equal() {
    let f = math_font();
    let master = f.masters[0].id.clone();
    let v = stored_variants(&f, &master, "integral");
    assert_eq!(v.v_variants, ["integral", "integral.size1", "integral.missing"]);
    assert_eq!(v.v_assembly.len(), 3);
    assert_eq!(GlyphVariants::from_plist(&v.to_plist()), v);
    assert!(is_extended_shape(&f, &master, "integral"));
    assert!(!is_extended_shape(&f, &master, "f"));
    // A part that is not four values makes ufo2ft refuse the assembly; it is left out here.
    let mut d = Dict::new();
    d.insert("vAssembly", Value::Array(fontcraft_model::plist::Array { items: vec![Value::String("x".into())], ..Default::default() }));
    assert!(GlyphVariants::from_plist(&Value::Dict(d)).v_assembly.is_empty());
}

#[test]
fn compiled_math_table_reads_back_equal() {
    for flavour in [Flavour::TrueType, Flavour::Cff] {
        let font = math_font();
        let out = compile(&font, &font.default_target().unwrap(), flavour, &Options::default()).unwrap();
        assert!(out.warnings.iter().any(|w| w.contains("integral.missing")), "{:?}", out.warnings);
        let file = FontRef::new(&out.bytes).unwrap();
        let math = file.math().unwrap();

        let c = math.math_constants().unwrap();
        assert_eq!(c.script_percent_scale_down(), 70);
        assert_eq!(c.axis_height().value().to_i16(), 250);
        assert_eq!(c.display_operator_min_height().to_u16(), 1300);
        assert_eq!(c.math_leading().value().to_i16(), 0);

        let info = math.math_glyph_info().unwrap();
        // `integral.top`'s italic correction went to the assembly, so only `f` keeps one.
        let ic = info.math_italics_correction_info().unwrap().unwrap();
        let cov = ic.coverage().unwrap();
        assert_eq!(cov.iter().collect::<Vec<_>>(), [gid(&out, "f")]);
        assert_eq!(ic.italics_correction()[0].value().to_i16(), 50);
        let ta = info.math_top_accent_attachment().unwrap().unwrap();
        assert_eq!(ta.top_accent_coverage().unwrap().get(gid(&out, "f")), Some(0));
        assert_eq!(ta.top_accent_attachment()[0].value().to_i16(), 250);
        let ext = info.extended_shape_coverage().unwrap().unwrap();
        let mut shapes: Vec<GlyphId16> = ext.iter().collect();
        shapes.sort();
        let mut expected = vec![gid(&out, "integral"), gid(&out, "integral.size1")];
        expected.sort();
        assert_eq!(shapes, expected);

        let kerns = info.math_kern_info().unwrap().unwrap();
        assert_eq!(kerns.math_kern_coverage().unwrap().get(gid(&out, "f")), Some(0));
        let record = &kerns.math_kern_info_records()[0];
        let tr = record.top_right_math_kern(kerns.offset_data()).unwrap().unwrap();
        // Sorted by height; the last height is dropped.
        assert_eq!(tr.correction_height().iter().map(|r| r.value().to_i16()).collect::<Vec<_>>(), [100]);
        assert_eq!(tr.kern_values().iter().map(|r| r.value().to_i16()).collect::<Vec<_>>(), [10, 20]);
        let bl = record.bottom_left_math_kern(kerns.offset_data()).unwrap().unwrap();
        assert_eq!(bl.kern_values().iter().map(|r| r.value().to_i16()).collect::<Vec<_>>(), [-30]);
        assert!(record.top_left_math_kern(kerns.offset_data()).is_none());

        let variants = math.math_variants().unwrap();
        assert_eq!(variants.min_connector_overlap().to_u16(), 20);
        assert!(variants.horiz_glyph_coverage().is_none());
        let vcov = variants.vert_glyph_coverage().unwrap().unwrap();
        assert_eq!(vcov.get(gid(&out, "integral")), Some(0));
        let construction = variants.vert_glyph_constructions().get(0).unwrap();
        let sizes: Vec<(GlyphId16, u16)> =
            construction.math_glyph_variant_records().iter().map(|r| (r.variant_glyph(), r.advance_measurement().to_u16())).collect();
        assert_eq!(sizes, [(gid(&out, "integral"), 1000), (gid(&out, "integral.size1"), 1600)]);
        let assembly = construction.glyph_assembly().unwrap().unwrap();
        assert_eq!(assembly.italics_correction().value().to_i16(), 30);
        let parts: Vec<(GlyphId16, u16, u16, u16, u16)> = assembly
            .part_records()
            .iter()
            .map(|p| {
                (
                    p.glyph_id(),
                    p.start_connector_length().to_u16(),
                    p.end_connector_length().to_u16(),
                    p.full_advance().to_u16(),
                    p.part_flags().bits(),
                )
            })
            .collect();
        assert_eq!(
            parts,
            [(gid(&out, "integral.bot"), 0, 100, 500, 0), (gid(&out, "integral.ext"), 100, 100, 200, 1), (gid(&out, "integral.top"), 100, 0, 500, 0),]
        );
        assert!(
            crate::plan::plan(&font, &font.default_target().unwrap(), flavour, &Options::default()).unwrap().tables.contains(&"MATH".to_string())
        );
    }
}

#[test]
fn a_font_without_math_data_has_no_math_table() {
    let mut f = Font::new("Plain");
    f.add_glyph("a", vec![0x61], 500.0).unwrap();
    f.glyph_mut("a").unwrap().layers[0].anchors.push(Anchor::new(ITALIC_ANCHOR, 550.0, 0.0));
    let out = compile_ttf(&f, &Options::default()).unwrap();
    assert!(FontRef::new(&out.bytes).unwrap().math().is_err());
}

#[test]
fn a_base_block_in_the_feature_code_is_written() {
    let mut f = math_font();
    let code = "table BASE {\n  HorizAxis.BaseTagList ideo romn;\n  HorizAxis.BaseScriptList latn romn -120 0;\n  HorizAxis.MinMax latn dflt -300, 900;\n} BASE;\n";
    Arc::make_mut(&mut f.features).prefixes.push(fontcraft_model::Snippet { name: "Baselines".into(), code: code.into(), ..Default::default() });
    let out = compile_ttf(&f, &Options::default()).unwrap();
    let base = FontRef::new(&out.bytes).unwrap().base().unwrap();
    let axis = base.horiz_axis().unwrap().unwrap();
    let tags: Vec<String> = axis.base_tag_list().unwrap().unwrap().baseline_tags().iter().map(|t| t.get().to_string()).collect();
    assert_eq!(tags, ["ideo", "romn"]);
}
