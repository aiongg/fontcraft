use std::sync::Arc;

use fontcraft_model::{Glyph, Layer, Snippet};
use skrifa::raw::TableProvider;
use skrifa::{FontRef, MetadataProvider};

use super::*;

/// Font::new's A–Z and a–z, plus `a.sc` and `f_i` without code points, a smcp and a liga
/// feature, and a hand-written kern feature.
fn font() -> Font {
    let mut font = Font::new("Preview");
    let master = font.masters.first().map(|m| m.id.clone()).unwrap_or_default();
    for name in ["a.sc", "f_i"] {
        let mut g = Glyph::new(name);
        g.layers.push(Layer::new(&master, 450.0));
        font.glyphs.push(g);
    }
    let features = Arc::make_mut(&mut font.features);
    features.features.push(Snippet { name: "smcp".into(), code: "sub a by a.sc;".into(), ..Default::default() });
    features.features.push(Snippet { name: "liga".into(), code: "sub f i by f_i;".into(), ..Default::default() });
    features.features.push(Snippet { name: "kern".into(), code: "pos A V -100;".into(), ..Default::default() });
    font
}

fn master(font: &Font) -> String {
    font.masters.first().map(|m| m.id.clone()).unwrap_or_default()
}

/// The Edit View font has the substitutions and no positioning; the unencoded glyphs are reached
/// through private-use code points of plane 15 that stay out of `cmap`.
#[test]
fn the_edit_view_preview_font_has_gsub_and_keeps_private_use_out_of_cmap() {
    let font = font();
    let built = layout_font(&font, &master(&font), PreviewKind::GsubOnly).unwrap();
    assert_eq!(built.glyph_order.first().map(String::as_str), Some(".notdef"));
    let gid = |name: &str| built.glyph_order.iter().position(|n| n == name).unwrap() as u16;
    let sc = built.private_use.iter().find(|(_, g)| *g == gid("a.sc")).map(|(cp, _)| *cp).unwrap();
    assert!((PLANE_15_START..=PLANE_15_END).contains(&sc), "{sc:X}");
    assert!(built.private_use.iter().any(|(_, g)| *g == gid("f_i")));
    assert!(!built.private_use.iter().any(|(_, g)| *g == gid("a") || *g == 0), "encoded glyphs and .notdef keep their own");

    let f = FontRef::new(&built.bytes).unwrap();
    let charmap = f.charmap();
    assert_eq!(charmap.map('a').map(|g| g.to_u32()), Some(u32::from(gid("a"))));
    for (cp, _) in &built.private_use {
        assert!(charmap.map(*cp).is_none(), "U+{cp:X} is in cmap");
    }
    assert!(f.gsub().is_ok());
    assert!(f.gpos().is_err(), "Edit View positions with live kerning and anchors");
    assert_eq!(f.maxp().unwrap().num_glyphs() as usize, built.glyph_order.len());
    let tags: Vec<String> = built.features.iter().map(|s| s.tag.to_string()).collect();
    assert!(tags.contains(&"smcp".to_string()) && tags.contains(&"liga".to_string()), "{tags:?}");
    assert!(!tags.contains(&"kern".to_string()), "{tags:?}");
}

/// Text Preview's font has `GPOS` too.
#[test]
fn the_full_preview_font_has_gpos() {
    let font = font();
    let built = layout_font(&font, &master(&font), PreviewKind::Full).unwrap();
    assert!(FontRef::new(&built.bytes).unwrap().gpos().is_ok());
    assert!(built.features.iter().any(|s| s.tag == "kern"));
}

/// Feature code with an error gives the diagnostics, and an unknown master is refused.
#[test]
fn the_preview_font_refuses_bad_code_and_unknown_masters() {
    let mut font = font();
    Arc::make_mut(&mut font.features).features.push(Snippet { name: "calt".into(), code: "sub a by nosuchglyph;".into(), ..Default::default() });
    assert!(matches!(layout_font(&font, &master(&font), PreviewKind::GsubOnly), Err(CompileError::Features(_))));
    assert!(layout_font(&font, "no-such-master", PreviewKind::GsubOnly).is_err());
}

/// A glyph encoded at the first private-use code point keeps it; the unencoded glyphs take the
/// next ones.
#[test]
fn private_use_code_points_skip_the_ones_glyphs_have() {
    let mut font = font();
    let master = master(&font);
    let mut g = Glyph::new("pua0");
    g.unicodes.push(PLANE_15_START);
    g.layers.push(Layer::new(&master, 500.0));
    font.glyphs.push(g);
    let built = layout_font(&font, &master, PreviewKind::GsubOnly).unwrap();
    assert!(built.private_use.iter().all(|(cp, _)| *cp != PLANE_15_START));
    assert_eq!(private_use_code_point(0), Some(PLANE_15_START));
    assert_eq!(private_use_code_point(65_534), Some(PLANE_16_START));
}
