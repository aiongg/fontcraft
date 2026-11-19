//! Export Window Contents (`file.exportWindowContents`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use base64::Engine as _;
use fontcraft_engine::Session;
use serde_json::{Value, json};

fn run(s: &mut Session, p: Value) -> Value {
    s.execute("file.exportWindowContents", &p).unwrap_or_else(|e| panic!("{p}: {e}"))
}

fn decoded(out: &Value) -> String {
    let bytes = base64::engine::general_purpose::STANDARD.decode(out["dataBase64"].as_str().unwrap()).unwrap();
    String::from_utf8(bytes).unwrap()
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Value {
    json!([{"x": x0, "y": y0}, {"x": x1, "y": y0}, {"x": x1, "y": y1}, {"x": x0, "y": y1}])
}

#[test]
fn window_contents_export_as_filled_pdf_and_editing_svg() {
    let mut s = Session::with_document();
    s.execute("path.addPath", &json!({"glyph": "A", "nodes": rect(60.0, 0.0, 560.0, 700.0)})).unwrap();
    s.execute("guide.add", &json!({"glyph": "A", "y": 300})).unwrap();
    s.execute("guide.addGlobal", &json!({"x": 100, "angle": 90})).unwrap();
    let (advance, ascender, descender) = {
        let font = s.font().unwrap();
        let master = &font.masters[0];
        (font.glyph("A").unwrap().layers[0].width, font.metric_pos(master, "ascender", 800.0), font.metric_pos(master, "descender", -200.0))
    };

    // SVG: both glyphs outlined, with their nodes and guides; the page spans both advances
    // (widened to the outline) and the master's descender to ascender, a tenth of the em around.
    let out = run(&mut s, json!({"text": "/A/A", "format": "svg"}));
    assert_eq!(out["glyphs"], json!(["A", "A"]), "{out}");
    let box_width = advance.max(560.0);
    assert_eq!(out["width"].as_f64().unwrap(), advance + box_width + 200.0, "{out}");
    assert_eq!(out["height"].as_f64().unwrap(), ascender.max(700.0) - descender.min(0.0) + 200.0, "{out}");
    let svg = decoded(&out);
    assert!(svg.starts_with("<?xml") && svg.contains("<svg"), "{svg}");
    assert_eq!(svg.matches("data-glyph=\"A\"").count(), 2, "{svg}");
    // Four on-curve nodes per glyph, drawn in the on-curve colour.
    assert_eq!(svg.matches("#d0342c").count(), 8, "{svg}");
    // The local guide sits at the same height in both glyphs and is drawn once; the global
    // vertical guide follows each glyph's origin.
    assert_eq!(svg.matches("stroke=\"#2fa86b\"").count(), 3, "{svg}");

    // Only the edited glyph shows nodes and guides.
    let svg = decoded(&run(&mut s, json!({"text": "/A/A", "format": "svg", "edited": [1]})));
    assert_eq!(svg.matches("#d0342c").count(), 4, "{svg}");
    assert_eq!(svg.matches("stroke=\"#2fa86b\"").count(), 2, "{svg}");

    // PDF: filled glyphs, each marked with its name, at the size asked for.
    let dir = std::env::temp_dir().join(format!("fontcraft-window-export-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("window.pdf");
    let out = run(&mut s, json!({"text": "A/A\n/A", "format": "pdf", "size": 100, "path": file}));
    assert!((out["scale"].as_f64().unwrap() - 0.1).abs() < 1e-12, "{out}");
    let pdf = std::fs::read(&file).unwrap();
    assert_eq!(out["bytes"].as_u64().unwrap(), pdf.len() as u64);
    let text = String::from_utf8_lossy(&pdf);
    assert!(text.starts_with("%PDF-"), "{text}");
    // The character and the two names give three glyphs over two lines.
    assert_eq!(out["glyphs"], json!(["A", "A", "A"]), "{out}");
    assert_eq!(text.matches("/Name (A)").count(), 3, "{text}");
    std::fs::remove_dir_all(&dir).unwrap();

    // Other directions lay out without failing.
    for direction in ["rtl", "ttb"] {
        let out = run(&mut s, json!({"text": "/A/A", "format": "pdf", "direction": direction, "kerning": "none"}));
        assert_eq!(out["glyphs"], json!(["A", "A"]), "{direction}: {out}");
    }

    // Failures name the problem.
    for (params, says) in [
        (json!({"text": "/A", "format": "gif"}), "format"),
        (json!({"text": "", "format": "svg"}), "text"),
        (json!({"text": "/A", "format": "svg", "edited": [5]}), "edited"),
        (json!({"text": "/A", "format": "svg", "master": "nope"}), "nope"),
        (json!({"text": "/A", "format": "svg", "lineWidth": 0}), "lineWidth"),
        (json!({"text": "/A", "format": "svg", "size": 0}), "size"),
        (json!({"text": "/A", "format": "svg", "direction": "up"}), "direction"),
    ] {
        let e = s.execute("file.exportWindowContents", &params).unwrap_err().to_string();
        assert!(e.contains(says), "{params}: {e}");
    }
}
