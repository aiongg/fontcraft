//! Disabling, copying and pasting feature entries (Font Info › Features sidebar):
//! font.features.setDisabled, copyEntries and pasteEntries.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

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

fn get(s: &mut Session, kind: &str, name: &str) -> Value {
    run(s, "font.features.get", json!({"kind": kind, "name": name}))
}

/// The tags of the features the last compile put in the font.
fn compiled_tags(s: &mut Session) -> Vec<String> {
    let r = run(s, "font.features.compile", json!({}));
    assert_eq!(r["ok"], true, "{r}");
    r["features"].as_array().unwrap().iter().map(|f| f["tag"].as_str().unwrap().to_string()).collect()
}

/// A source font with one entry of each kind and every flag the clipboard text carries.
fn source() -> Session {
    let mut s = Session::with_document();
    run(&mut s, "font.features.addClass", json!({"name": "Upper", "code": "A B C"}));
    run(&mut s, "font.features.setNotes", json!({"kind": "class", "name": "Upper", "notes": "capitals\nused by case"}));
    run(&mut s, "font.features.addPrefix", json!({"name": "Lookups", "code": "lookup L {\n  sub a by b;\n} L;"}));
    run(&mut s, "font.features.addFeature", json!({"tag": "case", "code": "sub @Upper by @Upper;"}));
    run(&mut s, "font.features.addFeature", json!({"tag": "ss01", "code": "sub a by b;\n\nsub c by d;"}));
    run(&mut s, "font.features.setLabel", json!({"tag": "ss01", "name": "Simple a"}));
    run(&mut s, "font.features.setNotes", json!({"kind": "feature", "name": "ss01", "notes": "for the logo"}));
    run(&mut s, "font.features.setDisabled", json!({"entries": [{"kind": "feature", "name": "ss01"}], "disabled": true}));
    // An automatic flag as a file from the reference app holds it.
    s.edit("automatic", |font| {
        let f = Arc::make_mut(&mut font.features);
        f.features.iter_mut().filter(|e| e.name == "case").for_each(|e| e.automatic = true);
        Ok(())
    })
    .unwrap();
    s
}

#[test]
fn copied_entries_paste_into_another_font_with_code_flags_and_notes() {
    let mut from = source();
    let all = json!({"entries": [
        {"kind": "feature", "name": "ss01"}, {"kind": "class", "name": "Upper"},
        {"kind": "feature", "name": "case"}, {"kind": "prefix", "name": "Lookups"},
    ]});
    let font = Arc::clone(&from.doc().unwrap().font);
    let undo = from.doc().unwrap().history.undo.len();
    let copied = run(&mut from, "font.features.copyEntries", all);
    assert!(Arc::ptr_eq(&font, &from.doc().unwrap().font) && from.doc().unwrap().history.undo.len() == undo, "Copy changes nothing");
    assert_eq!(copied["count"], 4);
    let text = copied["text"].as_str().unwrap().to_string();
    // Classes, then prefixes, then features; the disabled feature's code is commented out.
    let at = |needle: &str| text.find(needle).unwrap_or_else(|| panic!("no `{needle}` in {text}"));
    assert!(at("@Upper = [") < at("lookup L {") && at("lookup L {") < at("feature case {") && at("feature case {") < at("feature ss01 {"));
    assert!(text.contains("# sub a by b;\n#\n# sub c by d;"), "{text}");

    // Another font that already has a class Upper: the pasted class becomes Upper2 and the pasted
    // feature's reference follows it.
    let mut to = Session::with_document();
    run(&mut to, "font.features.addClass", json!({"name": "Upper", "code": "X"}));
    let r = run(&mut to, "font.features.pasteEntries", json!({"text": text}));
    assert_eq!(
        r["entries"],
        json!([
            {"kind": "prefix", "name": "Lookups", "index": 0},
            {"kind": "class", "name": "Upper2", "index": 1},
            {"kind": "feature", "name": "case", "index": 0},
            {"kind": "feature", "name": "ss01", "index": 1},
        ])
    );
    assert_eq!(r["renamed"], json!([{"kind": "class", "from": "Upper", "to": "Upper2"}]));

    let class = get(&mut to, "class", "Upper2");
    assert_eq!((class["code"].clone(), class["notes"].clone()), (json!("A B C"), json!("capitals\nused by case")));
    let prefix = get(&mut to, "prefix", "Lookups");
    assert_eq!(prefix["code"], "lookup L {\n  sub a by b;\n} L;");
    let case = get(&mut to, "feature", "case");
    assert_eq!(
        (case["code"].clone(), case["automatic"].clone(), case["disabled"].clone()),
        (json!("sub @Upper2 by @Upper2;"), json!(true), json!(false))
    );
    let ss01 = get(&mut to, "feature", "ss01");
    assert_eq!(ss01["code"], "sub a by b;\n\nsub c by d;");
    assert_eq!((ss01["disabled"].clone(), ss01["automatic"].clone(), ss01["notes"].clone()), (json!(true), json!(false), json!("for the logo")));
    assert_eq!(ss01["labels"], json!([{"language": "dflt", "name": "Simple a"}]));

    // Line breaks a clipboard turned into CRLF still read as the marked form.
    let mut crlf = Session::with_document();
    run(&mut crlf, "font.features.pasteEntries", json!({"text": text.replace('\n', "\r\n")}));
    assert_eq!(get(&mut crlf, "feature", "ss01")["notes"], "for the logo");

    // Plain feature code from elsewhere pastes as File › Import › Features reads it.
    let mut plain = Session::with_document();
    let r = run(&mut plain, "font.features.pasteEntries", json!({"text": "@Lower = [a b];\nfeature smcp {\n  sub a by b;\n} smcp;"}));
    assert_eq!(r["entries"], json!([{"kind": "class", "name": "Lower", "index": 0}, {"kind": "feature", "name": "smcp", "index": 0}]));
    assert_eq!(get(&mut plain, "class", "Lower")["code"], "a b");
}

