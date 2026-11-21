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
//! | Website, Donate | `pluginManager.openWebsite`, `pluginManager.openDonationPage`: the entry's link in the browser, through `FontCraftApp::opener` |
//! | Preview | `pluginManager.enlargePreview`: the entry's screenshot, read or downloaded, under the list |
//! | Indexes tab | the extension folders (Show: `file.showInFinder`), the log switch and the index list, each change a `settings.set` ([`IndexesModel`]; Settings › Extensions shows the same body) |
//!
//! The list is made again when it is opened, when a filter changes, when the installed modules, their switches or
//! their errors change, and after Reload. Each row carries the module's last error (`ExtensionState::module_errors`); module files that failed
//! to load are listed under the table. Opening the window reads the indexes again; downloads (HTTPS indexes, entry
//! files, screenshots) are `super::extension_downloads`.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;

use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::Session;
use fontcraft_engine::extension_index::{Entry, IndexSource, MAX_INDEXES, parse_sources, redact};
use serde_json::{Value, json};

use super::commands::{Module, UiCommand};
use super::extension_downloads::{self, sources_setting};
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
/// The setting of the Indexes tab's log switch.
pub const LOG_SETTING: &str = "addons.useSystemConsoleForScriptOutput";

const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: "window.pluginManager",
        label: "Extension Manager",
        menu: &["Window"],
        shortcut: None,
        params: "{tab?: \"extensions\" | \"indexes\"} open the Extension Manager on a tab (extensions) and read the extension indexes again (pluginManager.reloadIndexes); HTTPS indexes are downloaded in the background. Extensions: the extensions the indexes offer, the installed modules and the macros, with Install, Update, Remove, an on/off switch, Website, Donate, Preview, Install from File… and Reload. Indexes: the extension folders, the log switch and the list of indexes with an automatic-updates switch each",
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
    UiCommand {
        id: "pluginManager.enlargePreview",
        label: "Enlarge Preview",
        menu: &[],
        shortcut: None,
        params: "{id?: entry id} show the index entry's screenshot in the Extension Manager (read beside a local index, or downloaded in the background); without id, close the preview",
    },
    UiCommand {
        id: "pluginManager.openWebsite",
        label: "Open Website",
        menu: &[],
        shortcut: None,
        params: "{id: entry id} open the index entry's website in the system browser (an http:// or https:// link); refused when the entry names none or this build has no browser",
    },
    UiCommand {
        id: "pluginManager.openDonationPage",
        label: "Donate",
        menu: &[],
        shortcut: None,
        params: "{id: entry id, service?: string (ignored: an entry has one funding link)} open the index entry's donation page in the system browser; refused when the entry names none or this build has no browser",
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
    /// The file's size in bytes and its SHA-256, when the row gives them.
    pub size: Option<u64>,
    pub sha256: String,
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
            size: row.get("size").and_then(Value::as_u64),
            sha256: text("sha256"),
            id,
        })
    }
}

/// The window's tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Extensions,
    Indexes,
}

/// An entry's screenshot while it is shown: decoded, still loading, or why it cannot be shown.
#[derive(Clone, Default)]
pub struct Shot {
    pub id: String,
    pub image: Option<Arc<egui::ColorImage>>,
    pub error: Option<String>,
    /// Moves when the image or the error arrives.
    pub key: u64,
}

impl PartialEq for Shot {
    fn eq(&self, other: &Shot) -> bool {
        self.id == other.id && self.key == other.key && self.error == other.error && self.image.is_some() == other.image.is_some()
    }
}

impl std::fmt::Debug for Shot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Shot({}, image: {}, error: {:?})", self.id, self.image.is_some(), self.error)
    }
}

/// What a line of the index list shows next to its address.
#[derive(Clone, Debug, PartialEq)]
pub enum IndexState {
    /// Not read since the list changed.
    Unread,
    Downloading,
    Entries(u64),
    Error(String),
}

