//! Engine behaviour: the registry, undo, transactions, the M0 vertical slice.
//!
//! Integration tests (not unit tests) because they use `fontcraft-testkit`, which itself depends
//! on this crate: inside the crate its `Session` would be a different type.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use base64::Engine as _;
use fontcraft_testkit::{Fixture, check_session, rect, snapshot, temp_dir};
use serde_json::{Value, json};
use skrifa::MetadataProvider;
use skrifa::raw::TableProvider;

use fontcraft_engine::{EngineError, Session, command_specs, find_command};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

#[test]
fn registry_is_well_formed() {
    let specs = command_specs();
    assert!(specs.len() >= 15, "only {} commands", specs.len());
    let mut ids = std::collections::HashSet::new();
    for c in specs {
        assert!(ids.insert(c.id), "duplicate id {}", c.id);
        assert!(!c.label.is_empty() && !c.params.is_empty(), "{}: label and params doc are required", c.id);
        let mut parts = c.id.split('.');
        assert!(parts.clone().count() >= 2, "{}: ids are noun.verb", c.id);
        assert!(parts.all(|p| !p.is_empty() && p.chars().all(|ch| ch.is_ascii_alphanumeric())), "{}: ASCII letters, digits and dots only", c.id);
        assert!(c.params.starts_with('{'), "{}: the params doc starts with the parameter object", c.id);
        if let Some(sc) = c.shortcut {
            let order = ["Cmd", "Ctrl", "Alt", "Shift"];
            let mods: Vec<usize> = sc.split('+').filter_map(|m| order.iter().position(|o| *o == m)).collect();
            assert!(mods.windows(2).all(|w| w[0] < w[1]), "{}: modifiers are written Cmd+Ctrl+Alt+Shift+Key, got {sc}", c.id);
        }
    }
    // A shortcut may repeat when the copies work in different places (`when`).
    let mut seen = std::collections::HashSet::new();
    for c in specs.iter().filter(|c| c.shortcut.is_some()) {
        assert!(seen.insert((c.shortcut, c.when)), "{}: another command has the default shortcut {:?} where {:?}", c.id, c.shortcut, c.when);
    }
    assert!(find_command("file.new").is_some() && find_command("no.such").is_none());
}

#[test]
fn unknown_and_disabled_commands_are_errors() {
    let mut s = Session::new();
    assert!(matches!(s.execute("no.such", &json!({})), Err(EngineError::UnknownCommand(_))));
    assert!(matches!(s.execute("edit.undo", &json!({})), Err(EngineError::Disabled(_, why)) if why == "nothing to undo"));
    assert!(matches!(s.execute("glyph.newGlyph", &json!({})), Err(EngineError::Disabled(_, why)) if why == "no document open"));
}

#[test]
fn undo_and_redo_restore_the_font_exactly() {
    let mut s = Session::with_document();
    let start = s.font().unwrap().clone();
    run(&mut s, "glyph.newGlyph", json!({"name": "one"}));
    let one = s.font().unwrap().clone();
    run(&mut s, "path.addPath", json!({"glyph": "one", "nodes": rect(0.0, 0.0, 10.0, 10.0)}));
    let two = s.font().unwrap().clone();
    assert_eq!(run(&mut s, "edit.undo", json!({}))["undone"], "Add Path");
    assert_eq!(s.font().unwrap(), &one);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.font().unwrap(), &start);
    assert!(s.execute("edit.undo", &json!({})).is_err());
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(run(&mut s, "edit.redo", json!({}))["redone"], "Add Path");
    assert_eq!(s.font().unwrap(), &two);
    // A new change drops the redo steps.
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "spacing.setWidth", json!({"glyphs": ["one"], "value": 300}));
    assert!(s.execute("edit.redo", &json!({})).is_err());
}

#[test]
fn an_edit_that_changes_nothing_makes_no_undo_step() {
    let mut s = Session::with_document();
    run(&mut s, "spacing.setWidth", json!({"glyphs": ["A"], "value": 600}));
    let doc = s.doc().unwrap();
    assert_eq!((doc.history.undo.len(), doc.revision, doc.modified()), (0, 0, false), "A is already 600 wide");
    run(&mut s, "spacing.setWidth", json!({"glyphs": ["A"], "value": 610}));
    assert_eq!(s.doc().unwrap().history.undo.len(), 1);
    run(&mut s, "spacing.setWidth", json!({"glyphs": ["A"], "value": 610}));
    assert_eq!(s.doc().unwrap().history.undo.len(), 1);
}

