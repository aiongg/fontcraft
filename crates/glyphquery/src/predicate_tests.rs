//! The predicate language on a font built in code: the handbook's examples, the attributes
//! read from layers and outlines, `class(…)` over the font's feature classes, the Apple
//! predicate forms (`like`, `in {…}`, `between`, `=>`) and hostile input.

use std::sync::Arc;

use fontcraft_glyphdata::GlyphData;
use fontcraft_model::{Alignment, Anchor, Component, Font, Glyph, Hint, Layer, Node, NodeKind, Path, Shape, Snippet};
use fontcraft_plist::{Dict, Value};

use crate::predicate::{evaluate, parse, predicate_len};
use crate::*;

fn hint(kind: &str) -> Hint {
    let mut d = Dict::new();
    d.insert("type", Value::String(kind.into()));
    Hint { family: fontcraft_model::Family::Glyphs, value: Value::Dict(d) }
}

fn extra_layer(id: &str, master: &str) -> Layer {
    let mut l = Layer::new(master, 600.0);
    l.id = id.into();
    l
}

fn class(name: &str, code: &str) -> Snippet {
    Snippet { name: name.into(), code: code.into(), ..Snippet::default() }
}

/// `Font::new` (`.notdef`, `space`, A–Z, a–z) plus:
/// - A: a closed curve from (0,0) bulging to y 75 with off-curve points at y 100, width 600,
///   anchor `top` at (100, 700).
/// - Aacute: components A and acutecomb, aligned by the usual rule; B: a component and a path.
/// - C: a TrueType stem hint; D: a corner component; E: a colour palette layer; F: an SVG layer.
/// - a: three layers; b: tag `ipa`; c: tag `lang`; d: tag `todo`; e: note "done".
/// - alef-ar with the stored direction RTL.
/// - Feature classes Round (`o O @Bowls`), Bowls (`b d p q`), Tall (`b d l $[case == upper]`)
///   and a disabled class Off.
fn font() -> Font {
    let mut font = Font::new("Test");
    for (name, u) in [("acutecomb", 0x301), ("Aacute", 0xC1), ("alef-ar", 0x627)] {
        font.add_glyph(name, vec![u], 600.0).unwrap();
    }
    let edit = |font: &mut Font, name: &str, f: &dyn Fn(&mut Glyph)| {
        let mut g = font.glyph_mut(name).unwrap();
        f(&mut g);
    };
    edit(&mut font, "A", &|g| {
        let nodes = vec![
            Node::new(0.0, 0.0, NodeKind::Line),
            Node::new(0.0, 100.0, NodeKind::OffCurve),
            Node::new(200.0, 100.0, NodeKind::OffCurve),
            Node::new(200.0, 0.0, NodeKind::Curve),
        ];
        g.layers[0].shapes = vec![Shape::Path(Path::new(true, nodes))];
        g.layers[0].anchors.push(Anchor::new("top", 100.0, 700.0));
    });
    edit(&mut font, "Aacute", &|g| {
        g.layers[0].shapes = vec![Shape::Component(Component::new("A", 0.0, 0.0)), Shape::Component(Component::new("acutecomb", 0.0, 0.0))];
    });
    edit(&mut font, "B", &|g| {
        g.layers[0].shapes = vec![Shape::Component(Component::new("A", 0.0, 0.0)), Shape::Path(Path::rect(0.0, 0.0, 10.0, 10.0))];
    });
    edit(&mut font, "C", &|g| g.layers[0].info.hints.push(hint("TTStem")));
    edit(&mut font, "D", &|g| g.layers[0].info.hints.push(hint("Corner")));
    edit(&mut font, "E", &|g| {
        let mut l = extra_layer("colour0", "m01");
        l.info.attrs.color_palette = Some(fontcraft_model::PaletteEntry::Index(0));
        g.layers.push(l);
    });
    edit(&mut font, "F", &|g| {
        let mut l = extra_layer("svg", "m01");
        l.info.attrs.svg = true;
        g.layers.push(l);
    });
    edit(&mut font, "a", &|g| {
        g.layers.push(extra_layer("backup1", "m01"));
        g.layers.push(extra_layer("backup2", "m01"));
    });
    edit(&mut font, "b", &|g| g.info.tags.push("ipa".into()));
    edit(&mut font, "c", &|g| g.info.tags.push("lang".into()));
    edit(&mut font, "d", &|g| g.info.tags.push("todo".into()));
    edit(&mut font, "e", &|g| g.info.note = "done".into());
    edit(&mut font, "alef-ar", &|g| g.info.direction = Some("RTL".into()));
    let features = Arc::make_mut(&mut font.features);
    features.classes = vec![
        class("Round", "o O @Bowls # the round letters"),
        class("Bowls", "b d p q"),
        class("Tall", "b d l $[case == upper]"),
        Snippet { disabled: true, ..class("Off", "a") },
    ];
    font
}

