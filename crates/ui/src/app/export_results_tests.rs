#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::PathBuf;

use fontcraft_engine::Session;
use serde_json::{Value, json};

use super::{Group, reveal_msg};
use crate::app::Msg;
use crate::testing::TestApp;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-ui-results-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn state(t: &mut TestApp) -> Value {
    t.ok("ui.inspect", json!({}))["exportResults"].clone()
}

/// A font whose `bad name` no export may write.
fn bad_name() -> Session {
    let mut s = Session::with_document();
    s.edit("Add", |font| Ok(font.add_glyph("bad name", vec![0xE000], 500.0)?)).unwrap();
    s
}

/// A person's export that is stopped lists why in the window, and Show opens the glyph in a new tab; a request that
/// names the command gets the error and no window; a clean export opens no window and says what it wrote.
#[test]
fn the_window_lists_what_stopped_an_export() {
    let dir = temp("errors");
    let mut t = TestApp::with(bad_name());
    t.error("engine.execute", json!({"command": "file.exportStatic", "params": {"destination": dir.join("a")}}));
    assert_eq!(state(&mut t)["open"], false, "a request opens nothing");

    t.app.sent.push(Msg::run("file.exportStatic", json!({"destination": dir.join("a")})));
    t.frames(3);
    let shown = state(&mut t);
    assert_eq!((&shown["open"], &shown["rows"][0]["group"]), (&json!(true), &json!("error")), "{shown}");
    assert_eq!((&shown["rows"][0]["subject"], &shown["rows"][0]["reveals"]), (&json!("bad name"), &json!("glyph")), "{shown}");
    assert!(!dir.join("a").exists());
    let text = t.app.view.export_results.text(|group| format!("{group:?}"));
    assert!(text.starts_with("Error\n  bad name: ") && text.contains("position 4"), "{text}");
    let tabs = t.app.view.tabs.tabs.len();
    t.app.sent.push(reveal_msg(0));
    t.frames(2);
    assert_eq!(t.app.view.tabs.tabs.len(), tabs + 1, "{}", t.app.status);

    t.app
        .session
        .edit("Leave out", |font| {
            let mut glyph = font.glyph_mut("bad name")?;
            glyph.export = false;
            Ok(())
        })
        .unwrap();
    t.app.sent.push(Msg::run("file.exportStatic", json!({"destination": dir.join("b"), "timestamp": 0})));
    t.frames(3);
    let shown = state(&mut t);
    assert_eq!(shown["open"], false, "{shown}");
    assert!(t.app.status.contains("wrote 1 file"), "{}", t.app.status);
    assert_eq!((shown["rows"].as_array().unwrap().len(), &shown["rows"][0]["reveals"]), (1, &json!("file")), "{shown}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Skipped fonts and warnings of a reply are listed before its files, and a warning that names a glyph shows it.
#[test]
fn a_reply_is_grouped() {
    let mut t = TestApp::new();
    let reply = json!({
        "fonts": [{"instance": "Bold", "warnings": ["glyph `A`: its overlaps were kept"], "files": [{"path": "/x/Bold.ttf"}]}],
        "skipped": [{"instance": "Medium", "reason": "it cannot be interpolated", "glyphs": [{"glyph": "A", "message": "differs"}]}],
        "warnings": ["glyph `A`: its overlaps were kept"],
    });
    let status = super::after_run(&mut t.app, "file.exportStatic", &reply).unwrap();
    assert!(status.ends_with("wrote 1 file, 1 skipped, 2 warnings"), "{status}");
    let groups: Vec<Group> = t.app.view.export_results.rows.iter().map(|r| r.group).collect();
    assert_eq!(groups, [Group::Skipped, Group::Skipped, Group::Warning, Group::Warning, Group::File]);
    t.frames(3);
    assert_eq!(state(&mut t)["rows"][2]["reveals"], "glyph");
    assert_eq!(super::after_run(&mut t.app, "glyph.newGlyph", &reply), None);
}