#[test]
fn undo_snapshots_share_untouched_glyphs() {
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(0.0, 0.0, 10.0, 10.0)}));
    let doc = s.doc().unwrap();
    let before = &doc.history.undo[0].font;
    let (a_before, a_now) = (before.glyph("A").unwrap(), doc.font.glyph("A").unwrap());
    let (b_before, b_now) = (before.glyph("B").unwrap(), doc.font.glyph("B").unwrap());
    assert!(!std::sync::Arc::ptr_eq(a_before, a_now), "the edited glyph was copied");
    assert!(std::sync::Arc::ptr_eq(b_before, b_now), "an untouched glyph is shared, not copied");
}

#[test]
fn a_failed_command_changes_nothing() {
    let mut s = Fixture::Drawn.session();
    let before = snapshot(&s);
    // The second glyph does not exist: the first must not be removed either.
    assert!(s.execute("glyph.removeGlyph", &json!({"glyphs": ["A", "nope"]})).is_err());
    assert!(s.execute("spacing.setWidth", &json!({"glyphs": ["A", "nope"], "value": 1})).is_err());
    assert!(s.execute("font.info.set", &json!({"values": {"familyName": "Changed", "unitsPerEm": 5}})).is_err());
    assert!(s.execute("path.addPath", &json!({"glyph": "A", "nodes": [{"x": 0, "y": 0, "type": "curve"}]})).is_err());
    assert_eq!(snapshot(&s), before);
}

#[test]
fn queries_are_not_journaled_and_changes_are() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"familyName": "Journal"}));
    run(&mut s, "font.info.get", json!({}));
    run(&mut s, "commands.list", json!({}));
    run(&mut s, "glyph.newGlyph", json!({}));
    assert!(s.execute("glyph.newGlyph", &json!({"name": "A"})).is_err());
    let ids: Vec<&str> = s.journal().iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, ["file.new", "glyph.newGlyph"]);
}

#[test]
fn new_glyph_names_do_not_collide() {
    let mut s = Session::with_document();
    assert_eq!(run(&mut s, "glyph.newGlyph", json!({}))["name"], "newGlyph");
    assert_eq!(run(&mut s, "glyph.newGlyph", json!({}))["name"], "newGlyph.001");
    assert_eq!(run(&mut s, "glyph.newGlyph", json!({}))["name"], "newGlyph.002");
}

#[test]
fn vertical_slice_new_draw_save_open_export() {
    let dir = temp_dir("slice");
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"familyName": "Slice Sans"}));
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(60.0, 0.0, 560.0, 700.0)}));
    run(&mut s, "spacing.setWidth", json!({"glyphs": ["A"], "value": 620}));
    run(&mut s, "glyph.newGlyph", json!({"name": "uniE000", "unicodes": ["E000"], "width": 1000}));
    run(&mut s, "font.info.set", json!({"values": {"versionMajor": 2, "versionMinor": 5}}));
    assert!(s.doc().unwrap().modified());

    let file = dir.join("slice.fontcraft");
    let saved = run(&mut s, "file.save", json!({"path": file.to_string_lossy()}));
    assert!(saved["bytes"].as_u64().unwrap() > 100);
    assert!(!s.doc().unwrap().modified());
    assert_eq!(s.doc().unwrap().title, "slice");

    // A second session opens the file and gets the same font.
    let mut other = Session::new();
    run(&mut other, "file.open", json!({"path": file.to_string_lossy()}));
    assert_eq!(other.font().unwrap(), s.font().unwrap());
    assert!(!other.doc().unwrap().modified());

    // Export to memory and to a file; both are the same bytes for the same timestamp.
    let mem = run(&mut other, "file.exportStatic", json!({"timestamp": 1_700_000_000}));
    let bytes = base64::engine::general_purpose::STANDARD.decode(mem["dataBase64"].as_str().unwrap()).unwrap();
    let out = run(&mut other, "file.exportStatic", json!({"destination": dir.to_string_lossy(), "timestamp": 1_700_000_000}));
    let path = out["path"].as_str().unwrap();
    assert!(path.ends_with("SliceSans-Regular.ttf"), "{path}");
    assert_eq!(std::fs::read(path).unwrap(), bytes);

    let font = skrifa::FontRef::new(&bytes).unwrap();
    let a = font.charmap().map('A').unwrap();
    let metrics = font.glyph_metrics(skrifa::instance::Size::unscaled(), skrifa::instance::LocationRef::default());
    assert_eq!(metrics.advance_width(a), Some(620.0));
    assert!(font.charmap().map('\u{E000}').is_some());
    assert!((font.head().unwrap().font_revision().to_f64() - 2.005).abs() < 1e-4);
    assert_eq!(mem["glyphOrder"][0], ".notdef");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn closing_a_modified_document_needs_discard() {
    let mut s = Session::with_document();
    run(&mut s, "glyph.newGlyph", json!({}));
    assert!(s.execute("file.close", &json!({})).unwrap_err().to_string().contains("unsaved changes"));
    run(&mut s, "file.close", json!({"discard": true}));
    assert!(s.active().is_none());
    check_session(&s).unwrap();
    // An untouched new document closes without asking.
    run(&mut s, "file.new", json!({}));
    run(&mut s, "file.close", json!({}));
}

