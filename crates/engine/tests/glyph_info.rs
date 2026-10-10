//! Glyph property commands: renaming with references, Find and Replace, Update Glyph Info, the
//! property setters, Copy Glyph Names and the glyph-name and glyph-data queries.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::compile::{Options, compile_ttf};
use fontcraft_engine::model::plist::{Array, Str, Value as Plist};
use fontcraft_engine::model::{ColorLabel, Component, GLYPH_ORDER_PARAM, Shape};
use fontcraft_testkit::{check_session, rect};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// Runs a command that must fail, checks that the font is the same object as before, and
/// returns the message.
fn refused(s: &mut Session, id: &str, p: Value) -> String {
    let before = s.doc().unwrap().font.clone();
    let e = s.execute(id, &p).expect_err(id).to_string();
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "{id} {p}: a refused command changed the font");
    e
}

fn names(s: &Session) -> Vec<String> {
    s.font().unwrap().glyphs.iter().map(|g| g.name.clone()).collect()
}

fn info(s: &mut Session, glyph: &str) -> Value {
    run(s, "glyph.getInfo", json!({"glyphs": [glyph]}))["glyphs"][0].clone()
}

/// A font where `part` is drawn, used as a component and as the left metrics key of `B`, kerned
/// on both sides against `V`, a member of a UFO group, and named in the glyphOrder parameter.
fn built(part: &str) -> Session {
    let mut s = Session::with_document();
    run(&mut s, "glyph.newGlyph", json!({"name": part}));
    run(&mut s, "path.addPath", json!({"glyph": part, "nodes": rect(50.0, 0.0, 300.0, 200.0)}));
    s.edit("references", |f| {
        {
            let mut b = f.glyph_mut("B")?;
            b.layers[0].shapes.push(Shape::Component(Component::new(part, 100.0, 0.0)));
            b.info.metrics_keys.left = Some(format!("=={part}+10"));
        }
        let k = Arc::make_mut(&mut f.kerning);
        k.ltr.set("m01", part, "V", -40.0);
        k.ltr.set("m01", "V", part, -30.0);
        Arc::make_mut(&mut f.groups).push(fontcraft_engine::model::Group { name: "parts".into(), members: vec![part.into(), "B".into()] });
        let mut order: Vec<String> = f.glyphs.iter().map(|g| g.name.clone()).filter(|n| n != part).collect();
        order.insert(2, part.to_string());
        let items = order.iter().map(|n| Plist::String(Str::new(n.as_str()))).collect();
        Arc::make_mut(&mut f.info).params.set(GLYPH_ORDER_PARAM, Plist::Array(Array { items, ..Default::default() }));
        Ok(())
    })
    .unwrap();
    s
}

#[test]
fn rename_follows_references_like_a_font_built_under_the_new_name() {
    let mut renamed = built("part");
    let r = run(&mut renamed, "glyph.rename", json!({"glyph": "part", "newName": "piece"}));
    assert_eq!(r["glyph"], "piece");
    assert_eq!(r["stillUsed"], json!([]));
    assert_eq!(r["updated"].as_array().unwrap().len(), 6, "{}", r["updated"]);
    let expected = built("piece");
    assert_eq!(*renamed.font().unwrap(), *expected.font().unwrap(), "the model equals the font built with the new name");

    let a = compile_ttf(renamed.font().unwrap(), &Options::default()).unwrap();
    let b = compile_ttf(expected.font().unwrap(), &Options::default()).unwrap();
    assert!(a.glyph_order.iter().any(|n| n == "piece"));
    assert_eq!(a.bytes, b.bytes, "the compiled fonts are byte-identical");

    // One undo step brings every reference back.
    run(&mut renamed, "edit.undo", json!({}));
    assert_eq!(*renamed.font().unwrap(), *built("part").font().unwrap());
}

