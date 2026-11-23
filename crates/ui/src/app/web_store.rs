//! Modules and macros installed in the browser build, kept in the browser's storage between visits. Rows:
//! `plugin.installFromFile`, `plugin.pluginsFolder`.
//!
//! The web build has no Modules or Macros folder, so `plugin.installFromFile` installs for the session only. After an
//! install from bytes (`dataBase64`: File › Open…, Install from File, an index entry) succeeds, [`after_run`] keeps
//! the file under [`STORAGE_KEY`]; after `pluginManager.remove` it drops what is no longer installed. When the app
//! starts, [`restore`] installs every kept file again. A file that would take the store past [`MAX_STORED_CHARS`] or
//! [`MAX_STORED_FILES`] stays installed for this visit and the status line says it was not kept.
//!
//! The desktop app has folders and no browser storage: there [`WebExtensions`] holds no store and nothing here runs.
//! Tests give it a store in memory.

use craft_ext::platform::storage::{BrowserStorage, STORAGE_REFUSED};
use serde_json::{Value, json};

use super::{FontCraftApp, Msg};

/// The storage key the kept files are under.
pub const STORAGE_KEY: &str = "fontcraft.extensions";
/// Most characters the kept files take, their base64 text and names together. The browser gives a site about five
/// million characters for all its keys; this leaves room for the rest.
pub const MAX_STORED_CHARS: usize = 4 << 20;
/// Most files kept.
pub const MAX_STORED_FILES: usize = 64;
/// The version of the stored text.
const VERSION: u64 = 1;
/// Characters each entry adds beside its data, id and name (the JSON keys and quotes).
const ENTRY_OVERHEAD: usize = 48;

/// Text kept by key: the browser's storage in the web build, a map in tests.
pub trait KeyValue: Send {
    /// The text under `key`, if any.
    fn get(&self, key: &str) -> Option<String>;
    /// Keeps `value` under `key`; a refusal as a message.
    fn set(&mut self, key: &str, value: &str) -> Result<(), String>;
}

/// The browser's `localStorage`, opened at each call (its handle cannot be kept across threads).
struct Browser;

impl KeyValue for Browser {
    fn get(&self, key: &str) -> Option<String> {
        BrowserStorage::open()?.get(key)
    }

    fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        BrowserStorage::open().ok_or_else(|| STORAGE_REFUSED.to_string())?.set(key, value)
    }
}

/// One kept module or macro.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// `"module"` or `"macro"`.
    pub kind: String,
    /// The module's id, or the macro's file name: what `pluginManager.remove` takes.
    pub id: String,
    /// The file name it is installed under (`tighten.fcmacro`, `square.wasm`).
    pub name: String,
    /// The file's bytes, in base64.
    pub data: String,
}

impl Entry {
    fn chars(&self) -> usize {
        self.data.len() + self.id.len() + self.name.len() + self.kind.len() + ENTRY_OVERHEAD
    }

    /// The install that puts it back.
    fn install(&self) -> Msg {
        Msg::run("plugin.installFromFile", json!({"dataBase64": self.data, "name": self.name, "replace": true}))
    }
}

/// The kept files.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Kept {
    pub entries: Vec<Entry>,
}

