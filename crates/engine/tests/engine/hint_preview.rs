//! `hint.tt.renderPreview` and `hint.ps.renderPreview` (M10.23): the instance built in memory and
//! drawn grid-fitted by skrifa at pixel sizes, as outlines or pixels.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use base64::Engine as _;
use fontcraft_engine::Session;
use fontcraft_engine::model::{Node, NodeKind, Path, Shape};
use fontcraft_render::hinted::{HintedRenderer, Intent};
use serde_json::{Value, json};

/// `I`: a bar 100 to 200 by 0 to 700 in a 600-unit advance. `V`: a triangle (100, 0), (500, 0),
/// (300, 700), whose slanted sides cover pixels partly.
fn session() -> Session {
    let mut s = Session::with_document();
    s.edit("setup", |font| {
        {
            let mut g = font.glyph_mut("I")?;
            g.layer_mut("m01").unwrap().shapes.push(Shape::Path(Path::rect(100.0, 0.0, 200.0, 700.0)));
        }
        let mut g = font.glyph_mut("V")?;
        let n = |x, y| Node::new(x, y, NodeKind::Line);
        g.layer_mut("m01").unwrap().shapes.push(Shape::Path(Path::new(true, vec![n(100.0, 0.0), n(500.0, 0.0), n(300.0, 700.0)])));
        Ok(())
    })
    .unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

/// Whether a result field holds a base64 PNG file.
fn is_png(png_b64: &Value) -> bool {
    let bytes = base64::engine::general_purpose::STANDARD.decode(png_b64.as_str().unwrap()).unwrap();
    bytes.starts_with(b"\x89PNG")
}

#[test]
fn hint_tt_render_preview_gives_grid_fitted_outlines_and_pixels_per_size_and_intent() {
    let mut s = session();
    let r = run(&mut s, "hint.tt.renderPreview", json!({"glyphs": ["I"], "text": "V", "ppem": [10, 20]}));
    assert_eq!((&r["format"], &r["intent"], &r["instance"]), (&json!("ttf"), &json!("grayscale"), &json!("Regular")));
    assert_eq!(r["sizes"].as_array().unwrap().len(), 2);
    let i10 = &r["sizes"][0]["glyphs"][0];
    assert_eq!((&i10["glyph"], &i10["hinted"], &i10["instructed"]), (&json!("I"), &json!(true), &json!(false)));
    assert_eq!(i10["advance"], json!(6.0), "600 units at 10 PPM, rounded");
    let contours = i10["contours"].as_array().unwrap();
    assert_eq!(contours.len(), 1);
    let ys: Vec<f64> = contours[0].as_array().unwrap().iter().map(|pt| pt[1].as_f64().unwrap()).collect();
    assert!(ys.iter().all(|y| *y == 0.0 || *y == 7.0), "the bar spans 0 to 7 pixels: {ys:?}");
    assert_eq!(r["sizes"][1]["glyphs"][1]["glyph"], "V", "`text` follows `glyphs`");
    assert_eq!(r["sizes"][1]["glyphs"][0]["advance"], json!(12.0));

    for intent in ["grayscale", "cleartype", "directwrite"] {
        let r = run(&mut s, "hint.tt.renderPreview", json!({"glyphs": ["V"], "ppem": 20, "intent": intent, "output": "bitmap"}));
        assert_eq!(r["intent"], intent);
        let b = &r["sizes"][0]["glyphs"][0]["bitmap"];
        assert_eq!((&b["left"], &b["top"]), (&json!(2), &json!(14)), "{intent}: the triangle spans x 2 to 10, y 0 to 14");
        assert_eq!((&b["width"], &b["height"]), (&json!(8), &json!(14)), "{intent}");
        assert!(is_png(&b["png"]));
    }

    // The pixels themselves, from the renderer the command uses. GDI ClearType samples 6 times
    // across a pixel and once down it, so every alpha is a whole number of sixths; the slanted
    // sides of V give partial pixels in every intent.
    let compiled = fontcraft_engine::compile::compile_ttf(s.font().unwrap(), &Default::default()).unwrap();
    let gid = compiled.glyph_order.iter().position(|g| g == "V").unwrap() as u32;
    let bytes: std::sync::Arc<[u8]> = compiled.bytes.into();
    let sixths: Vec<u8> = (0..=6u32).map(|k| ((k * 255 + 3) / 6) as u8).collect();
    for intent in [Intent::Grayscale, Intent::GdiClearType, Intent::DirectWrite] {
        let renderer = HintedRenderer::new(bytes.clone(), intent).unwrap();
        let image = renderer.bitmap(gid, 20).unwrap();
        assert_eq!((image.width, image.height), (8, 14));
        let a: Vec<u8> = image.pixels.chunks(4).map(|px| px[3]).collect();
        assert!(image.pixels.chunks(4).all(|px| px[..3] == [0, 0, 0]), "black ink");
        assert!(a.contains(&255) && a.iter().any(|v| *v > 0 && *v < 255), "{intent:?}: full and partial pixels");
        if intent == Intent::GdiClearType {
            assert!(a.iter().all(|v| sixths.contains(v)), "cleartype alphas are sixths: {a:?}");
        }
        let outline = renderer.outline(gid, 20).unwrap();
        assert!(outline.hinted && outline.message.is_none());
        assert!(!renderer.clone().without_hinting().outline(gid, 20).unwrap().hinted);
        assert!(renderer.outline(gid, 0).is_err() && renderer.outline(gid, 401).is_err() && renderer.outline(99_999, 12).is_err());
    }
    assert!(HintedRenderer::new(std::sync::Arc::from(&b"not a font"[..]), Intent::Grayscale).is_err());

    for bad in [
        json!({"glyphs": ["I"]}),
        json!({"glyphs": ["I"], "ppem": 0}),
        json!({"glyphs": ["I"], "ppem": 401}),
        json!({"glyphs": ["I"], "ppem": 12.5}),
        json!({"glyphs": ["I"], "ppem": []}),
        json!({"glyphs": ["I"], "ppem": 12, "intent": "mono"}),
        json!({"glyphs": ["I"], "ppem": 12, "output": "svg"}),
        json!({"glyphs": ["nope"], "ppem": 12}),
        json!({"ppem": 12}),
        json!({"text": "\u{4E00}", "ppem": 12}),
        json!({"glyphs": ["I"], "ppem": 12, "instance": "Bold"}),
        json!({"glyphs": ["I"], "ppem": (1..=81).collect::<Vec<u32>>()}),
    ] {
        assert!(s.execute("hint.tt.renderPreview", &bad).is_err(), "{bad}");
    }
}

#[test]
fn hint_ps_render_preview_draws_a_line_hinted_or_not() {
    let mut s = session();
    let r = run(&mut s, "hint.ps.renderPreview", json!({"text": "IV", "ppem": 20, "instance": "m01"}));
    assert_eq!((&r["format"], &r["hinted"]), (&json!("otf"), &json!(true)));
    let size = &r["sizes"][0];
    assert_eq!(size["ppem"], 20);
    let (w, h) = (size["width"].as_u64().unwrap(), size["height"].as_u64().unwrap());
    assert!(w >= 24, "two advances of 12 pixels: {w}");
    assert!(h >= 20, "ascender 800 and descender -200 at 20 PPM: {h}");
    assert!(is_png(&size["png"]));

    for hinted in [true, false] {
        let r = run(&mut s, "hint.ps.renderPreview", json!({"text": "IV", "ppem": [12, 20], "hinted": hinted, "format": "json"}));
        assert_eq!(r["hinted"], hinted);
        for size in r["sizes"].as_array().unwrap() {
            let glyphs = size["glyphs"].as_array().unwrap();
            assert_eq!(glyphs.len(), 2);
            assert_eq!((&glyphs[0]["glyph"], &glyphs[1]["glyph"]), (&json!("I"), &json!("V")));
            assert!(glyphs.iter().all(|g| g["hinted"] == hinted && g["contours"].as_array().unwrap().len() == 1), "{size}");
            assert!(size.get("png").is_none());
        }
    }

    for bad in [
        json!({"ppem": 12}),
        json!({"text": "", "ppem": 12}),
        json!({"text": "I", "ppem": 12, "format": "svg"}),
        json!({"text": "I", "ppem": 12, "hinted": "yes"}),
    ] {
        assert!(s.execute("hint.ps.renderPreview", &bad).is_err(), "{bad}");
    }
}
