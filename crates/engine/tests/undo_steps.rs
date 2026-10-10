//! Every Font View and glyph-management command that changes the font makes exactly one undo
//! step: one `edit.undo` gives back the very font object it started from.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_testkit::rect;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

/// A starting font with a drawn `A`, a tagged and annotated `b`, an overridden category on `c`, a
/// production-style name and a list filter that names a glyph the font lacks.
fn fixture() -> Session {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"familyName": "Undo Sans"}));
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(50.0, 0.0, 300.0, 600.0)}));
    run(&mut s, "glyph.addTag", json!({"glyphs": ["b"], "tag": "round"}));
    run(&mut s, "glyph.setNote", json!({"glyphs": ["b"], "note": "check"}));
    run(&mut s, "glyph.setCategory", json!({"glyphs": ["c"], "category": "Symbol"}));
    run(&mut s, "fontView.addListFilter", json!({"name": "Undo extras", "glyphs": ["a", "zz"]}));
    s
}

#[test]
fn every_glyph_management_edit_is_one_undo_step() {
    let copied = {
        let mut s = fixture();
        let copy = run(&mut s, "edit.copy", json!({"glyphs": ["A"]}));
        copy["json"].as_str().unwrap_or_else(|| panic!("edit.copy gave no json: {copy}")).to_string()
    };
    let cases: Vec<(&str, Value)> = vec![
        ("glyph.addGlyphs", json!({"text": "a:c uni00E4 f_f_l"})),
        ("glyph.newGlyph", json!({"name": "A.alt"})),
        ("glyph.duplicateGlyph", json!({"glyphs": ["A", "b"]})),
        ("glyph.removeGlyph", json!({"glyphs": ["b", "c"]})),
        ("glyph.updateGlyphInfo", json!({"glyphs": ["c"]})),
        ("glyphInfo.addToFont", json!({"names": ["adieresis", "eth"]})),
        ("fontView.generateMissingGlyphs", json!({"entry": "filters/Undo extras"})),
        ("edit.paste", json!({"json": copied})),
        ("edit.pasteSpecial", json!({"json": copied, "mode": "selected", "targets": ["B"]})),
        ("glyph.rename", json!({"glyph": "A", "newName": "A.ss01"})),
        ("edit.findAndReplace", json!({"glyphs": ["a", "b"], "find": "", "replace": ".sc"})),
        ("glyph.setUnicodes", json!({"glyphs": ["A"], "unicodes": ["0391"]})),
        ("glyph.setProductionName", json!({"glyphs": ["A"], "productionName": "uni0041"})),
        ("glyph.setScript", json!({"glyphs": ["A", "B"], "script": "cyrillic"})),
        ("glyph.setCategory", json!({"glyphs": ["A", "B"], "category": "Symbol", "subCategory": "Other"})),
        ("glyph.setCase", json!({"glyphs": ["A", "B"], "case": "smallCaps"})),
        ("glyph.setDirection", json!({"glyphs": ["A", "B"], "direction": "RTL"})),
        ("glyph.setExport", json!({"glyphs": ["A", "B"], "export": false})),
        ("glyph.setColorLabel", json!({"glyphs": ["A", "B"], "color": "red"})),
        ("glyph.setLayerColorLabel", json!({"glyphs": ["A", "B"], "color": 3})),
        ("glyph.setTags", json!({"glyphs": ["A", "B"], "tags": ["x", "y"]})),
        ("glyph.addTag", json!({"glyphs": ["A", "B"], "tags": ["x", "y"]})),
        ("glyph.removeTag", json!({"glyphs": ["b"], "tag": "round"})),
        ("glyph.setSortName", json!({"glyphs": ["z"], "sortName": "0"})),
        ("glyph.setNote", json!({"glyphs": ["A", "B"], "note": "two glyphs"})),
    ];
    for (id, params) in cases {
        let mut s = fixture();
        let before = s.doc().unwrap().font.clone();
        let steps = s.doc().unwrap().history.undo.len();
        run(&mut s, id, params.clone());
        assert!(!Arc::ptr_eq(&s.doc().unwrap().font, &before), "{id} {params} changed nothing");
        assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1, "{id} {params}");
        run(&mut s, "edit.undo", json!({}));
        assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "{id} {params}: one undo did not give back the font");
    }
}
