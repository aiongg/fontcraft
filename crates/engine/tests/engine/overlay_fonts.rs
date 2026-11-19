//! Overlay Fonts: two fonts of a chromatic family stacked into one colour font, as palette layers
//! and as one master per source, and the preview query a view paints from.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::{EngineError, Session};
use fontcraft_model::{Component, Font, Glyph, Layer, Path, Shape};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn refused(s: &mut Session, id: &str, p: Value, needle: &str) {
    let open = s.documents().len();
    match s.execute(id, &p) {
        Err(EngineError::BadParams { message, .. }) => assert!(message.contains(needle), "{id} {p}: {message}"),
        other => panic!("{id} {p}: expected a refusal naming `{needle}`, got {other:?}"),
    }
    assert_eq!(s.documents().len(), open, "{id} {p}: a refused overlay opened a document");
}

/// "Base" (the bottom: `A` drawn, A–V kerned −50) and "Shade" at 2000 units per em: `A` drawn,
/// `star` that Base lacks, `Aalt` claiming A's code point, and `ring` built from a component.
fn family() -> Session {
    let mut base = Font::new("Base");
    base.glyph_mut("A").unwrap().layer_mut("m01").unwrap().shapes.push(Shape::Path(Path::rect(50.0, 0.0, 550.0, 700.0)));
    base.glyph_mut("A").unwrap().layer_mut("m01").unwrap().anchors.push(fontcraft_model::Anchor::new("top", 300.0, 700.0));
    Arc::make_mut(&mut base.kerning).ltr.set("m01", "A", "V", -50.0);

    let mut shade = Font::new("Shade");
    shade.units_per_em = 2000;
    shade.glyph_mut("A").unwrap().layer_mut("m01").unwrap().shapes.push(Shape::Path(Path::rect(100.0, 0.0, 500.0, 1400.0)));
    let mut star = Glyph::new("star");
    star.unicodes = vec![0x2605];
    let mut layer = Layer::new("m01", 900.0);
    layer.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 800.0, 800.0)));
    star.layers.push(layer);
    shade.glyphs.push(star);
    let mut alt = Glyph::new("Aalt");
    alt.unicodes = vec![0x41];
    alt.layers.push(Layer::new("m01", 1200.0));
    shade.glyphs.push(alt);
    let mut ring = Glyph::new("ring");
    let mut layer = Layer::new("m01", 1200.0);
    layer.shapes.push(Shape::Component(Component::new("A", 0.0, 0.0)));
    ring.layers.push(layer);
    shade.glyphs.push(ring);

    let mut s = Session::new();
    s.add_document(base, None).unwrap();
    s.add_document(shade, None).unwrap();
    s
}

fn sources() -> Value {
    json!([
        {"font": 0, "color": "#FF0000"},
        {"font": "Untitled 2", "color": "#0000FF", "opacity": 0.5, "offset": [10, -10]},
    ])
}

/// The x and y ranges of the nodes of the paths in `shapes`.
fn extent(shapes: &[Shape]) -> (f64, f64, f64, f64) {
    let mut e = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for shape in shapes {
        let Shape::Path(p) = shape else { panic!("only paths expected, got {shape:?}") };
        for n in &p.nodes {
            e = (e.0.min(n.x), e.1.max(n.x), e.2.min(n.y), e.3.max(n.y));
        }
    }
    e
}

#[test]
fn overlay_fonts_stacks_sources_as_palette_layers() {
    let mut s = family();
    let base_before = Arc::clone(&s.documents()[0].font);
    let out = run(&mut s, "font.overlayFonts", json!({"layers": sources(), "output": "colorLayers"}));
    assert_eq!(out["document"], 2);
    assert_eq!(s.documents().len(), 3, "the overlay opens as a new document");
    assert_eq!(out["palette"], json!(["#FF0000FF", "#0000FF80"]), "opacity 0.5 halves the alpha");
    assert!(Arc::ptr_eq(&base_before, &s.documents()[0].font), "the sources do not change");
    let warnings = out["warnings"].as_array().unwrap();
    assert!(warnings.iter().any(|w| w.as_str().unwrap().contains("2000 units per em")), "{warnings:?}");
    assert!(warnings.iter().any(|w| w.as_str().unwrap().contains("U+0041")), "{warnings:?}");

    let f = s.font().unwrap();
    assert_eq!(f.family_name, "Base");
    assert_eq!(f.masters.len(), 1);
    let a = f.glyph("A").unwrap();
    let main = a.layer("m01").unwrap();
    assert_eq!(main.width, 600.0, "the bottom source gives the width");
    assert_eq!(main.shapes.len(), 2, "the master layer holds every source's outlines");
    assert_eq!(main.anchors.len(), 1, "anchors come from the bottom source");
    assert_eq!(f.kerning.ltr.get("m01", "A", "V"), Some(-50.0));

    let listed = run(&mut s, "layer.listColorLayers", json!({"glyphs": ["A", "star", "ring"]}));
    let layers = |i: usize| listed["glyphs"][i]["layers"].as_array().unwrap().clone();
    let a_layers = layers(0);
    assert_eq!(a_layers.len(), 2);
    assert_eq!((a_layers[0]["kind"].as_str(), a_layers[0]["index"].as_i64()), (Some("palette"), Some(0)));
    assert_eq!((a_layers[1]["kind"].as_str(), a_layers[1]["index"].as_i64()), (Some("palette"), Some(1)));
    assert_eq!(layers(1).len(), 1, "a glyph only the upper source has gets its colour layer only");
    assert_eq!(layers(2)[0]["index"], 1);

    let f = s.font().unwrap();
    let a = f.glyph("A").unwrap();
    let shade = a.layers.iter().find(|l| l.id == a_layers[1]["layer"].as_str().unwrap()).unwrap();
    // 2000 → 1000 units per em halves the outline, then the offset moves it.
    assert_eq!(extent(&shade.shapes), (60.0, 260.0, -10.0, 690.0));
    let star = f.glyph("star").unwrap();
    assert_eq!(star.unicodes, vec![0x2605]);
    assert_eq!(star.layer("m01").unwrap().width, 450.0, "a glyph the bottom lacks takes its scaled width from its source");
    assert!(f.glyph("Aalt").unwrap().unicodes.is_empty(), "A's code point stays with A");
    let ring = f.glyph("ring").unwrap();
    assert!(ring.layer("m01").unwrap().shapes.iter().all(|s| matches!(s, Shape::Path(_))), "components are resolved to paths");
    assert_eq!(extent(&ring.layer("m01").unwrap().shapes), (60.0, 260.0, -10.0, 690.0));

    let palettes = run(&mut s, "colorPalette.list", json!({}));
    assert_eq!(palettes["palettes"], json!([["#FF0000FF", "#0000FF80"]]));
}

