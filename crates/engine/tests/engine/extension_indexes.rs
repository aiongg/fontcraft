//! Extension indexes: the user's list in settings, reading local and fetched HTTPS indexes,
//! the first index winning, installing an entry after its checksum, and updates.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};

use base64::Engine as _;
use fontcraft_engine::Session;
use fontcraft_engine::extensions;
use fontcraft_plugins::{fixtures, sha256};
use fontcraft_testkit::temp_dir;
use serde_json::{Value, json};

const ID: &str = "org.test.square";

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn err(s: &mut Session, id: &str, p: Value) -> String {
    match s.execute(id, &p) {
        Ok(v) => panic!("{id} should fail, gave {v}"),
        Err(e) => e.to_string(),
    }
}

/// A filter module `id` in version `version` that returns an empty layer.
fn module(id: &str, version: &str) -> Vec<u8> {
    let manifest = format!(r#"{{"id":"{id}","name":"Square","version":"{version}","kind":"filter"}}"#);
    fixtures::returning(&manifest, r#"{"shapes":[]}"#)
}

/// An index entry for `bytes`, stored at `url`; `sha` replaces the true checksum.
fn entry(id: &str, version: &str, url: &str, bytes: &[u8], sha: Option<&str>) -> Value {
    json!({
        "id": id, "kind": "filter", "version": version,
        "titles": {"en": format!("Square {version}")}, "descriptions": {"en": "Squares every glyph."},
        "vendor": "Test Foundry", "url": url, "sha256": sha.map_or_else(|| sha256::hex(bytes), str::to_string), "size": bytes.len(),
        "homepage": "https://example.com/square",
    })
}

fn write_index(path: &Path, entries: Vec<Value>) {
    let text = json!({"format": "fontcraft.extensions", "version": 1, "entries": entries});
    std::fs::write(path, serde_json::to_vec_pretty(&text).unwrap()).unwrap();
}

/// A session with a document and the extension folders in a fresh data folder, and a shelf
/// folder for index files.
fn setup(tag: &str) -> (Session, PathBuf, PathBuf) {
    let data = temp_dir(tag).join(format!("data-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    let shelf = data.join("shelf");
    std::fs::create_dir_all(&shelf).unwrap();
    let mut s = Session::with_document();
    assert!(extensions::open_folders(s.extensions_mut(), Some(&data), false).is_empty());
    (s, data, shelf)
}

fn set_indexes(s: &mut Session, lines: &[String]) {
    run(s, "settings.set", json!({"id": "addons.alternatePluginRepos", "value": lines.join("\n")}));
}

fn installed_version(s: &Session) -> Option<String> {
    s.extensions().registry.get(ID).map(|m| m.plugin.manifest().version.clone())
}

#[test]
fn plugin_manager_reload_indexes_reads_the_list_and_the_first_index_wins() {
    let (mut s, data, shelf) = setup("indexes-merge");
    let (one, two) = (module(ID, "1"), module(ID, "2"));
    std::fs::write(shelf.join("one.wasm"), &one).unwrap();
    let first = shelf.join("first.json");
    let second = shelf.join("second.json");
    write_index(&first, vec![entry(ID, "1", "one.wasm", &one, None)]);
    write_index(&second, vec![entry(ID, "2", "two.wasm", &two, None), entry("org.test.other", "1", "other.wasm", &one, None)]);
    set_indexes(
        &mut s,
        &[
            first.display().to_string(),
            format!("manual {}", second.display()),
            "relative/index.json".into(),
            "https://u:secret@example.com/i.json".into(),
        ],
    );

    let out = run(&mut s, "pluginManager.reloadIndexes", json!({}));
    let indexes = out["indexes"].as_array().unwrap();
    assert_eq!(indexes.len(), 4);
    assert_eq!((indexes[0]["entries"].as_u64(), indexes[1]["automaticUpdates"].as_bool()), (Some(1), Some(false)));
    assert!(indexes[2]["error"].as_str().unwrap().contains("full path"), "{out}");
    assert!(indexes[3]["error"].as_str().unwrap().contains("not fetched"), "{out}");
    assert_eq!(indexes[3]["source"], "https://example.com/i.json", "credentials never reach a result");
    assert_eq!(s.extensions().indexes.len(), 2, "unreadable indexes are left out");

    let listed = run(&mut s, "pluginManager.list", json!({}));
    let rows = listed["entries"].as_array().unwrap();
    let square: Vec<&Value> = rows.iter().filter(|r| r["id"] == ID).collect();
    assert_eq!(square.len(), 1, "an id is listed once");
    assert_eq!(square[0]["version"], "1", "the first index wins");
    assert!(rows.iter().any(|r| r["id"] == "org.test.other"));

    // The first index's file is installed after its checksum is checked.
    let installed = run(&mut s, "pluginManager.install", json!({"id": ID}));
    assert_eq!(installed["version"], "1");
    assert!(data.join("Modules").join("one.wasm").exists());
}

#[test]
fn plugin_manager_install_with_a_wrong_checksum_writes_nothing() {
    let (mut s, data, shelf) = setup("indexes-checksum");
    let bytes = module(ID, "1");
    std::fs::write(shelf.join("square.wasm"), &bytes).unwrap();
    let index = shelf.join("index.json");
    write_index(&index, vec![entry(ID, "1", "square.wasm", &bytes, Some(&"0".repeat(64)))]);
    run(&mut s, "pluginManager.reloadIndexes", json!({"indexes": [index.display().to_string()]}));
    let e = err(&mut s, "pluginManager.install", json!({"id": ID}));
    assert!(e.contains("does not match"), "{e}");
    assert!(!data.join("Modules").join("square.wasm").exists());
    assert!(s.extensions().registry.is_empty());
}

#[test]
fn plugin_manager_reload_indexes_reads_a_fetched_https_index() {
    let (mut s, _data, _shelf) = setup("indexes-https");
    let bytes = module(ID, "4");
    let index = json!({"format": "fontcraft.extensions", "version": 1, "entries": [entry(ID, "4", "files/square.wasm", &bytes, None)]});
    let fetched = base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(&index).unwrap());
    set_indexes(&mut s, &["https://user:pw@example.com/ext/index.json".into()]);
    let out = run(&mut s, "pluginManager.reloadIndexes", json!({"fetched": [fetched]}));
    assert_eq!(out["indexes"][0]["entries"], 1, "{out}");
    let listed = run(&mut s, "pluginManager.list", json!({"state": "notInstalled"}));
    assert_eq!(listed["entries"][0]["index"], "https://example.com/ext/index.json");
    // The entry's file is on the network: the desktop app downloads it.
    let e = err(&mut s, "pluginManager.install", json!({"id": ID}));
    assert!(e.contains("dataBase64"), "{e}");
    let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
    assert_eq!(run(&mut s, "pluginManager.install", json!({"id": ID, "dataBase64": data}))["version"], "4");
    assert!(err(&mut s, "pluginManager.reloadIndexes", json!({"fetched": vec![Value::Null; 65]})).contains("at most"));
}

#[test]
fn plugin_manager_updates_are_offered_and_installed_when_automatic() {
    let (mut s, _data, shelf) = setup("indexes-updates");
    let (one, two) = (module(ID, "1"), module(ID, "2"));
    std::fs::write(shelf.join("square-1.wasm"), &one).unwrap();
    std::fs::write(shelf.join("square-2.wasm"), &two).unwrap();
    let index = shelf.join("index.json");
    write_index(&index, vec![entry(ID, "1", "square-1.wasm", &one, None)]);
    let line = index.display().to_string();
    set_indexes(&mut s, std::slice::from_ref(&line));
    run(&mut s, "pluginManager.reloadIndexes", json!({}));
    run(&mut s, "pluginManager.install", json!({"id": ID}));
    run(&mut s, "plugin.setEnabled", json!({"id": ID, "enabled": false}));
    assert_eq!(s.extensions().registry.get(ID).map(|m| m.enabled), Some(false));

    // The index now lists version 2. With manual updates it is offered only.
    write_index(&index, vec![entry(ID, "2", "square-2.wasm", &two, None)]);
    set_indexes(&mut s, &[format!("manual {line}")]);
    let out = run(&mut s, "pluginManager.reloadIndexes", json!({}));
    assert_eq!(out["updates"][0]["version"], "2", "{out}");
    assert_eq!((out["updates"][0]["automatic"].as_bool(), out["updates"][0]["installed"].as_bool()), (Some(false), Some(false)));
    assert_eq!(installed_version(&s).as_deref(), Some("1"));
    let listed = run(&mut s, "pluginManager.list", json!({"state": "installed"}));
    assert_eq!(listed["entries"][0]["update"], true);

    // With automatic updates the update installs, and the module stays off as the user left it.
    set_indexes(&mut s, &[line]);
    let out = run(&mut s, "pluginManager.reloadIndexes", json!({}));
    assert_eq!(out["updates"][0]["installed"], true, "{out}");
    assert_eq!(installed_version(&s).as_deref(), Some("2"));
    assert_eq!(s.extensions().registry.get(ID).map(|m| m.enabled), Some(false));
    let again = run(&mut s, "pluginManager.reloadIndexes", json!({}));
    assert!(again["updates"].as_array().unwrap().is_empty(), "nothing left to update");

    // In safe mode updates are offered but not installed.
    s.extensions_mut().safe_mode = true;
    write_index(&index, vec![entry(ID, "1", "square-1.wasm", &one, None)]);
    let out = run(&mut s, "pluginManager.reloadIndexes", json!({}));
    assert_eq!(out["updates"][0]["installed"], false);
    assert_eq!(installed_version(&s).as_deref(), Some("2"));
}

#[test]
fn plugin_set_enabled_turns_a_module_on_and_off() {
    let (mut s, data, _shelf) = setup("indexes-switch");
    let bytes = module(ID, "1");
    let path = data.join("square.wasm");
    std::fs::write(&path, &bytes).unwrap();
    run(&mut s, "plugin.installFromFile", json!({"path": path}));
    let off = run(&mut s, "plugin.setEnabled", json!({"id": ID, "enabled": false}));
    assert_eq!((off["enabled"].as_bool(), off["runs"].as_bool()), (Some(false), Some(false)));
    let switches = extensions::read_switches(&data.join("Modules")).unwrap();
    assert_eq!(switches.get("square.wasm"), Some(&false), "the switch is kept in the Modules folder");
    let on = run(&mut s, "plugin.setEnabled", json!({"id": ID, "enabled": true}));
    assert_eq!(on["runs"], true);
    assert!(err(&mut s, "plugin.setEnabled", json!({"id": "org.test.missing", "enabled": true})).contains("org.test.missing"));
    assert!(err(&mut s, "plugin.setEnabled", json!({"id": ID})).contains("enabled"));
}
