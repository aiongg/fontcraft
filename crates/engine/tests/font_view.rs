//! Font View's filtering commands: fontView.queryGlyphs, the filter commands, the Languages list
//! and missing glyphs.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::settings::Reserved;
use fontcraft_engine::{EngineError, Session};
use fontcraft_model::{Path, Shape};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn refused(s: &mut Session, id: &str, p: Value, needle: &str) {
    match s.execute(id, &p) {
        Err(EngineError::BadParams { message, .. }) => assert!(message.contains(needle), "{id} {p}: {message}"),
        other => panic!("{id} {p}: expected a refusal naming `{needle}`, got {other:?}"),
    }
}

fn names(v: &Value) -> Vec<String> {
    v["glyphs"].as_array().unwrap().iter().map(|n| n.as_str().unwrap().to_string()).collect()
}

/// A second master `m02`; A has a path on m01 only, B on m02 only.
fn two_masters() -> Session {
    let mut s = Session::with_document();
    s.edit("setup", |font| {
        let mut m = font.default_master()?.clone();
        m.id = "m02".into();
        m.name = "Bold".into();
        Arc::make_mut(&mut font.masters).push(m);
        for i in 0..font.glyphs.len() {
            if let Some(mut g) = font.glyphs.get_mut_at(i) {
                g.layers.push(fontcraft_model::Layer::new("m02", 600.0));
            }
        }
        font.glyph_mut("A")?.layers[0].shapes.push(Shape::Path(Path::rect(0.0, 0.0, 10.0, 10.0)));
        font.glyph_mut("B")?.layers[1].shapes.push(Shape::Path(Path::rect(0.0, 0.0, 10.0, 10.0)));
        Ok(())
    })
    .unwrap();
    s
}

#[test]
fn query_glyphs_combines_entries_rules_and_search() {
    let mut s = two_masters();
    let q = |s: &mut Session, p: Value| names(&run(s, "fontView.queryGlyphs", p));
    assert_eq!(q(&mut s, json!({"entries": ["categories/Separator", "categories/Letter/Uppercase"], "search": "A"})), vec!["space", "A"]);
    let lower =
        q(&mut s, json!({"entries": [["categories", "Letter", "Lowercase"]], "search": {"query": "[a-c]", "regex": true, "matchCase": true}}));
    assert_eq!(lower, vec!["a", "b", "c"]);
    // The count rule reads the first master; the layer rule reads the master given.
    let count_rule = json!({"all": [{"property": "countOfPaths", "op": ">", "value": 0}]});
    assert_eq!(q(&mut s, json!({"rules": count_rule, "master": "m02"})), vec![".notdef", "A"]);
    let layer_rule = json!({"all": [{"property": "custom", "value": "layer.countOfPaths > 0"}]});
    assert_eq!(q(&mut s, json!({"rules": layer_rule, "master": "m02"})), vec!["B"]);
    let r = run(&mut s, "fontView.queryGlyphs", json!({"sidebar": true}));
    assert_eq!(r["count"], r["total"]);
    let sections: Vec<&str> = r["sidebar"].as_array().unwrap().iter().map(|e| e["kind"].as_str().unwrap()).collect();
    assert_eq!(sections, vec!["all", "section", "section", "section"]);

    let deep = format!("{}name == 'A'{}", "(".repeat(100), ")".repeat(100));
    refused(&mut s, "fontView.queryGlyphs", json!({"rules": {"all": [{"property": "custom", "value": deep}]}}), "nested");
    refused(&mut s, "fontView.queryGlyphs", json!({"search": {"query": "(?:a{1000}){1000}", "regex": true}}), "regular expression");
    refused(&mut s, "fontView.queryGlyphs", json!({"master": "m09"}), "m09");
    refused(&mut s, "fontView.queryGlyphs", json!({"entries": ["shapes/A"]}), "sidebar path");
    refused(&mut s, "fontView.queryGlyphs", json!({"search": {"query": "A", "fuzzy": true}}), "fuzzy");
}

