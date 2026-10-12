//! Compare commands across open documents: compareFonts.diff, compareFonts.useLeft,
//! compareFonts.useRight and compareFamily.report, on fonts built in code.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::model::{Anchor, Component, Font, Master, Path, Shape};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn font_of(s: &Session, i: usize) -> Arc<Font> {
    s.documents()[i].font.clone()
}

/// Two documents: "Left" with a drawn A (a box, a `top` anchor) and the pair A–V −40; "Right"
/// with A wider and empty, an x-height of 520 and the pair T–o −60. The first one is active.
fn two_documents() -> Session {
    let mut left = Font::new("Left");
    {
        let mut a = left.glyph_mut("A").unwrap();
        let l = a.layer_mut("m01").unwrap();
        l.shapes.push(Shape::Path(Path::rect(50.0, 0.0, 550.0, 700.0)));
        l.anchors.push(Anchor::new("top", 300.0, 700.0));
    }
    Arc::make_mut(&mut left.kerning).ltr.set("m01", "A", "V", -40.0);
    let mut right = Font::new("Right");
    right.glyph_mut("A").unwrap().layer_mut("m01").unwrap().width = 640.0;
    right.set_metric_pos("m01", "x-height", 520.0).unwrap();
    Arc::make_mut(&mut right.kerning).ltr.set("m01", "T", "o", -60.0);
    let mut s = Session::new();
    s.add_document(left, None).unwrap();
    s.add_document(right, None).unwrap();
    run(&mut s, "document.activate", json!({"index": 0}));
    s
}

fn sides() -> Value {
    json!({"left": {"document": 0}, "right": {"document": 1}})
}

fn with(mut base: Value, key: &str, v: Value) -> Value {
    base[key] = v;
    base
}

fn row<'a>(rows: &'a [Value], category: &str, key: &str) -> Option<&'a Value> {
    rows.iter().find(|r| r["category"] == category && r["key"] == key)
}

#[test]
fn compare_fonts_diff_lists_only_differing_rows_of_two_documents() {
    let mut s = two_documents();
    let out = run(&mut s, "compareFonts.diff", sides());
    assert_eq!(out["left"]["title"], "Untitled");
    assert_eq!(out["right"]["document"], 1);
    let rows = out["rows"].as_array().unwrap();
    let family = row(rows, "fontInfo", "familyName").expect("family names differ");
    assert_eq!((family["left"].clone(), family["right"].clone()), (json!("Left"), json!("Right")));
    assert!(row(rows, "fontInfo", "unitsPerEm").is_none(), "equal values are not listed");
    let x = row(rows, "masterMetrics", "metric:x-height").expect("x-heights differ");
    assert_eq!(x["right"]["pos"], 520.0);
    let a = row(rows, "glyph", "A").expect("A differs");
    assert_eq!(a["differences"], json!(["width", "shapes", "anchors"]));
    assert_eq!(a["left"]["paths"], 1);
    assert!(row(rows, "glyph", "B").is_none(), "identical glyphs are not listed");
    assert_eq!(row(rows, "kerning", "A V").unwrap()["right"], Value::Null);
    assert_eq!(row(rows, "kerning", "T o").unwrap()["left"], Value::Null);
    // Categories narrow the rows and keep the window's order.
    let only = run(&mut s, "compareFonts.diff", with(sides(), "categories", json!(["kerning", "glyph"])));
    let cats: Vec<&str> = only["rows"].as_array().unwrap().iter().map(|r| r["category"].as_str().unwrap()).collect();
    assert!(cats.windows(2).all(|w| w[0] == w[1] || (w[0] == "glyph" && w[1] == "kerning")), "{cats:?}");
    // One master against itself: nothing differs.
    assert_eq!(run(&mut s, "compareFonts.diff", json!({}))["rows"], json!([]));
}

