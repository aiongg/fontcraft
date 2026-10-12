//! Compile state, diagnostics and the code editor's queries (Font Info › Features): the
//! font.features.compile, listErrors, applyFix, jumpToError, showErrorDetails, evaluateToken,
//! complete and previewName commands.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, clippy::string_slice)]

use std::sync::Arc;

use fontcraft_engine::{EngineError, Session};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

/// Runs a command that must be refused with a message holding `needle`, and checks that the font
/// is the very same pointer afterwards.
fn refused(s: &mut Session, id: &str, p: Value, needle: &str) {
    let before = Arc::clone(&s.doc().unwrap().font);
    match s.execute(id, &p) {
        Err(EngineError::BadParams { message, .. }) => assert!(message.contains(needle), "{id} {p}: {message}"),
        other => panic!("{id} {p}: expected a refusal naming `{needle}`, got {other:?}"),
    }
    assert!(Arc::ptr_eq(&before, &s.doc().unwrap().font), "{id} {p}: a refused command changed the font");
}

fn set_code(s: &mut Session, kind: &str, name: &str, code: &str) {
    run(s, "font.features.setCode", json!({"kind": kind, "name": name, "code": code}));
}

fn code_of(s: &mut Session, kind: &str, name: &str) -> String {
    run(s, "font.features.get", json!({"kind": kind, "name": name}))["code"].as_str().unwrap().to_string()
}

/// The first error of the last compile.
fn first_error(result: &Value) -> &Value {
    result["diagnostics"].as_array().unwrap().iter().find(|d| d["severity"] == "error").unwrap_or_else(|| panic!("no error in {result}"))
}

#[test]
fn compile_reports_a_broken_feature_at_its_entry_and_range_and_keeps_the_state() {
    let mut s = Session::with_document();
    run(&mut s, "font.features.addClass", json!({"name": "Upper", "code": "A B"}));
    run(&mut s, "font.features.addFeature", json!({"tag": "liga", "code": "sub a by b;\nsub c by zzz;"}));
    let font = Arc::clone(&s.doc().unwrap().font);
    let undo = s.doc().unwrap().history.undo.len();

    let r = run(&mut s, "font.features.compile", json!({}));
    assert_eq!(r["ok"], false);
    assert_eq!(r["cached"], false);
    assert_eq!(r["preview"], false, "no preview font from code with errors");
    let e = first_error(&r);
    assert_eq!(e["message"], "glyph not in font");
    assert_eq!(e["entry"], json!({"kind": "feature", "name": "liga", "index": 0}));
    let range = e["range"].as_array().unwrap();
    let (start, end) = (range[0].as_u64().unwrap() as usize, range[1].as_u64().unwrap() as usize);
    assert_eq!(&"sub a by b;\nsub c by zzz;"[start..end], "zzz");
    assert_eq!((e["line"].clone(), e["column"].clone()), (json!(2), json!(10)));
    // Compile changes no font data and makes no undo step.
    assert!(Arc::ptr_eq(&font, &s.doc().unwrap().font));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);

    // fontInfo.features.inlineErrors: the sidebar's error flag and the error list.
    let list = run(&mut s, "font.features.list", json!({"kind": "feature"}));
    assert_eq!(list["entries"][0]["errors"], true);
    let errors = run(&mut s, "font.features.listErrors", json!({}));
    assert_eq!((errors["compiled"].clone(), errors["current"].clone(), errors["errors"].clone()), (json!(true), json!(true), json!(1)));
    let only_class = run(&mut s, "font.features.listErrors", json!({"kind": "class", "name": "Upper"}));
    assert_eq!(only_class["diagnostics"], json!([]));

    // The same code again reuses the result; force compiles it again.
    assert_eq!(run(&mut s, "font.features.compile", json!({}))["cached"], true);
    assert_eq!(run(&mut s, "font.features.compile", json!({"force": true}))["cached"], false);

    // An edit makes the state stale; the corrected code compiles and lists its features
    // (features.menuNeedsCompile: the menu reads them from the state).
    set_code(&mut s, "feature", "liga", "sub a by b;\nsub c by d;");
    assert_eq!(run(&mut s, "font.features.listErrors", json!({}))["current"], false);
    let r = run(&mut s, "font.features.compile", json!({"master": "Regular"}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["master"], "m01");
    let liga = r["features"].as_array().unwrap().iter().find(|f| f["tag"] == "liga").unwrap_or_else(|| panic!("{r}"));
    assert_eq!(liga["table"], "GSUB");
    let state = s.feature_state().unwrap();
    assert!(state.is_current(s.doc().unwrap()) && !state.has_errors());
    assert!(state.features.iter().any(|f| f.tag.to_string() == "liga"));
}

