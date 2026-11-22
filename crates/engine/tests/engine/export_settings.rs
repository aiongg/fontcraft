//! The Export dialog's kept settings (`file.exportSettings`) and Export All reusing them.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};

use fontcraft_engine::Session;
use fontcraft_engine::settings::{Reserved, UserSettings};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-export-settings-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn names(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> =
        std::fs::read_dir(dir).map(|e| e.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    out.sort();
    out
}

fn font_with_outline() -> Session {
    let mut s = Session::with_document();
    let nodes = json!([{"x": 60, "y": 0}, {"x": 560, "y": 0}, {"x": 560, "y": 700}, {"x": 60, "y": 700}]);
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": nodes}));
    s
}

#[test]
fn export_all_without_parameters_repeats_the_last_export() {
    let dir = temp("repeat");
    let out = dir.join("out");
    let mut s = font_with_outline();
    run(
        &mut s,
        "file.exportStatic",
        json!({"outlines": "cff", "containers": ["plain", "woff"], "removeOverlap": false, "destination": out, "timestamp": 0}),
    );
    let first = names(&out);
    assert_eq!(first.len(), 2, "{first:?}");
    let kept = run(&mut s, "file.exportSettings", json!({}));
    assert_eq!(kept["otf"]["outlines"], "cff");
    assert_eq!(kept["otf"]["containers"], json!(["plain", "woff"]));
    assert_eq!(kept["otf"]["removeOverlap"], false);
    assert_eq!(kept["otf"]["useDestination"], true);
    assert!(kept["otf"]["destination"].as_str().unwrap().ends_with("out"));
    assert_eq!(kept["variable"], Value::Null);

    std::fs::remove_dir_all(&out).unwrap();
    let all = run(&mut s, "file.exportAll", json!({"timestamp": 0}));
    assert_eq!(names(&out), first, "the same files as the export before");
    assert_eq!(all["written"], 2);
}

#[test]
fn a_failed_export_keeps_the_settings_before() {
    let dir = temp("failed");
    let mut s = font_with_outline();
    run(&mut s, "file.exportStatic", json!({"outlines": "truetype", "destination": dir.join("a"), "timestamp": 0}));
    let before = run(&mut s, "file.exportSettings", json!({"tab": "otf"}));
    let e = s.execute("file.exportStatic", &json!({"outlines": "cff", "instances": ["No Such Instance"], "destination": dir.join("b")}));
    assert!(e.is_err());
    assert_eq!(run(&mut s, "file.exportSettings", json!({"tab": "otf"})), before);
}

#[test]
fn export_all_needs_a_ticked_folder() {
    let dir = temp("unticked");
    let mut s = font_with_outline();
    // Never exported: no folder anywhere, which headless is an error naming the document.
    let e = s.execute("file.exportAll", &json!({})).unwrap_err().to_string();
    assert!(e.contains("no export folder"), "{e}");
    // A folder kept with its box unticked does not count.
    run(&mut s, "file.exportStatic", json!({"destination": dir.join("a"), "useDestination": false, "timestamp": 0}));
    let e = s.execute("file.exportAll", &json!({})).unwrap_err().to_string();
    assert!(e.contains("no export folder"), "{e}");
    // A destination given to Export All wins over the kept settings.
    run(&mut s, "file.exportAll", json!({"destination": dir.join("b"), "timestamp": 0}));
    assert_eq!(names(&dir.join("b")).len(), 1);
}

#[test]
fn saved_documents_keep_their_settings_in_the_settings_file() {
    let dir = temp("saved");
    let mut s = font_with_outline();
    run(&mut s, "file.saveAs", json!({"path": dir.join("Kept.fontcraft")}));
    run(&mut s, "file.exportStatic", json!({"containers": ["woff2"], "destination": dir.join("out"), "timestamp": 0}));
    let stored: Value = s.settings().reserved(Reserved::ExportSettings).unwrap().unwrap();
    assert!(stored[0]["path"].as_str().unwrap().ends_with("Kept.fontcraft"), "{stored}");
    assert_eq!(stored[0]["otf"]["containers"], json!(["woff2"]));

    // Through the settings file and back, as the app saves and loads it.
    let (back, problems) = UserSettings::from_json(&s.settings().to_json()).unwrap();
    assert!(problems.is_empty(), "{problems:?}");
    let mut fresh = Session::with_document();
    *fresh.settings_mut() = back;
    run(&mut fresh, "file.open", json!({"path": dir.join("Kept.fontcraft")}));
    let kept = run(&mut fresh, "file.exportSettings", json!({"tab": "otf"}));
    assert_eq!(kept["settings"]["containers"], json!(["woff2"]), "{kept}");

    // A hand-edited settings file: an entry that does not check out is reported and not used.
    let hostile = json!([{"path": dir.join("Kept.fontcraft"), "otf": {"outlines": "cff", "script": "x"}}]);
    fresh.settings_mut().set_reserved(Reserved::ExportSettings, &hostile).unwrap();
    let kept = run(&mut fresh, "file.exportSettings", json!({}));
    assert_eq!(kept["otf"], Value::Null);
    assert_eq!(kept["problems"].as_array().unwrap().len(), 1, "{kept}");
}