#[test]
fn compare_fonts_use_left_copies_a_glyph_into_the_other_document_as_one_undo_step() {
    let mut s = two_documents();
    let out = run(&mut s, "compareFonts.useLeft", with(sides(), "row", json!({"category": "glyph", "key": "A"})));
    assert_eq!(out["changed"], true);
    assert_eq!(out["document"], 1);
    let (l, r) = (font_of(&s, 0), font_of(&s, 1));
    let (la, ra) = (l.glyph("A").unwrap().layer("m01").unwrap(), r.glyph("A").unwrap().layer("m01").unwrap());
    assert_eq!((ra.width, &ra.shapes, &ra.anchors), (la.width, &la.shapes, &la.anchors));
    assert_eq!(s.documents()[1].history.undo.len(), 1, "one undo step in the receiving document");
    assert!(s.documents()[0].history.undo.is_empty(), "the giving document is untouched");
    assert_eq!(s.doc().unwrap().title, s.documents()[0].title, "the active document stays active");
    let after = run(&mut s, "compareFonts.diff", with(sides(), "categories", json!(["glyph"])));
    assert_eq!(after["rows"], json!([]));
    // Copying again changes nothing.
    let again = run(&mut s, "compareFonts.useLeft", with(sides(), "row", json!({"category": "glyph", "key": "A"})));
    assert_eq!(again["changed"], false);
}

#[test]
fn compare_fonts_use_right_writes_kerning_metrics_and_font_info_to_the_left() {
    let mut s = two_documents();
    for (category, key) in [("kerning", "T o"), ("kerning", "A V"), ("masterMetrics", "metric:x-height"), ("fontInfo", "familyName")] {
        run(&mut s, "compareFonts.useRight", with(sides(), "row", json!({"category": category, "key": key})));
    }
    let l = font_of(&s, 0);
    assert_eq!(l.kerning.ltr.get("m01", "T", "o"), Some(-60.0));
    assert_eq!(l.kerning.ltr.get("m01", "A", "V"), None, "a pair the right lacks is removed on the left");
    assert_eq!(l.metric_pos(l.master("m01").unwrap(), "x-height", 0.0), 520.0);
    assert_eq!(l.family_name, "Right");
    assert_eq!(s.documents()[0].history.undo.len(), 4);
    assert!(s.documents()[1].history.undo.is_empty());
    let rest = run(&mut s, "compareFonts.diff", with(sides(), "categories", json!(["fontInfo", "masterMetrics", "kerning"])));
    assert_eq!(rest["rows"], json!([]));
}

#[test]
fn kerning_rows_keep_sides_with_spaces_apart() {
    let mut s = two_documents();
    let mut left = font_of(&s, 0).as_ref().clone();
    Arc::make_mut(&mut left.kerning).ltr.set("m01", "x y", "z", -10.0);
    Arc::make_mut(&mut left.kerning).ltr.set("m01", "x", "y z", -20.0);
    s.add_document(left, None).unwrap();
    let pair = json!({"left": {"document": 2}, "right": {"document": 1}});
    let out = run(&mut s, "compareFonts.diff", with(pair.clone(), "categories", json!(["kerning"])));
    let rows = out["rows"].as_array().unwrap();
    let spaced: Vec<&Value> = rows.iter().filter(|r| r["label"] == "x y z").collect();
    assert_eq!(spaced.len(), 2, "two pairs, two rows: {rows:?}");
    for r in &spaced {
        run(&mut s, "compareFonts.useLeft", with(pair.clone(), "row", (*r).clone()));
    }
    let right = font_of(&s, 1);
    assert_eq!(right.kerning.ltr.get("m01", "x y", "z"), Some(-10.0));
    assert_eq!(right.kerning.ltr.get("m01", "x", "y z"), Some(-20.0));
    assert_eq!(right.kerning.ltr.get("m01", "x", "y"), None);
}

#[test]
fn compare_fonts_use_left_refuses_a_value_the_giving_side_lacks_and_changes_nothing() {
    let mut s = two_documents();
    let mut right = font_of(&s, 1).as_ref().clone();
    right.add_glyph("Aring", vec![], 600.0).unwrap();
    s.add_document(right, None).unwrap();
    let before: Vec<Arc<Font>> = s.documents().iter().map(|d| d.font.clone()).collect();
    let pair = json!({"left": {"document": 0}, "right": {"document": 2}});
    for row in [
        json!({"category": "glyph", "key": "Aring"}),
        json!({"category": "glyph", "key": ""}),
        json!({"category": "kerning", "key": "A"}),
        json!({"category": "fontInfo", "key": "nonsense"}),
    ] {
        assert!(s.execute("compareFonts.useLeft", &with(pair.clone(), "row", row.clone())).is_err(), "{row}");
    }
    assert!(s.execute("compareFonts.useLeft", &json!({"left": {"document": 9}, "row": {"category": "glyph", "key": "A"}})).is_err());
    for (d, f) in s.documents().iter().zip(&before) {
        assert!(Arc::ptr_eq(&d.font, f), "a refused command changes nothing");
    }
}