impl Kept {
    /// Reads the stored text; the problems found, one line each. Text over the limits, or not of this version, keeps
    /// nothing; an entry that is not a module or macro is left out.
    pub fn decode(text: &str) -> (Kept, Vec<String>) {
        let mut problems = Vec::new();
        if text.len() > MAX_STORED_CHARS + MAX_STORED_FILES * ENTRY_OVERHEAD {
            problems.push(format!("the kept modules and macros are larger than {} MiB; none were installed", MAX_STORED_CHARS >> 20));
            return (Kept::default(), problems);
        }
        let value: Value = match serde_json::from_str(text) {
            Ok(value) => value,
            Err(e) => {
                problems.push(format!("the kept modules and macros could not be read: {e}"));
                return (Kept::default(), problems);
            }
        };
        if value.get("version").and_then(Value::as_u64) != Some(VERSION) {
            problems.push("the kept modules and macros are of another version; none were installed".into());
            return (Kept::default(), problems);
        }
        let mut kept = Kept::default();
        let listed = value.get("entries").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        for item in listed.iter().take(MAX_STORED_FILES) {
            let field = |key: &str| item.get(key).and_then(Value::as_str).map(str::to_owned);
            let entry = match (field("kind"), field("id"), field("name"), field("data")) {
                (Some(kind), Some(id), Some(name), Some(data))
                    if matches!(kind.as_str(), "module" | "macro") && super::dropped::is_extension_name(&name) =>
                {
                    Entry { kind, id, name, data }
                }
                _ => {
                    problems.push("a kept module or macro was not one and was left out".into());
                    continue;
                }
            };
            if kept.put(entry).is_err() {
                problems.push("the kept modules and macros are over the limit; the rest were left out".into());
                break;
            }
        }
        if listed.len() > MAX_STORED_FILES {
            problems.push(format!("only the first {MAX_STORED_FILES} kept modules and macros were installed"));
        }
        (kept, problems)
    }

    /// The text to store.
    pub fn encode(&self) -> String {
        let entries: Vec<Value> = self.entries.iter().map(|e| json!({"kind": e.kind, "id": e.id, "name": e.name, "data": e.data})).collect();
        json!({"version": VERSION, "entries": entries}).to_string()
    }

    /// Characters the kept files take.
    pub fn chars(&self) -> usize {
        self.entries.iter().map(Entry::chars).sum()
    }

    /// Keeps `entry`, replacing the one of the same kind and id. Refused, with nothing changed, when the store would
    /// pass [`MAX_STORED_CHARS`] or [`MAX_STORED_FILES`].
    pub fn put(&mut self, entry: Entry) -> Result<(), String> {
        let same = |e: &Entry| e.kind == entry.kind && e.id == entry.id;
        let others: usize = self.entries.iter().filter(|e| !same(e)).map(Entry::chars).sum();
        let count = self.entries.iter().filter(|e| !same(e)).count();
        if count >= MAX_STORED_FILES {
            return Err(format!("the browser keeps at most {MAX_STORED_FILES} modules and macros"));
        }
        if others.saturating_add(entry.chars()) > MAX_STORED_CHARS {
            return Err(format!("the browser keeps at most {} MiB of modules and macros", MAX_STORED_CHARS >> 20));
        }
        match self.entries.iter_mut().find(|e| same(e)) {
            Some(old) => *old = entry,
            None => self.entries.push(entry),
        }
        Ok(())
    }

    /// Keeps only the entries `installed` accepts; whether any went.
    pub fn retain(&mut self, installed: impl Fn(&Entry) -> bool) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| installed(e));
        self.entries.len() != before
    }
}

/// Where the app keeps what it installs in the browser.
#[derive(Default)]
pub struct WebExtensions {
    store: Option<Box<dyn KeyValue>>,
    /// Set while [`restore`] installs the kept files, so they are not kept a second time.
    restoring: bool,
}

impl WebExtensions {
    /// The browser's storage in the web build; none on the desktop.
    pub fn browser() -> WebExtensions {
        let store = BrowserStorage::open().map(|_| Box::new(Browser) as Box<dyn KeyValue>);
        WebExtensions { store, restoring: false }
    }

    /// Keeps the files in `store`.
    pub fn with_store(store: Box<dyn KeyValue>) -> WebExtensions {
        WebExtensions { store: Some(store), restoring: false }
    }

    fn read(&self) -> (Kept, Vec<String>) {
        match self.store.as_ref().and_then(|s| s.get(STORAGE_KEY)) {
            Some(text) => Kept::decode(&text),
            None => (Kept::default(), Vec::new()),
        }
    }

