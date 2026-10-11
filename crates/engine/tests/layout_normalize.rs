//! `layout.normalize` on a font the engine exports: kerning set in the source comes back as one
//! flat rule, the text can be written to a file, and bad parameters change nothing.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use fontcraft_engine::Session;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-normalize-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Value {
    json!([{"x": x0, "y": y0}, {"x": x1, "y": y0}, {"x": x1, "y": y1}, {"x": x0, "y": y1}])
}

#[test]
fn layout_normalize_prints_exported_kerning_as_one_rule() {
    let dir = temp("kern");
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(60.0, 0.0, 560.0, 700.0)}));
    run(&mut s, "path.addPath", json!({"glyph": "O", "nodes": rect(40.0, -10.0, 600.0, 710.0)}));
    run(&mut s, "kerning.setPair", json!({"left": "A", "right": "O", "value": -40}));
    let font = dir.join("kern.ttf");
    run(&mut s, "file.exportStatic", json!({"destination": font, "timestamp": 0}));

    let out = run(&mut s, "layout.normalize", json!({"font": font}));
    let text = out["text"].as_str().unwrap();
    assert!(text.starts_with("# GPOS #\n"), "{text}");
    assert!(text.contains("\n# kern: "), "{text}");
    assert!(text.lines().any(|l| l == "A -40 O"), "{text}");

    // GDEF alone: the export has no ligature carets, so nothing prints.
    let gdef = run(&mut s, "layout.normalize", json!({"font": font, "table": "gdef"}));
    assert_eq!(gdef["text"], "");

    let written = dir.join("kern.txt");
    let out = run(&mut s, "layout.normalize", json!({"font": font, "output": written}));
    assert_eq!(std::fs::read_to_string(&written).unwrap(), text);
    assert_eq!(out["bytes"], text.len());

    // Refusals write nothing.
    let refused = dir.join("refused.txt");
    for bad in [
        json!({"font": font, "table": "kern", "output": refused}),
        json!({"font": font, "location": {"wght": 700}, "output": refused}),
        json!({"font": font, "output": font}),
        json!({"font": dir.join("missing.ttf"), "output": refused}),
    ] {
        assert!(s.execute("layout.normalize", &bad).is_err(), "{bad}");
        assert!(!refused.exists(), "{bad}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
