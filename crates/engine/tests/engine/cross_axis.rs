//! Cross-axis mappings and Interpolate Fonts: the mappings survive designspace and `.glyphs`,
//! refused requests change nothing, two fonts interpolate and extrapolate point by point, and
//! incompatible fonts are refused with the first difference.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::cmd::cross_axis::{CrossAxisMapping, CrossAxisMappings, avar_mappings, mappings};
use fontcraft_model::{Anchor, Axis, Component, Font, Glyph, GlyphTable, Layer, Path, Shape};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

/// Runs a command that must fail and checks that the font is the same object as before.
fn refused(s: &mut Session, id: &str, p: Value) -> String {
    let before = s.doc().unwrap().font.clone();
    let docs = s.documents().len();
    let e = s.execute(id, &p).expect_err(id).to_string();
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "{id} {p}: a refused command changed the font");
    assert_eq!(s.documents().len(), docs, "{id} {p}: a refused command opened a document");
    e
}

/// Two masters on a weight and a width axis: Regular at (400, 100), Bold Condensed at (700, 75).
fn variable_font() -> Font {
    let mut f = Font::new("Cross Sans");
    f.axes = Arc::new(vec![
        Axis { name: "Weight".into(), tag: "wght".into(), ..Default::default() },
        Axis { name: "Width".into(), tag: "wdth".into(), ..Default::default() },
    ]);
    let mut masters = (*f.masters).clone();
    masters[0].location = vec![400.0, 100.0];
    let mut bold = masters[0].clone();
    bold.id = "m02".into();
    bold.name = "Bold Condensed".into();
    bold.location = vec![700.0, 75.0];
    masters.push(bold);
    f.masters = Arc::new(masters);
    let glyphs = f
        .glyphs
        .iter()
        .map(|g| {
            let mut g = Glyph::clone(g);
            let mut layer = g.layers[0].clone();
            layer.id = "m02".into();
            layer.master = "m02".into();
            g.layers.push(layer);
            Arc::new(g)
        })
        .collect();
    f.glyphs = GlyphTable::from_glyphs(glyphs);
    f
}

fn expected() -> CrossAxisMappings {
    CrossAxisMappings {
        description: Some("Heavy weights narrow".into()),
        mappings: vec![
            CrossAxisMapping {
                description: Some("bold is condensed".into()),
                input: vec![("wght".into(), 700.0), ("wdth".into(), 100.0)],
                output: vec![("wdth".into(), 87.5)],
            },
            CrossAxisMapping { description: None, input: vec![("wght".into(), 550.0)], output: vec![("wght".into(), 520.0), ("wdth".into(), 90.0)] },
        ],
    }
}

fn with_mappings() -> Session {
    let mut s = Session::new();
    s.add_document(variable_font(), None).unwrap();
    let r = run(
        &mut s,
        "font.setCrossAxisMappings",
        json!({
            "description": "  Heavy weights narrow ",
            "mappings": [
                // Keys in any order come back in the font's axis order.
                {"input": {"wdth": 100, "wght": 700}, "output": {"wdth": 87.5}, "description": "bold is condensed"},
                {"input": {"wght": 550}, "output": {"wdth": 90, "wght": 520}},
            ],
        }),
    );
    assert_eq!(r, expected().to_json());
    s
}

#[test]
fn font_set_cross_axis_mappings_survive_designspace_and_glyphs() {
    let s = with_mappings();
    assert_eq!(mappings(s.font().unwrap()), expected());
    let avar = avar_mappings(s.font().unwrap());
    assert_eq!(avar.len(), 2);
    assert_eq!(avar[0].output, vec![(*b"wdth", 87.5)]);

    let dir = std::env::temp_dir().join(format!("fontcraft-cross-axis-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (file, params) in [("Cross.designspace", json!({})), ("Cross.glyphs", json!({"version": 3}))] {
        let mut s = with_mappings();
        let path = dir.join(file);
        let mut p = params.clone();
        p["path"] = json!(path);
        run(&mut s, "file.saveAs", p);
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": path}));
        assert_eq!(mappings(back.font().unwrap()), expected(), "{file}");
    }
    std::fs::remove_dir_all(&dir).unwrap();

    // An empty list without a description removes the key; doing it again changes nothing.
    let mut s = with_mappings();
    run(&mut s, "font.setCrossAxisMappings", json!({"mappings": []}));
    assert!(mappings(s.font().unwrap()).is_empty());
    assert!(s.font().unwrap().info.user_data.get(fontcraft_model::fc_keys::CROSS_AXIS_MAPPINGS).is_none());
    let cleared = s.doc().unwrap().font.clone();
    run(&mut s, "font.setCrossAxisMappings", json!({"mappings": []}));
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &cleared));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(mappings(s.font().unwrap()), expected(), "setting the mappings is one undo step");
}

