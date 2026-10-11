#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_features::evaluate::PairEvaluator;
use fontcraft_model::{Anchor, Component, Font, Path, Shape, Snippet};
use skrifa::FontRef;
use skrifa::raw::TableProvider;
use skrifa::raw::tables::gpos::PositionSubtables;
use skrifa::raw::types::{GlyphId16, Tag};

use crate::{Compiled, Flavour, Options, compile_ttf, plan};

/// A base with a top anchor, a stacking mark, the composite of the two (whose anchors come from
/// propagation) and a kerned pair.
fn font() -> Font {
    let mut f = Font::new("Mark Sans");
    for (name, cps, width) in [("A", vec![0x41], 600.0), ("V", vec![0x56], 600.0), ("acutecomb", vec![0x301], 0.0), ("Aacute", vec![0xC1], 600.0)] {
        f.add_glyph(name, cps, width).unwrap();
    }
    {
        let mut a = f.glyph_mut("A").unwrap();
        a.layers[0].shapes.push(Shape::Path(Path::rect(0.0, 0.0, 600.0, 700.0)));
        a.layers[0].anchors.push(Anchor::new("top", 300.0, 700.0));
    }
    f.glyph_mut("V").unwrap().layers[0].shapes.push(Shape::Path(Path::rect(0.0, 0.0, 600.0, 700.0)));
    {
        let mut m = f.glyph_mut("acutecomb").unwrap();
        m.layers[0].shapes.push(Shape::Path(Path::rect(50.0, 500.0, 150.0, 800.0)));
        m.layers[0].anchors.push(Anchor::new("_top", 100.0, 500.0));
        m.layers[0].anchors.push(Anchor::new("top", 100.0, 800.0));
    }
    {
        let mut c = f.glyph_mut("Aacute").unwrap();
        c.layers[0].shapes.push(Shape::Component(Component::new("A", 0.0, 0.0)));
        c.layers[0].shapes.push(Shape::Component(Component::new("acutecomb", 200.0, 200.0)));
    }
    Arc::make_mut(&mut f.kerning).ltr.set("m01", "A", "V", -50.0);
    f
}

/// Each language system's features and their lookups, then each lookup's type.
fn layout(out: &Compiled) -> (Vec<String>, Vec<&'static str>) {
    let font = FontRef::new(&out.bytes).unwrap();
    let gpos = font.gpos().unwrap();
    let scripts = gpos.script_list().unwrap();
    let features = gpos.feature_list().unwrap();
    let mut systems = Vec::new();
    for record in scripts.script_records() {
        let script = record.script(scripts.offset_data()).unwrap();
        let Some(lang_sys) = script.default_lang_sys().transpose().unwrap() else { continue };
        let mut feats = Vec::new();
        for index in lang_sys.feature_indices() {
            let f = &features.feature_records()[usize::from(index.get())];
            let lookups: Vec<String> = f.feature(features.offset_data()).unwrap().lookup_list_indices().iter().map(|i| i.get().to_string()).collect();
            feats.push(format!("{}[{}]", f.feature_tag(), lookups.join(",")));
        }
        systems.push(format!("{}/dflt {}", record.script_tag(), feats.join(" ")));
    }
    let list = gpos.lookup_list().unwrap();
    let kinds = (0..usize::from(list.lookup_count()))
        .map(|i| match list.lookups().get(i).unwrap().subtables().unwrap() {
            PositionSubtables::Pair(_) => "pair",
            PositionSubtables::Cursive(_) => "curs",
            PositionSubtables::MarkToBase(_) => "base",
            PositionSubtables::MarkToLig(_) => "lig",
            PositionSubtables::MarkToMark(_) => "mkmk",
            _ => "other",
        })
        .collect();
    (systems, kinds)
}

fn gid(out: &Compiled, name: &str) -> GlyphId16 {
    GlyphId16::new(out.glyph_order.iter().position(|n| n == name).unwrap() as u16)
}

#[test]
fn a_compile_writes_mark_attachment_after_the_kerning_and_the_gdef_classes() {
    let f = font();
    let out = compile_ttf(&f, &Options::default()).unwrap();
    // U+0301's script extensions include abvm scripts, so its stacking also goes to abvm, as the
    // reference compilers write it. The marks are registered for the default language system.
    let (systems, kinds) = layout(&out);
    assert_eq!(systems, ["DFLT/dflt abvm[1] kern[0] mark[2] mkmk[3]", "latn/dflt kern[0]"], "{:?}", out.warnings);
    assert_eq!(kinds, ["pair", "mkmk", "base", "mkmk"]);

    let font = FontRef::new(&out.bytes).unwrap();
    let classes = font.gdef().unwrap().glyph_class_def().unwrap().unwrap();
    let class = |name: &str| classes.get(gid(&out, name));
    assert_eq!((class("A"), class("Aacute"), class("acutecomb"), class("V")), (1, 1, 3, 0));
    let eval = PairEvaluator::new(&out.bytes).unwrap();
    assert_eq!(eval.value(Tag::new(b"latn"), gid(&out, "A").to_u16(), gid(&out, "V").to_u16()), -50);

    let plan = plan::plan(&f, &f.default_target().unwrap(), Flavour::TrueType, &Options::default()).unwrap();
    assert!(plan.tables.iter().any(|t| t == "GPOS") && plan.tables.iter().any(|t| t == "GDEF"), "{:?}", plan.tables);
}

#[test]
fn a_hand_written_mark_feature_keeps_only_the_generated_mark_feature_out() {
    let mut f = font();
    Arc::make_mut(&mut f.features).features.push(Snippet {
        name: "mark".into(),
        code: "pos base A <anchor 300 700> mark @TOP;".into(),
        ..Default::default()
    });
    let out = compile_ttf(&f, &Options::default()).unwrap();
    let (systems, kinds) = layout(&out);
    assert_eq!(systems, ["DFLT/dflt abvm[1] kern[0] mkmk[2]", "latn/dflt kern[0]"]);
    assert_eq!(kinds, ["pair", "mkmk", "mkmk"]);
    assert!(out.warnings.iter().any(|w| w.contains("hand-written mark feature")), "{:?}", out.warnings);
}

#[test]
fn the_language_systems_of_the_feature_code_get_the_mark_features() {
    let mut f = font();
    Arc::make_mut(&mut f.features).prefixes.push(Snippet {
        name: "Languagesystems".into(),
        code: "languagesystem DFLT dflt;\n\nlanguagesystem latn dflt;\n".into(),
        automatic: true,
        ..Default::default()
    });
    let out = compile_ttf(&f, &Options::default()).unwrap();
    // With Latin declared, no glyph counts for abvm.
    let (systems, kinds) = layout(&out);
    assert_eq!(systems, ["DFLT/dflt kern[0] mark[1] mkmk[2]", "latn/dflt kern[0] mark[1] mkmk[2]"]);
    assert_eq!(kinds, ["pair", "base", "mkmk"]);
}
