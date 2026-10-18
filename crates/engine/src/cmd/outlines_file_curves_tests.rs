//! Behaviour tests of the whole-font curve and component commands of M12.29 on sources and on
//! TrueType fonts compiled from them, all built in code.

use serde_json::json;

use crate::Session;

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-outlines-curves-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `O`: a closed path with one cubic curve, which needs five quadratic pieces; `L`: a rectangle.
fn curved() -> Session {
    let mut s = Session::with_document();
    let o = json!([
        {"x": 0, "y": 0}, {"x": 0, "y": 550, "type": "offcurve"}, {"x": 450, "y": 1000, "type": "offcurve"}, {"x": 1000, "y": 1000, "type": "curve"},
        {"x": 1000, "y": 0}
    ]);
    s.execute("path.addPath", &json!({"glyph": "O", "nodes": o})).unwrap();
    let l = json!([{"x": 0, "y": 0}, {"x": 0, "y": 500}, {"x": 100, "y": 500}, {"x": 100, "y": 0}]);
    s.execute("path.addPath", &json!({"glyph": "L", "nodes": l})).unwrap();
    s
}

fn kinds(font: &fontcraft_model::Font, glyph: &str) -> Vec<fontcraft_model::NodeKind> {
    let g = font.glyph(glyph).unwrap();
    g.layers[0]
        .shapes
        .iter()
        .filter_map(|s| match s {
            fontcraft_model::Shape::Path(p) => Some(p.nodes.iter().map(|n| n.kind).collect::<Vec<_>>()),
            _ => None,
        })
        .flatten()
        .collect()
}

