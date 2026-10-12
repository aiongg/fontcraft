//! Behaviour tests for automatic layers, recipe directives and anchor expressions (M13.16), run
//! through the engine hook [`crate::expressions::follow_changes`] as the engine runs it.

use std::sync::Arc;

use fontcraft_model::fc_keys::ANCHOR_EXPRESSIONS;
use fontcraft_model::plist::{Dict, Str, Value};
use fontcraft_model::{Anchor, Font, Glyph, GlyphTable, Layer, Shape, metric};

use crate::auto_layer::{AutoRecipe, parse_auto_recipe, set_layer_recipe};
use crate::expressions::{AnchorExpressions, anchor_expressions, evaluate_glyphs, follow_changes, set_anchor_expressions};
use crate::propagate::effective_anchors;
use crate::{ComponentsError, MAX_RECIPE_BYTES};

const M: &str = "m01";

fn glyph(name: &str, width: f64, anchors: &[(&str, f64, f64)]) -> Glyph {
    let mut g = Glyph::new(name);
    let mut layer = Layer::new(M, width);
    layer.anchors = anchors.iter().map(|(n, x, y)| Anchor::new(*n, *x, *y)).collect();
    g.layers.push(layer);
    g
}

fn font(glyphs: Vec<Glyph>) -> Font {
    let mut f = Font::new("Auto");
    f.glyphs = GlyphTable::from_glyphs(glyphs.into_iter().map(Arc::new).collect());
    f
}

fn edit(f: &Font, name: &str, change: impl FnOnce(&mut Layer)) -> Font {
    let mut next = f.clone();
    let mut g = next.glyph_mut(name).unwrap();
    change(g.layers.first_mut().unwrap());
    drop(g);
    next
}

/// The components of `name`'s master layer as (glyph, dx, dy).
fn components(f: &Font, name: &str) -> Vec<(String, f64, f64)> {
    f.glyph(name).unwrap().layers[0]
        .shapes
        .iter()
        .filter_map(|s| match s {
            Shape::Component(c) => {
                let k = c.affine().as_coeffs();
                Some((c.glyph.clone(), k[4], k[5]))
            }
            _ => None,
        })
        .collect()
}

fn anchor(f: &Font, glyph: &str, name: &str) -> (f64, f64) {
    let a = f.glyph(glyph).unwrap().layers[0].anchors.iter().find(|a| a.name == name).unwrap().clone();
    (a.x, a.y)
}

fn with_expressions(f: &Font, glyph: &str, name: &str, x: Option<&str>, y: Option<&str>) -> Font {
    edit(f, glyph, |l| {
        let a = l.anchors.iter_mut().find(|a| a.name == name).unwrap();
        set_anchor_expressions(a, &AnchorExpressions { x: x.map(str::to_string), y: y.map(str::to_string) });
    })
}

#[test]
fn recipe_directives_are_read_beside_the_components() {
    let r = parse_auto_recipe("a + acutecomb # built by hand\n#! from the Latin set\n#?noanchors", "aacute").unwrap();
    assert_eq!(r, AutoRecipe { parts: vec!["a".into(), "acutecomb".into()], note: Some("from the Latin set".into()), no_anchors: true });
    let r = parse_auto_recipe("a+acutecomb=aacute", "aacute").unwrap();
    assert_eq!((r.parts.len(), r.note, r.no_anchors), (2, None, false));
    let refused = |text: &str| matches!(parse_auto_recipe(text, "aacute"), Err(ComponentsError::BadRecipe(_)));
    assert!(refused("a+acutecomb=eacute"), "the recipe builds the glyph it is stored on");
    assert!(refused("a\nacutecomb"), "one line of components");
    assert!(refused("# only a comment"), "a recipe names components");
    assert!(refused("a+acutecomb #?nomarks"), "an unknown keyword is refused");
    assert!(refused("aacute+acutecomb"), "a glyph is not its own component");
    let long = "a+".repeat(MAX_RECIPE_BYTES);
    assert!(matches!(parse_auto_recipe(&long, "aacute"), Err(ComponentsError::LimitExceeded { .. })));
}

fn accent_font() -> Font {
    font(vec![glyph("a", 500.0, &[("top", 250.0, 500.0), ("bottom", 250.0, 0.0)]), glyph("aacute", 600.0, &[])])
}

fn acutecomb() -> Glyph {
    glyph("acutecomb", 0.0, &[("_top", 100.0, 500.0), ("top", 100.0, 700.0)])
}