#[test]
fn rename_converts_names_follows_the_data_and_refuses_clashes() {
    let mut s = Session::with_document();
    let r = run(&mut s, "glyph.rename", json!({"glyph": "A", "newName": "plus"}));
    assert_eq!(r["unicodes"], json!(["002B"]), "a new base name takes the data's code point");
    let r = run(&mut s, "glyph.rename", json!({"glyph": "plus", "newName": "plus.sc"}));
    assert_eq!(r["unicodes"], json!(["002B"]), "a suffix change keeps the code points");
    let r = run(&mut s, "glyph.rename", json!({"glyph": "Z", "newName": "zzzz"}));
    assert_eq!(r["unicodes"], json!(["005A"]), "a name the data does not know keeps them");
    let r = run(&mut s, "glyph.rename", json!({"glyph": "Y", "newName": "uni0416"}));
    assert_eq!((&r["glyph"], &r["unicodes"]), (&json!("Zhe-cy"), &json!(["0416"])), "a production name becomes the nice name");
    let r = run(&mut s, "glyph.rename", json!({"glyph": "W", "newName": "uni0416", "updateInfo": false}));
    assert_eq!((&r["glyph"], &r["unicodes"]), (&json!("uni0416"), &json!(["0057"])), "updateInfo off renames as given");

    s.edit("custom naming", |f| {
        Arc::make_mut(&mut f.info).settings.disables_nice_names = Some(true);
        Ok(())
    })
    .unwrap();
    let r = run(&mut s, "glyph.rename", json!({"glyph": "X", "newName": "uni0417"}));
    assert_eq!((&r["glyph"], &r["unicodes"]), (&json!("uni0417"), &json!(["0417"])), "custom naming keeps the typed name");

    assert!(refused(&mut s, "glyph.rename", json!({"glyph": "B", "newName": "C"})).contains("already a glyph `C`"));
    assert!(refused(&mut s, "glyph.rename", json!({"glyph": "B", "newName": "a b"})).contains("illegal character"));
    assert!(refused(&mut s, "glyph.rename", json!({"glyph": "nope", "newName": "x"})).contains("no glyph"));
    check_session(&s).unwrap();
}

#[test]
fn rename_refuses_when_a_locked_glyph_uses_the_glyph() {
    let mut s = built("part");
    run(&mut s, "glyph.toggleLocked", json!({"glyphs": ["B"], "locked": true}));
    assert!(refused(&mut s, "glyph.rename", json!({"glyph": "part", "newName": "piece"})).contains("locked"));
    // A locked glyph itself may be renamed.
    run(&mut s, "glyph.toggleLocked", json!({"glyphs": ["C"], "locked": true}));
    run(&mut s, "glyph.rename", json!({"glyph": "C", "newName": "C.alt"}));
    assert!(s.font().unwrap().glyph("C.alt").is_some());
}

#[test]
fn find_and_replace_renames_all_or_nothing() {
    let mut s = built("part");
    let r = run(&mut s, "edit.findAndReplace", json!({"glyphs": ["a", "b", "part"], "find": "", "replace": ".ss01"}));
    assert_eq!(r["renamed"].as_array().unwrap().len(), 3);
    let font = s.font().unwrap();
    assert!(font.glyph("a.ss01").is_some() && font.glyph("a").is_none());
    assert_eq!(font.glyph("a.ss01").unwrap().unicodes, [0x61], "a suffix change keeps the code points");
    let b = font.glyph("B").unwrap();
    assert!(matches!(&b.layers[0].shapes[0], Shape::Component(c) if c.glyph == "part.ss01"), "references follow");

    let r = run(&mut s, "edit.findAndReplace", json!({"glyphs": ["a.ss01", "b.ss01"], "find": "ss01", "replace": "ss02"}));
    assert_eq!(r["renamed"], json!([{"from": "a.ss01", "to": "a.ss02"}, {"from": "b.ss01", "to": "b.ss02"}]));

    // A clash anywhere refuses the whole batch.
    let e = refused(&mut s, "edit.findAndReplace", json!({"glyphs": ["a.ss02", "b.ss02", "c"], "find": "c", "replace": "d"}));
    assert!(e.contains("`c` → `d`: the name is taken"), "{e}");
    let e = refused(&mut s, "edit.findAndReplace", json!({"glyphs": ["a.ss02", "b.ss02"], "find": "a", "replace": "b"}));
    assert!(e.contains("taken"), "{e}");
    let e = refused(&mut s, "edit.findAndReplace", json!({"glyphs": ["a.ss02", "b.ss02"], "find": ".", "replace": " "}));
    assert!(e.contains("nothing was renamed"), "{e}");

    // One undo step per batch.
    run(&mut s, "edit.undo", json!({}));
    assert!(s.font().unwrap().glyph("a.ss01").is_some() && s.font().unwrap().glyph("b.ss01").is_some());
    check_session(&s).unwrap();
}