#[test]
fn outlines_convert_curves_on_a_source_and_a_binary() {
    use fontcraft_model::NodeKind;
    let dir = scratch("convert");
    let s = curved();
    let source = dir.join("in.fontcraft");
    crate::io::save(s.font().unwrap(), &source, crate::io::SaveFormat::Native).unwrap();
    let ttf = fontcraft_compile::compile_ttf(s.font().unwrap(), &fontcraft_compile::Options::default()).unwrap();
    let binary = dir.join("in.ttf");
    std::fs::write(&binary, &ttf.bytes).unwrap();
    let mut session = Session::new();

    // A source: every cubic curve becomes quadratic, the masters together.
    let r = session.execute("outlines.convertCurves", &json!({"input": source, "to": "quadratic"})).unwrap();
    let result = &r["results"][0];
    assert_eq!(result["kind"], json!("source"));
    assert_eq!(result["changed"], json!(["O", "L"]), "the contours of both are reversed as TrueType wants them");
    let out = std::path::PathBuf::from(result["output"].as_str().unwrap());
    assert_eq!(out, dir.join("in.quadratic.fontcraft"));
    let (font, _) = crate::io::open(&out, None).unwrap();
    let k = kinds(&font, "O");
    assert!(k.contains(&NodeKind::QCurve) && !k.contains(&NodeKind::Curve), "{k:?}");
    let (input, _) = crate::io::open(&source, None).unwrap();
    assert!(kinds(&input, "O").contains(&NodeKind::Curve), "the input is never changed");

    // A compiled font: the quadratic run comes back as one cubic curve in glyf.
    let out = dir.join("cubic.ttf");
    let r = session.execute("outlines.convertCurves", &json!({"inputs": [binary], "to": "cubic", "output": out})).unwrap();
    let result = &r["results"][0];
    assert_eq!(result["kind"], json!("ttf"));
    assert!(result["changed"].as_array().unwrap().contains(&json!("O")));
    assert_eq!(result["maxError"], json!(1.0));
    let after = fontcraft_tables::read(&std::fs::read(&out).unwrap(), 0).unwrap();
    assert_eq!(after.get(*b"head").unwrap()[52..54], [0, 1], "glyphDataFormat 1: glyf holds cubic curves");
    let before = fontcraft_tables::read(&ttf.bytes, 0).unwrap();
    for tag in [*b"name", *b"cmap", *b"OS/2", *b"post"] {
        assert_eq!(before.get(tag), after.get(tag), "{} is copied", fontcraft_tables::sfnt::tag_str(tag));
    }
    // And back: no cubic curve is left.
    let back = dir.join("back.ttf");
    session.execute("outlines.convertCurves", &json!({"input": out, "to": "quadratic", "output": back})).unwrap();
    let back = fontcraft_tables::read(&std::fs::read(&back).unwrap(), 0).unwrap();
    assert_eq!(back.get(*b"head").unwrap()[52..54], [0, 0]);

    // Refusals change nothing.
    let refused = dir.join("refused.ttf");
    for bad in [
        json!({"input": binary, "to": "sideways", "output": refused}),
        json!({"input": binary, "to": "cubic", "mixed": true, "output": refused}),
        json!({"input": binary, "to": "quadratic", "allCubic": true, "output": refused}),
        json!({"input": binary, "to": "cubic", "maxError": 0, "output": refused}),
        json!({"inputs": [binary, source], "to": "quadratic", "interpolatable": true, "outputDir": dir.join("set")}),
        json!({"input": source, "to": "quadratic", "mixed": true, "output": dir.join("mixed.fontcraft")}),
    ] {
        assert!(session.execute("outlines.convertCurves", &bad).is_err(), "{bad}");
    }
    assert!(!refused.exists() && !dir.join("set").exists() && !dir.join("mixed.fontcraft").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn outlines_componentize_with_a_reference_and_by_detection() {
    let dir = scratch("componentize");
    let mut s = Session::with_document();
    let square = |y: i64| json!([{"x": 0, "y": y}, {"x": 0, "y": y + 100}, {"x": 100, "y": y + 100}, {"x": 100, "y": y}]);
    s.execute("path.addPath", &json!({"glyph": "period", "nodes": square(0)})).unwrap();
    s.execute("path.addPath", &json!({"glyph": "dothigh", "nodes": square(300)})).unwrap();
    s.execute("glyph.newGlyph", &json!({"name": "colon"})).unwrap();
    for base in ["period", "dothigh"] {
        s.execute("glyph.addComponent", &json!({"glyph": "colon", "component": base})).unwrap();
    }
    let ufo_dir = dir.join("project");
    std::fs::create_dir_all(&ufo_dir).unwrap();
    let source = ufo_dir.join("Dots.ufo");
    crate::io::save(s.font().unwrap(), &source, crate::io::SaveFormat::Ufo).unwrap();
    let ttf = fontcraft_compile::compile_ttf(s.font().unwrap(), &fontcraft_compile::Options::default()).unwrap();
    let composite = ufo_dir.join("Dots-composite.ttf");
    std::fs::write(&composite, &ttf.bytes).unwrap();
    let mut session = Session::new();
    let flat = ufo_dir.join("Dots.ttf");
    session.execute("outlines.decompose", &json!({"input": composite, "output": flat})).unwrap();

    // The source beside the font says how colon is built.
    let r = session.execute("outlines.componentize", &json!({"font": flat})).unwrap();
    assert_eq!(r["changed"], json!(["colon"]));
    assert_eq!(r["reference"], json!(source.display().to_string()));
    let out = std::path::PathBuf::from(r["output"].as_str().unwrap());
    assert_eq!(out, ufo_dir.join("Dots.components.ttf"));
    let font = fontcraft_tables::read(&std::fs::read(&out).unwrap(), 0).unwrap();
    let again = session.execute("outlines.decompose", &json!({"input": out, "output": dir.join("again.ttf")})).unwrap();
    assert_eq!(again["results"][0]["changed"], json!(["colon"]), "colon is a composite again");
    assert!(font.get(*b"glyf").is_some());

    // Without a source, matching contours: dothigh is period moved up, and colon two periods.
    let r = session.execute("outlines.componentize", &json!({"font": flat, "detect": true, "output": dir.join("detected.ttf")})).unwrap();
    assert_eq!(r["changed"], json!(["dothigh", "colon"]));

    // No source beside the font, and none named: refused, nothing written.
    let alone = dir.join("alone.ttf");
    std::fs::copy(&flat, &alone).unwrap();
    assert!(session.execute("outlines.componentize", &json!({"font": alone})).is_err());
    assert!(!dir.join("alone.components.ttf").exists());
    assert!(session.execute("outlines.componentize", &json!({"font": flat, "detect": true, "reference": source})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
