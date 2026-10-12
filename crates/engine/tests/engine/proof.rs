//! `font.proof` on a font built in code: page count, glyphs per page and glyph order of the SVG
//! sheets, and the structure of the HTML specimen.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};

use fontcraft_engine::Session;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

/// A font with `A`, `O` and `B` drawn, exported as TrueType into a fresh folder.
fn font(tag: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("fontcraft-proof-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::with_document();
    let rect = |x0: f64, y0: f64, x1: f64, y1: f64| json!([{"x": x0, "y": y0}, {"x": x1, "y": y0}, {"x": x1, "y": y1}, {"x": x0, "y": y1}]);
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(60.0, 0.0, 560.0, 700.0)}));
    run(&mut s, "path.addPath", json!({"glyph": "O", "nodes": rect(80.0, 0.0, 500.0, 700.0)}));
    run(&mut s, "path.addPath", json!({"glyph": "B", "nodes": rect(70.0, 0.0, 520.0, 700.0)}));
    let ttf = dir.join("Proof.ttf");
    run(&mut s, "file.exportStatic", json!({ "destination": ttf, "timestamp": 0 }));
    (dir, ttf)
}

fn files(result: &Value) -> Vec<PathBuf> {
    result["proofs"][0]["files"].as_array().unwrap().iter().map(|f| PathBuf::from(f.as_str().unwrap())).collect()
}

/// The `data-glyph` names of a page, in document order.
fn shown(svg: &str) -> Vec<String> {
    svg.split("data-glyph=\"").skip(1).map(|rest| rest.split('"').next().unwrap().to_string()).collect()
}

fn glyph_order(s: &mut Session, ttf: &Path) -> Vec<String> {
    let mut order = Vec::new();
    for page in files(&run(
        s,
        "font.proof",
        json!({ "fonts": [ttf], "layout": "outline", "format": "svg", "outputDir": ttf.parent().unwrap().join("order") }),
    )) {
        order.extend(shown(&std::fs::read_to_string(page).unwrap()));
    }
    order
}

#[test]
fn proof_svg_pages_follow_glyph_order_and_glyphs_per_page() {
    let (dir, ttf) = font("svg");
    let mut s = Session::new();
    let order = glyph_order(&mut s, &ttf);
    assert!(order.len() >= 4, "notdef and the three drawn glyphs: {order:?}");
    for name in ["A", "B", "O"] {
        assert!(order.iter().any(|n| n == name), "{name} in {order:?}");
    }

    // Two glyphs per page: ceil(n / 2) pages, every glyph once, in glyph order.
    let out = dir.join("palette");
    let result = run(&mut s, "font.proof", json!({ "fonts": [ttf], "layout": "palette", "format": "svg", "glyphsPerPage": 2, "outputDir": out }));
    let pages = files(&result);
    assert_eq!(result["proofs"][0]["pages"], pages.len());
    let mut seen = Vec::new();
    for (i, page) in pages.iter().enumerate() {
        assert!(page.file_name().unwrap().to_string_lossy().starts_with("Proof.proof-"), "{}", page.display());
        let svg = std::fs::read_to_string(page).unwrap();
        assert!(svg.starts_with("<?xml") && svg.trim_end().ends_with("</svg>"));
        let names = shown(&svg);
        assert!(!names.is_empty() && names.len() <= 4, "page {i} shows {names:?}");
        seen.extend(names);
    }
    assert_eq!(seen, order, "the palette shows every glyph once, in glyph order");

    // A selection with a range, an exclusion and a name sort.
    let result = run(
        &mut s,
        "font.proof",
        json!({ "fonts": [ttf], "layout": "palette", "format": "svg", "glyphs": "A,B,O", "excludeGlyphs": ["B"], "sort": "name", "glyphsPerPage": 156, "outputDir": dir.join("picked") }),
    );
    let pages = files(&result);
    assert_eq!(pages.len(), 1);
    assert_eq!(shown(&std::fs::read_to_string(&pages[0]).unwrap()), vec!["A", "O"]);

    // The waterfall embeds the font and sets text in it.
    let result = run(
        &mut s,
        "font.proof",
        json!({ "fonts": [ttf], "layout": "waterfall", "format": "svg", "text": "AOB", "outputDir": dir.join("waterfall") }),
    );
    let svg = std::fs::read_to_string(&files(&result)[0]).unwrap();
    assert!(svg.contains("@font-face") && svg.contains("data:font/ttf;base64,"));
    assert_eq!(svg.matches("class=\"s\"").count(), 12, "one line per default waterfall size");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn proof_html_embeds_woff2_with_sample_charset_and_controls() {
    let (dir, ttf) = font("html");
    let mut s = Session::new();
    let out = dir.join("Proof.html");
    let result = run(&mut s, "font.proof", json!({ "fonts": [ttf], "layout": "palette", "format": "html", "output": out, "text": "AOB" }));
    assert_eq!(result["proofs"][0]["pages"], 1);
    let html = std::fs::read_to_string(&out).unwrap();
    assert!(html.starts_with("<!DOCTYPE html>"));
    assert!(html.contains("src:url(data:font/woff2;base64,d09GMg"), "the font is embedded as WOFF2 (`wOF2` in Base64)");
    assert!(html.contains("id=\"sample\"") && html.contains(">AOB</div>"));
    for cp in ["U+0041", "U+0042", "U+004F"] {
        assert!(html.contains(&format!("data-codepoint=\"{cp}\"")), "{cp} in the character set");
    }
    assert!(html.contains("id=\"sizes\"") && html.contains("id=\"waterfall\""));
    assert!(!html.contains("<script"), "the page carries no script");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn proof_refusals_write_nothing() {
    let (dir, ttf) = font("refuse");
    let mut s = Session::new();
    let out = dir.join("out");
    for bad in [
        json!({ "fonts": [ttf], "layout": "hints", "format": "svg", "outputDir": out }),
        json!({ "fonts": [ttf], "layout": "palette", "format": "svg", "glyphs": ["nope"], "outputDir": out }),
        json!({ "fonts": [ttf], "layout": "palette", "format": "svg", "black": true, "outputDir": out }),
        json!({ "fonts": [ttf], "layout": "palette", "format": "svg", "inPlace": true }),
        json!({ "fonts": [ttf], "layout": "palette", "format": "svg", "size": 1.0e9, "outputDir": out }),
        json!({ "fonts": [ttf], "layout": "outline", "format": "svg", "location": { "wght": 400 }, "outputDir": out }),
    ] {
        assert!(s.execute("font.proof", &bad).is_err(), "{bad}");
    }
    assert!(!out.exists(), "a refused proof writes nothing");
    std::fs::remove_dir_all(&dir).unwrap();
}