#[test]
fn font_set_cross_axis_mappings_refuses_bad_mappings_and_changes_nothing() {
    let mut s = with_mappings();
    let one = |input: Value, output: Value| json!({"mappings": [{"input": input, "output": output}]});
    let e = refused(&mut s, "font.setCrossAxisMappings", one(json!({"opsz": 12}), json!({"wght": 400})));
    assert!(e.contains("no axis `opsz`") && e.contains("wght, wdth"), "{e}");
    let e = refused(&mut s, "font.setCrossAxisMappings", one(json!({}), json!({"wght": 400})));
    assert!(e.contains("names no axis"), "{e}");
    let e = refused(&mut s, "font.setCrossAxisMappings", one(json!({"wght": 400}), json!({"wdth": "wide"})));
    assert!(e.contains("mappings[0].output.wdth"), "{e}");
    refused(&mut s, "font.setCrossAxisMappings", one(json!({"wght": 400}), json!({"wdth": 1e300})));
    refused(&mut s, "font.setCrossAxisMappings", json!({"mappings": [1]}));
    refused(&mut s, "font.setCrossAxisMappings", json!({"mappings": [], "description": "x".repeat(5000)}));
    refused(&mut s, "font.setCrossAxisMappings", json!({"mappings": [], "description": "bell\u{7}"}));
    let many: Vec<Value> = (0..1025).map(|i| json!({"input": {"wght": i}, "output": {"wdth": 100}})).collect();
    refused(&mut s, "font.setCrossAxisMappings", json!({"mappings": many}));

    // A font without axes has nothing to map.
    let mut plain = Session::with_document();
    let e = refused(&mut plain, "font.setCrossAxisMappings", one(json!({"wght": 400}), json!({"wght": 500})));
    assert!(e.contains("no axes"), "{e}");
}

/// A single-master font: `A` a rectangle with a `top` anchor, `B` a component of `A`, the
/// ascender, and the kerning pair A V. The scales the tests use are exact binary
/// fractions, so every expected number is exact.
fn simple_font(family: &str, scale: f64, ascender: f64, kern: f64) -> Font {
    let mut f = Font::new(family);
    {
        let mut a = f.glyph_mut("A").unwrap();
        a.layers[0].width = 600.0 * scale;
        a.layers[0].shapes.push(Shape::Path(Path::rect(50.0 * scale, 0.0, 550.0 * scale, 700.0)));
        a.layers[0].anchors.push(Anchor::new("top", 300.0 * scale, 700.0));
    }
    {
        let mut b = f.glyph_mut("B").unwrap();
        b.layers[0].shapes.push(Shape::Component(Component::new("A", 10.0 * scale, 0.0)));
    }
    Arc::make_mut(&mut f.masters)[0].metric_values[0].pos = ascender;
    Arc::make_mut(&mut f.kerning).ltr.set("m01", "A", "V", kern);
    f
}

/// A session with the two fonts open; their titles.
fn two_fonts(a: Font, b: Font) -> (Session, String, String) {
    let mut s = Session::new();
    s.add_document(a, None).unwrap();
    s.add_document(b, None).unwrap();
    let titles: Vec<String> = s.documents().iter().map(|d| d.title.clone()).collect();
    (s, titles[0].clone(), titles[1].clone())
}