    fn write(&mut self, kept: &Kept) -> Result<(), String> {
        let text = kept.encode();
        self.store.as_mut().map_or(Ok(()), |s| s.set(STORAGE_KEY, &text))
    }
}

/// Whether `entry` is still installed in the session.
fn installed(app: &FontCraftApp, entry: &Entry) -> bool {
    let ext = app.session.extensions();
    match entry.kind.as_str() {
        "module" => ext.registry.get(&entry.id).is_some(),
        _ => ext.find_macro(&entry.id).is_some(),
    }
}

/// Installs every kept file again, at startup. A file that no longer installs is dropped from the store. Every problem
/// goes to the log, the last to the status line.
pub fn restore(app: &mut FontCraftApp) {
    if app.web_extensions.store.is_none() {
        return;
    }
    let (mut kept, mut problems) = app.web_extensions.read();
    app.web_extensions.restoring = true;
    for entry in &kept.entries {
        if let Err(e) = app.run_now(entry.install(), false) {
            problems.push(format!("{}: {e}", entry.name));
        }
    }
    app.web_extensions.restoring = false;
    if kept.retain(|e| installed(app, e))
        && let Err(e) = app.web_extensions.write(&kept)
    {
        problems.push(e);
    }
    for problem in &problems {
        log::warn!("{problem}");
    }
    if let Some(last) = problems.last() {
        app.status = last.clone();
    }
}

/// Keeps a file installed from bytes, or drops what a removal took away, after command `id` ran with `params` and
/// answered `reply`.
pub fn after_run(app: &mut FontCraftApp, id: &str, params: &Value, reply: &Value) {
    let ours = matches!(id, "plugin.installFromFile" | "pluginManager.install" | "pluginManager.remove");
    if !ours || app.web_extensions.store.is_none() || app.web_extensions.restoring {
        return;
    }
    let (mut kept, _) = app.web_extensions.read();
    let changed = match id {
        "plugin.installFromFile" | "pluginManager.install" => {
            let Some(data) = params.get("dataBase64").and_then(Value::as_str) else { return };
            let Some(entry) = entry_of(data, params, reply) else { return };
            if let Err(e) = kept.put(entry) {
                app.status = format!("Installed for this visit only: {e}");
                return;
            }
            true
        }
        "pluginManager.remove" => kept.retain(|e| installed(app, e)),
        _ => false,
    };
    if changed && let Err(e) = app.web_extensions.write(&kept) {
        app.status = format!("The browser did not keep the change to the installed modules and macros: {e}");
    }
}

