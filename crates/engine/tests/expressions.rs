//! Anchor expressions and automatic layers (M13.16) through the commands and the engine's hook:
//! anchors that follow width and metric edits, failures reported as warnings, Remove Anchor
//! Expressions, and an automatic layer that fills in once its missing component is added.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::model::{Anchor, Font, Shape};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn refused(s: &mut Session, id: &str, p: Value, needle: &str) {
    let before = s.doc().unwrap().font.clone();
    let err = s.execute(id, &p).expect_err(id).to_string();
    assert!(err.contains(needle), "{id}: expected an error naming {needle:?}, got {err:?}");
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "{id}: a refused command changes nothing");
}

fn anchor(f: &Font, glyph: &str, name: &str) -> (f64, f64) {
    let a = f.glyph(glyph).unwrap().layers[0].anchors.iter().find(|a| a.name == name).unwrap();
    (a.x, a.y)
}

fn components(f: &Font, glyph: &str) -> Vec<String> {
    f.glyph(glyph).unwrap().layers[0]
        .shapes
        .iter()
        .filter_map(|s| match s {
            Shape::Component(c) => Some(c.glyph.clone()),
            _ => None,
        })
        .collect()
}

fn with_top_anchor() -> Session {
    let mut s = Session::with_document();
    run(&mut s, "anchor.add", json!({"glyph": "n", "name": "top", "x": 0, "y": 0}));
    s
}

#[test]
fn anchor_set_expressions_follow_width_and_metric_edits() {
    let mut s = with_top_anchor();
    let r = run(&mut s, "anchor.setExpressions", json!({"glyph": "n", "anchor": "top", "x": "width/2", "y": "xHeight+20"}));
    assert_eq!((r["x"].clone(), r["y"].clone()), (json!(300.0), json!(520.0)));
    assert_eq!(r["expressions"], json!({"x": "width/2", "y": "xHeight+20"}));
    assert_eq!(r["warnings"], json!([]));

    run(&mut s, "spacing.setWidth", json!({"glyphs": ["n"], "value": 500}));
    assert_eq!(anchor(s.font().unwrap(), "n", "top"), (250.0, 520.0), "width/2 follows a width change");

    run(&mut s, "font.master.set", json!({"masters": ["m01"], "metrics": {"x-height": {"position": 540}}}));
    assert_eq!(anchor(s.font().unwrap(), "n", "top"), (250.0, 560.0), "xHeight+20 follows a metric edit");

    run(&mut s, "edit.undo", json!({}));
    assert_eq!(anchor(s.font().unwrap(), "n", "top"), (250.0, 520.0), "the anchor moved in the edit's own undo step");

    let r = run(&mut s, "anchor.setExpressions", json!({"glyph": "n", "anchor": "top", "y": null}));
    assert_eq!(r["expressions"], json!({"x": "width/2"}), "null removes one coordinate's expression");
}

#[test]
fn anchor_set_expressions_warns_and_keeps_the_anchor_for_a_missing_name() {
    let mut s = Session::with_document();
    run(&mut s, "anchor.add", json!({"glyph": "n", "name": "top", "x": 100, "y": 700}));
    let r = run(&mut s, "anchor.setExpressions", json!({"glyph": "n", "anchor": "top", "x": "guide:axis", "y": "capHeight"}));
    assert_eq!((r["x"].clone(), r["y"].clone()), (json!(100.0), json!(700.0)), "x stays; y is the cap height, 700");
    let warnings = r["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["axis"], json!("x"));
    assert!(warnings[0]["message"].as_str().unwrap().contains("guide:axis"));

    refused(&mut s, "anchor.setExpressions", json!({"glyph": "n", "anchor": "top", "x": "width/"}), "`x`");
    refused(&mut s, "anchor.setExpressions", json!({"glyph": "n", "anchor": "top", "x": "1+".repeat(1 << 19)}), "bytes");
    refused(&mut s, "anchor.setExpressions", json!({"glyph": "n", "anchor": "top", "x": "(".repeat(100) + "1" + &")".repeat(100)}), "nested");
    refused(&mut s, "anchor.setExpressions", json!({"glyph": "n", "anchor": "nothing", "x": "1"}), "no anchor");
}

#[test]
fn anchor_remove_expressions_keeps_positions() {
    let mut s = with_top_anchor();
    run(&mut s, "anchor.setExpressions", json!({"glyph": "n", "anchor": "top", "x": "width/2"}));
    let r = run(&mut s, "anchor.removeExpressions", json!({"glyphs": ["n"]}));
    assert_eq!((r["glyphs"].clone(), r["removed"].clone()), (json!(["n"]), json!(1)));
    run(&mut s, "spacing.setWidth", json!({"glyphs": ["n"], "value": 400}));
    assert_eq!(anchor(s.font().unwrap(), "n", "top"), (300.0, 0.0), "without its expression the anchor stays");
    let r = run(&mut s, "anchor.removeExpressions", json!({}));
    assert_eq!(r["removed"], json!(0));
}

#[test]
fn glyph_set_auto_layer_recipe_fills_in_when_the_missing_part_is_added() {
    let mut s = Session::with_document();
    s.edit("fixture", |f| {
        let mut a = f.glyph_mut("a")?;
        a.layers[0].anchors.push(Anchor::new("top", 300.0, 500.0));
        Ok(())
    })
    .unwrap();
    run(&mut s, "glyph.newGlyph", json!({"name": "aacute"}));
    let r = run(&mut s, "glyph.setAutoLayerRecipe", json!({"glyph": "aacute", "recipe": "a+acutecomb #! accented a\n#?noanchors"}));
    assert_eq!(r["pending"], json!([{"glyph": "aacute", "missing": ["acutecomb"]}]));
    let f = s.font().unwrap();
    assert!(components(f, "aacute").is_empty(), "the layer waits for acutecomb");
    assert_eq!(f.glyph("aacute").unwrap().info.note, "accented a", "#! goes into the glyph note");

    run(&mut s, "glyph.newGlyph", json!({"name": "acutecomb", "width": 0}));
    assert_eq!(components(s.font().unwrap(), "aacute"), ["a", "acutecomb"], "adding the part fills the layer in");

    refused(&mut s, "glyph.setAutoLayerRecipe", json!({"glyph": "a", "recipe": "aacute"}), "contain itself");
    refused(&mut s, "glyph.setAutoLayerRecipe", json!({"glyph": "aacute", "recipe": "a+acutecomb #?nothing"}), "unknown directive");

    let r = run(&mut s, "glyph.setAutoLayerRecipe", json!({"glyph": "aacute", "auto": false}));
    assert_eq!(r["glyphs"], json!(["aacute"]));
    assert_eq!(components(s.font().unwrap(), "aacute"), ["a", "acutecomb"], "an ordinary layer keeps what it holds");
}
