//! The Macro Panel's editor: Save writes the engine's macro file and Open reads it back, Run Selection runs only its
//! lines as one undo step, and Record adds the recorded steps.

use std::sync::Arc;

use craft::host::Host;
use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::Session;
use fontcraft_engine::macros::Macro;
use serde_json::{Value, json};

use crate::app::{APP, FontCraftApp, Msg};

fn app() -> FontCraftApp {
    FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()))
}

fn shell(app: &mut FontCraftApp, id: &str, params: Value) -> Result<Value, String> {
    app.run_now(Msg::Shell(CommandRef::with_params(id, params)), false)
}

fn ok(app: &mut FontCraftApp, id: &str, params: Value) {
    shell(app, id, params).unwrap_or_else(|e| panic!("{id}: {e}"));
}

fn has(app: &FontCraftApp, glyph: &str) -> bool {
    app.session.font().unwrap().glyph(glyph).is_some()
}

const THREE: &str = "glyph.newGlyph {\"name\":\"x1\"}\nglyph.newGlyph {\"name\":\"x2\"}\nglyph.newGlyph {\"name\":\"x3\"}";

/// Save writes the editor's lines as the steps of a `.fcmacro` file named after the file; Open puts them back, one
/// per line; a line that is not a step refuses the save and writes nothing.
#[test]
fn save_writes_the_editor_as_a_macro_file_and_open_reads_it_back() {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("fontcraft-ui-macro-panel-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("three.fcmacro").to_str().unwrap().to_string();
    let mut app = app();
    ok(&mut app, "macroPanel.setText", json!({ "text": THREE }));
    ok(&mut app, "macroPanel.save", json!({ "path": path }));
    let saved = Macro::parse(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved.title, "three");
    let steps: Vec<(String, Value)> = saved.steps.iter().map(|s| (s.command.clone(), s.params.clone())).collect();
    assert_eq!(steps, ["x1", "x2", "x3"].map(|n| ("glyph.newGlyph".to_string(), json!({ "name": n }))).to_vec(), "one step per line, in order");
    assert!(!has(&app, "x1"), "saving runs nothing");

    ok(&mut app, "macroPanel.setText", json!({ "text": "" }));
    ok(&mut app, "macroPanel.open", json!({ "path": path }));
    assert_eq!(app.view.commands.console.editor, THREE);
    assert_eq!(app.view.commands.console.editor_macro.title, "three");

    let bad = dir.join("bad.fcmacro").to_str().unwrap().to_string();
    ok(&mut app, "macroPanel.setText", json!({ "text": "glyph.newGlyph {not json" }));
    let e = shell(&mut app, "macroPanel.save", json!({ "path": bad })).unwrap_err();
    assert!(e.contains("line 1"), "{e}");
    assert!(!std::path::Path::new(&bad).exists());
    let _ = std::fs::remove_dir_all(dir);
}

/// Run Selection runs only the lines given, as one undo step; lines outside the text are refused and change nothing.
#[test]
fn run_selection_runs_its_lines_as_one_undo_step() {
    let mut app = app();
    ok(&mut app, "macroPanel.setText", json!({ "text": THREE }));
    let e = shell(&mut app, "macroPanel.runSelection", json!({ "first": 3, "last": 4 })).unwrap_err();
    assert!(e.contains("1 to 3"), "{e}");
    ok(&mut app, "macroPanel.runSelection", json!({ "first": 2, "last": 3 }));
    assert_eq!((has(&app, "x1"), has(&app, "x2"), has(&app, "x3")), (false, true, true));
    app.session.execute("edit.undo", &json!({})).unwrap();
    assert!(!has(&app, "x2") && !has(&app, "x3"), "the lines are one undo step");
    ok(&mut app, "macroPanel.runAll", json!({}));
    assert!(has(&app, "x1") && has(&app, "x2") && has(&app, "x3"));
}

/// Recording into the editor adds each font-changing command that ran, one line each, after the text there.
#[test]
fn record_adds_the_recorded_steps_to_the_editor() {
    let mut app = app();
    ok(&mut app, "macroPanel.setText", json!({ "text": "glyph.newGlyph {\"name\":\"x1\"}" }));
    ok(&mut app, "macroPanel.record", json!({ "on": true }));
    assert!(app.session.extensions().recording.is_some());
    app.session.execute("glyph.newGlyph", &json!({ "name": "x9" })).unwrap();
    ok(&mut app, "macroPanel.record", json!({}));
    assert!(app.session.extensions().recording.is_none());
    assert_eq!(app.view.commands.console.editor, "glyph.newGlyph {\"name\":\"x1\"}\nglyph.newGlyph {\"name\":\"x9\"}");
}
