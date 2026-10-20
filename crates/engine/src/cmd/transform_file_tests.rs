//! Behaviour tests of `font.transform` on a source and on a TrueType font compiled from it,
//! both built in code.

use serde_json::json;

use crate::Session;

/// A session whose font has `A`: a 100 × 100 square at the origin.
fn drawn() -> Session {
    let mut s = Session::with_document();
    let nodes = json!([{"x": 0, "y": 0}, {"x": 100, "y": 0}, {"x": 100, "y": 100}, {"x": 0, "y": 100}]);
    s.execute("path.addPath", &json!({"glyph": "A", "nodes": nodes})).unwrap();
    s
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-transform-file-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn upem_of(path: &std::path::Path) -> u16 {
    let font = fontcraft_tables::read(&std::fs::read(path).unwrap(), 0).unwrap();
    let head = font.get(*b"head").unwrap();
    u16::from_be_bytes([head[18], head[19]])
}

#[test]
fn font_transform_scales_units_per_em_and_rotates_a_binary() {
    let dir = scratch("binary");
    let s = drawn();
    let ttf = fontcraft_compile::compile_ttf(s.font().unwrap(), &fontcraft_compile::Options::default()).unwrap();
    let binary = dir.join("in.ttf");
    std::fs::write(&binary, &ttf.bytes).unwrap();
    let before = std::fs::read(&binary).unwrap();
    let mut session = Session::new();

    let out = dir.join("scaled.ttf");
    let r = session.execute("font.transform", &json!({"font": binary, "unitsPerEm": 2048, "output": out})).unwrap();
    assert_eq!(r["unitsPerEm"], json!({"from": 1000, "to": 2048}));
    assert_eq!(r["hintsKept"], json!(true));
    assert_eq!(upem_of(&out), 2048);
    assert_eq!(std::fs::read(&binary).unwrap(), before, "the input is never changed");

    let out = dir.join("rotated.ttf");
    let r = session.execute("font.transform", &json!({"font": binary, "rotate": 90, "glyphs": ["A"], "output": out})).unwrap();
    assert_eq!(r["matrix"], json!([0.0, -1.0, 1.0, 0.0, 0.0, 0.0]));
    assert!(r["glyphsChanged"].as_u64().unwrap() >= 1);
    assert_eq!(upem_of(&out), 1000);

    // Refusals write nothing.
    let refused = dir.join("refused.ttf");
    assert!(session.execute("font.transform", &json!({"font": binary, "output": refused})).is_err(), "nothing to do");
    assert!(session.execute("font.transform", &json!({"font": binary, "unitsPerEm": 8, "output": refused})).is_err());
    assert!(session.execute("font.transform", &json!({"font": binary, "matrix": [1, 0, 0, 1, 0], "output": refused})).is_err());
    assert!(session.execute("font.transform", &json!({"font": binary, "rotate": 10, "matrix": [1, 0, 0, 1, 0, 0], "output": refused})).is_err());
    assert!(session.execute("font.transform", &json!({"font": binary, "scale": 1000, "output": refused})).is_err(), "coordinates past ±32,767");
    assert!(session.execute("font.transform", &json!({"font": binary, "rotate": 90, "glyphs": ["nope"], "output": refused})).is_err());
    assert!(session.execute("font.transform", &json!({"font": binary, "unitsPerEm": 2048, "glyphTable": "t.txt", "output": refused})).is_err());
    assert!(!refused.exists());
    // Without an output the result goes beside the input under a derived name.
    let r = session.execute("font.transform", &json!({"font": binary, "unitsPerEm": 2000})).unwrap();
    assert!(r["output"].as_str().unwrap().ends_with("in.transformed.ttf"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn font_transform_scales_a_source_and_refuses_its_glyph_transforms() {
    let dir = scratch("source");
    let s = drawn();
    let source = dir.join("in.fontcraft");
    crate::io::save(s.font().unwrap(), &source, crate::io::SaveFormat::Native).unwrap();
    let mut session = Session::new();
    let out = dir.join("out.fontcraft");
    let r = session.execute("font.transform", &json!({"font": source, "unitsPerEm": 2000, "output": out})).unwrap();
    assert_eq!(r["unitsPerEm"], json!({"from": 1000, "to": 2000}));
    let (font, _) = crate::io::open(&out, None).unwrap();
    assert_eq!(font.units_per_em, 2000);
    let (input, _) = crate::io::open(&source, None).unwrap();
    assert_eq!(input.units_per_em, 1000, "the input is never changed");
    let refused = dir.join("refused.fontcraft");
    assert!(session.execute("font.transform", &json!({"font": source, "rotate": 90, "output": refused})).is_err());
    assert!(!refused.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn font_transform_converts_with_to_and_scales_cff_outlines() {
    let dir = scratch("to");
    let s = drawn();
    let ttf = fontcraft_compile::compile_ttf(s.font().unwrap(), &fontcraft_compile::Options::default()).unwrap();
    let binary = dir.join("in.ttf");
    std::fs::write(&binary, &ttf.bytes).unwrap();
    let mut session = Session::new();

    let r = session.execute("font.transform", &json!({"font": binary, "unitsPerEm": 2048, "to": "otf"})).unwrap();
    let otf = std::path::PathBuf::from(r["output"].as_str().unwrap());
    assert!(otf.to_string_lossy().ends_with("in.transformed.otf"), "{otf:?}");
    let font = fontcraft_tables::read(&std::fs::read(&otf).unwrap(), 0).unwrap();
    assert!(font.get(*b"CFF ").is_some() && font.get(*b"glyf").is_none(), "the outlines are converted to CFF");
    assert_eq!(upem_of(&otf), 2048);

    // The CFF font scales back: every coordinate, 205 for the square's 100, returns to its value.
    let back = dir.join("back.otf");
    let r = session.execute("font.transform", &json!({"font": otf, "unitsPerEm": 1000, "output": back})).unwrap();
    assert_eq!(r["unitsPerEm"], json!({"from": 2048, "to": 1000}));
    assert_eq!(upem_of(&back), 1000);
    let font = fontcraft_tables::read(&std::fs::read(&back).unwrap(), 0).unwrap();
    let head = font.get(*b"head").unwrap();
    let original = fontcraft_tables::read(&ttf.bytes, 0).unwrap();
    let original_head = original.get(*b"head").unwrap();
    assert_eq!(head[36..44], original_head[36..44], "the font box comes back");

    let refused = dir.join("refused.ttf");
    assert!(session.execute("font.transform", &json!({"font": binary, "unitsPerEm": 2048, "to": "pfa", "output": refused})).is_err());
    assert!(
        session.execute("font.transform", &json!({"font": otf, "rotate": 90, "output": refused})).is_err(),
        "a matrix on CFF outlines is refused for now"
    );
    assert!(!refused.exists());
    let _ = std::fs::remove_dir_all(&dir);
}