fn data() -> GlyphData {
    GlyphData::new().unwrap()
}

fn run(font: &Font, data: &GlyphData, text: &str) -> Vec<String> {
    let env = Env::new(font, data, None).unwrap();
    evaluate(&parse(text).unwrap_or_else(|e| panic!("{text}: {e}")), &env).unwrap()
}

#[test]
fn the_handbooks_examples_select_their_glyphs() {
    let (font, data) = (font(), data());
    let cases: &[(&str, &[&str])] = &[
        // Components are resolved: B and Aacute draw A.
        ("layer.maxY == 75", &["A", "B", "Aacute"]),
        ("name beginswith \"A\" AND layer.maxX == 200 AND layer.minX == 0 AND layer.minY == 0", &["A", "Aacute"]),
        ("layer.RSB == 400 AND layer.LSB == 0", &["A", "B", "Aacute"]),
        ("layer.anchors.top.y > 600", &["A"]),
        ("isAligned == true", &["Aacute"]),
        ("layer.isAligned", &["Aacute"]),
        ("hasHints == true", &["C"]),
        ("hasTrueTypeHints", &["C"]),
        ("hasPostScriptHints", &[]),
        ("hasCorners", &["D"]),
        ("isColorGlyph == true", &["E"]),
        ("isAnyColorGlyph == true", &["E", "F"]),
        ("hasSpecialLayers", &["E", "F"]),
        ("countOfLayers == 3", &["a"]),
        ("direction == RTL", &["alef-ar"]),
        ("\"ipa\" in tags OR (\"lang\" in tags AND case == lower)", &["b", "c"]),
        ("name in class(Round) AND name in class(Tall)", &["O", "b", "d"]),
        ("name in class(Bowls) AND case == lower", &["b", "d", "p", "q"]),
        ("name in class(Off)", &[]),
        ("script == \"greek\" AND case == upper", &[]),
        ("name endswith \".sc\"", &[]),
    ];
    for (text, want) in cases {
        assert_eq!(run(&font, &data, text), *want, "{text}");
    }
    let not_done = run(&font, &data, "\"todo\" in tags OR NOT note contains \"done\"");
    assert!(not_done.contains(&"d".to_string()) && !not_done.contains(&"e".to_string()));
    assert_eq!(not_done.len(), font.glyphs.len() - 1);
    // Font View has no classes of its own: the font's are read when a predicate asks.
    let hidden = Catalog::new(Env::new(&font, &data, None).unwrap())
        .select(&Selection { rules: Some(parse("name in class(Tall)").unwrap()), ..Selection::default() })
        .unwrap();
    assert!(hidden.contains(&"l".to_string()) && hidden.contains(&"Q".to_string()) && !hidden.contains(&"a".to_string()));
}