#[test]
fn find_and_replace_with_a_regular_expression_uses_numbered_groups() {
    let mut s = built("x.cv03");
    run(&mut s, "glyph.newGlyph", json!({"name": "y.cv12"}));
    let r = run(
        &mut s,
        "edit.findAndReplace",
        json!({"glyphs": ["x.cv03", "y.cv12", "a"], "find": "\\.cv(\\d)(\\d)", "replace": ".ss\\2\\1", "regex": true}),
    );
    assert_eq!(r["renamed"], json!([{"from": "x.cv03", "to": "x.ss30"}, {"from": "y.cv12", "to": "y.ss21"}]));
    let b = s.font().unwrap().glyph("B").unwrap().clone();
    assert!(matches!(&b.layers[0].shapes[0], Shape::Component(c) if c.glyph == "x.ss30"), "references follow");
    // Every match in a name is replaced; `\0` is the whole match.
    let r = run(&mut s, "edit.findAndReplace", json!({"glyphs": ["x.ss30"], "find": "[0-9]", "replace": "\\0\\0", "regex": true}));
    assert_eq!(r["renamed"], json!([{"from": "x.ss30", "to": "x.ss3300"}]));

    for (find, replace, needle) in
        [("(", "x", "`(`"), ("a(b)", "\\2", "group \\2"), ("(?:a{1000}){1000}", "x", "cannot be used"), ("^", ".", "nothing was renamed")]
    {
        let e = refused(&mut s, "edit.findAndReplace", json!({"glyphs": ["a", "b"], "find": find, "replace": replace, "regex": true}));
        assert!(e.contains(needle), "{find} → {replace}: {e}");
    }
    check_session(&s).unwrap();
}

#[test]
fn update_glyph_info_renames_resyncs_and_drops_overrides() {
    let mut s = Session::with_document();
    run(&mut s, "glyph.newGlyph", json!({"name": "uni0416"}));
    run(&mut s, "glyph.newGlyph", json!({"name": "uni0042"}));
    run(&mut s, "glyph.setCategory", json!({"glyphs": ["A"], "category": "Symbol"}));
    run(&mut s, "glyph.setNote", json!({"glyphs": ["A"], "note": "keep me"}));
    let r = run(&mut s, "glyph.updateGlyphInfo", json!({"glyphs": ["uni0416", "uni0042", "A"]}));
    assert_eq!(r["glyphs"], json!(["Zhe-cy", "uni0042", "A"]));
    assert_eq!(r["renamed"], json!([{"from": "uni0416", "to": "Zhe-cy"}]));
    assert_eq!(r["kept"][0]["glyph"], "uni0042");
    assert_eq!(r["kept"][0]["name"], "B");
    let a = info(&mut s, "A");
    assert_eq!(a["properties"]["category"], json!({"value": "Letter", "source": "database"}));
    assert_eq!(a["note"], "keep me");
    assert_eq!(info(&mut s, "Zhe-cy")["unicodes"], json!(["0416"]));

    // Under custom naming the names stay.
    run(&mut s, "glyph.newGlyph", json!({"name": "uni0417"}));
    s.edit("custom naming", |f| {
        Arc::make_mut(&mut f.info).settings.disables_nice_names = Some(true);
        Ok(())
    })
    .unwrap();
    let r = run(&mut s, "glyph.updateGlyphInfo", json!({"glyphs": ["uni0417"]}));
    assert_eq!(r["renamed"], json!([]));
    assert!(s.font().unwrap().glyph("uni0417").is_some());
}

