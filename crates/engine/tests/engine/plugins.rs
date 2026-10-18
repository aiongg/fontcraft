//! Module commands: installing from a file, from bytes and from an index entry, the Modules
//! folder and its manual copies, safe mode, running a filter module, and the info queries.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use fontcraft_engine::Session;
use fontcraft_engine::extension_index::{Entry, EntryKind, Index};
use fontcraft_engine::extensions::{self, LoadedIndex};
use fontcraft_plugins::{fixtures, sha256, wat};
use fontcraft_testkit::{rect, temp_dir};
use serde_json::{Value, json};

const ID: &str = "org.test.fixed";

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn err(s: &mut Session, id: &str, p: Value) -> String {
    match s.execute(id, &p) {
        Ok(v) => panic!("{id} should fail, gave {v}"),
        Err(e) => e.to_string(),
    }
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// The shapes of a 100 × 100 square at the origin, as a filter returns them.
fn square_shapes() -> Value {
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(0.0, 0.0, 100.0, 100.0)}));
    serde_json::to_value(&s.font().unwrap().glyph("A").unwrap().layers[0].shapes).unwrap()
}

/// A filter module `id` (version `version`) that turns every layer into the square and logs a
/// line.
fn square_filter(id: &str, version: &str) -> Vec<u8> {
    let manifest = format!(r#"{{"id":"{id}","name":"Square","version":"{version}","kind":"filter"}}"#);
    fixtures::returning(&manifest, &json!({"shapes": square_shapes(), "log": ["squared"]}).to_string())
}

/// A filter module that always returns `output`.
fn filter_returning(id: &str, output: &str) -> Vec<u8> {
    fixtures::returning(&wat::manifest(id, "filter", "{}"), output)
}

/// A session with a document and the extension folders in a fresh data folder.
fn with_folders(tag: &str, safe_mode: bool) -> (Session, PathBuf) {
    let data = temp_dir(tag).join(format!("data-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(&data).unwrap();
    let mut s = Session::with_document();
    assert!(extensions::open_folders(s.extensions_mut(), Some(&data), safe_mode).is_empty());
    (s, data)
}

fn modules(data: &Path) -> PathBuf {
    data.join(extensions::MODULES_FOLDER)
}

fn undo_labels(s: &Session) -> Vec<String> {
    s.doc().unwrap().history.undo.iter().map(|h| h.label.to_string()).collect()
}

#[test]
fn plugin_install_from_file_and_from_base64_replaces_and_plugin_manager_remove_moves_the_file_out() {
    let (mut s, data) = with_folders("plugins-install", false);
    let source = data.join("square.wasm");
    std::fs::write(&source, square_filter(ID, "1")).unwrap();
    let out = run(&mut s, "plugin.installFromFile", json!({"path": source}));
    assert_eq!((out["kind"].as_str(), out["file"].as_str(), out["enabled"].as_bool()), (Some("filter"), Some("square.wasm"), Some(true)));
    assert!(modules(&data).join("square.wasm").is_file(), "copied into the Modules folder");
    assert_eq!(extensions::read_switches(&modules(&data)).unwrap().get("square.wasm"), Some(&true));

    // Another build of the same id needs replace: true, and keeps the file name.
    let v2 = square_filter(ID, "2");
    let e = err(&mut s, "plugin.installFromFile", json!({"dataBase64": b64(&v2)}));
    assert!(e.contains("installed already") && e.contains("replace"), "{e}");
    let out = run(&mut s, "plugin.installFromFile", json!({"dataBase64": b64(&v2), "replace": true}));
    assert_eq!((out["replaced"].as_bool(), out["file"].as_str(), out["version"].as_str()), (Some(true), Some("square.wasm"), Some("2")));
    assert_eq!(std::fs::read(modules(&data).join("square.wasm")).unwrap(), v2);

    let info = run(&mut s, "plugin.info", json!({"id": ID}));
    assert_eq!((info["version"].as_str(), info["enabled"].as_bool(), info["runs"].as_bool()), (Some("2"), Some(true), Some(true)));
    assert_eq!(info["sha256"].as_str(), Some(sha256::hex(&v2).as_str()));
    let listed = run(&mut s, "pluginManager.list", json!({"state": "installed", "type": "filter", "query": "SQUARE"}));
    assert_eq!(listed["count"], 1);
    assert_eq!(run(&mut s, "pluginManager.list", json!({"state": "notInstalled"}))["count"], 0);

    let out = run(&mut s, "pluginManager.remove", json!({"id": ID}));
    let moved = PathBuf::from(out["movedTo"].as_str().unwrap());
    assert!(moved.starts_with(data.join(extensions::REMOVED_FOLDER)) && moved.is_file(), "{moved:?}");
    assert!(!modules(&data).join("square.wasm").exists());
    assert!(s.extensions().registry.is_empty());
    assert!(err(&mut s, "plugin.info", json!({"id": ID})).contains("no module"));
}

#[test]
fn plugin_no_manual_move_a_copied_module_is_new_and_off_until_install_from_file_turns_it_on() {
    let (mut s, data) = with_folders("plugins-manual", false);
    std::fs::create_dir_all(modules(&data)).unwrap();
    std::fs::write(modules(&data).join("copy.wasm"), square_filter(ID, "1")).unwrap();
    std::fs::write(modules(&data).join("broken.wasm"), b"\0asm junk").unwrap();
    let problems = extensions::poll_folders(s.extensions_mut());
    assert!(problems.iter().any(|p| p.contains("new module") && p.contains(ID)), "{problems:?}");
    assert!(problems.iter().any(|p| p.contains("broken.wasm")), "{problems:?}");
    let info = run(&mut s, "plugin.info", json!({"id": ID}));
    assert_eq!((info["enabled"].as_bool(), info["new"].as_bool(), info["runs"].as_bool()), (Some(false), Some(true), Some(false)));
    let e = err(&mut s, "plugin.runFilter", json!({"id": ID, "glyphs": ["A"]}));
    assert!(e.contains("turned off"), "{e}");
    assert_eq!(run(&mut s, "app.info", json!({}))["failedModules"][0]["file"], "broken.wasm");

    let out = run(&mut s, "plugin.installFromFile", json!({"name": "copy.wasm"}));
    assert_eq!(out["enabled"], true);
    assert_eq!(extensions::read_switches(&modules(&data)).unwrap().get("copy.wasm"), Some(&true));
    run(&mut s, "plugin.runFilter", json!({"id": ID, "glyphs": ["A"]}));

    // Deleting the file uninstalls the module at the next poll (plugin.uninstall).
    std::fs::remove_file(modules(&data).join("copy.wasm")).unwrap();
    extensions::poll_folders(s.extensions_mut());
    assert!(s.extensions().registry.get(ID).is_none());
}

#[test]
fn plugin_run_filter_on_two_glyphs_is_one_undo_step() {
    let mut s = Session::with_document();
    run(&mut s, "plugin.installFromFile", json!({"dataBase64": b64(&square_filter(ID, "1"))}));
    let start = s.font().unwrap().clone();
    let undo_before = undo_labels(&s).len();
    let out = run(&mut s, "plugin.runFilter", json!({"id": ID, "glyphs": ["A", "O"]}));
    assert_eq!(out["glyphs"], json!(["A", "O"]));
    assert_eq!(out["log"], json!(["squared", "squared"]));
    let square = square_shapes();
    for name in ["A", "O"] {
        let layer = &s.font().unwrap().glyph(name).unwrap().layers[0];
        assert_eq!(serde_json::to_value(&layer.shapes).unwrap(), square, "{name}");
    }
    assert_eq!(undo_labels(&s).len(), undo_before + 1);
    assert_eq!(undo_labels(&s).last().map(String::as_str), Some("Square"));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.font().unwrap(), &start, "one undo takes back every layer");
    assert!(s.extensions().log.len() >= 2);
}

#[test]
fn plugin_locked_paths_and_glyphs_a_locked_glyph_or_a_failing_module_changes_nothing() {
    let mut s = Session::with_document();
    run(&mut s, "plugin.installFromFile", json!({"dataBase64": b64(&square_filter(ID, "1"))}));
    run(&mut s, "glyph.toggleLocked", json!({"glyphs": ["O"], "locked": true}));
    let start = s.font().unwrap().clone();
    let e = err(&mut s, "plugin.runFilter", json!({"id": ID, "glyphs": ["A", "O"]}));
    assert!(e.contains("locked"), "{e}");
    assert_eq!(s.font().unwrap(), &start);

    let failing = filter_returning("org.test.fails", r#"{"error":"nope"}"#);
    run(&mut s, "plugin.installFromFile", json!({"dataBase64": b64(&failing)}));
    let e = err(&mut s, "plugin.runFilter", json!({"id": "org.test.fails", "glyphs": ["A"]}));
    assert!(e.contains("nope"), "{e}");
    assert!(run(&mut s, "plugin.info", json!({"id": "org.test.fails"}))["lastError"].as_str().unwrap().contains("nope"));

    let mut far = square_shapes();
    far[0]["path"]["nodes"][1]["x"] = json!(1e9);
    let wild = filter_returning("org.test.wild", &json!({"shapes": far}).to_string());
    run(&mut s, "plugin.installFromFile", json!({"dataBase64": b64(&wild)}));
    assert!(s.execute("plugin.runFilter", &json!({"id": "org.test.wild", "glyphs": ["A"]})).is_err());
    assert!(s.execute("plugin.runFilter", &json!({"id": ID, "glyphs": ["A"], "params": 5})).is_err());
    assert_eq!(s.font().unwrap(), &start, "no failed run changed the font");
}

#[test]
fn app_launch_without_plugins_safe_mode_lists_modules_but_runs_none_until_app_load_extensions() {
    let (mut s, data) = with_folders("plugins-safe", true);
    std::fs::create_dir_all(modules(&data)).unwrap();
    std::fs::write(modules(&data).join("square.wasm"), square_filter(ID, "1")).unwrap();
    extensions::save_switches(&modules(&data), &BTreeMap::from([("square.wasm".to_string(), true)])).unwrap();
    s.extensions_mut().module_switches = extensions::read_switches(&modules(&data)).unwrap();
    extensions::poll_folders(s.extensions_mut());
    let listed = run(&mut s, "pluginManager.list", json!({}));
    assert_eq!((listed["count"].as_u64(), listed["safeMode"].as_bool()), (Some(1), Some(true)));
    assert_eq!(listed["entries"][0]["enabled"], true);
    let e = err(&mut s, "plugin.runFilter", json!({"id": ID, "glyphs": ["A"]}));
    assert!(e.contains("safe mode"), "{e}");
    assert_eq!(run(&mut s, "app.info", json!({}))["safeMode"], true);

    assert_eq!(run(&mut s, "app.loadExtensions", json!({}))["enabled"], 1);
    run(&mut s, "plugin.runFilter", json!({"id": ID, "glyphs": ["A"]}));
    assert!(s.execute("app.loadExtensions", &json!({})).is_err(), "only in safe mode");
}

#[test]
fn app_info_reports_the_version_modules_folders_and_limits() {
    let (mut s, data) = with_folders("plugins-info", false);
    s.extensions_mut().surfaces.insert("control".into(), "listening on 127.0.0.1:7985".into());
    run(&mut s, "plugin.installFromFile", json!({"dataBase64": b64(&square_filter(ID, "1")), "name": "sq"}));
    let info = run(&mut s, "app.info", json!({}));
    assert!(!info["version"].as_str().unwrap().is_empty());
    assert_eq!(info["modules"][0]["id"], ID);
    assert_eq!(info["modules"][0]["file"], "sq.wasm");
    assert_eq!(info["modulesFolder"].as_str().map(PathBuf::from), Some(modules(&data)));
    assert_eq!(info["surfaces"]["control"], "listening on 127.0.0.1:7985");
    assert_eq!(info["limits"]["maxInstalled"], 256);
}

#[test]
fn plugin_manager_install_checks_the_index_entry_before_writing() {
    let (mut s, data) = with_folders("plugins-index", false);
    let bytes = square_filter(ID, "3");
    let shelf = data.join("shelf");
    std::fs::create_dir_all(&shelf).unwrap();
    std::fs::write(shelf.join("square-3.wasm"), &bytes).unwrap();
    let entry = |sha: String| Entry {
        id: ID.into(),
        kind: EntryKind::Filter,
        version: "3".into(),
        titles: BTreeMap::from([("en".to_string(), "Square".to_string())]),
        descriptions: BTreeMap::new(),
        vendor: "Tests".into(),
        url: "square-3.wasm".into(),
        sha256: sha,
        size: bytes.len() as u64,
        homepage: None,
        funding: None,
        screenshot: None,
    };
    let source = shelf.join("index.json").display().to_string();
    s.extensions_mut().indexes = vec![LoadedIndex { source: source.clone(), index: Index { entries: vec![entry("0".repeat(64))] } }];
    let listed = run(&mut s, "pluginManager.list", json!({"state": "notInstalled"}));
    assert_eq!(listed["entries"][0]["source"], "index");
    let e = err(&mut s, "pluginManager.install", json!({"id": ID}));
    assert!(e.contains("does not match"), "{e}");
    assert!(!modules(&data).join("square-3.wasm").exists(), "a mismatch writes nothing");
    assert!(s.extensions().registry.is_empty());

    s.extensions_mut().indexes = vec![LoadedIndex { source, index: Index { entries: vec![entry(sha256::hex(&bytes))] } }];
    let out = run(&mut s, "pluginManager.install", json!({"id": ID, "version": "3"}));
    assert_eq!((out["file"].as_str(), out["enabled"].as_bool()), (Some("square-3.wasm"), Some(true)));
    let listed = run(&mut s, "pluginManager.list", json!({"state": "installed"}));
    assert_eq!((listed["entries"][0]["installedVersion"].as_str(), listed["entries"][0]["update"].as_bool()), (Some("3"), Some(false)));
    assert!(err(&mut s, "pluginManager.install", json!({"id": ID, "version": "9"})).contains("version"));
}