#[test]
fn a_disabled_feature_leaves_gsub_and_comes_back_when_enabled() {
    let mut s = Session::with_document();
    run(&mut s, "font.features.addFeature", json!({"tag": "liga", "code": "sub a by b;"}));
    run(&mut s, "font.features.addFeature", json!({"tag": "smcp", "code": "sub c by d;"}));
    assert_eq!(compiled_tags(&mut s), ["liga", "smcp"]);

    let smcp = json!([{"kind": "feature", "name": "smcp"}]);
    let r = run(&mut s, "font.features.setDisabled", json!({"entries": smcp, "disabled": true}));
    assert_eq!(r, json!({"entries": [{"kind": "feature", "name": "smcp", "index": 1, "disabled": true}], "changed": 1}));
    assert_eq!(compiled_tags(&mut s), ["liga"]);
    let listed = run(&mut s, "font.features.list", json!({"kind": "feature"}));
    assert_eq!((listed["entries"][0]["disabled"].clone(), listed["entries"][1]["disabled"].clone()), (json!(false), json!(true)), "still listed");

    run(&mut s, "font.features.setDisabled", json!({"entries": smcp, "disabled": false}));
    assert_eq!(compiled_tags(&mut s), ["liga", "smcp"]);
}

#[test]
fn disable_and_paste_are_one_undo_step_each() {
    let mut s = source();
    let text = run(
        &mut s,
        "font.features.copyEntries",
        json!({"entries": [{"kind": "class", "name": "Upper"}, {"kind": "feature", "name": "case"}]}),
    )["text"]
        .clone();
    let steps = [
        (
            "font.features.setDisabled",
            json!({"entries": [{"kind": "class", "name": "Upper"}, {"kind": "prefix", "name": "Lookups"}], "disabled": true}),
        ),
        ("font.features.setDisabled", json!({"entries": [{"kind": "feature", "name": "ss01"}], "disabled": false})),
        ("font.features.pasteEntries", json!({"text": text})),
    ];
    for (id, p) in steps {
        let before = Arc::clone(&s.doc().unwrap().font);
        run(&mut s, id, p.clone());
        assert!(!Arc::ptr_eq(&before, &s.doc().unwrap().font), "{id} {p} changed nothing");
        run(&mut s, "edit.undo", json!({}));
        assert!(Arc::ptr_eq(&before, &s.doc().unwrap().font), "{id}: undo did not give back the very font");
        run(&mut s, "edit.redo", json!({}));
    }

    // Entries already in the asked state make no undo step.
    let undo = s.doc().unwrap().history.undo.len();
    let r = run(&mut s, "font.features.setDisabled", json!({"entries": [{"kind": "class", "name": "Upper"}], "disabled": true}));
    assert_eq!(r["changed"], 0);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
}

#[test]
fn disable_copy_and_paste_refuse_bad_requests_and_change_nothing() {
    let mut s = source();
    refused(&mut s, "font.features.setDisabled", json!({"entries": [{"kind": "class", "name": "Upper"}]}), "`disabled` is required");
    refused(&mut s, "font.features.setDisabled", json!({"entries": [{"kind": "feature", "name": "kern"}], "disabled": true}), "no feature `kern`");
    refused(&mut s, "font.features.setDisabled", json!({"entries": [], "disabled": true}), "`entries` is empty");
    refused(&mut s, "font.features.copyEntries", json!({"entries": []}), "`entries` is empty");
    refused(&mut s, "font.features.pasteEntries", json!({"text": " \n"}), "no feature code");
    // The marked form with a name no class may have.
    let bad = "# entry: {\"kind\":\"class\",\"name\":\"9lives\"}\n@9lives = [\na\n];\n";
    refused(&mut s, "font.features.pasteEntries", json!({"text": bad}), "class name");
    refused(&mut s, "font.features.pasteEntries", json!({"text": "feature liga {\n  sub a by b;\n"}), "never closed");
}