#[test]
fn a_token_error_is_reported_at_the_token_and_a_token_edit_is_not_served_from_the_cache() {
    let mut s = Session::with_document();
    run(&mut s, "font.numbers.add", json!({"name": "shift", "values": {"m01": 10}}));
    run(&mut s, "font.features.addFeature", json!({"tag": "kern", "code": "pos a $missing;"}));
    let r = run(&mut s, "font.features.compile", json!({}));
    let e = first_error(&r);
    assert_eq!(e["entry"]["name"], "kern");
    assert_eq!(e["range"], json!([6, 14]), "{e}");

    set_code(&mut s, "feature", "kern", "pos a $shift;");
    assert_eq!(run(&mut s, "font.features.compile", json!({}))["ok"], true);
    // The literal value gives the same compiled text; the stored code is new, so its result is
    // mapped again even when it is reused.
    set_code(&mut s, "feature", "kern", "pos a 10;");
    let r = run(&mut s, "font.features.compile", json!({}));
    assert_eq!((r["ok"].clone(), r["cached"].clone()), (json!(true), json!(true)));
}

#[test]
fn each_fix_gives_code_that_compiles() {
    let mut s = Session::with_document();
    run(&mut s, "glyph.newGlyph", json!({"name": "f_i"}));

    // A glyph name in the wrong case.
    run(&mut s, "font.features.addFeature", json!({"tag": "liga", "code": "sub f i by F_I;"}));
    let r = run(&mut s, "font.features.compile", json!({}));
    let e = first_error(&r).clone();
    assert_eq!(e["fixes"], json!(["Replace `F_I` with `f_i`"]));
    let undo = s.doc().unwrap().history.undo.len();
    let fixed = run(&mut s, "font.features.applyFix", json!({"error": e["id"]}));
    assert_eq!(fixed["code"], "sub f i by f_i;");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "a fix is one undo step");
    assert_eq!(run(&mut s, "font.features.compile", json!({}))["ok"], true);

    // A missing semicolon, before a comment.
    set_code(&mut s, "feature", "liga", "sub f i by f_i # ligature\n");
    let r = run(&mut s, "font.features.compile", json!({}));
    let e = first_error(&r).clone();
    assert_eq!(e["fixes"], json!(["Insert a semicolon"]), "{r}");
    run(&mut s, "font.features.applyFix", json!({"error": e["id"], "fix": 0}));
    assert_eq!(code_of(&mut s, "feature", "liga"), "sub f i by f_i; # ligature\n");
    assert_eq!(run(&mut s, "font.features.compile", json!({}))["ok"], true);

    // A class used before its entry: the entry moves up.
    run(&mut s, "font.features.addClass", json!({"name": "Both", "code": "@Lower A"}));
    run(&mut s, "font.features.addClass", json!({"name": "Lower", "code": "a b"}));
    let r = run(&mut s, "font.features.compile", json!({}));
    let e = first_error(&r).clone();
    assert_eq!(e["entry"]["name"], "Both");
    assert_eq!(e["fixes"], json!(["Move the class @Lower before @Both"]), "{r}");
    run(&mut s, "font.features.applyFix", json!({"error": e["id"]}));
    let classes = run(&mut s, "font.features.list", json!({"kind": "class"}));
    let names: Vec<&str> = classes["entries"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Lower", "Both"]);
    assert_eq!(run(&mut s, "font.features.compile", json!({}))["ok"], true);
}

#[test]
fn fixes_and_error_queries_refuse_what_they_cannot_do() {
    let mut s = Session::with_document();
    run(&mut s, "font.features.addFeature", json!({"tag": "liga", "code": "sub a by zzz;"}));
    refused(&mut s, "font.features.applyFix", json!({"error": 0}), "not been compiled");
    refused(&mut s, "font.features.jumpToError", json!({"kind": "feature", "name": "liga"}), "not been compiled");
    run(&mut s, "font.features.compile", json!({}));
    refused(&mut s, "font.features.applyFix", json!({"error": 0}), "has no fix");
    refused(&mut s, "font.features.applyFix", json!({"error": 7}), "no error 7");
    refused(&mut s, "font.features.showErrorDetails", json!({"error": 0, "kind": "feature", "name": "calt"}), "no feature `calt`");

    set_code(&mut s, "feature", "liga", "sub a by B_;");
    run(&mut s, "font.features.compile", json!({}));
    set_code(&mut s, "feature", "liga", "sub a by b;");
    refused(&mut s, "font.features.applyFix", json!({"error": 0}), "changed since the last compile");
    refused(&mut s, "font.features.jumpToError", json!({"kind": "feature", "name": "liga"}), "changed since the last compile");

    // Automatic code is read-only.
    set_code(&mut s, "feature", "liga", "sub f i by F;\nsub a by b");
    s.edit("automatic", |font| {
        Arc::make_mut(&mut font.features).features.iter_mut().for_each(|e| e.automatic = true);
        Ok(())
    })
    .unwrap();
    let r = run(&mut s, "font.features.compile", json!({}));
    let id = first_error(&r)["id"].clone();
    refused(&mut s, "font.features.applyFix", json!({"error": id}), "generated automatically");
}