/// One line of the index list: its address without credentials, its automatic-updates switch and its state.
#[derive(Clone, Debug, PartialEq)]
pub struct IndexRow {
    pub shown: String,
    pub automatic: bool,
    pub state: IndexState,
}

/// The Indexes tab, which Settings › Extensions also shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IndexesModel {
    /// The Modules and Macros folders; `None` when the session has none.
    pub modules_folder: Option<String>,
    pub macros_folder: Option<String>,
    /// The extension log also goes to standard error.
    pub log_to_stderr: bool,
    /// The index list as stored, with credentials: the edits write it back whole.
    pub sources: Vec<IndexSource>,
    /// What each line shows, in the order of `sources`.
    pub rows: Vec<IndexRow>,
    /// The last download or install problem.
    pub problem: Option<String>,
}

/// The Indexes tab's model from the settings, the extension state and the downloads.
pub fn indexes_model(app: &FontCraftApp) -> IndexesModel {
    let ext = app.session.extensions();
    let folder = |p: &Option<PathBuf>| p.as_ref().map(|p| p.display().to_string());
    let sources: Vec<IndexSource> = parse_sources(&sources_setting(app)).into_iter().take(MAX_INDEXES).collect();
    let view = &app.view.extensions.manager;
    let rows = sources
        .iter()
        .map(|s| {
            let shown = redact(&s.location);
            let state = if app.view.extensions.downloads.index_running(&s.location) {
                IndexState::Downloading
            } else {
                match view.reported.iter().find(|r| r.0 == shown) {
                    Some((_, _, Some(error))) => IndexState::Error(error.clone()),
                    Some((_, Some(n), None)) => IndexState::Entries(*n),
                    _ => IndexState::Unread,
                }
            };
            IndexRow { shown, automatic: s.automatic_updates, state }
        })
        .collect();
    IndexesModel {
        modules_folder: folder(&ext.modules_dir),
        macros_folder: folder(&ext.macros_dir),
        log_to_stderr: app.session.settings().get_bool(LOG_SETTING).unwrap_or(false),
        sources,
        rows,
        problem: view.problem.clone(),
    }
}

/// The Extension Manager's view state.
#[derive(Clone, Debug, PartialEq)]
pub struct ManagerView {
    pub open: bool,
    pub tab: Tab,
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
    /// The Indexes tab.
    pub indexes: IndexesModel,
    /// The screenshot shown under the list.
    pub preview: Option<Shot>,
    /// The last download or install problem.
    pub problem: Option<String>,
    /// Moves with every change of what the window shows, so the dialog knows to take it again.
    pub revision: u64,
    /// The extensions' revision and filters the list was made for; `None`: make it again.
    seen: Option<(u64, String, String, String)>,
    /// The last `pluginManager.reloadIndexes` report: each index's shown address, entry count or error.
    pub(crate) reported: Vec<(String, Option<u64>, Option<String>)>,
    /// The index list the indexes were last read for.
    pub(crate) seen_sources: Option<String>,
}

impl Default for ManagerView {
    fn default() -> Self {
        ManagerView {
            open: false,
            tab: Tab::Extensions,
            state: "all".into(),
            kind: "all".into(),
            query: String::new(),
            entries: Vec::new(),
            failures: Vec::new(),
            safe_mode: false,
            note: None,
            indexes: IndexesModel::default(),
            preview: None,
            problem: None,
            revision: 0,
            seen: None,
            reported: Vec::new(),
            seen_sources: None,
        }
    }
}

impl ManagerView {
    /// Makes the list again on the next [`refresh`], whatever changed.
    pub fn invalidate(&mut self) {
        self.seen = None;
    }