#[test]
fn filters_live_in_the_settings_and_change_no_document() {
    let mut s = Session::with_document();
    let font_before = s.doc().unwrap().font.clone();
    run(&mut s, "fontView.addFilterFolder", json!({"name": "Work"}));
    let smart = json!({"any": [{"property": "glyphName", "op": "is", "value": "B"}, {"property": "glyphName", "op": "is", "value": "C"}]});
    assert_eq!(
        run(&mut s, "fontView.addSmartFilter", json!({"name": "B or C", "rules": smart, "folder": "Work"}))["filter"],
        json!(["Work", "B or C"])
    );
    run(&mut s, "fontView.addListFilter", json!({"name": "Order", "glyphs": ["C a", "B\nzz"]}));
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &font_before), "filters change no document");
    let stored: Value = s.settings().reserved(Reserved::Filters).unwrap().unwrap();
    assert_eq!(stored[1]["glyphs"], json!(["C", "a", "B", "zz"]));

    let q = |s: &mut Session, entries: Value| names(&run(s, "fontView.queryGlyphs", json!({"entries": entries})));
    assert_eq!(q(&mut s, json!(["filters/Order"])), vec!["C", "a", "B"]);
    assert_eq!(q(&mut s, json!(["filters/Order", "filters/Work"])), vec!["C", "B"]);
    assert_eq!(q(&mut s, json!(["filters/Work"])), vec!["B", "C"]);

    run(&mut s, "fontView.editFilter", json!({"filter": "Work/B or C", "rules": {"all": [{"property": "glyphName", "op": "is", "value": "D"}]}}));
    assert_eq!(q(&mut s, json!(["filters/Work"])), vec!["D"]);
    assert_eq!(run(&mut s, "fontView.renameFilter", json!({"filter": "Work/B or C", "name": "Just D"}))["filter"], json!(["Work", "Just D"]));
    assert_eq!(run(&mut s, "fontView.moveFilter", json!({"filter": "Order", "parent": "Work", "index": 0}))["filter"], json!(["Work", "Order"]));
    assert_eq!(q(&mut s, json!(["filters/Work"])), vec!["B", "C", "D", "a"], "a folder shows the union of its filters in glyph order");

    // A refused command leaves the stored filters as they were.
    let before: Value = s.settings().reserved(Reserved::Filters).unwrap().unwrap();
    refused(&mut s, "fontView.addSmartFilter", json!({"name": "Order", "folder": "Work"}), "already");
    refused(&mut s, "fontView.editFilter", json!({"filter": "Work/Order", "rules": {"all": []}}), "list filter");
    refused(&mut s, "fontView.moveFilter", json!({"filter": "Work", "parent": "Work/Order", "index": 0}), "into itself");
    refused(
        &mut s,
        "fontView.addSmartFilter",
        json!({"name": "x", "rules": {"all": [{"property": "glyphName", "op": "matches", "value": "(?:a{1000}){1000}"}]}}),
        "regular expression",
    );
    let after: Value = s.settings().reserved(Reserved::Filters).unwrap().unwrap();
    assert_eq!(before, after);

    let r = run(&mut s, "fontView.removeFilter", json!({"filter": "Work"}));
    assert_eq!(r["kind"], json!("folder"));
    let stored: Value = s.settings().reserved(Reserved::Filters).unwrap().unwrap();
    assert_eq!(stored, json!([]), "a folder takes its contents with it");
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &font_before));
}

#[test]
fn set_visible_scripts_chooses_the_languages_list() {
    let mut s = Session::with_document();
    assert_eq!(
        run(&mut s, "fontView.setVisibleScripts", json!({"show": ["arabic"], "hide": ["greek"]}))["scripts"],
        json!(["latin", "cyrillic", "arabic"])
    );
    assert_eq!(run(&mut s, "fontView.setVisibleScripts", json!({"hide": ["latin"]}))["scripts"], json!(["cyrillic", "arabic"]));
    let r = run(&mut s, "fontView.queryGlyphs", json!({"sidebar": true}));
    let languages: Vec<&str> = r["sidebar"][2]["children"].as_array().unwrap().iter().map(|e| e["label"].as_str().unwrap()).collect();
    assert_eq!(languages, vec!["cyrillic", "arabic", "latin"], "a script the font uses is listed anyway");
    refused(&mut s, "fontView.setVisibleScripts", json!({"show": ["Latin!"]}), "script id");
    refused(&mut s, "fontView.setVisibleScripts", json!({}), "show");
}

#[test]
fn missing_glyphs_of_a_list_filter_are_listed_and_generated() {
    let mut s = Session::with_document();
    run(&mut s, "fontView.addListFilter", json!({"name": "Want", "glyphs": "A B zz A-cy"}));
    let r = run(&mut s, "fontView.listMissingGlyphs", json!({"entry": "filters/Want"}));
    assert_eq!(r["missing"], json!(["zz", "A-cy"]));
    assert_eq!((r["present"].clone(), r["expected"].clone()), (json!(2), json!(4)));

    let before = s.doc().unwrap().font.clone();
    refused(&mut s, "fontView.generateMissingGlyphs", json!({"entry": "filters/Want", "glyphs": ["A"]}), "not missing");
    refused(&mut s, "fontView.listMissingGlyphs", json!({"entry": "all"}), "full glyph set");
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before));

    assert_eq!(run(&mut s, "fontView.generateMissingGlyphs", json!({"entry": "filters/Want", "glyphs": ["A-cy"]}))["added"], json!(["A-cy"]));
    assert_eq!(s.font().unwrap().glyph("A-cy").unwrap().unicodes, vec![0x410]);
    assert_eq!(run(&mut s, "fontView.generateMissingGlyphs", json!({"entry": "filters/Want"}))["added"], json!(["zz"]));
    assert_eq!(run(&mut s, "fontView.listMissingGlyphs", json!({"entry": "filters/Want"}))["missing"], json!([]));
    run(&mut s, "edit.undo", json!({}));
    assert!(s.font().unwrap().glyph("zz").is_none(), "one undo step");
    assert!(s.font().unwrap().glyph("A-cy").is_some());
}
