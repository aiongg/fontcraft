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

#[test]
fn opening_and_saving_record_recent_documents() {
    use fontcraft_engine::settings::MAX_RECENT_DOCUMENTS;
    let dir = std::env::temp_dir().join(format!("fontcraft-engine-recent-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = |name: &str| dir.join(name).to_string_lossy().into_owned();
    let recent = |s: &Session| s.settings().recent_documents();

    let mut s = Session::with_document();
    run(&mut s, "file.saveAs", json!({"path": path("a.fontcraft")}));
    assert_eq!(recent(&s), [path("a.fontcraft")]);
    run(&mut s, "file.saveAs", json!({"path": path("b.fontcraft")}));
    run(&mut s, "file.save", json!({"path": path("c.fontcraft")}));
    assert_eq!(recent(&s), [path("c.fontcraft"), path("b.fontcraft"), path("a.fontcraft")]);
    // Opening puts a document first, once.
    run(&mut s, "file.open", json!({"path": path("a.fontcraft")}));
    assert_eq!(recent(&s), [path("a.fontcraft"), path("c.fontcraft"), path("b.fontcraft")]);
    run(&mut s, "file.close", json!({}));
    run(&mut s, "file.open", json!({"path": path("b.fontcraft")}));
    assert_eq!(recent(&s)[0], path("b.fontcraft"));
    // A failed open records nothing; a file that is gone is left out when the list is read.
    assert!(s.execute("file.open", &json!({"path": path("missing.fontcraft")})).is_err());
    std::fs::remove_file(path("c.fontcraft")).unwrap();
    assert_eq!(recent(&s), [path("b.fontcraft"), path("a.fontcraft")]);
    assert_eq!(s.settings().stored_recent_documents().len(), 3, "the stored list keeps it");

    // At most ten, newest first.
    for i in 0..12 {
        run(&mut s, "file.saveAs", json!({"path": path(&format!("n{i}.fontcraft"))}));
    }
    let list = recent(&s);
    assert_eq!(list.len(), MAX_RECENT_DOCUMENTS);
    assert_eq!(list[0], path("n11.fontcraft"));
    let _ = std::fs::remove_dir_all(&dir);
}
