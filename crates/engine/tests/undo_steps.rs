//! Every Font View, glyph-management, spacing and kerning command that changes the font makes
//! exactly one undo step: one `edit.undo` gives back the very font object it started from. The
//! refusals and no-op runs of the spacing and kerning commands leave the very same font.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::model::plist::Value as PValue;
use fontcraft_engine::model::{Font, Param};
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

/// Setup commands, then the command whose undo step is checked, with its parameters.
type Staged = (Vec<(&'static str, Value)>, &'static str, Value);

/// A font for the spacing and kerning commands: `T`, `Tcaron`, `o`, `oacute`, `n`, `A` and `V`
/// drawn on master `m01`, `oacute` keyed to `=o` and out of sync, groups `T` and `o` on both sides, the pairs
/// `@T @o -50` and `A V -30`, and a second master `m02` whose metrics follow the first.
fn spacing_fixture() -> Session {
    let mut s = Session::with_document();
    run(&mut s, "glyph.addGlyphs", json!({"text": "Tcaron oacute"}));
    for (glyph, x) in [("T", 20.0), ("Tcaron", 20.0), ("o", 40.0), ("oacute", 40.0), ("n", 50.0), ("A", 10.0), ("V", 10.0)] {
        run(&mut s, "path.addPath", json!({"glyph": glyph, "nodes": rect(x, 0.0, 400.0, 500.0)}));
    }
    run(&mut s, "spacing.setWidth", json!({"glyphs": ["T", "Tcaron", "o", "oacute", "n", "A", "V"], "value": 480}));
    run(&mut s, "spacing.setLeftSidebearing", json!({"glyphs": ["oacute"], "value": "=o"}));
    // `oacute` out of sync with its key.
    run(&mut s, "spacing.setLeftSidebearing", json!({"glyphs": ["o"], "value": 45}));
    for id in ["kerning.setLeftGroup", "kerning.setRightGroup"] {
        run(&mut s, id, json!({"glyphs": ["T", "Tcaron"], "group": "T"}));
        run(&mut s, id, json!({"glyphs": ["o", "oacute"], "group": "o"}));
    }
    run(&mut s, "kerning.setPair", json!({"left": "@T", "right": "@o", "value": -50}));
    run(&mut s, "kerning.setPair", json!({"left": "A", "right": "V", "value": -30}));
    s.edit("fixture", |f: &mut Font| {
        let mut m = f.masters[0].clone();
        m.id = "m02".into();
        m.name = "Linked".into();
        m.params.0.push(Param::new("Link Metrics With First Master", PValue::Bool(true)));
        Arc::make_mut(&mut f.masters).push(m);
        Ok(())
    })
    .unwrap();
    s
}

#[test]
fn every_spacing_and_kerning_edit_is_one_undo_step() {
    let imported = {
        let mut s = spacing_fixture();
        run(&mut s, "kerning.setPair", json!({"left": "A", "right": "V", "value": -80}));
        run(&mut s, "spacing.setWidth", json!({"glyphs": ["n"], "value": 520}));
        run(&mut s, "file.exportMetrics", json!({"format": "metrics"}))["text"].as_str().unwrap().to_string()
    };
    let g = |names: &[&str]| json!(names);
    let cases: Vec<(&str, Value)> = vec![
        ("spacing.setLeftSidebearing", json!({"glyphs": g(&["n"]), "value": 60})),
        ("spacing.setLeftSidebearing", json!({"glyphs": g(&["n"]), "value": "=o"})),
        ("spacing.setRightSidebearing", json!({"glyphs": g(&["n", "o"]), "value": 70})),
        ("spacing.setWidth", json!({"glyphs": g(&["A"]), "value": 600})),
        ("spacing.setTopSidebearing", json!({"glyphs": g(&["n"]), "value": 120})),
        ("spacing.setBottomSidebearing", json!({"glyphs": g(&["n"]), "value": 30})),
        ("spacing.increaseLeftSidebearing", json!({"glyphs": g(&["n"]), "amount": 5})),
        ("spacing.increaseLeftSidebearingBy10", json!({"glyphs": g(&["n"])})),
        ("spacing.decreaseLeftSidebearing", json!({"glyphs": g(&["n"])})),
        ("spacing.decreaseLeftSidebearingBy10", json!({"glyphs": g(&["n"])})),
        ("spacing.increaseRightSidebearing", json!({"glyphs": g(&["n"])})),
        ("spacing.increaseRightSidebearingBy10", json!({"glyphs": g(&["n"])})),
        ("spacing.decreaseRightSidebearing", json!({"glyphs": g(&["n"])})),
        ("spacing.decreaseRightSidebearingBy10", json!({"glyphs": g(&["n"])})),
        ("spacing.shiftOutlineLeft", json!({"glyphs": g(&["n"])})),
        ("spacing.shiftOutlineLeftBy10", json!({"glyphs": g(&["n"])})),
        ("spacing.shiftOutlineRight", json!({"glyphs": g(&["n"])})),
        ("spacing.shiftOutlineRightBy10", json!({"glyphs": g(&["n"])})),
        ("spacing.updateMetricsKey", json!({"glyph": "oacute", "field": "left"})),
        ("glyph.updateMetrics", json!({"glyphs": g(&["oacute"])})),
        ("glyph.updateMetricsForAllMasters", json!({"glyphs": g(&["oacute"])})),
        ("glyph.transformMetrics", json!({"glyph": "A", "width": 500})),
        ("kerning.setPair", json!({"left": "T", "right": "o", "value": -20})),
        ("kerning.removePair", json!({"left": "A", "right": "V"})),
        ("kerning.increaseLeft", json!({"left": "T", "right": "o"})),
        ("kerning.increaseLeftBy10", json!({"left": "T", "right": "o"})),
        ("kerning.decreaseLeft", json!({"left": "T", "right": "o"})),
        ("kerning.decreaseLeftBy10", json!({"left": "T", "right": "o"})),
        ("kerning.increaseRight", json!({"left": "T", "right": "o"})),
        ("kerning.increaseRightBy10", json!({"left": "T", "right": "o"})),
        ("kerning.decreaseRight", json!({"left": "T", "right": "o"})),
        ("kerning.decreaseRightBy10", json!({"left": "T", "right": "o"})),
        ("kerning.setLeftGroup", json!({"glyphs": g(&["n"]), "group": "n"})),
        ("kerning.setRightGroup", json!({"glyphs": g(&["n"]), "group": "n"})),
        ("kerning.setTopGroup", json!({"glyphs": g(&["n"]), "group": "top.n"})),
        ("kerning.setBottomGroup", json!({"glyphs": g(&["n"]), "group": "bottom.n"})),
        ("kerning.addException", json!({"left": "Tcaron", "right": "oacute", "side": "right"})),
        ("kerning.renameGroup", json!({"side": "left", "from": "T", "to": "Tee", "scope": "font"})),
        ("kerning.pastePairs", json!({"pairs": [{"left": "n", "right": "o", "value": -12}]})),
        ("kerning.pastePairs", json!({"pairs": [{"left": "A", "right": "V", "value": -12}], "onConflict": "overwrite"})),
        ("kerning.deletePairs", json!({"pairs": [{"left": "A", "right": "V"}]})),
        ("file.importMetrics", json!({"text": imported, "importMetrics": true})),
    ];
    for (id, params) in cases {
        let mut s = spacing_fixture();
        let before = s.doc().unwrap().font.clone();
        let steps = s.doc().unwrap().history.undo.len();
        run(&mut s, id, params.clone());
        assert!(!Arc::ptr_eq(&s.doc().unwrap().font, &before), "{id} {params} changed nothing");
        assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1, "{id} {params}");
        run(&mut s, "edit.undo", json!({}));
        assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "{id} {params}: one undo did not give back the font");
    }

    // An exception, then its removal; a removed glyph's pairs, then Clean Up; an exception
    // that equals its group pair, then Compress.
    let staged: Vec<Staged> = vec![
        (
            vec![("kerning.addException", json!({"left": "Tcaron", "right": "oacute", "side": "right"}))],
            "kerning.removeException",
            json!({"left": "Tcaron", "right": "oacute", "side": "right"}),
        ),
        (vec![("glyph.removeGlyph", json!({"glyphs": ["V"]}))], "kerning.cleanUp", json!({})),
        (vec![("kerning.setPair", json!({"left": "Tcaron", "right": "@o", "value": -50}))], "kerning.compress", json!({})),
    ];
    for (setup, id, params) in staged {
        let mut s = spacing_fixture();
        for (setup_id, setup_params) in setup {
            run(&mut s, setup_id, setup_params);
        }
        let before = s.doc().unwrap().font.clone();
        let steps = s.doc().unwrap().history.undo.len();
        run(&mut s, id, params.clone());
        assert!(!Arc::ptr_eq(&s.doc().unwrap().font, &before), "{id} {params} changed nothing");
        assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1, "{id} {params}");
        run(&mut s, "edit.undo", json!({}));
        assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "{id} {params}: one undo did not give back the font");
    }

    // Refusals and runs with nothing to do leave the very same font and make no undo step.
    let unchanged: Vec<(&str, Value)> = vec![
        ("spacing.setLeftSidebearing", json!({"glyphs": g(&["n"]), "value": "=o+"})),
        ("spacing.setWidth", json!({"glyphs": g(&["n"]), "value": "=nope"})),
        ("spacing.setWidth", json!({"glyphs": g(&["n"]), "layer": "m02", "value": 10})),
        ("spacing.increaseLeftSidebearing", json!({"glyphs": g(&["n"]), "layer": "m02"})),
        ("kerning.setPair", json!({"left": "A", "right": "V", "value": -5, "master": "m02"})),
        ("kerning.setLeftGroup", json!({"glyphs": g(&["n"]), "group": "a b"})),
        ("kerning.renameGroup", json!({"side": "left", "from": "T", "to": "a b", "scope": "font"})),
        ("kerning.pastePairs", json!({"pairs": [{"left": "A", "right": "V", "value": -12}], "onConflict": "ask"})),
        ("kerning.compress", json!({})),
        ("kerning.cleanUp", json!({})),
        ("glyph.transformMetrics", json!({"glyph": "nope", "width": 500})),
    ];
    for (id, params) in unchanged {
        let mut s = spacing_fixture();
        let before = s.doc().unwrap().font.clone();
        let steps = s.doc().unwrap().history.undo.len();
        let _ = s.execute(id, &params);
        assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "{id} {params} changed the font");
        assert_eq!(s.doc().unwrap().history.undo.len(), steps, "{id} {params} made an undo step");
    }
}