#[test]
fn apple_predicate_forms_read_back_from_their_text() {
    let (font, data) = (font(), data());
    for (text, want) in [
        ("name like \"?acute\"", vec!["Aacute"]),
        ("name LIKE 'alef*'", vec!["alef-ar"]),
        ("name in {\"b\", \"c\", \"zz\"}", vec!["b", "c"]),
        ("case in {upper, smallCaps} AND name beginswith \"A\"", vec!["A", "Aacute"]),
        ("\"todo\" in tags OR tags in {\"ipa\"}", vec!["b", "d"]),
        ("countOfLayers between {2, 3}", vec!["E", "F", "a"]),
        ("countOfLayers => 3", vec!["a"]),
        ("countOfLayers =< 1 AND name == \"b\"", vec!["b"]),
        ("isAligned in {yes}", vec!["Aacute"]),
    ] {
        assert_eq!(run(&font, &data, text), want, "{text}");
        let expr = parse(text).unwrap();
        let printed = expr.to_string();
        assert_eq!(parse(&printed).unwrap_or_else(|e| panic!("{printed}: {e}")), expr, "{text}");
    }
}

#[test]
fn hostile_predicates_and_classes_fail_with_a_message() {
    for (text, needle) in [
        ("name in {", "not closed"),
        ("name in {\"a\" \"b\"}", "`,`"),
        ("name in {}", "value"),
        ("countOfLayers between {1}", "two numbers"),
        ("countOfLayers between 1", "braces"),
        ("name between {1, 2}", "text attributes"),
        ("case in {huge}", "one of"),
        ("name like 3", "text"),
        ("mastersCompatible", "not supported"),
    ] {
        let e = parse(text).unwrap_err();
        assert!(e.0.contains(needle), "{text}: {e}");
    }
    assert!(parse(&format!("name like \"{}\"", "*".repeat(MAX_PATTERN_BYTES + 1))).is_err());
    assert_eq!(predicate_len("name matches \"[a-z]\"] rest"), Some(20));
    assert_eq!(predicate_len("name == 'a]b'"), None);
    assert_eq!(predicate_len("name == \"\\\"]\"]"), Some(13));

    let data = data();
    let mut font = Font::new("Test");
    let classes = &mut Arc::make_mut(&mut font.features).classes;
    classes.push(class("X", "@Y"));
    classes.push(class("Y", "a @X"));
    for i in 0..20 {
        classes.push(class(&format!("C{i}"), &format!("@C{}", i + 1)));
    }
    classes.push(class("C20", "a"));
    classes.push(class("Open", "a $[name == \"b\""));
    classes.push(class("Many", &"@Wide ".repeat(100)));
    classes.push(class("Wide", "$[true]"));
    for (name, needle) in [("X", "refers to itself"), ("C0", "levels deep"), ("Open", "not closed"), ("Nope", "no class")] {
        let e = feature_classes(&font, &data, None, Some([name].as_slice())).unwrap_err();
        assert!(e.0.contains(needle), "{name}: {e}");
    }
    assert_eq!(feature_classes(&font, &data, None, Some(["Many"].as_slice())).unwrap().get("Many").map(|m| m.len()), Some(font.glyphs.len()));
    // A broken class leaves Font View's class tests false instead of failing the query.
    assert_eq!(run(&font, &data, "name in class(C20)"), Vec::<String>::new());
}

#[test]
fn alignment_follows_the_font_setting_and_the_component_rule() {
    let (mut font, data) = (font(), data());
    {
        let mut b = font.glyph_mut("B").unwrap();
        if let Some(Shape::Component(c)) = b.layers[0].shapes.get_mut(0) {
            c.info.alignment = Alignment::Forced;
        }
    }
    assert_eq!(run(&font, &data, "layer.isAligned"), vec!["B", "Aacute"]);
    Arc::make_mut(&mut font.info).settings.disables_automatic_alignment = Some(true);
    assert_eq!(run(&font, &data, "layer.isAligned OR isAligned"), Vec::<String>::new());
}