#[test]
fn list_glyphs_selects_columns_and_source_names() {
    let mut s = Fixture::Drawn.session();
    let list = run(&mut s, "font.listGlyphs", json!({ "glyphs": "A,extra", "excludeGlyphs": ["extra"], "columns": ["name", "kind"] }));
    let glyphs = list["glyphs"].as_array().unwrap();
    assert_eq!(glyphs.len(), 1, "{list}");
    let keys: Vec<&String> = glyphs[0].as_object().unwrap().keys().collect();
    assert_eq!(keys.len(), 3, "gid and the columns asked for: {keys:?}");
    assert_eq!((glyphs[0]["name"].as_str(), glyphs[0]["kind"].as_str()), (Some("A"), Some("simple")));
    let by_code = run(&mut s, "font.listGlyphs", json!({ "glyphs": "U+E000", "columns": ["name"] }));
    assert_eq!(by_code["glyphs"][0]["name"], "extra");
    let dir = temp_dir("list-goadb");
    let goadb = dir.join("GlyphOrderAndAliasDB");
    std::fs::write(&goadb, "A a.source\n").unwrap();
    let mapped = run(&mut s, "font.listGlyphs", json!({ "glyphs": "A..A", "columns": ["name"], "goadb": goadb }));
    assert_eq!(mapped["glyphs"][0]["name"], "a.source", "a GOADB shows the source name");
    assert!(matches!(s.execute("font.listGlyphs", &json!({ "columns": ["cid"] })), Err(EngineError::Unsupported(_))));
    assert!(s.execute("font.listGlyphs", &json!({ "columns": ["width"] })).is_err());
    assert!(s.execute("font.listGlyphs", &json!({ "glyphs": "nope" })).is_err());
    assert!(s.execute("font.listGlyphs", &json!({ "names": "source" })).is_err(), "source names need a GOADB");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn list_glyphs_and_info_report_the_font() {
    let mut s = Fixture::Drawn.session();
    let list = run(&mut s, "font.listGlyphs", json!({}));
    let glyphs = list["glyphs"].as_array().unwrap();
    let a = glyphs.iter().find(|g| g["name"] == "A").unwrap();
    assert_eq!((a["unicodes"][0].as_str(), a["advance"].as_f64(), a["bounds"][2].as_f64()), (Some("0041"), Some(620.0), Some(560.0)));
    assert_eq!(glyphs.iter().find(|g| g["name"] == "B").unwrap()["bounds"], Value::Null);
    assert_eq!(run(&mut s, "font.info.get", json!({"keys": ["unitsPerEm"]})), json!({"unitsPerEm": 1000}));
    assert!(s.execute("font.info.get", &json!({"keys": ["nope"]})).is_err());
    let inspect = run(&mut s, "document.inspect", json!({}));
    assert_eq!(inspect["documents"][0]["undo"].as_array().unwrap().len(), 4);
}

#[test]
fn modified_follows_the_saved_font_through_undo_and_redo() {
    let dir = temp_dir("saved");
    let mut s = Session::with_document();
    assert!(!s.doc().unwrap().modified(), "a new, untouched document is not modified");
    run(&mut s, "glyph.newGlyph", json!({"name": "one"}));
    assert!(s.doc().unwrap().modified());
    run(&mut s, "edit.undo", json!({}));
    assert!(!s.doc().unwrap().modified(), "undoing back to the starting font");
    run(&mut s, "edit.redo", json!({}));
    run(&mut s, "file.save", json!({"path": dir.join("a.fontcraft").to_string_lossy()}));
    assert!(!s.doc().unwrap().modified());
    run(&mut s, "edit.undo", json!({}));
    assert!(s.doc().unwrap().modified(), "the file holds the glyph, the document no longer does");
    run(&mut s, "edit.redo", json!({}));
    assert!(!s.doc().unwrap().modified(), "back at what is on disk");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_edit_that_fails_leaves_the_font_as_it_was_even_outside_execute() {
    let mut s = Session::with_document();
    let before = s.font().unwrap().clone();
    // The closure succeeds but the result is invalid.
    assert!(
        s.edit("bad", |f| {
            f.units_per_em = 1;
            Ok(())
        })
        .is_err()
    );
    assert_eq!(s.font().unwrap(), &before);
    // The closure itself fails after changing its copy.
    let r: Result<(), EngineError> = s.edit("bad", |f| {
        f.family_name = "Changed".into();
        Err(EngineError::Unsupported("stop".into()))
    });
    assert!(r.is_err());
    assert_eq!(s.font().unwrap(), &before);
    assert!(s.doc().unwrap().history.undo.is_empty() && !s.doc().unwrap().modified());
}

#[test]
fn hostile_values_are_refused_at_the_command() {
    let mut s = Session::with_document();
    let refused = |s: &mut Session, id: &str, p: Value| {
        let before = snapshot(s);
        let r = s.execute(id, &p);
        assert!(r.is_err(), "{id} {p} was accepted");
        assert_eq!(snapshot(s), before, "{id} {p} changed the session");
    };
    refused(&mut s, "glyph.newGlyph", json!({"name": "n".repeat(256)}));
    refused(&mut s, "glyph.newGlyph", json!({"name": "a\u{0}b"}));
    refused(&mut s, "glyph.newGlyph", json!({"name": "wide", "width": 1e308}));
    refused(&mut s, "glyph.newGlyph", json!({"name": "narrow", "width": -1}));
    refused(&mut s, "glyph.setUnicodes", json!({"glyphs": [], "unicodes": ["0041"]}));
    refused(&mut s, "spacing.setWidth", json!({"glyphs": [], "value": 500}));
    refused(&mut s, "font.info.set", json!({"values": {"familyName": "F".repeat(40_000)}}));
    refused(&mut s, "font.info.set", json!({"values": {"versionMajor": 40_000}}));
    // Coordinates that once exhausted memory at export are refused when the path is added.
    refused(
        &mut s,
        "path.addPath",
        json!({"glyph": "A", "nodes": [{"x": 0, "y": 0}, {"x": 1e30, "y": 0, "type": "offcurve"}, {"x": 0, "y": 1e30, "type": "offcurve"}, {"x": 1e30, "y": 1e30, "type": "curve"}]}),
    );
    refused(&mut s, "path.addPath", json!({"glyph": "A", "nodes": [{"x": 1_000_001, "y": 0}]}));
    // Not a regular file.
    refused(&mut s, "file.open", json!({"path": std::env::temp_dir().to_string_lossy()}));
    // A path within the model's limits but beyond a TrueType outline's: export says which glyph.
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(0.0, 0.0, 20_000.0, 100.0)}));
    let e = s.execute("file.exportStatic", &json!({})).unwrap_err().to_string();
    assert!(e.contains("glyph `A`") && e.contains("TrueType"), "{e}");
}