/// The entry for an install of `data` that answered `reply`: the module's id or the macro's file name, and the file
/// name to install it under again.
fn entry_of(data: &str, params: &Value, reply: &Value) -> Option<Entry> {
    let kind = if reply.get("kind").and_then(Value::as_str)? == "macro" { "macro" } else { "module" };
    let id = reply.get("id").and_then(Value::as_str)?.to_owned();
    let given = params.get("name").and_then(Value::as_str).or_else(|| reply.get("file").and_then(Value::as_str));
    let name = match (kind, given) {
        ("macro", _) => reply.get("file").and_then(Value::as_str)?.to_owned(),
        (_, Some(name)) if super::dropped::is_extension_name(name) => name.to_owned(),
        _ => format!("{id}.{}", fontcraft_engine::extensions::MODULE_EXTENSION),
    };
    // Valid base64 by now: the install decoded it.
    Some(Entry { kind: kind.into(), id, name, data: data.trim().to_owned() })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use base64::Engine as _;
    use craft::host::Host;
    use fontcraft_engine::Session;

    use super::*;
    use crate::app::APP;

    const MACRO: &[u8] = br#"{"format": "fontcraft.macro", "version": 1, "title": "Tighten", "description": "Tightens spacing.", "steps": []}"#;

    /// A browser storage in memory, shared by the apps of one test as a browser's is by its visits.
    #[derive(Clone, Default)]
    struct Memory(Arc<Mutex<HashMap<String, String>>>);

    impl KeyValue for Memory {
        fn get(&self, key: &str) -> Option<String> {
            self.0.lock().unwrap().get(key).cloned()
        }

        fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
            self.0.lock().unwrap().insert(key.into(), value.into());
            Ok(())
        }
    }

    fn browser_app(memory: &Memory) -> FontCraftApp {
        let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
        app.web_extensions = WebExtensions::with_store(Box::new(memory.clone()));
        restore(&mut app);
        app
    }

    fn entry(id: &str, data_chars: usize) -> Entry {
        Entry { kind: "macro".into(), id: id.into(), name: format!("{id}.fcmacro"), data: "A".repeat(data_chars) }
    }

    /// A macro installed from bytes in one visit is installed again in the next; removing it drops it from the store.
    #[test]
    fn a_macro_installed_in_the_browser_is_back_at_the_next_start_until_removed() {
        let memory = Memory::default();
        let mut first = browser_app(&memory);
        let data = base64::engine::general_purpose::STANDARD.encode(MACRO);
        first.run_now(Msg::run("plugin.installFromFile", json!({"dataBase64": data, "name": "tighten.fcmacro"})), false).unwrap();
        assert!(first.session.extensions().find_macro("tighten.fcmacro").is_some());
        assert!(memory.get(STORAGE_KEY).is_some_and(|t| t.contains("tighten.fcmacro")));

        let mut second = browser_app(&memory);
        assert!(second.session.extensions().find_macro("tighten.fcmacro").is_some(), "{}", second.status);
        second.run_now(Msg::run("pluginManager.remove", json!({"id": "tighten.fcmacro"})), false).unwrap();
        let (kept, problems) = Kept::decode(&memory.get(STORAGE_KEY).unwrap());
        assert!(kept.entries.is_empty() && problems.is_empty(), "{kept:?} {problems:?}");

        let third = browser_app(&memory);
        assert!(third.session.extensions().find_macro("tighten.fcmacro").is_none());
    }

    /// The store round-trips, replaces an entry of the same id, and refuses one that would pass the size limit
    /// without changing what it holds.
    #[test]
    fn the_store_keeps_entries_within_its_limits() {
        let mut kept = Kept::default();
        kept.put(entry("a", 10)).unwrap();
        kept.put(entry("a", 20)).unwrap();
        assert_eq!(kept.entries.len(), 1);
        let (back, problems) = Kept::decode(&kept.encode());
        assert_eq!(back, kept);
        assert!(problems.is_empty());

        let before = kept.clone();
        assert!(kept.put(entry("big", MAX_STORED_CHARS)).is_err());
        assert_eq!(kept, before);

        let mut many = Kept::default();
        for i in 0..MAX_STORED_FILES {
            many.put(entry(&format!("m{i}"), 1)).unwrap();
        }
        assert!(many.put(entry("one more", 1)).is_err());
        assert_eq!(many.entries.len(), MAX_STORED_FILES);
    }

    /// Stored text that is too long, garbled, of another version, or holds entries that are not modules or macros,
    /// keeps nothing of what is wrong and says so.
    #[test]
    fn stored_text_that_is_wrong_keeps_nothing_of_it() {
        let (kept, problems) = Kept::decode(&"x".repeat(MAX_STORED_CHARS * 2));
        assert!(kept.entries.is_empty() && problems.len() == 1);
        let (kept, problems) = Kept::decode("{not json");
        assert!(kept.entries.is_empty() && problems.len() == 1);
        let (kept, problems) = Kept::decode(r#"{"version": 99, "entries": []}"#);
        assert!(kept.entries.is_empty() && problems.len() == 1);
        let text = r#"{"version": 1, "entries": [{"kind": "macro", "id": "a", "name": "notes.txt", "data": ""}, {"kind": "module", "id": "b", "name": "b.wasm", "data": "AA=="}]}"#;
        let (kept, problems) = Kept::decode(text);
        assert_eq!(kept.entries.len(), 1);
        assert_eq!(problems.len(), 1);
    }
}