    /// Records a change the dialog must show.
    pub fn touch(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let params = &command.params;
    let text = |key: &str| params.get(key).and_then(Value::as_str);
    let view = &mut app.view.extensions.manager;
    match command.id.as_str() {
        "window.pluginManager" => {
            view.tab = match text("tab") {
                None | Some("extensions") => Tab::Extensions,
                Some("indexes") => Tab::Indexes,
                Some(other) => return Err(format!("`tab` is \"extensions\" or \"indexes\", not {other:?}")),
            };
            view.open = true;
            view.invalidate();
            view.touch();
            let params = extension_downloads::reload_params(app);
            // A failed reading leaves the window open with the reason under the list.
            if let Err(e) = app.run_command("pluginManager.reloadIndexes", params) {
                app.view.extensions.manager.problem = Some(e);
            }
        }
        "pluginManager.enlargePreview" => match text("id") {
            None => {
                view.preview = None;
                view.touch();
            }
            Some(id) => {
                let id = id.to_string();
                extension_downloads::show_screenshot(app, &id)?;
            }
        },
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
        "pluginManager.openWebsite" | "pluginManager.openDonationPage" => return open_link(app, command.id.as_str(), params),
        other => return Err(format!("no Extension Manager command `{other}`")),
    }
    refresh(app);
    Ok(())
}

/// The first entry with `id` in the session's indexes (the first index wins), with its index's location.
pub fn find_entry(session: &Session, id: &str) -> Option<(Entry, String)> {
    session.extensions().indexes.iter().find_map(|l| l.index.find(id, None).map(|e| (e.clone(), l.source.clone())))
}

/// Opens the entry's website or donation page with the app's opener, which takes only http(s) links.
fn open_link(app: &mut FontCraftApp, command: &str, params: &Value) -> Result<(), String> {
    let id = params.get("id").and_then(Value::as_str).ok_or("give the entry's `id`")?;
    let (entry, _) = find_entry(&app.session, id).ok_or_else(|| format!("no extension index offers {id:?}"))?;
    let (link, what) = if command == "pluginManager.openWebsite" { (entry.homepage, "website") } else { (entry.funding, "donation page") };
    let link = link.ok_or_else(|| format!("{id:?} names no {what}"))?;
    app.opener.open_url(&link)
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
    if !app.view.extensions.manager.open {
        return;
    }
    let indexes = indexes_model(app);
    let view = &mut app.view.extensions.manager;
    if view.indexes != indexes {
        view.indexes = indexes;
        view.touch();
    }
    let view = &app.view.extensions.manager;
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

    fn opened_ok(_: &str) -> Result<(), String> {
        Ok(())
    }

    /// Website and Donate open the index entry's links through the app's opener; without one, or without the link,
    /// they say why.
    #[test]
    fn extension_manager_opens_website_and_donation_links() {
        use fontcraft_engine::extension_index::Index;
        use fontcraft_engine::extensions::LoadedIndex;

        let mut app = FontCraftApp::new(Session::new(), Arc::new(Host::builder(crate::app::APP).build()));
        let bytes = serde_json::to_vec(&json!({"format": "fontcraft.extensions", "version": 1, "entries": [{
            "id": "tighten", "kind": "macro", "version": "1", "titles": {"en": "Tighten"}, "descriptions": {}, "vendor": "Test Foundry", "url": "tighten.fcmacro", "size": 10,
            "sha256": "0".repeat(64), "homepage": "https://example.com/home", "funding": "https://example.com/give money",
        }]}))
        .unwrap();
        let index = Index::parse(&bytes).unwrap();
        app.session.extensions_mut().indexes.push(LoadedIndex { source: "https://example.test/index.json".into(), index });

        let error = shell(&mut app, "pluginManager.openWebsite", json!({"id": "tighten"})).unwrap_err();
        assert!(error.contains("browser"), "{error}");
        app.opener = craft_ext::platform::open::Opener::new(Some(opened_ok), None);
        shell(&mut app, "pluginManager.openWebsite", json!({"id": "tighten"})).unwrap();
        let error = shell(&mut app, "pluginManager.openDonationPage", json!({"id": "tighten"})).unwrap_err();
        assert!(error.contains("not a web link"), "only http(s) links open: {error}");
        assert!(shell(&mut app, "pluginManager.openWebsite", json!({"id": "loosen"})).unwrap_err().contains("no extension index"));
    }
}
