//! The Extension Manager's state and commands (`window.pluginManager` and its filters). The window lists the
//! extensions the session's indexes offer, the installed modules and the macros of the Macros folder, through
//! `pluginManager.list`; the dialog itself is `dialogs::extension_manager`.
//!
//! | Control | Does |
//! |---|---|
//! | install-state and type buttons, search field | `pluginManager.filterByInstallState`, `pluginManager.filterByType`, `pluginManager.search` (view state) |
//! | Install, Update | `pluginManager.install` with the entry's id (and version for Update) |
//! | Remove | `pluginManager.remove` |
//! | On | `plugin.setEnabled` |
//! | Install from File… | `plugin.installFromFile` without `path`: the file picker asks for a module or macro |
//! | Reload | `pluginManager.reloadIndexes`, then the list is made again |
//!
//! The list is made again when it is opened, when a filter changes, when the installed modules, their switches or
//! their errors change, and after Reload. Each row carries the module's last error (`ExtensionState::module_errors`); module files that failed
//! to load are listed under the table. Entries on the network are installed only by the desktop app's downloader,
//! which this window does not have yet: their Install reports the engine's refusal in the status line.

use std::hash::{DefaultHasher, Hash, Hasher};

use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::Session;
use serde_json::{Value, json};

use super::commands::{Module, UiCommand};
use crate::app::FontCraftApp;

pub const MODULE: Module = Module { commands: COMMANDS, run };

/// Longest search text kept, in characters.
pub const MAX_QUERY_CHARS: usize = 1_000;
/// Most rows the window lists.
pub const MAX_ENTRIES: usize = 4_096;
/// Most failed module files listed.
pub const MAX_FAILURES: usize = 256;
/// Longest error text kept per row, in characters.
const MAX_ERROR_CHARS: usize = 500;

const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: "window.pluginManager",
        label: "Extension Manager",
        menu: &["Window"],
        shortcut: None,
        params: "{tab?: \"extensions\"} open the Extension Manager: the extensions the indexes offer, the installed modules and the macros, with Install, Update, Remove, an on/off switch, Install from File… and Reload",
    },
    UiCommand {
        id: "pluginManager.filterByInstallState",
        label: "All / Installed / Not Installed",
        menu: &[],
        shortcut: None,
        params: "{state: \"all\" | \"installed\" | \"notInstalled\"} show every extension, the installed ones or the others",
    },
    UiCommand {
        id: "pluginManager.filterByType",
        label: "All / Filters / Reporters / Checks / Macros",
        menu: &[],
        shortcut: None,
        params: "{type: \"all\" | \"filter\" | \"reporter\" | \"check\" | \"macro\"} show one kind of extension, or every kind",
    },
    UiCommand {
        id: "pluginManager.search",
        label: "Search",
        menu: &[],
        shortcut: None,
        params: "{query: string} show the extensions whose id, title, vendor or description holds the text, ignoring case; an empty query shows all",
    },
];

/// The install-state filter values.
pub const STATES: [&str; 3] = ["all", "installed", "notInstalled"];
/// The type filter values.
pub const KINDS: [&str; 5] = ["all", "filter", "reporter", "check", "macro"];

/// One row of the list.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ManagerEntry {
    /// A module id, an index entry id, or a macro's path.
    pub id: String,
    /// `filter`, `reporter`, `check` or `macro`.
    pub kind: String,
    pub title: String,
    pub vendor: String,
    pub version: String,
    pub description: String,
    pub installed: bool,
    /// The installed version, when an index offers another.
    pub installed_version: Option<String>,
    /// An index offers another version than the installed one.
    pub update: bool,
    /// The module's switch; `None` for macros and entries not installed.
    pub enabled: Option<bool>,
    /// Copied into the Modules folder by hand and not yet turned on.
    pub new: bool,
    /// An index offers it: Install and Update apply.
    pub from_index: bool,
    /// The module's last error.
    pub error: Option<String>,
}