#[test]
fn database_property_setters_override_and_null_returns_to_the_data() {
    let mut s = Session::with_document();
    let glyphs = json!(["A", "B"]);
    run(&mut s, "glyph.setProductionName", json!({"glyphs": glyphs, "productionName": "uniF000"}));
    run(&mut s, "glyph.setScript", json!({"glyphs": glyphs, "script": "cyrillic"}));
    run(&mut s, "glyph.setCategory", json!({"glyphs": glyphs, "category": "Symbol", "subCategory": "Math"}));
    run(&mut s, "glyph.setCase", json!({"glyphs": glyphs, "case": "noCase"}));
    run(&mut s, "glyph.setDirection", json!({"glyphs": glyphs, "direction": "RTL"}));
    let b = info(&mut s, "B");
    let p = &b["properties"];
    assert_eq!(p["productionName"], json!({"value": "uniF000", "source": "stored"}));
    assert_eq!(p["script"], json!({"value": "cyrillic", "source": "stored"}));
    assert_eq!(p["category"], json!({"value": "Symbol", "source": "stored"}));
    assert_eq!(p["subCategory"], json!({"value": "Math", "source": "stored"}));
    assert_eq!(p["case"], json!({"value": null, "source": "stored"}));
    assert_eq!(p["direction"], json!({"value": "RTL", "source": "stored"}));
    assert_eq!(b["hasCustomInfo"], true);
    assert_eq!(
        run(&mut s, "glyphData.productionName", json!({"glyphs": ["B", "C"]}))["glyphs"],
        json!([{"glyph": "B", "productionName": "uniF000", "source": "stored"}, {"glyph": "C", "productionName": "C", "source": "default"}])
    );

    run(&mut s, "glyph.setProductionName", json!({"glyphs": glyphs, "productionName": null}));
    run(&mut s, "glyph.setScript", json!({"glyphs": glyphs, "script": ""}));
    run(&mut s, "glyph.setCategory", json!({"glyphs": glyphs, "category": null, "subCategory": null}));
    run(&mut s, "glyph.setCase", json!({"glyphs": glyphs, "case": null}));
    run(&mut s, "glyph.setDirection", json!({"glyphs": glyphs, "direction": null}));
    let b = info(&mut s, "B");
    assert_eq!(b["properties"]["category"], json!({"value": "Letter", "source": "database"}));
    assert_eq!(b["properties"]["case"], json!({"value": "upper", "source": "database"}));
    assert_eq!(b["hasCustomInfo"], false);
    assert!(s.font().unwrap().glyph("B").unwrap().info.is_default(), "nothing stored is left");

    for (id, p) in [
        ("glyph.setCategory", json!({"glyphs": glyphs, "category": "Letters"})),
        ("glyph.setCategory", json!({"glyphs": glyphs})),
        ("glyph.setCase", json!({"glyphs": glyphs, "case": "title"})),
        ("glyph.setCase", json!({"glyphs": glyphs})),
        ("glyph.setDirection", json!({"glyphs": glyphs, "direction": "up"})),
        ("glyph.setScript", json!({"glyphs": glyphs, "script": "Latin"})),
        ("glyph.setProductionName", json!({"glyphs": glyphs, "productionName": "a b"})),
        ("glyph.setProductionName", json!({"glyphs": [], "productionName": "x"})),
    ] {
        refused(&mut s, id, p);
    }
}