#[test]
fn an_automatic_layer_is_pending_until_its_missing_part_is_added() {
    let base = accent_font();
    let set = edit(&base, "aacute", |l| set_layer_recipe(l, Some("a+acutecomb")));
    let pending = follow_changes(&base, set).unwrap();
    assert!(components(&pending, "aacute").is_empty(), "acutecomb is missing: the layer waits, empty");

    let mut added = pending.clone();
    added.glyphs.push(acutecomb());
    let built = follow_changes(&pending, added).unwrap();
    assert_eq!(
        components(&built, "aacute"),
        [("a".to_string(), 0.0, 0.0), ("acutecomb".to_string(), 150.0, 0.0)],
        "the layer fills in, the mark aligned on the base's top anchor"
    );
    assert_eq!(built.glyph("aacute").unwrap().layers[0].width, 500.0, "alignment gives the width");

    let unchanged = follow_changes(&built, built.clone()).unwrap();
    assert!(Arc::ptr_eq(built.glyph("aacute").unwrap(), unchanged.glyph("aacute").unwrap()), "a built layer is left alone");

    let mut removed = built.clone();
    removed.glyphs.remove("acutecomb");
    let waiting = follow_changes(&built, removed).unwrap();
    assert!(components(&waiting, "aacute").is_empty(), "removing a part makes the layer pending again");
}

#[test]
fn no_anchors_stops_anchor_inheritance() {
    let mut start = accent_font();
    start.glyphs.push(acutecomb());
    let plain = follow_changes(&start, edit(&start, "aacute", |l| set_layer_recipe(l, Some("a+acutecomb")))).unwrap();
    let names = |f: &Font| effective_anchors(f, "aacute", M).unwrap().into_iter().map(|a| a.name).collect::<Vec<_>>();
    assert_eq!(names(&plain), ["top", "bottom"], "the built glyph inherits its components' anchors");
    let none = follow_changes(&plain, edit(&plain, "aacute", |l| set_layer_recipe(l, Some("a+acutecomb\n#?noanchors")))).unwrap();
    assert_eq!(components(&none, "aacute").len(), 2);
    assert!(names(&none).is_empty(), "#?noanchors keeps only the layer's own anchors");
}

#[test]
fn anchor_expressions_follow_width_and_metric_changes() {
    let base = font(vec![glyph("n", 600.0, &[("top", 0.0, 0.0)])]);
    let set = with_expressions(&base, "n", "top", Some("width/2"), Some("xHeight+20"));
    let f = follow_changes(&base, set).unwrap();
    assert_eq!(anchor(&f, "n", "top"), (300.0, 520.0));

    let wider = follow_changes(&f, edit(&f, "n", |l| l.width = 500.0)).unwrap();
    assert_eq!(anchor(&wider, "n", "top"), (250.0, 520.0), "width/2 follows the width");

    let mut taller = wider.clone();
    taller.set_metric_pos(M, metric::X_HEIGHT, 540.0).unwrap();
    let taller = follow_changes(&wider, taller).unwrap();
    assert_eq!(anchor(&taller, "n", "top"), (250.0, 560.0), "xHeight+20 follows the master's x-height");
}

#[test]
fn an_expression_that_cannot_be_worked_out_keeps_the_anchor_and_is_reported() {
    let base = font(vec![glyph("n", 600.0, &[("top", 10.0, 20.0)])]);
    let set = with_expressions(&base, "n", "top", Some("guide:missing + 1"), Some("1/0"));
    let (f, failures) = evaluate_glyphs(set, &["n".to_string()]);
    assert_eq!(anchor(&f, "n", "top"), (10.0, 20.0));
    assert_eq!(failures.len(), 2);
    assert!(failures[0].message.contains("guide:missing"), "{}", failures[0].message);
    assert_eq!((failures[1].axis, failures[1].anchor.as_str()), ("y", "top"));

    // A hostile value written into the file: a megabyte of text, and deep nesting.
    let hostile = edit(&f, "n", |l| {
        let mut d = Dict::new();
        d.insert("x", Value::String(Str::new("1+".repeat(1 << 19) + "1")));
        d.insert("y", Value::String(Str::new(format!("{}1{}", "(".repeat(10_000), ")".repeat(10_000)))));
        l.anchors[0].info.user_data.0.insert(ANCHOR_EXPRESSIONS, Value::Dict(d));
    });
    assert!(anchor_expressions(&hostile.glyph("n").unwrap().layers[0].anchors[0]).is_some());
    let (after, failures) = evaluate_glyphs(hostile, &["n".to_string()]);
    assert_eq!(anchor(&after, "n", "top"), (10.0, 20.0));
    assert_eq!(failures.len(), 2);
}