#[test]
fn overlay_fonts_makes_one_master_per_source() {
    let mut s = family();
    let out = run(&mut s, "font.overlayFonts", json!({"layers": sources(), "output": "masters"}));
    assert!(out.get("palette").is_none());
    let made = out["masters"].as_array().unwrap();
    assert_eq!(made.len(), 2);
    assert_eq!(made[0]["name"], "Base Regular");
    assert_eq!(made[1]["name"], "Shade Regular");
    let (bottom, upper) = (made[0]["id"].as_str().unwrap().to_string(), made[1]["id"].as_str().unwrap().to_string());
    assert_ne!(bottom, upper);

    let f = s.font().unwrap();
    assert_eq!(f.masters.len(), 2);
    assert!(f.axes.is_empty());
    let a = f.glyph("A").unwrap();
    assert_eq!(extent(&a.layer(&bottom).unwrap().shapes), (50.0, 550.0, 0.0, 700.0));
    let upper_a = a.layer(&upper).unwrap();
    assert_eq!(extent(&upper_a.shapes), (60.0, 260.0, -10.0, 690.0));
    assert_eq!(upper_a.width, 600.0, "every master has the bottom source's widths");
    assert!(f.glyph("B").unwrap().layer(&upper).unwrap().shapes.is_empty());
    assert!(f.glyph("star").unwrap().layer(&bottom).unwrap().shapes.is_empty(), "the bottom source does not draw star");
    assert_eq!(f.kerning.ltr.get(&bottom, "A", "V"), Some(-50.0));
    assert_eq!(f.kerning.ltr.get(&upper, "A", "V"), Some(-50.0), "every master has the bottom source's kerning");
    assert!(f.glyphs.iter().all(|g| g.layers.iter().all(|l| l.is_master_layer())));
}

#[test]
fn overlay_preview_returns_the_stacked_outlines() {
    let mut s = family();
    let out = run(&mut s, "font.overlayPreview", json!({"layers": sources(), "glyphs": ["A", "star", "nosuch"]}));
    assert_eq!(s.documents().len(), 2, "the preview opens nothing");
    assert_eq!(out["unitsPerEm"], 1000);
    let a = &out["glyphs"][0];
    assert_eq!(a["width"], 600.0);
    let layers = a["layers"].as_array().unwrap();
    assert_eq!(layers.len(), 2);
    assert_eq!((layers[0]["source"].as_i64(), layers[0]["color"].as_str()), (Some(0), Some("#FF0000FF")));
    assert_eq!((layers[1]["source"].as_i64(), layers[1]["color"].as_str()), (Some(1), Some("#0000FF80")));
    let path = layers[1]["path"].as_str().unwrap();
    assert!(path.starts_with('M') && path.contains("60") && path.contains("690"), "{path}");
    assert_eq!(out["glyphs"][1]["width"], 450.0);
    assert_eq!(out["glyphs"][1]["layers"].as_array().unwrap().len(), 1);
    assert!(out["glyphs"][2]["width"].is_null());
    assert!(out["glyphs"][2]["layers"].as_array().unwrap().is_empty());
}

#[test]
fn overlay_fonts_refuses_bad_sources() {
    let mut s = family();
    refused(&mut s, "font.overlayFonts", json!({"layers": [], "output": "masters"}), "empty");
    refused(&mut s, "font.overlayFonts", json!({"layers": sources(), "output": "layers"}), "output");
    refused(&mut s, "font.overlayFonts", json!({"layers": [{"font": 5}], "output": "masters"}), "no open document");
    refused(&mut s, "font.overlayFonts", json!({"layers": [{"font": 0, "scale": 0}], "output": "masters"}), "scale");
    refused(&mut s, "font.overlayFonts", json!({"layers": [{"font": 0, "opacity": 2}], "output": "masters"}), "opacity");
    refused(&mut s, "font.overlayFonts", json!({"layers": [{"font": 0, "color": "red"}], "output": "masters"}), "color");
    refused(&mut s, "font.overlayFonts", json!({"layers": [{"font": 0, "master": "Bold"}], "output": "masters"}), "master");
    refused(&mut s, "font.overlayFonts", json!({"layers": [{"font": 0, "offset": [0, 2e6]}], "output": "masters"}), "offset");
    refused(&mut s, "font.overlayPreview", json!({"layers": sources(), "glyphs": []}), "empty");
}
