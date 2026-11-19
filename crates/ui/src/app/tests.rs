use std::sync::Arc;

use craft::host::Host;
use craft::ui::shell::app::ShellApp;
use craft::ui::shell::embed::CloseDecision;
use fontcraft_engine::Session;
use serde_json::json;

use super::{APP, FontCraftApp, Msg};

fn app() -> FontCraftApp {
    FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()))
}

/// A posted width change runs once in `drain`, reaches the view model, and makes closing ask first.
#[test]
fn a_posted_command_runs_projects_and_guards_the_close() {
    let mut app = app();
    let glyph = app.models.glyph_metrics.glyph.clone().unwrap();
    let before = app.models.glyph_metrics.revision();
    assert!(matches!(app.close_requested(), CloseDecision::Close), "an unmodified font closes");

    app.sent.push(Msg::run("spacing.setWidth", json!({ "glyphs": [glyph], "value": 640 })));
    app.drain();
    assert_eq!(app.models.glyph_metrics.width, Some(640.0));
    assert_ne!(app.models.glyph_metrics.revision(), before);
    assert!(app.status.is_empty(), "{}", app.status);

    assert!(matches!(app.close_requested(), CloseDecision::Hold));
    assert!(app.closing.is_open(), "the question about unsaved changes opens");
}

/// A command the engine refuses leaves its message in the status line and nothing else.
#[test]
fn a_refused_command_reaches_the_status_line() {
    let mut app = app();
    app.sent.push(Msg::run("spacing.setWidth", json!({ "glyphs": ["no such glyph"], "value": 1 })));
    app.drain();
    assert!(app.status.starts_with("spacing.setWidth"), "{}", app.status);
    assert!(!app.has_unsaved_changes());
}
