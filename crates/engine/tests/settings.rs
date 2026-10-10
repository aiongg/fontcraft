//! The user settings commands.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use fontcraft_engine::settings::Reserved;
use fontcraft_engine::{EngineError, Session};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

#[test]
fn settings_commands() {
    let mut s = Session::new();
    assert_eq!(run(&mut s, "settings.get", json!({"id": "appearance.displayUnicodeValue"}))["value"], json!(false));
    run(&mut s, "settings.set", json!({"id": "appearance.displayUnicodeValue", "value": true}));
    assert_eq!(run(&mut s, "settings.get", json!({}))["settings"]["appearance.displayUnicodeValue"], json!(true));
    run(&mut s, "settings.set", json!({"id": "appearance.displayUnicodeValue", "value": null}));
    assert_eq!(s.settings().get_bool("appearance.displayUnicodeValue"), Some(false));

    for (params, needle) in [
        (json!({"id": "no.such", "value": 1}), "no.such"),
        (json!({"id": "start.showAtLaunch", "value": "no"}), "start.showAtLaunch"),
        (json!({"id": "fontView.filters", "value": []}), "fontView.filters"),
        (json!({"id": "start.showAtLaunch"}), "value"),
    ] {
        match s.execute("settings.set", &params) {
            Err(EngineError::BadParams { message, .. }) => assert!(message.contains(needle), "{message}"),
            other => panic!("{params}: {other:?}"),
        }
    }
    // A command that fails after changing the settings leaves them as they were.
    let steps = json!({"steps": [
        {"command": "settings.set", "params": {"id": "start.showAtLaunch", "value": false}},
        {"command": "settings.set", "params": {"id": "no.such", "value": false}},
    ]});
    let mut s = Session::with_document();
    let e = s.execute("command.batch", &steps).unwrap_err().to_string();
    assert!(e.contains("no.such"), "{e}");
    assert_eq!(s.settings().get_bool("start.showAtLaunch"), Some(true));
    // Reserved ids are read through settings.get and written by their own commands.
    s.settings_mut().set_reserved(Reserved::RecentDocuments, &["/fonts/a.glyphs"]).unwrap();
    assert_eq!(run(&mut s, "settings.get", json!({"id": "file.recentDocuments"}))["value"], json!(["/fonts/a.glyphs"]));
}