impl ManagerEntry {
    fn read(row: &Value, session: &Session) -> Option<ManagerEntry> {
        let text = |key: &str| row.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
        let flag = |key: &str| row.get(key).and_then(Value::as_bool).unwrap_or(false);
        let id = row.get("id")?.as_str()?.to_string();
        let error = session.extensions().module_errors.get(&id).map(|e| e.chars().take(MAX_ERROR_CHARS).collect());
        Some(ManagerEntry {
            kind: text("kind"),
            title: text("title"),
            vendor: text("vendor"),
            version: text("version"),
            description: text("description"),
            installed: flag("installed"),
            installed_version: row.get("installedVersion").and_then(Value::as_str).map(str::to_string),
            update: flag("update"),
            enabled: row.get("enabled").and_then(Value::as_bool),
            new: flag("new"),
            from_index: text("source") == "index",
            error,
            id,
        })
    }
}

/// The Extension Manager's view state.
#[derive(Clone, Debug, PartialEq)]
pub struct ManagerView {
    pub open: bool,
    /// `all`, `installed` or `notInstalled`.
    pub state: String,
    /// `all` or an entry kind.
    pub kind: String,
    pub query: String,
    /// The rows `pluginManager.list` gave for the filters.
    pub entries: Vec<ManagerEntry>,
    /// Module files that failed to load, with the reason.
    pub failures: Vec<(String, String)>,
    /// Modules are listed but none runs.
    pub safe_mode: bool,
    /// Why the list could not be made.
    pub note: Option<String>,
    /// Moves with every change of the list, so the dialog knows to take it again.
    pub revision: u64,
    /// The extensions' revision and filters the list was made for; `None`: make it again.
    seen: Option<(u64, String, String, String)>,
}

impl Default for ManagerView {
    fn default() -> Self {
        ManagerView {
            open: false,
            state: "all".into(),
            kind: "all".into(),
            query: String::new(),
            entries: Vec::new(),
            failures: Vec::new(),
            safe_mode: false,
            note: None,
            revision: 0,
            seen: None,
        }
    }
}

impl ManagerView {
    /// Makes the list again on the next [`refresh`], whatever changed.
    pub fn invalidate(&mut self) {
        self.seen = None;
    }
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let params = &command.params;
    let text = |key: &str| params.get(key).and_then(Value::as_str);
    let view = &mut app.view.extensions.manager;
    match command.id.as_str() {
        "window.pluginManager" => {
            match text("tab") {
                None | Some("extensions") => {}
                Some("indexes") => return Err("the Indexes tab is not in this window yet".into()),
                Some(other) => return Err(format!("`tab` is \"extensions\", not {other:?}")),
            }
            view.open = true;
            view.invalidate();
        }
        "pluginManager.filterByInstallState" => {
            let value = text("state").ok_or("give `state`")?;
            if !STATES.contains(&value) {
                return Err(format!("`state` is \"all\", \"installed\" or \"notInstalled\", not {value:?}"));
            }
            view.state = value.to_string();
        }
        "pluginManager.filterByType" => {
            let value = text("type").ok_or("give `type`")?;
            if !KINDS.contains(&value) {
                return Err(format!("`type` is \"all\", \"filter\", \"reporter\", \"check\" or \"macro\", not {value:?}"));
            }
            view.kind = value.to_string();
        }
        "pluginManager.search" => {
            view.query = text("query").unwrap_or_default().chars().take(MAX_QUERY_CHARS).collect();
        }
        other => return Err(format!("no Extension Manager command `{other}`")),
    }
    refresh(app);
    Ok(())
}

/// What the list depends on in the extension state: the installed modules and macros, each module's switch, the
/// errors and the indexes. The revision alone does not move when a switch changes.
fn fingerprint(session: &Session) -> u64 {
    let ext = session.extensions();
    let mut h = DefaultHasher::new();
    ext.revision().hash(&mut h);
    for m in ext.registry.list() {
        (m.plugin.id(), m.enabled, m.new).hash(&mut h);
    }
    ext.module_errors.hash(&mut h);
    ext.module_failures.hash(&mut h);
    ext.safe_mode.hash(&mut h);
    for index in &ext.indexes {
        index.source.hash(&mut h);
    }
    h.finish()
}