#[test]
fn export_colour_labels_tags_notes_and_sort_names() {
    let mut s = Session::with_document();
    run(&mut s, "glyph.setExport", json!({"glyphs": ["A", "B"], "export": false}));
    assert!(!s.font().unwrap().glyph("B").unwrap().export);

    run(&mut s, "glyph.setColorLabel", json!({"glyphs": ["A", "B"], "color": "Light Green"}));
    assert_eq!(s.font().unwrap().glyph("A").unwrap().info.color, Some(ColorLabel::Index(4)));
    run(&mut s, "glyph.setColorLabel", json!({"glyphs": ["A"], "color": null}));
    assert_eq!(s.font().unwrap().glyph("A").unwrap().info.color, None);
    refused(&mut s, "glyph.setColorLabel", json!({"glyphs": ["A"], "color": 12}));

    run(&mut s, "glyph.toggleLocked", json!({"glyphs": ["C"], "locked": true}));
    let r = run(&mut s, "glyph.setLayerColorLabel", json!({"glyphs": ["A", "C"], "color": 3}));
    assert_eq!(r["glyphs"], json!(["A"]));
    assert_eq!(r["skipped"][0]["glyph"], "C");
    assert_eq!(s.font().unwrap().glyph("A").unwrap().layers[0].info.color, Some(ColorLabel::Index(3)));

    run(&mut s, "glyph.setTags", json!({"glyphs": ["A", "B"], "tags": ["round", " Top ", "round"]}));
    assert_eq!(s.font().unwrap().glyph("B").unwrap().info.tags, ["Top", "round"]);
    run(&mut s, "glyph.addTag", json!({"glyphs": ["A", "D"], "tag": "accent"}));
    assert_eq!(s.font().unwrap().glyph("A").unwrap().info.tags, ["Top", "accent", "round"]);
    run(&mut s, "glyph.removeTag", json!({"glyphs": ["A", "B", "D"], "tag": "round"}));
    assert_eq!(s.font().unwrap().glyph("B").unwrap().info.tags, ["Top"]);
    assert_eq!(s.font().unwrap().glyph("D").unwrap().info.tags, ["accent"]);
    refused(&mut s, "glyph.addTag", json!({"glyphs": ["A"], "tag": "a,b"}));
    refused(&mut s, "glyph.setTags", json!({"glyphs": ["A"], "tags": [" "]}));

    run(&mut s, "glyph.setNote", json!({"glyphs": ["E"], "note": "check the bowl"}));
    assert_eq!(info(&mut s, "E")["note"], "check the bowl");

    run(&mut s, "glyph.setSortName", json!({"glyphs": ["F"], "sortName": "zzz"}));
    assert_eq!(names(&s).last().map(String::as_str), Some("F"), "a sort name places the glyph after the others of its category");
    assert_eq!(info(&mut s, "F")["properties"]["sortName"], json!({"value": "zzz", "source": "stored"}));
    run(&mut s, "glyph.setSortName", json!({"glyphs": ["F"], "sortName": null}));
    assert_eq!(s.font().unwrap().glyph("F").unwrap().info.sort_name, None);

    // One batch is one undo step.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.font().unwrap().glyph("F").unwrap().info.sort_name.as_deref(), Some("zzz"));
    check_session(&s).unwrap();
}