#[test]
fn font_interpolate_fonts_at_50_and_150_percent() {
    let (mut s, a, b) = two_fonts(simple_font("Narrow", 1.0, 800.0, -50.0), simple_font("Wide", 1.25, 900.0, -100.0));
    let before: Vec<Arc<Font>> = s.documents().iter().map(|d| d.font.clone()).collect();
    for (amount, t) in [(50.0, 0.5), (150.0, 1.5)] {
        let r = run(&mut s, "font.interpolateFonts", json!({"a": a, "b": b, "amount": amount}));
        let font = s.font().unwrap();
        assert_eq!(r["title"], json!(s.doc().unwrap().title), "the new font is the active document");
        assert_eq!(r["glyphs"], json!(font.glyphs.len()));
        assert_eq!(font.family_name, "Narrow");
        let at = |x: f64, y: f64| x + (y - x) * t;
        let layer = &font.glyph("A").unwrap().layers[0];
        assert_eq!(layer.width, at(600.0, 750.0));
        let Shape::Path(p) = &layer.shapes[0] else { panic!("A keeps its path") };
        let xs: Vec<f64> = p.nodes.iter().map(|n| n.x).collect();
        let expected: Vec<f64> = Path::rect(at(50.0, 62.5), 0.0, at(550.0, 687.5), 700.0).nodes.iter().map(|n| n.x).collect();
        assert_eq!(xs, expected);
        assert_eq!((layer.anchors[0].x, layer.anchors[0].y), (at(300.0, 375.0), 700.0));
        let Shape::Component(c) = &font.glyph("B").unwrap().layers[0].shapes[0] else { panic!("B keeps its component") };
        assert_eq!(c.affine().as_coeffs()[4], at(10.0, 12.5));
        assert_eq!(font.masters[0].metric_values[0].pos, at(800.0, 900.0));
        assert_eq!(font.kerning.ltr.get("m01", "A", "V"), Some(at(-50.0, -100.0)));
    }
    assert_eq!(s.documents().len(), 4, "each call opens a new document");
    for (d, font) in s.documents().iter().zip(&before) {
        assert!(Arc::ptr_eq(&d.font, font), "the two fonts are unchanged");
    }

    // 0 and 100 give the two fonts' own numbers.
    run(&mut s, "font.interpolateFonts", json!({"a": a, "b": b, "amount": 100}));
    assert_eq!(s.font().unwrap().glyph("A").unwrap().layers[0].width, 750.0);
    run(&mut s, "font.interpolateFonts", json!({"a": a, "b": b, "amount": 0}));
    assert_eq!(s.font().unwrap().glyph("A").unwrap().layers[0].width, 600.0);
}

#[test]
fn font_interpolate_fonts_refuses_incompatible_fonts_with_the_first_difference() {
    let mut extra_node = simple_font("Other", 1.0, 800.0, 0.0);
    {
        let mut a = extra_node.glyph_mut("A").unwrap();
        let Shape::Path(p) = &mut a.layers[0].shapes[0] else { panic!() };
        p.nodes.push(fontcraft_model::Node::new(300.0, 750.0, fontcraft_model::NodeKind::Line));
    }
    let (mut s, a, b) = two_fonts(simple_font("Base", 1.0, 800.0, 0.0), extra_node);
    let e = refused(&mut s, "font.interpolateFonts", json!({"a": a, "b": b, "amount": 50}));
    assert!(e.contains("glyph `A`") && e.contains("4 nodes in the first font and 5 in the second"), "{e}");

    let mut extra_glyph = simple_font("Other", 1.0, 800.0, 0.0);
    let mut alt = Glyph::new("A.alt");
    alt.layers.push(Layer::new("m01", 600.0));
    extra_glyph.glyphs.push(alt);
    let (mut s, a, b) = two_fonts(simple_font("Base", 1.0, 800.0, 0.0), extra_glyph);
    let e = refused(&mut s, "font.interpolateFonts", json!({"a": a, "b": b, "amount": 50}));
    assert!(e.contains("`A.alt` is in the second font only"), "{e}");

    let mut other_component = simple_font("Other", 1.0, 800.0, 0.0);
    {
        let mut g = other_component.glyph_mut("B").unwrap();
        g.layers[0].shapes[0] = Shape::Component(Component::new("C", 0.0, 0.0));
    }
    let (mut s, a, b) = two_fonts(simple_font("Base", 1.0, 800.0, 0.0), other_component);
    let e = refused(&mut s, "font.interpolateFonts", json!({"a": a, "b": b, "amount": 50}));
    assert!(e.contains("glyph `B`") && e.contains("of `A` in the first font and of `C` in the second"), "{e}");

    let (mut s, a, b) = two_fonts(simple_font("Base", 1.0, 800.0, 0.0), variable_font());
    let e = refused(&mut s, "font.interpolateFonts", json!({"a": a, "b": b, "amount": 50}));
    assert!(e.contains("2 masters"), "{e}");

    // Extrapolating past the coordinate limit is refused, and so is an amount out of range.
    let (mut s, a, b) = two_fonts(simple_font("Base", 1.0, 800.0, 0.0), simple_font("Wide", 3.0, 800.0, 0.0));
    let e = refused(&mut s, "font.interpolateFonts", json!({"a": a, "b": b, "amount": 99_000}));
    assert!(e.contains("beyond"), "{e}");
    refused(&mut s, "font.interpolateFonts", json!({"a": a, "b": b, "amount": 200_000}));
    refused(&mut s, "font.interpolateFonts", json!({"a": a, "b": "no such font", "amount": 50}));
}