#[test]
fn queries_skip_the_snapshot_but_still_change_nothing() {
    let mut s = Fixture::Drawn.session();
    let before = snapshot(&s);
    for id in ["font.info.get", "font.listGlyphs", "commands.list", "document.inspect"] {
        run(&mut s, id, json!({}));
        assert!(s.execute(id, &json!({"keys": 5, "layer": 5, "pattern": 5})).is_err() || id == "document.inspect");
    }
    assert_eq!(snapshot(&s), before);
}

#[test]
fn saves_and_exports_are_not_listed_as_queries() {
    let mut s = Session::new();
    let list = run(&mut s, "commands.list", json!({}));
    let query = |id: &str| list["commands"].as_array().unwrap().iter().find(|c| c["id"] == id).unwrap()["query"].as_bool().unwrap();
    for id in ["file.save", "file.saveAs", "file.exportStatic", "file.close", "edit.undo"] {
        assert!(!query(id), "{id} writes or changes state");
    }
    for id in ["font.info.get", "font.listGlyphs", "commands.list", "document.inspect"] {
        assert!(query(id), "{id} only reads");
    }
}

#[test]
fn a_batch_is_one_undo_step_and_fails_as_a_whole() {
    let mut s = Session::with_document();
    let start = s.font().unwrap().clone();
    let out = run(
        &mut s,
        "command.batch",
        json!({ "label": "Make Alt", "steps": [
            { "command": "glyph.newGlyph", "params": { "name": "A.alt" } },
            { "command": "spacing.setWidth", "params": { "glyphs": ["A.alt"], "value": 640 } },
        ]}),
    );
    assert_eq!(out["results"].as_array().map(Vec::len), Some(2));
    let doc = s.doc().unwrap();
    assert_eq!(doc.history.undo.iter().map(|h| h.label.as_ref()).collect::<Vec<_>>(), ["Make Alt"]);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.font().unwrap(), &start, "one undo takes back both steps");

    // A failing step leaves nothing behind, not even the steps before it.
    let err = s
        .execute(
            "command.batch",
            &json!({ "steps": [
                { "command": "glyph.newGlyph", "params": { "name": "B.alt" } },
                { "command": "spacing.setWidth", "params": { "glyphs": ["nope"], "value": 1 } },
            ]}),
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("step 1"), "{err}");
    assert!(s.font().unwrap().glyph("B.alt").is_none());
    assert!(s.batch_is_idle());
    assert!(s.execute("command.batch", &json!({ "steps": [{ "command": "edit.undo" }] })).is_err());
}

