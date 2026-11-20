//! The positional Indic features end to end (M8.32): Update writes pres from the i-matra
//! variants' anchors and the Devanagari language systems, and the compiled font shapes each
//! conjunct with its variant.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_compile::preview::{PreviewKind, layout_font};
use fontcraft_engine::Session;
use fontcraft_model::Anchor;
use fontcraft_shaping::{ShapeRequest, ShapingFont, Tag, shape};
use serde_json::json;

/// Five i-matra widths, three conjuncts and ka, with the anchors the reference's Devanagari
/// tutorial asks for: `imatra` on each matra, `top` on each base's stem. Variant n reaches
/// 200 + 200n units past its advance; the plain matra reaches 420.
fn devanagari(s: &mut Session) {
    s.edit("devanagari", |font| {
        let master = font.masters.first().map(|m| m.id.clone()).unwrap_or_default();
        let mut glyphs: Vec<GlyphSpec> = vec![
            ("ka-deva".into(), vec![0x0915], 500.0, vec![("top", 420.0)]),
            ("ssa-deva".into(), vec![0x0937], 500.0, vec![("top", 420.0)]),
            ("ya-deva".into(), vec![0x092F], 500.0, vec![("top", 420.0)]),
            ("ta-deva".into(), vec![0x0924], 500.0, vec![("top", 420.0)]),
            ("halant-deva".into(), vec![0x094D], 0.0, vec![]),
            ("iMatra-deva".into(), vec![0x093F], 200.0, vec![("imatra", 620.0)]),
        ];
        for n in 1..=5u32 {
            glyphs.push((format!("iMatra-deva.{n:02}"), vec![], 200.0, vec![("imatra", 400.0 + 200.0 * f64::from(n))]));
        }
        for (name, stem) in [("k_ssa-deva", 610.0), ("k_ya-deva", 1010.0), ("t_ta-deva", 1390.0)] {
            glyphs.push((name.into(), vec![], stem + 80.0, vec![("top", stem)]));
        }
        for (name, unicodes, width, anchors) in glyphs {
            font.add_glyph(&name, unicodes, width)?;
            let mut g = font.glyph_mut(&name)?;
            if let Some(layer) = g.layer_mut(&master) {
                layer.anchors = anchors.into_iter().map(|(a, x)| Anchor::new(a, x, 700.0)).collect();
            }
        }
        Ok(())
    })
    .unwrap();
}

#[test]
fn pres_shapes_each_conjunct_with_the_i_matra_variant_its_stem_needs() {
    let mut s = Session::with_document();
    devanagari(&mut s);
    let report = s.execute("font.features.update", &json!({})).unwrap();
    assert!(report["added"].as_array().unwrap().contains(&json!({"kind": "feature", "name": "pres"})), "{report}");

    let font = Arc::clone(&s.doc().unwrap().font);
    let master = font.masters.first().map(|m| m.id.clone()).unwrap();
    let built = layout_font(&font, &master, PreviewKind::GsubOnly).unwrap();
    let shaper = ShapingFont::new(Arc::from(built.bytes.as_slice()), &built.private_use).unwrap();
    let names = |text: &str| -> Vec<String> {
        let req = ShapeRequest { script: Some(Tag::new(b"Deva")), ..ShapeRequest::new(text) };
        shape(&shaper, &req).unwrap().iter().map(|g| built.glyph_order.get(usize::from(g.glyph)).cloned().unwrap_or_default()).collect()
    };
    assert_eq!(names("\u{0915}\u{093F}"), ["iMatra-deva", "ka-deva"], "ka keeps the plain matra");
    assert_eq!(names("\u{0915}\u{094D}\u{0937}\u{093F}"), ["iMatra-deva.02", "k_ssa-deva"]);
    assert_eq!(names("\u{0915}\u{094D}\u{092F}\u{093F}"), ["iMatra-deva.04", "k_ya-deva"]);
    assert_eq!(names("\u{0924}\u{094D}\u{0924}\u{093F}"), ["iMatra-deva.05", "t_ta-deva"]);
}

#[test]
fn update_registers_the_devanagari_features_under_dev2_and_deva() {
    let mut s = Session::with_document();
    devanagari(&mut s);
    s.execute("font.features.update", &json!({})).unwrap();
    let font = Arc::clone(&s.doc().unwrap().font);
    let prefix = font.features.prefixes.iter().find(|p| p.name == "Languagesystems").unwrap();
    assert!(prefix.automatic);
    assert_eq!(prefix.code, "languagesystem DFLT dflt;\n\nlanguagesystem dev2 dflt;\nlanguagesystem deva dflt;\n");

    let r = s.execute("font.features.compile", &json!({})).unwrap();
    assert_eq!(r["ok"], true, "{r}");
    let features = r["features"].as_array().unwrap();
    for tag in ["akhn", "cjct", "pres"] {
        let f = features.iter().find(|f| f["tag"] == tag).unwrap_or_else(|| panic!("{tag} compiled: {r}"));
        let systems = f["languageSystems"].as_array().unwrap();
        for system in [json!(["dev2", "dflt"]), json!(["deva", "dflt"])] {
            assert!(systems.contains(&system), "{tag} under {system}: {r}");
        }
    }
}

/// Name, code points, width and anchors.
type GlyphSpec = (String, Vec<u32>, f64, Vec<(&'static str, f64)>);