#[test]
fn compare_fonts_compares_two_masters_of_one_document() {
    let mut f = Font::new("Two");
    Arc::make_mut(&mut f.masters).push(Master { id: "m02".into(), name: "Bold".into(), ..Default::default() });
    f.glyph_mut("A").unwrap().layers.push(fontcraft_engine::model::Layer::new("m02", 700.0));
    let mut s = Session::new();
    s.add_document(f, None).unwrap();
    let pair = json!({"left": {"master": "Regular"}, "right": {"master": "Bold"}});
    let out = run(&mut s, "compareFonts.diff", with(pair.clone(), "categories", json!(["glyph"])));
    let a = row(out["rows"].as_array().unwrap(), "glyph", "A").unwrap();
    assert_eq!(a["differences"], json!(["width"]));
    assert_eq!(row(out["rows"].as_array().unwrap(), "glyph", "B").unwrap()["differences"], json!(["missing"]));
    run(&mut s, "compareFonts.useLeft", with(pair, "row", json!({"category": "glyph", "key": "B"})));
    let f = font_of(&s, 0);
    assert_eq!(f.glyph("B").unwrap().layer("m02").unwrap().width, 600.0, "a missing master layer is added");
}

#[test]
fn compare_family_report_shows_one_attribute_per_document_with_filters() {
    let mut s = two_documents();
    {
        // Left gets Aacute (A plus an anchor) and a left kerning group on A; Right has neither.
        let mut left = font_of(&s, 0).as_ref().clone();
        left.add_glyph("Aacute", vec![0xC1], 600.0).unwrap();
        let mut g = left.glyph_mut("Aacute").unwrap();
        g.layer_mut("m01").unwrap().shapes.push(Shape::Component(Component::new("A", 0.0, 0.0)));
        drop(g);
        left.glyph_mut("A").unwrap().kern_groups.left = Some("A".into());
        s.edit("setup", |f| {
            *f = left;
            Ok(())
        })
        .unwrap();
    }
    let report = |s: &mut Session, attribute: &str, filter: &str| run(s, "compareFamily.report", json!({"attribute": attribute, "filter": filter}));

    let out = report(&mut s, "componentNames", "all");
    assert_eq!(out["columns"].as_array().unwrap().len(), 2);
    assert_eq!(out["columns"][0]["master"], "m01");
    assert_eq!(out["rows"], json!([{"glyph": "Aacute", "cells": [["A"], {"missing": true}], "same": false}]));

    let out = report(&mut s, "leftKerningGroup", "all");
    assert_eq!(out["rows"], json!([{"glyph": "A", "cells": ["A", null], "same": false}]));

    let out = report(&mut s, "anchors", "ignoreIfMissingInOne");
    assert_eq!(out["rows"], json!([{"glyph": "A", "cells": [["top"], null], "same": false}]));

    let missing = report(&mut s, "glyphs", "missingInOne");
    let names: Vec<&str> = missing["rows"].as_array().unwrap().iter().map(|r| r["glyph"].as_str().unwrap()).collect();
    assert_eq!(names, ["Aacute"]);

    let all = report(&mut s, "glyphs", "all");
    assert_eq!(all["total"], font_of(&s, 0).glyphs.len());
    let page = run(&mut s, "compareFamily.report", json!({"attribute": "glyphs", "offset": 2, "limit": 3, "documents": [1, "Untitled"]}));
    assert_eq!(page["rows"].as_array().unwrap().len(), 3);
    assert_eq!(page["columns"][0]["document"], 1, "columns follow the order given");
    assert!(s.execute("compareFamily.report", &json!({"attribute": "colour"})).is_err());
}
