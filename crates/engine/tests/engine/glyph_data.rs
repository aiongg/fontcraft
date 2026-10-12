//! The session's glyph data, new glyphs' properties and places, and Last Changed stamps.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use fontcraft_engine::Session;
use fontcraft_engine::clock::fixed_clock;
use fontcraft_engine::model::plist::{Str, Value as Plist};
use fontcraft_engine::model::{PropSource, glyph_props};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn names(s: &Session) -> Vec<String> {
    s.font().unwrap().glyphs.iter().map(|g| g.name.clone()).collect()
}

fn set_param(s: &mut Session, name: &str, value: Plist) {
    s.edit("param", |f| {
        std::sync::Arc::make_mut(&mut f.info).params.set(name, value);
        Ok(())
    })
    .unwrap();
}

#[test]
fn new_glyph_gets_database_properties_and_its_place() {
    let mut s = Session::with_document();
    // Move `space` to the end: the stored order is kept, not re-sorted.
    s.edit("move", |f| {
        let a = f.glyphs.remove("space").unwrap();
        f.glyphs.insert(usize::MAX, fontcraft_engine::model::Glyph::clone(&a));
        Ok(())
    })
    .unwrap();
    let r = run(&mut s, "glyph.newGlyph", json!({"name": "A-cy"}));
    assert_eq!(r["unicodes"], json!(["0410"]));
    let order = names(&s);
    let at = order.iter().position(|n| n == "A-cy").unwrap();
    assert_eq!(r["index"], json!(at));
    assert_eq!(order.get(at - 1).map(String::as_str), Some("z"), "after the Latin letters");
    assert_eq!(order.last().map(String::as_str), Some("space"), "the stored order stays as it was");

    let data = s.glyph_data().unwrap();
    let g = s.font().unwrap().glyph("A-cy").unwrap().clone();
    let p = glyph_props(&g, &data, false);
    assert_eq!((p.script.as_deref(), p.script.source), (Some("cyrillic"), PropSource::Database));
    assert_eq!(p.production_name.as_deref(), Some("uni0410"));

    // Explicit code points win; an alternate gets none.
    assert_eq!(run(&mut s, "glyph.newGlyph", json!({"name": "A-cy.alt"}))["unicodes"], json!([]));
    assert_eq!(run(&mut s, "glyph.newGlyph", json!({"name": "mine", "unicodes": ["E000"]}))["unicodes"], json!(["E000"]));
    // One undo step per new glyph.
    run(&mut s, "edit.undo", json!({}));
    assert!(s.font().unwrap().glyph("mine").is_none());
}

#[test]
fn custom_glyph_data_and_the_ignore_parameter() {
    let mut s = Session::with_document();
    let user = br#"<glyphData><glyph name="myStar" unicode="E100" category="Symbol"/></glyphData>"#;
    let problems = s.set_user_glyph_data(&[("GlyphData.xml", user), ("GlyphData-bad.xml", b"<glyphData><glyph")]);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("GlyphData-bad.xml"), "{problems:?}");
    let own = br#"<glyphData><glyph name="myStar" unicode="E200" category="Letter"/></glyphData>"#;
    assert!(s.set_source_glyph_data(&[("GlyphData.xml", own)]).unwrap().is_empty());

    let cp = |s: &Session| s.glyph_data().unwrap().record("myStar").and_then(|r| r.unicode);
    assert_eq!(cp(&s), Some(0xE200), "the source's file wins over the user's");
    set_param(&mut s, "Ignore custom GlyphData file", Plist::Bool(true));
    assert_eq!(cp(&s), Some(0xE100), "the parameter turns the source's file off");
    assert_eq!(run(&mut s, "glyph.newGlyph", json!({"name": "myStar"}))["unicodes"], json!(["E100"]));
}

#[test]
fn edits_stamp_last_changed_unless_the_font_says_not_to() {
    let mut s = Session::with_document();
    let stamp = |s: &Session, g: &str| s.font().unwrap().glyph(g).unwrap().info.last_change.clone();
    run(&mut s, "spacing.setWidth", json!({"glyphs": ["A"], "value": 630}));
    assert_eq!(stamp(&s, "A"), None, "a session without a clock stamps nothing");

    s.set_clock(Some(fixed_clock(1_791_460_800)));
    run(&mut s, "spacing.setWidth", json!({"glyphs": ["A"], "value": 640}));
    assert_eq!(stamp(&s, "A").as_deref(), Some("2026-10-08 12:00:00 +0000"));
    assert_eq!(stamp(&s, "B"), None, "untouched glyphs keep theirs");
    run(&mut s, "glyph.newGlyph", json!({"name": "new"}));
    assert!(stamp(&s, "new").is_some());

    set_param(&mut s, "Write lastChange", Plist::Bool(false));
    run(&mut s, "spacing.setWidth", json!({"glyphs": ["B"], "value": 640}));
    assert_eq!(stamp(&s, "B"), None);
    set_param(&mut s, "Write lastChange", Plist::String(Str::new("1")));
    set_param(&mut s, "Disable Last Change", Plist::Bool(true));
    run(&mut s, "spacing.setWidth", json!({"glyphs": ["B"], "value": 650}));
    assert_eq!(stamp(&s, "B"), None);
}
