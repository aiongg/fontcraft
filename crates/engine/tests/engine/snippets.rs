//! Feature code snippets: font.features.listSnippets, insertSnippet and showSnippetFolder, with the
//! user's snippet files in a folder of the test's own.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::Path;
use std::sync::Arc;

use fontcraft_engine::{EngineError, Session};
use fontcraft_testkit::temp_dir;
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

fn code(s: &mut Session, tag: &str) -> String {
    run(s, "font.features.get", json!({"kind": "feature", "name": tag}))["code"].as_str().unwrap().to_string()
}

/// A folder with the handbook's example file, its TOML form, a file that is not a property list
/// and a file of another name.
fn snippet_folder(tag: &str) -> std::path::PathBuf {
    let dir = temp_dir(tag);
    let write = |name: &str, text: &str| std::fs::write(dir.join(name), text).unwrap();
    write(
        "FeatureSnippetsDecomposition.plist",
        "(\n  {\n    title = \"Soft Dotted\";\n    code = \"lookupflag UseMarkFilteringSet @TopMark;\nsub @SoftDotted' @TopMark by @Dotless;\";\n  },\n  {\n    title = \"Decompose Precomposed\";\n    code = \"sub @Precomp' lookup decomp @Mark;\";\n  }\n)\n",
    );
    write("FeatureSnippetsMine.toml", "[[snippet]]\ntitle = \"Soft Dotted\"\ncode = \"sub i' @TopMark by idotless;\"\n");
    write("FeatureSnippetsBroken.plist", "this is not ( a property list");
    write("Readme.txt", "not a snippet file");
    dir
}

fn folder_str(dir: &Path) -> String {
    dir.to_string_lossy().into_owned()
}

#[test]
fn list_snippets_gives_the_built_in_list_then_one_group_per_user_file() {
    let dir = snippet_folder("snippets-list");
    let mut s = Session::with_document();
    let r = run(&mut s, "font.features.listSnippets", json!({"folder": folder_str(&dir)}));
    let builtin: Vec<&str> = r["builtIn"].as_array().unwrap().iter().map(|b| b["code"].as_str().unwrap()).collect();
    assert!(builtin.iter().any(|c| c.starts_with("# Automatic Code")), "{builtin:?}");

    let groups: Vec<(&str, usize)> =
        r["groups"].as_array().unwrap().iter().map(|g| (g["name"].as_str().unwrap(), g["snippets"].as_array().unwrap().len())).collect();
    assert_eq!(groups, [("Decomposition", 2), ("Mine", 1)]);
    assert_eq!(r["groups"][0]["snippets"][0]["title"], "Soft Dotted");
    assert_eq!(r["groups"][0]["snippets"][0]["code"], "lookupflag UseMarkFilteringSet @TopMark;\nsub @SoftDotted' @TopMark by @Dotless;");

    let warnings = r["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].as_str().unwrap().starts_with("FeatureSnippetsBroken.plist: "), "{warnings:?}");

    // A folder that does not exist has no user snippets and no warnings.
    let r = run(&mut s, "font.features.listSnippets", json!({"folder": folder_str(&dir.join("missing"))}));
    assert_eq!(r["groups"], json!([]));
    assert_eq!(r["warnings"], json!([]));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn insert_snippet_replaces_the_selection_in_one_undo_step() {
    let dir = snippet_folder("snippets-insert");
    let folder = folder_str(&dir);
    let mut s = Session::with_document();
    run(&mut s, "font.features.addFeature", json!({"tag": "kern", "code": "pos a b -20;\npos x y 15;"}));

    // At the start of the second line: the placeholder goes on a line of its own.
    let r = run(&mut s, "font.features.insertSnippet", json!({"kind": "feature", "name": "kern", "snippet": "Automatic Code", "position": 13}));
    assert_eq!(r, json!({"kind": "feature", "name": "kern", "index": 0, "caret": 13 + "# Automatic Code\n".len()}));
    assert_eq!(code(&mut s, "kern"), "pos a b -20;\n# Automatic Code\npos x y 15;");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(code(&mut s, "kern"), "pos a b -20;\npos x y 15;");

    // Over a selection, from a user group; positions count characters, not bytes.
    run(&mut s, "font.features.addFeature", json!({"tag": "ccmp", "code": "# é\nsub a by b;"}));
    let r = run(
        &mut s,
        "font.features.insertSnippet",
        json!({"kind": "feature", "name": "ccmp", "snippet": "Soft Dotted", "group": "Mine", "position": 4, "end": 15, "folder": folder}),
    );
    assert_eq!(r["caret"], 4 + "sub i' @TopMark by idotless;".chars().count());
    assert_eq!(code(&mut s, "ccmp"), "# é\nsub i' @TopMark by idotless;");

    // Without a group the first file's snippet of that title is used.
    run(&mut s, "font.features.addFeature", json!({"tag": "locl", "code": ""}));
    run(&mut s, "font.features.insertSnippet", json!({"kind": "feature", "name": "locl", "snippet": "Soft Dotted", "folder": folder}));
    assert!(code(&mut s, "locl").starts_with("lookupflag UseMarkFilteringSet"));

    // Refusals leave the font as it was.
    let id = "font.features.insertSnippet";
    refused(&mut s, id, json!({"kind": "feature", "name": "kern", "snippet": "Nothing", "folder": folder}), "no snippet is titled `Nothing`");
    refused(&mut s, id, json!({"kind": "feature", "name": "kern", "snippet": "Soft Dotted", "group": "Other", "folder": folder}), "group `Other`");
    refused(&mut s, id, json!({"kind": "feature", "name": "kern", "snippet": "Automatic Code", "position": 99}), "within the code");
    refused(&mut s, id, json!({"kind": "feature", "name": "kern", "snippet": "Automatic Code", "position": 5, "end": 2}), "before `position`");
    refused(&mut s, id, json!({"kind": "feature", "name": "liga", "snippet": "Automatic Code"}), "no feature `liga`");
    s.edit("automatic", |font| {
        Arc::make_mut(&mut font.features).features.iter_mut().filter(|e| e.name == "kern").for_each(|e| e.automatic = true);
        Ok(())
    })
    .unwrap();
    refused(&mut s, id, json!({"kind": "feature", "name": "kern", "snippet": "Automatic Code"}), "generated automatically");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn show_snippet_folder_creates_the_folder_once() {
    let dir = temp_dir("snippets-show").join("snippets");
    let mut s = Session::new();
    let r = run(&mut s, "font.features.showSnippetFolder", json!({"folder": folder_str(&dir)}));
    assert_eq!(r, json!({"path": folder_str(&dir), "created": true}));
    assert!(dir.is_dir());
    let r = run(&mut s, "font.features.showSnippetFolder", json!({"folder": folder_str(&dir)}));
    assert_eq!(r["created"], false);
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}

#[test]
fn without_a_session_folder_only_the_built_in_snippets_are_listed() {
    let mut s = Session::new();
    let r = run(&mut s, "font.features.listSnippets", json!({}));
    assert_eq!(r["folder"], Value::Null);
    assert_eq!(r["groups"], json!([]));
    assert!(!r["builtIn"].as_array().unwrap().is_empty());
    assert!(matches!(s.execute("font.features.showSnippetFolder", &json!({})), Err(EngineError::Unsupported(_))));

    let dir = snippet_folder("snippets-session");
    s.set_snippets_dir(Some(dir.clone()));
    let r = run(&mut s, "font.features.listSnippets", json!({}));
    assert_eq!(r["folder"], json!(folder_str(&dir)));
    assert!(!r["groups"].as_array().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}