/// Makes the list again while the window is open and the extensions or the filters changed since it was made.
pub fn refresh(app: &mut FontCraftApp) {
    let view = &app.view.extensions.manager;
    if !view.open {
        return;
    }
    let key = (fingerprint(&app.session), view.state.clone(), view.kind.clone(), view.query.clone());
    if view.seen.as_ref() == Some(&key) {
        return;
    }
    let mut params = serde_json::Map::new();
    params.insert("state".into(), json!(view.state));
    if view.kind != "all" {
        params.insert("type".into(), json!(view.kind));
    }
    if !view.query.trim().is_empty() {
        params.insert("query".into(), json!(view.query.trim()));
    }
    let listed = app.session.query("pluginManager.list", &Value::Object(params));
    let failures: Vec<(String, String)> = app.session.extensions().module_failures.iter().take(MAX_FAILURES).cloned().collect();
    let (entries, safe_mode, note) = match listed {
        Ok(v) => {
            let rows = v.get("entries").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
            let entries = rows.iter().take(MAX_ENTRIES).filter_map(|row| ManagerEntry::read(row, &app.session)).collect();
            (entries, v.get("safeMode").and_then(Value::as_bool).unwrap_or(false), None)
        }
        Err(e) => (Vec::new(), app.session.extensions().safe_mode, Some(e.to_string())),
    };
    let view = &mut app.view.extensions.manager;
    view.entries = entries;
    view.failures = failures;
    view.safe_mode = safe_mode;
    view.note = note;
    view.seen = Some(key);
    view.revision = view.revision.wrapping_add(1);
    app.view_changed();
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use std::sync::Arc;

    use craft::host::Host;
    use fontcraft_engine::plugins::fixtures;

    use super::*;
    use crate::app::Msg;

    fn shell(app: &mut FontCraftApp, id: &str, params: Value) -> Result<Value, String> {
        app.run_now(Msg::Shell(CommandRef::with_params(id, params)), false)
    }

    fn ids(app: &FontCraftApp) -> Vec<String> {
        app.view.extensions.manager.entries.iter().map(|e| e.id.clone()).collect()
    }

    /// The window lists the installed modules, filters them by kind and search text, and follows the on/off switch
    /// and removal the engine commands make.
    #[test]
    fn extension_manager_lists_filters_and_follows_the_switch_and_removal() {
        let mut app = FontCraftApp::new(Session::new(), Arc::new(Host::builder(crate::app::APP).build()));
        for (id, kind) in [("box", "reporter"), ("square", "filter")] {
            let manifest = format!(r#"{{"id":"{id}","name":"{id} module","version":"2","kind":"{kind}","description":"draws a {id}"}}"#);
            let output = if kind == "filter" { json!({"shapes": []}) } else { json!({}) };
            app.session.extensions_mut().registry.install_bytes(&fixtures::returning(&manifest, &output.to_string()), None).unwrap();
        }
        assert!(ids(&app).is_empty(), "nothing is listed while the window is closed");
        shell(&mut app, "window.pluginManager", json!({})).unwrap();
        assert!(app.view.extensions.manager.open);
        assert_eq!(ids(&app), ["box", "square"]);
        let square = app.view.extensions.manager.entries.iter().find(|e| e.id == "square").unwrap().clone();
        assert_eq!((square.kind.as_str(), square.version.as_str(), square.installed, square.enabled), ("filter", "2", true, Some(true)));

        shell(&mut app, "pluginManager.filterByType", json!({"type": "reporter"})).unwrap();
        assert_eq!(ids(&app), ["box"]);
        shell(&mut app, "pluginManager.filterByType", json!({"type": "all"})).unwrap();
        shell(&mut app, "pluginManager.search", json!({"query": "SQUARE"})).unwrap();
        assert_eq!(ids(&app), ["square"]);
        assert!(shell(&mut app, "pluginManager.filterByType", json!({"type": "plugins"})).is_err());
        assert!(shell(&mut app, "pluginManager.filterByInstallState", json!({"state": "gone"})).is_err());
        shell(&mut app, "pluginManager.search", json!({"query": ""})).unwrap();

        app.run_now(Msg::run("plugin.setEnabled", json!({"id": "square", "enabled": false})), false).unwrap();
        refresh(&mut app);
        let square = app.view.extensions.manager.entries.iter().find(|e| e.id == "square").unwrap();
        assert_eq!(square.enabled, Some(false), "the list follows the switch");

        app.run_now(Msg::run("pluginManager.remove", json!({"id": "box"})), false).unwrap();
        refresh(&mut app);
        assert_eq!(ids(&app), ["square"], "a removed module leaves the list");
        shell(&mut app, "pluginManager.filterByInstallState", json!({"state": "notInstalled"})).unwrap();
        assert!(ids(&app).is_empty());
    }
}