#[test]
fn copy_glyph_names_in_every_format() {
    let mut s = Session::with_document();
    for name in ["five", "six", "seven", "five.alt"] {
        run(&mut s, "glyph.newGlyph", json!({"name": name}));
    }
    let copy = |s: &mut Session, glyphs: Value, format: &str| -> String {
        run(s, "glyph.copyGlyphNames", json!({"glyphs": glyphs, "format": format}))["text"].as_str().unwrap().to_string()
    };
    let three = json!(["seven", "five", "six"]);
    assert_eq!(copy(&mut s, three.clone(), "spaceSeparated"), "five six seven ");
    assert_eq!(copy(&mut s, three.clone(), "onePerLine"), "five\nsix\nseven");
    assert_eq!(copy(&mut s, three.clone(), "commaSeparated"), "five, six, seven, ");
    assert_eq!(copy(&mut s, three.clone(), "slashed"), "/five/six/seven");
    assert_eq!(copy(&mut s, three.clone(), "pythonList"), "\"five\",\n\"six\",\n\"seven\",");
    assert_eq!(copy(&mut s, json!(["five.alt", "five"]), "unicodes"), "0035\n- # five.alt");
    run(&mut s, "glyph.setUnicodes", json!({"glyphs": ["six"], "unicodes": ["0036", "1F600"]}));
    assert_eq!(copy(&mut s, json!(["six"]), "unicodes"), "0036, 1F600");
    refused(&mut s, "glyph.copyGlyphNames", json!({"glyphs": three, "format": "tabbed"}));
}

#[test]
fn validate_glyph_names_reports_position_and_duplicates() {
    let mut s = Session::with_document();
    assert_eq!(run(&mut s, "font.validateGlyphNames", json!({}))["invalid"], json!([]));
    s.edit("bad names", |f| {
        f.add_glyph("bad name", Vec::new(), 500.0)?;
        f.add_glyph("1abc", Vec::new(), 500.0)?;
        f.add_glyph("_part", Vec::new(), 500.0)?;
        Ok(())
    })
    .unwrap();
    let r = run(&mut s, "font.validateGlyphNames", json!({}));
    let invalid = r["invalid"].as_array().unwrap();
    assert_eq!(invalid.len(), 2, "{invalid:?}");
    let bad = invalid.iter().find(|i| i["glyph"] == "bad name").unwrap();
    assert_eq!((&bad["problem"], &bad["position"], &bad["character"]), (&json!("character"), &json!(4), &json!(" ")));
    let digit = invalid.iter().find(|i| i["glyph"] == "1abc").unwrap();
    assert_eq!(digit["problem"], "firstCharacter");
    assert_eq!(run(&mut s, "font.validateGlyphNames", json!({"glyphs": ["A"]}))["checked"], 1);
}

#[test]
fn glyph_data_lookup_and_search() {
    let mut s = Session::with_document();
    let zhe = run(&mut s, "glyphData.lookup", json!({"name": "Zhe-cy"}));
    assert_eq!(zhe["unicode"], json!({"value": "0416", "source": "database"}));
    assert_eq!(zhe["category"]["value"], "Letter");
    assert_eq!(zhe["found"]["via"], "name");
    assert_eq!(run(&mut s, "glyphData.lookup", json!({"unicode": "0416"}))["name"], "Zhe-cy");
    assert_eq!(run(&mut s, "glyphData.lookup", json!({"char": "Ж"}))["name"], "Zhe-cy");
    let by_production = run(&mut s, "glyphData.lookup", json!({"name": "uni0416"}));
    assert_eq!(by_production["found"], json!({"name": "Zhe-cy", "via": "productionName", "custom": false}));
    let sc = run(&mut s, "glyphData.lookup", json!({"name": "A-cy.sc"}));
    assert_eq!(sc["productionName"], json!({"value": "uni0410.sc", "source": "synthesised"}));
    refused(&mut s, "glyphData.lookup", json!({"name": "A", "char": "A"}));

    let r = run(&mut s, "glyphData.search", json!({"query": "zhe-cy", "limit": 50}));
    assert!(r["results"].as_array().unwrap().iter().any(|x| x["name"] == "Zhe-cy" && x["unicode"] == "0416"));
    assert!(r["count"].as_u64().unwrap() <= 50);
    refused(&mut s, "glyphData.search", json!({"query": "a", "sortBy": "size"}));

    // Both work without a document too.
    let mut empty = Session::new();
    assert_eq!(run(&mut empty, "glyphData.lookup", json!({"name": "A"}))["unicode"]["value"], "0041");
}