#[test]
fn a_batch_inside_a_batch_belongs_to_the_outer_one() {
    let mut s = Session::with_document();
    run(
        &mut s,
        "command.batch",
        json!({ "label": "Outer", "steps": [
            { "command": "command.batch", "params": { "label": "Inner", "steps": [{ "command": "glyph.newGlyph", "params": { "name": "x.alt" } }] } },
            { "command": "glyph.newGlyph", "params": { "name": "y.alt" } },
        ]}),
    );
    assert_eq!(s.doc().unwrap().history.undo.iter().map(|h| h.label.as_ref()).collect::<Vec<_>>(), ["Outer"]);
}

#[test]
fn documents_are_activated_by_index_and_a_file_opens_once() {
    let dir = temp_dir("activate");
    let mut s = Session::with_document();
    let path = dir.join("One.fontcraft");
    run(&mut s, "file.saveAs", json!({ "path": path }));
    run(&mut s, "file.new", json!({ "familyName": "Two" }));
    assert_eq!(s.font().unwrap().family_name, "Two");
    run(&mut s, "document.activate", json!({ "index": 0 }));
    assert_eq!(s.font().unwrap().family_name, "Untitled");
    run(&mut s, "document.activate", json!({ "index": 1 }));
    let again = run(&mut s, "file.open", json!({ "path": path }));
    assert_eq!(again["alreadyOpen"], true);
    assert_eq!(s.documents().len(), 2, "no second copy");
    assert_eq!(s.font().unwrap().family_name, "Untitled");
    run(&mut s, "document.activate", json!({ "index": 1 }));
    run(&mut s, "document.activate", json!({ "path": path }));
    assert_eq!(s.font().unwrap().family_name, "Untitled");
    assert!(s.execute("document.activate", &json!({ "index": 7 })).is_err());
    assert!(s.execute("document.activate", &json!({})).is_err());
}
