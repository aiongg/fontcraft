//! Behaviour tests of the whole-font outline commands of M12.28 on a source and on a TrueType
//! font compiled from it, both built in code.

use serde_json::json;

use crate::Session;

/// A session whose font has `A`: two overlapping squares; `B`: one square.
fn drawn() -> Session {
    let mut s = Session::with_document();
    let rect = |x0: f64, y0: f64, x1: f64, y1: f64| json!([{"x": x0, "y": y0}, {"x": x1, "y": y0}, {"x": x1, "y": y1}, {"x": x0, "y": y1}]);
    for (glyph, nodes) in [("A", rect(0.0, 0.0, 100.0, 100.0)), ("A", rect(50.0, 50.0, 150.0, 150.0)), ("B", rect(0.0, 0.0, 100.0, 100.0))] {
        s.execute("path.addPath", &json!({"glyph": glyph, "nodes": nodes})).unwrap();
    }
    s
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-outlines-file-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn paths_of(font: &fontcraft_model::Font, glyph: &str) -> usize {
    let g = font.glyph(glyph).unwrap();
    g.layers[0].shapes.iter().filter(|s| matches!(s, fontcraft_model::Shape::Path(_))).count()
}

#[test]
fn outlines_remove_overlaps_on_a_source_and_a_binary() {
    let dir = scratch("overlaps");
    let s = drawn();
    let source = dir.join("in.fontcraft");
    crate::io::save(s.font().unwrap(), &source, crate::io::SaveFormat::Native).unwrap();
    let options = fontcraft_compile::Options { remove_overlap: false, ..fontcraft_compile::Options::default() };
    let ttf = fontcraft_compile::compile_ttf(s.font().unwrap(), &options).unwrap();
    let binary = dir.join("in.ttf");
    std::fs::write(&binary, &ttf.bytes).unwrap();

    let mut session = Session::new();
    let out = dir.join("out.fontcraft");
    let r = session.execute("outlines.removeOverlaps", &json!({"input": source, "output": out})).unwrap();
    assert_eq!(r["changed"], json!(["A"]));
    let (font, _) = crate::io::open(&out, None).unwrap();
    assert_eq!(paths_of(&font, "A"), 1, "the two squares are merged");
    assert_eq!(paths_of(&font, "B"), 1);
    let (input, _) = crate::io::open(&source, None).unwrap();
    assert_eq!(paths_of(&input, "A"), 2, "the input is never changed");

    let out = dir.join("out.ttf");
    let r = session.execute("outlines.removeOverlaps", &json!({"input": binary, "output": out})).unwrap();
    assert_eq!(r["kind"], json!("ttf"));
    assert_eq!(r["changed"], json!(["A"]));
    let before = fontcraft_tables::read(&std::fs::read(&binary).unwrap(), 0).unwrap();
    let after = fontcraft_tables::read(&std::fs::read(&out).unwrap(), 0).unwrap();
    for tag in [*b"name", *b"cmap", *b"OS/2", *b"post"] {
        assert_eq!(before.get(tag), after.get(tag), "{} is copied", fontcraft_tables::sfnt::tag_str(tag));
    }
    assert_ne!(before.get(*b"glyf"), after.get(*b"glyf"));

    // Without an output the result would replace the input: refused, nothing written.
    assert!(session.execute("outlines.removeOverlaps", &json!({"input": binary})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn outlines_repairs_on_a_cff_font() {
    let dir = scratch("cff");
    let s = drawn();
    let font = s.font().unwrap();
    let options = fontcraft_compile::Options { remove_overlap: false, ..fontcraft_compile::Options::default() };
    let otf = fontcraft_compile::compile(font, &font.default_target().unwrap(), fontcraft_compile::Flavour::Cff, &options).unwrap();
    let binary = dir.join("in.otf");
    std::fs::write(&binary, &otf.bytes).unwrap();

    let mut session = Session::new();
    let out = dir.join("out.otf");
    let r = session.execute("outlines.removeOverlaps", &json!({"input": binary, "output": out, "keepUnusedSubroutines": true})).unwrap();
    assert_eq!(r["kind"], json!("otf"));
    assert_eq!(r["changed"], json!(["A"]));
    let before = fontcraft_tables::read(&std::fs::read(&binary).unwrap(), 0).unwrap();
    let after = fontcraft_tables::read(&std::fs::read(&out).unwrap(), 0).unwrap();
    for tag in before.tags().into_iter().filter(|t| *t != *b"CFF ") {
        assert_eq!(before.get(tag), after.get(tag), "{} is copied", fontcraft_tables::sfnt::tag_str(tag));
    }
    assert_ne!(before.get(*b"CFF "), after.get(*b"CFF "));

    // The compiler writes the PostScript convention, which is the default for CFF.
    let r = session.execute("outlines.correctDirection", &json!({"input": binary, "output": dir.join("direction.otf"), "glyphs": ["B"]})).unwrap();
    assert_eq!(r["changed"], json!([]));
    let r = session.execute("outlines.decompose", &json!({"input": binary, "output": dir.join("decomposed.otf")}));
    assert!(r.is_err(), "CFF outlines have no components");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn outlines_direction_extremes_round_and_decompose() {
    let dir = scratch("others");
    let s = drawn();
    let options = fontcraft_compile::Options { remove_overlap: false, ..fontcraft_compile::Options::default() };
    let ttf = fontcraft_compile::compile_ttf(s.font().unwrap(), &options).unwrap();
    let binary = dir.join("in.ttf");
    std::fs::write(&binary, &ttf.bytes).unwrap();
    let mut session = Session::new();

    let r = session
        .execute("outlines.correctDirection", &json!({"input": binary, "output": dir.join("d.ttf"), "flavour": "reverse", "glyphs": ["B"]}))
        .unwrap();
    assert_eq!(r["changed"], json!(["B"]));
    let r = session.execute("outlines.correctDirection", &json!({"input": binary, "output": dir.join("d2.ttf"), "flavour": "sideways"}));
    assert!(r.is_err());

    let r = session.execute("outlines.addExtremes", &json!({"input": binary, "output": dir.join("e.ttf")})).unwrap();
    assert_eq!(r["changed"], json!([]), "squares have their extremes");
    assert!(session.execute("outlines.addExtremes", &json!({"input": binary, "output": dir.join("e2.ttf"), "minSegment": 7})).is_err());

    let r = session.execute("outlines.roundCoordinates", &json!({"input": binary, "output": dir.join("r.ttf"), "grid": 40, "glyphs": "A"})).unwrap();
    assert_eq!(r["changed"], json!(["A"]));
    assert!(r["valuesChanged"].as_u64().unwrap() > 0);

    std::fs::create_dir_all(dir.join("dec")).unwrap();
    let r = session.execute("outlines.decompose", &json!({"inputs": [binary], "outputDir": dir.join("dec")})).unwrap();
    assert_eq!(r["results"][0]["changed"], json!([]), "no composites");
    assert!(dir.join("dec").join("in.ttf").exists());

    assert!(session.execute("outlines.decompose", &json!({"inputs": [binary, binary], "output": dir.join("x.ttf")})).is_err());
    // Two inputs that would be written to one file are refused before anything is written.
    let other = dir.join("other");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::copy(&binary, other.join("in.ttf")).unwrap();
    let clash = dir.join("clash");
    std::fs::create_dir_all(&clash).unwrap();
    assert!(session.execute("outlines.decompose", &json!({"inputs": [binary, other.join("in.ttf")], "outputDir": clash})).is_err());
    assert!(!clash.join("in.ttf").exists(), "nothing is written");
    assert!(session.execute("outlines.decompose", &json!({"inputs": [binary, binary], "inPlace": true})).is_err(), "the same input twice");
    assert!(session.execute("outlines.roundCoordinates", &json!({"input": binary, "output": dir.join("l.ttf"), "layer": "public.default"})).is_err());
    assert!(session.execute("outlines.roundCoordinates", &json!({"input": dir.join("missing.ttf"), "output": dir.join("m.ttf")})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// The `.ufo` chosen beside a font is the first by name in any listing order, and a listing over
/// the limit is refused rather than cut.
#[test]
fn the_source_beside_a_font_is_chosen_by_name_in_any_order() {
    use std::path::PathBuf;
    let names = ["c.ufo", "a.ufo", "b.ufo"];
    for order in [[0, 1, 2], [2, 1, 0], [1, 2, 0]] {
        let paths = order.iter().map(|&i| PathBuf::from(names[i]));
        assert_eq!(super::first_by_name(paths, 3), Ok(Some(PathBuf::from("a.ufo"))));
    }
    assert_eq!(super::first_by_name(names.iter().map(|n| PathBuf::from(*n)), 2), Err(()));
    assert_eq!(super::first_by_name(std::iter::empty(), 2), Ok(None));

    let dir = scratch("source-beside");
    for name in ["b.ufo", "a.ufo"] {
        std::fs::create_dir_all(dir.join(name)).unwrap();
    }
    assert_eq!(super::source_beside(&dir.join("font.ttf")).unwrap(), dir.join("a.ufo"));
}
