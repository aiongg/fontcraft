//! The author documentation against the code it describes (`extensions.authoringDocs`,
//! `sdk.templates`, `script.importExistingPython`): every command the porting guide names is
//! registered, every macro example in the docs is a valid macro of registered commands, and each
//! module template's manifest loads with the kind of its folder.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use fontcraft_engine::find_command;
use fontcraft_engine::macros::Macro;
use fontcraft_plugins::Manifest;

fn repo_file(path: &str) -> String {
    let full = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").join(path);
    std::fs::read_to_string(&full).unwrap_or_else(|e| panic!("{}: {e}", full.display()))
}

/// Whether `s` has the shape of a command id: lower-case words joined by dots.
fn looks_like_command_id(s: &str) -> bool {
    s.starts_with(|c: char| c.is_ascii_lowercase())
        && s.contains('.')
        && s.split('.').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric()))
}

#[test]
fn porting_guide_names_only_registered_commands() {
    let text = repo_file("docs/porting.md");
    let section = text.split("\n## Common tasks").nth(1).expect("the Common tasks section").split("\n## ").next().unwrap();
    let mut ids = Vec::new();
    for row in section.lines().filter(|l| l.starts_with('|') && !l.starts_with("|---")) {
        let last = row.trim().trim_end_matches('|').rsplit('|').next().unwrap();
        ids.extend(last.split('`').skip(1).step_by(2).filter(|s| looks_like_command_id(s)).map(str::to_string));
    }
    assert!(ids.len() > 50, "the tables were read: {ids:?}");
    let missing: Vec<&String> = ids.iter().filter(|id| find_command(id).is_none()).collect();
    assert!(missing.is_empty(), "the porting guide names commands that are not registered: {missing:?}");
}

#[test]
fn macro_examples_in_the_docs_are_valid_macros() {
    let mut examples = 0;
    for doc in ["docs/macros.md", "docs/porting.md"] {
        let text = repo_file(doc);
        for block in text.split("```json\n").skip(1).filter_map(|b| b.split("\n```").next()) {
            if !block.contains("\"fontcraft.macro\"") {
                continue;
            }
            let parsed = Macro::parse(block.as_bytes()).unwrap_or_else(|e| panic!("{doc}: {e}\n{block}"));
            parsed.check().unwrap_or_else(|e| panic!("{doc}: {e}\n{block}"));
            examples += 1;
        }
    }
    assert_eq!(examples, 4, "the three examples of macros.md and the one of porting.md");
}

#[test]
fn module_templates_have_valid_manifests() {
    for kind in ["filter", "reporter", "check"] {
        let text = repo_file(&format!("examples/module-template/{kind}/manifest.json"));
        let manifest = Manifest::parse(text.as_bytes()).unwrap_or_else(|e| panic!("{kind} template: {e}"));
        assert_eq!(manifest.kind.name(), kind);
        assert!(manifest.validate_params(&serde_json::Value::Null).is_ok(), "{kind}: the defaults are valid parameters");
    }
}
