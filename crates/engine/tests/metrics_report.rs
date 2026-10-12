//! `font.reportMetrics` on a font exported from a document.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use fontcraft_engine::Session;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-metrics-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn font_report_metrics_reads_an_exported_font() {
    let dir = temp("report");
    let mut s = Session::with_document();
    let ttf = dir.join("Untitled.ttf");
    run(&mut s, "file.exportStatic", json!({ "destination": ttf, "timestamp": 0 }));
    let before = std::fs::read(&ttf).unwrap();

    let r = run(
        &mut s,
        "font.reportMetrics",
        json!({ "font": ttf, "format": "json", "glyphs": ".notdef,space", "sidebearings": true, "statistics": true }),
    );
    assert_eq!(r["unitsPerEm"], 1000);
    let notdef = &r["glyphs"][0];
    assert_eq!((notdef["name"].as_str(), notdef["advance"].as_f64()), (Some(".notdef"), Some(500.0)));
    assert_eq!(notdef["bounds"], json!([50.0, 0.0, 450.0, 700.0]));
    assert_eq!((notdef["lsb"].as_f64(), notdef["rsb"].as_f64()), (Some(50.0), Some(50.0)));
    // The frame of .notdef: 400 × 700 minus 300 × 600, with the outer contour clockwise.
    let area = notdef["statistics"]["area"].as_f64().unwrap();
    assert!((area.abs() - 100_000.0).abs() < 1e-6, "{area}");
    assert!(r["glyphs"][1].get("bounds").is_none(), "space has no outline");
    assert!(r.get("text").is_none());

    let afm = dir.join("Untitled.afm");
    let w = run(&mut s, "font.reportMetrics", json!({ "font": ttf, "format": "afm", "units": 1000, "output": afm }));
    assert!(w["written"].is_string() && w.get("text").is_none());
    let text = std::fs::read_to_string(&afm).unwrap();
    assert!(text.starts_with("StartFontMetrics 2.0\n") && text.contains("C 65 ; WX 600 ; N A ;"), "{text}");

    let t = run(&mut s, "font.reportMetrics", json!({ "font": ttf, "glyphs": ["A"], "aggregateBbox": true }));
    assert!(t["text"].as_str().unwrap().contains("glyph[2] {A,0x0041,600,{0,0,0,0}}"), "{t}");

    assert!(s.execute("font.reportMetrics", &json!({ "font": ttf, "format": "pdf" })).is_err());
    assert!(s.execute("font.reportMetrics", &json!({ "font": ttf, "fontOnly": true })).is_err(), "fontOnly needs statistics");
    assert!(s.execute("font.reportMetrics", &json!({ "font": ttf, "glyphs": "nosuch" })).is_err());
    assert!(s.execute("font.reportMetrics", &json!({ "font": dir.join("missing.ttf") })).is_err());
    assert_eq!(std::fs::read(&ttf).unwrap(), before, "the input is not changed");
    std::fs::remove_dir_all(&dir).unwrap();
}