#[test]
fn jump_to_error_and_details_point_at_the_faulty_code() {
    let mut s = Session::with_document();
    run(&mut s, "font.features.addFeature", json!({"tag": "calt", "code": "sub a by b;"}));
    run(&mut s, "font.features.addFeature", json!({"tag": "liga", "code": "# ligatures\n  sub a by nope;"}));
    run(&mut s, "font.features.compile", json!({}));
    let jump = run(&mut s, "font.features.jumpToError", json!({"kind": "feature", "name": "liga"}));
    assert_eq!(jump["entry"], json!({"kind": "feature", "name": "liga", "index": 1}));
    assert_eq!((jump["line"].clone(), jump["column"].clone()), (json!(2), json!(12)));
    refused(&mut s, "font.features.jumpToError", json!({"kind": "feature", "name": "calt"}), "found nothing");

    let details = run(&mut s, "font.features.showErrorDetails", json!({"error": jump["id"]}));
    let text = details["details"].as_str().unwrap();
    assert!(text.starts_with("Error in feature `liga`, line 2, column 12: glyph not in font"), "{text}");
    assert!(text.ends_with("\n  sub a by nope;\n           ^^^^"), "{text}");
}

#[test]
fn evaluate_token_gives_numbers_and_glyph_lists() {
    let mut s = Session::with_document();
    run(&mut s, "font.numbers.add", json!({"name": "tracking", "values": {"m01": 10}}));
    let r = run(&mut s, "font.features.evaluateToken", json!({"token": "$tracking"}));
    assert_eq!(r, json!({"token": "$tracking", "master": "m01", "kind": "number", "value": 10}));
    assert_eq!(run(&mut s, "font.features.evaluateToken", json!({"token": "${tracking * 2 + 1}"}))["value"], 21);
    let r = run(&mut s, "font.features.evaluateToken", json!({"token": "$[name == \"b\" OR name == \"a\"]"}));
    assert_eq!(r["glyphs"], json!(["a", "b"]));
    refused(&mut s, "font.features.evaluateToken", json!({"token": "$nothing"}), "nothing");
    refused(&mut s, "font.features.evaluateToken", json!({"token": "$tracking", "master": "Bold"}), "no master `Bold`");
}

#[test]
fn complete_offers_glyphs_classes_lookups_and_numbers_by_position() {
    let mut s = Session::with_document();
    run(&mut s, "glyph.newGlyph", json!({"name": "f_i"}));
    run(&mut s, "font.features.addClass", json!({"name": "Upper", "code": "A B"}));
    run(&mut s, "font.features.addPrefix", json!({"code": "lookup Smallcaps { sub a by A; } Smallcaps;"}));
    run(&mut s, "font.features.addFeature", json!({"tag": "liga", "code": "sub f i by f"}));
    let labels = |s: &mut Session, code: &str| -> Vec<String> {
        let r = run(s, "font.features.complete", json!({"kind": "feature", "name": "liga", "position": code.len(), "code": code}));
        r["completions"].as_array().unwrap().iter().map(|c| c["label"].as_str().unwrap().to_string()).collect()
    };
    // The stored code when no text is given.
    let r = run(&mut s, "font.features.complete", json!({"kind": "feature", "name": "liga", "position": 12}));
    assert_eq!(r["completions"][0], json!({"label": "f", "kind": "glyph", "replace": [11, 12]}));
    assert!(labels(&mut s, "sub f i by f").contains(&"f_i".to_string()));
    assert_eq!(labels(&mut s, "sub @Up"), ["@Upper"]);
    assert_eq!(labels(&mut s, "lookup Sm"), ["Smallcaps"]);
    run(&mut s, "font.numbers.add", json!({"name": "tracking"}));
    assert_eq!(labels(&mut s, "pos a $tr"), ["$tracking"]);
    refused(&mut s, "font.features.complete", json!({"kind": "feature", "name": "liga", "position": 99}), "not a character boundary");
}

#[test]
fn preview_name_lists_a_glyph_or_the_members_of_a_class() {
    let mut s = Session::with_document();
    run(&mut s, "font.features.addClass", json!({"name": "Lower", "code": "a b"}));
    run(&mut s, "font.features.addClass", json!({"name": "Mixed", "code": "@Lower A missing @Lower"}));
    let r = run(&mut s, "font.features.previewName", json!({"name": "@Mixed"}));
    assert_eq!(r, json!({"kind": "class", "name": "Mixed", "glyphs": ["a", "b", "A"], "unknown": ["missing"]}));
    let r = run(&mut s, "font.features.previewName", json!({"name": "a"}));
    assert_eq!(r, json!({"kind": "glyph", "name": "a", "unicodes": ["0061"], "export": true}));
    refused(&mut s, "font.features.previewName", json!({"name": "nothing"}), "no glyph or class");
}
