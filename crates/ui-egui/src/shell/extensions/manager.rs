//! The Extension Manager (`window.pluginManager`): the extensions the user's indexes offer, the
//! installed modules and the macros, with the install-state and type filters, the search field,
//! Install, Update, Remove and an on/off switch per module; and an Indexes tab with the list of
//! indexes, an automatic-updates switch per index and the extension folders (the Settings ›
//! Extensions section until the Settings window exists).
//!
//! The window reads engine state and acts through engine commands: `pluginManager.list`,
//! `pluginManager.reloadIndexes`, `pluginManager.install`, `pluginManager.remove`,
//! `plugin.setEnabled` and `settings.set`. Network access is the desktop app's: it supplies a
//! blocking HTTPS fetch ([`Platform`]), which runs on a background thread per download. HTTPS
//! indexes are downloaded when the window opens or the list changes and kept for the session;
//! the indexes are read again when they arrive. Entry files on the network are downloaded,
//! then installed with `pluginManager.install {dataBase64}`, which checks size and SHA-256
//! before writing anything. Without a fetch (the web build, tests) only local indexes are read.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use egui::{Align2, RichText};
use fontcraft_engine::extension_index::{
    Entry, IndexSource, Location, MAX_INDEX_BYTES, MAX_SCREENSHOT_BYTES, SOURCES_SETTING, parse_sources, redact, sources_text,
};
use serde_json::{Value, json};

use super::super::{Args, UiCommand};
use crate::app::FontcraftApp;
use crate::panels::Pending;

pub const OPEN: UiCommand = (
    "window.pluginManager",
    "Extension Manager",
    &["Window"],
    "",
    "{tab?: \"extensions\" | \"indexes\"} open the Extension Manager on a tab (extensions) and read the extension indexes again (pluginManager.reloadIndexes); HTTPS indexes are downloaded in the background → {entries: count, indexes: count}",
);
pub const FILTER_STATE: UiCommand = (
    "pluginManager.filterByInstallState",
    "All / Installed / Not Installed",
    &[],
    "",
    "{state: \"all\" | \"installed\" | \"notInstalled\"} show every extension, the installed ones or the others → {state, count}",
);
pub const FILTER_TYPE: UiCommand = (
    "pluginManager.filterByType",
    "All / Filters / Reporters / Checks / Macros",
    &[],
    "",
    "{type: \"all\" | \"filter\" | \"reporter\" | \"check\" | \"macro\"} show one kind of extension, or every kind → {type, count}",
);
pub const SEARCH: UiCommand = (
    "pluginManager.search",
    "Search",
    &[],
    "",
    "{query: string} show the extensions whose id, title, vendor or description holds the text, ignoring case; an empty query shows all → {query, count}",
);
pub const ENLARGE: UiCommand = (
    "pluginManager.enlargePreview",
    "Enlarge Preview",
    &[],
    "",
    "{id?: entry id} show the entry's screenshot at full size (downloaded in the background when it is on the network); without id, close the preview → {id?, screenshot?}",
);
pub const WEBSITE: UiCommand =
    ("pluginManager.openWebsite", "Open Website", &[], "", "{id: entry id} open the entry's website in the system browser → {id, url}");
pub const DONATE: UiCommand = (
    "pluginManager.openDonationPage",
    "Donate",
    &[],
    "",
    "{id: entry id, service?: string (ignored: an entry has one funding link)} open the entry's donation page in the system browser → {id, url}",
);

pub const COMMANDS: [UiCommand; 7] = [OPEN, FILTER_STATE, FILTER_TYPE, SEARCH, ENLARGE, WEBSITE, DONATE];

/// The interface language sent with screenshot downloads.
const LANGUAGE: &str = "en";
/// Longest search text kept, in characters.
const MAX_QUERY_CHARS: usize = 1_000;
/// Most downloads running at once.
const MAX_DOWNLOADS: usize = 8;
/// Largest entry file downloaded: the module limit.
const MAX_ENTRY_BYTES: usize = 32 << 20;
/// How often the window looks for finished downloads while some run, in milliseconds.
const DOWNLOAD_POLL_MS: u64 = 200;
/// Characters of a checksum shown in the list.
const SHORT_SHA: usize = 12;

/// A blocking HTTPS GET: the URL (which may carry credentials), the largest body accepted, and
/// the language for `Accept-Language`. The desktop app supplies it.
pub type FetchFn = fn(&str, usize, &str) -> Result<Vec<u8>, String>;
/// Opens an http(s) link in the system browser.
pub type OpenUrlFn = fn(&str) -> Result<(), String>;

/// What the platform provides. Not part of the window's state: two states compare equal
/// whatever it is.
#[derive(Clone, Copy, Default)]
pub struct Platform {
    pub fetch: Option<FetchFn>,
    pub open_url: Option<OpenUrlFn>,
}

impl PartialEq for Platform {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl std::fmt::Debug for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Platform(fetch: {}, open_url: {})", self.fetch.is_some(), self.open_url.is_some())
    }
}

/// A download and what to do with it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Job {
    /// An HTTPS index, by its location in the list.
    Index(String),
    /// An entry's file, installed when it arrives; `keep_off` puts back a module's off switch.
    Install { id: String, version: String, keep_off: bool },
    /// An entry's screenshot.
    Screenshot(String),
}

#[derive(Default)]
struct Inbox {
    running: BTreeSet<Job>,
    done: Vec<(Job, Result<Vec<u8>, String>)>,
}

/// The downloads in flight, shared with their threads.
#[derive(Clone, Default)]
pub struct Downloads(Arc<Mutex<Inbox>>);

impl Downloads {
    fn with<T>(&self, f: impl FnOnce(&mut Inbox) -> T) -> T {
        let mut inbox = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&mut inbox)
    }

    /// Whether any download is running.
    pub fn busy(&self) -> bool {
        self.with(|i| !i.running.is_empty())
    }
}

impl PartialEq for Downloads {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl std::fmt::Debug for Downloads {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Downloads(busy: {})", self.busy())
    }
}

/// A decoded screenshot and its texture.
#[derive(Clone, Default)]
pub struct Preview {
    pub id: String,
    pub image: Option<Arc<egui::ColorImage>>,
    texture: Option<egui::TextureHandle>,
    pub error: Option<String>,
}

impl PartialEq for Preview {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.error == other.error && self.image.is_some() == other.image.is_some()
    }
}

impl std::fmt::Debug for Preview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Preview({}, image: {}, error: {:?})", self.id, self.image.is_some(), self.error)
    }
}

/// The window's tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Extensions,
    Indexes,
}

/// The Extension Manager's state, kept in [`super::ExtensionsState`] for the session only.
#[derive(Clone, Debug, PartialEq)]
pub struct ManagerState {
    pub open: bool,
    pub tab: Tab,
    /// `all`, `installed` or `notInstalled`.
    pub state: String,
    /// `all` or an entry kind.
    pub kind: String,
    pub query: String,
    /// The rows `pluginManager.list` gave for the filters.
    pub entries: Vec<Value>,
    /// The last `pluginManager.reloadIndexes` report: indexes and updates.
    pub indexes: Vec<Value>,
    pub updates: Vec<Value>,
    /// HTTPS indexes downloaded this session, by location.
    pub fetched: BTreeMap<String, Vec<u8>>,
    /// The index list the indexes were last read for, and the extensions' revision the rows
    /// were listed at.
    seen_sources: Option<String>,
    seen_revision: Option<u64>,
    pub preview: Option<Preview>,
    /// The Add field of the Indexes tab.
    pub new_index: String,
    /// The last problem, shown under the list.
    pub note: Option<String>,
    pub platform: Platform,
    pub downloads: Downloads,
}

impl Default for ManagerState {
    fn default() -> Self {
        Self {
            open: false,
            tab: Tab::Extensions,
            state: "all".into(),
            kind: "all".into(),
            query: String::new(),
            entries: Vec::new(),
            indexes: Vec::new(),
            updates: Vec::new(),
            fetched: BTreeMap::new(),
            seen_sources: None,
            seen_revision: None,
            preview: None,
            new_index: String::new(),
            note: None,
            platform: Platform::default(),
            downloads: Downloads::default(),
        }
    }
}

fn state(app: &mut FontcraftApp) -> &mut ManagerState {
    &mut app.state.panels.extensions.manager
}

/// The text of the user's index list.
fn sources_setting(app: &FontcraftApp) -> String {
    app.session.settings().get(SOURCES_SETTING).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    match id {
        "window.pluginManager" => {
            let tab = match args.string("tab")? {
                None | Some("extensions") => Tab::Extensions,
                Some("indexes") => Tab::Indexes,
                Some(other) => return Err(args.err(format!("`tab` is \"extensions\" or \"indexes\", not {other:?}"))),
            };
            let st = state(app);
            st.open = true;
            st.tab = tab;
            reload(app);
            let st = state(app);
            Ok(json!({"entries": st.entries.len(), "indexes": st.indexes.len()}))
        }
        "pluginManager.filterByInstallState" => {
            let value = args.string("state")?.ok_or_else(|| args.err("give `state`"))?;
            if !matches!(value, "all" | "installed" | "notInstalled") {
                return Err(args.err(format!("`state` is \"all\", \"installed\" or \"notInstalled\", not {value:?}")));
            }
            state(app).state = value.to_string();
            refresh(app);
            Ok(json!({"state": value, "count": state(app).entries.len()}))
        }
        "pluginManager.filterByType" => {
            let value = args.string("type")?.ok_or_else(|| args.err("give `type`"))?;
            if !matches!(value, "all" | "filter" | "reporter" | "check" | "macro") {
                return Err(args.err(format!("`type` is \"all\", \"filter\", \"reporter\", \"check\" or \"macro\", not {value:?}")));
            }
            state(app).kind = value.to_string();
            refresh(app);
            Ok(json!({"type": value, "count": state(app).entries.len()}))
        }
        "pluginManager.search" => {
            let query: String = args.string("query")?.unwrap_or_default().chars().take(MAX_QUERY_CHARS).collect();
            state(app).query = query.clone();
            refresh(app);
            Ok(json!({"query": query, "count": state(app).entries.len()}))
        }
        "pluginManager.enlargePreview" => match args.string("id")? {
            None => {
                state(app).preview = None;
                Ok(json!({}))
            }
            Some(entry_id) => {
                let (entry, index) = find_entry(app, entry_id).ok_or_else(|| args.err(format!("no extension index offers {entry_id:?}")))?;
                let shot = entry.screenshot.as_deref().ok_or_else(|| args.err(format!("{entry_id:?} has no screenshot")))?;
                let location = Location::of_source(&index).map_err(|e| args.err(e))?.resolve(shot);
                show_screenshot(app, entry_id, &location)?;
                let shown = match &location {
                    Location::Https(url) => redact(url),
                    Location::Local(path) => path.display().to_string(),
                };
                Ok(json!({"id": entry_id, "screenshot": shown}))
            }
        },
        "pluginManager.openWebsite" | "pluginManager.openDonationPage" => {
            let entry_id = args.string("id")?.ok_or_else(|| args.err("give the entry's `id`"))?;
            let (entry, _) = find_entry(app, entry_id).ok_or_else(|| args.err(format!("no extension index offers {entry_id:?}")))?;
            let (link, what) = if id == WEBSITE.0 { (entry.homepage, "website") } else { (entry.funding, "donation page") };
            let link = link.ok_or_else(|| args.err(format!("{entry_id:?} names no {what}")))?;
            let open = state(app).platform.open_url.ok_or_else(|| args.err("this build cannot open a browser"))?;
            open(&link)?;
            Ok(json!({"id": entry_id, "url": link}))
        }
        other => Err(format!("unknown command `{other}`")),
    }
}

/// The first entry with `id` in the session's indexes (the first index wins), with its index's
/// location.
fn find_entry(app: &FontcraftApp, id: &str) -> Option<(Entry, String)> {
    app.session.extensions().indexes.iter().find_map(|l| l.index.find(id, None).map(|e| (e.clone(), l.source.clone())))
}

/// Lists the extensions again for the filters.
pub fn refresh(app: &mut FontcraftApp) {
    let st = state(app);
    let mut params = json!({"state": st.state});
    if let Some(o) = params.as_object_mut() {
        if st.kind != "all" {
            o.insert("type".into(), json!(st.kind));
        }
        if !st.query.trim().is_empty() {
            o.insert("query".into(), json!(st.query.trim()));
        }
    }
    let listed = app.run("pluginManager.list", &params);
    let revision = app.session.extensions().revision();
    let st = state(app);
    st.seen_revision = Some(revision);
    match listed {
        Ok(v) => st.entries = v.get("entries").and_then(Value::as_array).cloned().unwrap_or_default(),
        Err(e) => st.note = Some(e),
    }
}

/// Reads the indexes again (with the HTTPS ones downloaded so far), starts the downloads of the
/// others and of automatic updates on the network, then lists the extensions.
pub fn reload(app: &mut FontcraftApp) {
    let text = sources_setting(app);
    let sources = parse_sources(&text);
    let st = state(app);
    let fetched: Vec<Value> = sources
        .iter()
        .map(|s| match st.fetched.get(&s.location) {
            Some(bytes) => json!(base64::engine::general_purpose::STANDARD.encode(bytes)),
            None => Value::Null,
        })
        .collect();
    st.seen_sources = Some(text);
    match app.run("pluginManager.reloadIndexes", &json!({"fetched": fetched})) {
        Ok(report) => {
            let st = state(app);
            st.indexes = report.get("indexes").and_then(Value::as_array).cloned().unwrap_or_default();
            st.updates = report.get("updates").and_then(Value::as_array).cloned().unwrap_or_default();
            if let Some(f) = report.get("failed").and_then(Value::as_array).and_then(|f| f.first()) {
                st.note = Some(format!(
                    "{}: {}",
                    f.get("id").and_then(Value::as_str).unwrap_or_default(),
                    f.get("error").and_then(Value::as_str).unwrap_or_default()
                ));
            }
        }
        Err(e) => state(app).note = Some(e),
    }
    for source in &sources {
        if matches!(Location::of_source(&source.location), Ok(Location::Https(_))) && !state(app).fetched.contains_key(&source.location) {
            start(app, Job::Index(source.location.clone()), &source.location, MAX_INDEX_BYTES);
        }
    }
    let network_updates: Vec<(String, String)> = state(app)
        .updates
        .iter()
        .filter(|u| u.get("automatic") == Some(&json!(true)) && u.get("network") == Some(&json!(true)) && u.get("installed") != Some(&json!(true)))
        .filter_map(|u| Some((u.get("id")?.as_str()?.to_string(), u.get("version")?.as_str()?.to_string())))
        .collect();
    if !app.session.extensions().safe_mode {
        for (id, version) in network_updates {
            install_entry(app, &id, Some(&version));
        }
    }
    refresh(app);
}

/// Installs the entry `id` (in `version`, else the first listed): directly when its file is
/// local, else after downloading it. A module that is off stays off.
pub fn install_entry(app: &mut FontcraftApp, id: &str, version: Option<&str>) {
    let found = app.session.extensions().indexes.iter().find_map(|l| l.index.find(id, version).map(|e| (e.clone(), l.source.clone())));
    let Some((entry, index)) = found else {
        state(app).note = Some(format!("no extension index offers {id:?}"));
        return;
    };
    let keep_off = app.session.extensions().registry.get(id).is_some_and(|m| !m.enabled);
    match Location::of_source(&index).map(|l| l.resolve(&entry.url)) {
        Ok(Location::Https(url)) => {
            let max = usize::try_from(entry.size).unwrap_or(MAX_ENTRY_BYTES).min(MAX_ENTRY_BYTES);
            start(app, Job::Install { id: entry.id.clone(), version: entry.version.clone(), keep_off }, &url, max);
        }
        Ok(Location::Local(_)) => {
            let done = app.run("pluginManager.install", &json!({"id": entry.id, "version": entry.version}));
            finish_install(app, &entry.id, keep_off, done);
        }
        Err(e) => state(app).note = Some(e),
    }
}

fn finish_install(app: &mut FontcraftApp, id: &str, keep_off: bool, done: Result<Value, String>) {
    let done = done.and_then(|_| if keep_off { app.run("plugin.setEnabled", &json!({"id": id, "enabled": false})) } else { Ok(json!({})) });
    if let Err(e) = done {
        state(app).note = Some(e);
    }
    refresh(app);
}

/// Shows the screenshot at `location`: read now when local, else downloaded.
fn show_screenshot(app: &mut FontcraftApp, id: &str, location: &Location) -> Result<(), String> {
    state(app).preview = Some(Preview { id: id.to_string(), ..Default::default() });
    match location {
        Location::Local(path) => {
            let bytes = fontcraft_engine::extensions::read_module_file(path).map_err(|e| format!("{}: {e}", path.display()))?;
            take_screenshot(state(app), id, Ok(bytes));
        }
        Location::Https(url) => start(app, Job::Screenshot(id.to_string()), url, MAX_SCREENSHOT_BYTES),
    }
    match state(app).preview.as_ref().and_then(|p| p.error.clone()) {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

fn take_screenshot(st: &mut ManagerState, id: &str, bytes: Result<Vec<u8>, String>) {
    let Some(preview) = st.preview.as_mut().filter(|p| p.id == id) else { return };
    let decoded = bytes.and_then(|b| {
        if b.len() > MAX_SCREENSHOT_BYTES {
            return Err(format!("the screenshot is larger than {} MiB", MAX_SCREENSHOT_BYTES >> 20));
        }
        fontcraft_engine::imagery::decode_rgba(&b, fontcraft_engine::imagery::MAX_PIXELS).map_err(|e| e.to_string())
    });
    match decoded.and_then(|image| {
        let size = [usize::try_from(image.width).map_err(|e| e.to_string())?, usize::try_from(image.height).map_err(|e| e.to_string())?];
        Ok(egui::ColorImage::from_rgba_unmultiplied(size, &image.pixels))
    }) {
        Ok(image) => preview.image = Some(Arc::new(image)),
        Err(e) => preview.error = Some(format!("the screenshot cannot be shown: {e}")),
    }
}

/// Starts downloading `url` on a background thread, unless the same job runs already.
fn start(app: &mut FontcraftApp, job: Job, url: &str, max: usize) {
    let st = state(app);
    let Some(fetch) = st.platform.fetch else {
        st.note = Some(format!("{} is on the network, and this build cannot download", redact(url)));
        if let Job::Screenshot(_) = &job
            && let Some(p) = st.preview.as_mut()
        {
            p.error = st.note.clone();
        }
        return;
    };
    let downloads = st.downloads.clone();
    let admitted = downloads.with(|i| {
        if i.running.contains(&job) || i.running.len() >= MAX_DOWNLOADS {
            return false;
        }
        i.running.insert(job.clone());
        true
    });
    if !admitted {
        return;
    }
    let url = url.to_string();
    let work = move || {
        let result = fetch(&url, max, LANGUAGE);
        downloads.with(|i| {
            i.running.remove(&job);
            i.done.push((job, result));
        });
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        // A platform that cannot start a thread runs the download here instead.
        let work = Arc::new(Mutex::new(Some(work)));
        let in_thread = work.clone();
        let spawned = std::thread::Builder::new().name("extension download".into()).spawn(move || {
            if let Some(w) = in_thread.lock().ok().and_then(|mut w| w.take()) {
                w();
            }
        });
        if spawned.is_err()
            && let Some(w) = work.lock().ok().and_then(|mut w| w.take())
        {
            w();
        }
    }
    #[cfg(target_arch = "wasm32")]
    work();
}

/// Handles the downloads that finished: reads the indexes again when one arrived, installs
/// downloaded entry files, shows screenshots. Returns whether anything finished.
pub fn poll(app: &mut FontcraftApp) -> bool {
    let done = state(app).downloads.with(|i| std::mem::take(&mut i.done));
    if done.is_empty() {
        return false;
    }
    let mut indexes_arrived = false;
    for (job, result) in done {
        match job {
            Job::Index(location) => match result {
                Ok(bytes) => {
                    state(app).fetched.insert(location, bytes);
                    indexes_arrived = true;
                }
                Err(e) => state(app).note = Some(format!("{}: {e}", redact(&location))),
            },
            Job::Install { id, version, keep_off } => {
                let done = result.and_then(|bytes| {
                    let data = base64::engine::general_purpose::STANDARD.encode(bytes);
                    app.run("pluginManager.install", &json!({"id": id, "version": version, "dataBase64": data}))
                });
                finish_install(app, &id, keep_off, done);
            }
            Job::Screenshot(id) => take_screenshot(state(app), &id, result),
        }
    }
    if indexes_arrived {
        reload(app);
    }
    true
}

/// Writes the index list and reads the indexes again.
fn set_sources(app: &mut FontcraftApp, sources: &[IndexSource]) {
    let text = sources_text(sources);
    match app.run("settings.set", &json!({"id": SOURCES_SETTING, "value": text})) {
        Ok(_) => reload(app),
        Err(e) => state(app).note = Some(e),
    }
}

enum Event {
    Run(&'static str, Value),
    Engine(&'static str, Value),
    Install(String, Option<String>),
    Tab(Tab),
    Sources(Vec<IndexSource>),
    Reload,
    ClosePreview,
}

/// Draws the window while it is open, after handling finished downloads and changes made
/// elsewhere (an install from the command line, a changed index list).
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    if !state(app).open {
        return;
    }
    poll(app);
    let (text, revision) = (sources_setting(app), app.session.extensions().revision());
    let st = &app.state.panels.extensions.manager;
    if st.seen_sources.as_deref() != Some(text.as_str()) {
        reload(app);
    } else if st.seen_revision != Some(revision) {
        refresh(app);
    }
    if state(app).downloads.busy() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(DOWNLOAD_POLL_MS));
    }
    let sources = parse_sources(&sources_setting(app));
    let folders = {
        let ext = app.session.extensions();
        let shown = |p: &Option<std::path::PathBuf>| p.as_ref().map_or_else(|| "none in this session".to_string(), |p| p.display().to_string());
        (shown(&ext.modules_dir), shown(&ext.macros_dir), ext.safe_mode)
    };
    let log_to_stderr = app.session.settings().get_bool("addons.useSystemConsoleForScriptOutput").unwrap_or(false);
    let FontcraftApp { state: ui_state, tokens, .. } = app;
    let st = &mut ui_state.panels.extensions.manager;
    let mut events = Vec::new();
    let mut open = true;
    egui::Window::new("Extension Manager")
        .id(egui::Id::new("window.pluginManager"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(tokens.dialog_spacing * 80.0)
        .pivot(Align2::CENTER_TOP)
        .default_pos(egui::pos2(ui.ctx().content_rect().center().x, tokens.dialog_top))
        .show(ui.ctx(), |ui| {
            ui.horizontal(|ui| {
                if ui.selectable_label(st.tab == Tab::Extensions, "Extensions").clicked() {
                    events.push(Event::Tab(Tab::Extensions));
                }
                if ui.selectable_label(st.tab == Tab::Indexes, "Indexes").clicked() {
                    events.push(Event::Tab(Tab::Indexes));
                }
            });
            ui.separator();
            match st.tab {
                Tab::Extensions => extensions_tab(ui, st, tokens, folders.2, &mut events),
                Tab::Indexes => indexes_tab(ui, st, tokens, &sources, (&folders.0, &folders.1), log_to_stderr, &mut events),
            }
            if let Some(note) = &st.note {
                ui.add_space(tokens.dialog_spacing);
                ui.label(RichText::new(note.as_str()).color(tokens.error_text));
            }
        });
    if let Some(preview) = &mut st.preview {
        let mut preview_open = true;
        egui::Window::new(format!("Preview: {}", preview.id))
            .id(egui::Id::new("pluginManager.enlargePreview"))
            .open(&mut preview_open)
            .collapsible(false)
            .resizable(true)
            .show(ui.ctx(), |ui| {
                if let Some(e) = &preview.error {
                    ui.label(RichText::new(e.as_str()).color(tokens.error_text));
                } else if let Some(image) = &preview.image {
                    let texture = preview.texture.get_or_insert_with(|| {
                        ui.ctx().load_texture(format!("pluginManager.preview.{}", preview.id), (**image).clone(), egui::TextureOptions::LINEAR)
                    });
                    egui::ScrollArea::both().show(ui, |ui| ui.image((texture.id(), texture.size_vec2())));
                } else {
                    ui.label(RichText::new("Downloading…").color(tokens.text_dim));
                }
            });
        if !preview_open {
            events.push(Event::ClosePreview);
        }
    }
    if !open {
        st.open = false;
        st.preview = None;
    }
    for event in events {
        match event {
            Event::Run(id, params) | Event::Engine(id, params) => pending.push((id, params)),
            Event::Install(id, version) => install_entry(app, &id, version.as_deref()),
            Event::Tab(tab) => state(app).tab = tab,
            Event::Sources(list) => set_sources(app, &list),
            Event::Reload => {
                state(app).fetched.clear();
                reload(app);
            }
            Event::ClosePreview => state(app).preview = None,
        }
    }
}

fn text<'a>(row: &'a Value, key: &str) -> &'a str {
    row.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn extensions_tab(ui: &mut egui::Ui, st: &mut ManagerState, tokens: &crate::theme::Tokens, safe_mode: bool, events: &mut Vec<Event>) {
    ui.horizontal_wrapped(|ui| {
        for (value, label) in [("all", "All"), ("installed", "Installed"), ("notInstalled", "Not Installed")] {
            if ui.selectable_label(st.state == value, label).clicked() {
                events.push(Event::Run(FILTER_STATE.0, json!({"state": value})));
            }
        }
        ui.separator();
        for (value, label) in [("all", "All"), ("filter", "Filters"), ("reporter", "Reporters"), ("check", "Checks"), ("macro", "Macros")] {
            if ui.selectable_label(st.kind == value, label).clicked() {
                events.push(Event::Run(FILTER_TYPE.0, json!({"type": value})));
            }
        }
        ui.separator();
        let mut query = st.query.clone();
        if ui.add(egui::TextEdit::singleline(&mut query).hint_text("Search")).changed() {
            events.push(Event::Run(SEARCH.0, json!({"query": query})));
        }
    });
    if safe_mode {
        ui.label(RichText::new("Safe mode: modules are listed but none runs until Help › Load Extensions.").color(tokens.text_dim));
    }
    ui.add_space(tokens.dialog_spacing);
    if st.entries.is_empty() {
        let hint = if st.indexes.is_empty() { "No extensions. Add an extension index in the Indexes tab." } else { "No extension matches." };
        ui.label(RichText::new(hint).color(tokens.text_dim));
        return;
    }
    egui::ScrollArea::vertical().id_salt("pluginManager.entries").max_height(tokens.list_row_height * 24.0).auto_shrink([false, true]).show(
        ui,
        |ui| {
            for row in &st.entries {
                entry_row(ui, row, tokens, events);
                ui.separator();
            }
        },
    );
}

fn entry_row(ui: &mut egui::Ui, row: &Value, tokens: &crate::theme::Tokens, events: &mut Vec<Event>) {
    let id = text(row, "id").to_string();
    let flag = |key: &str| row.get(key).and_then(Value::as_bool).unwrap_or(false);
    let from_index = text(row, "source") == "index";
    ui.horizontal_wrapped(|ui| {
        ui.strong(text(row, "title"));
        let mut kind = format!("{} {}", text(row, "kind"), text(row, "version"));
        if flag("update") {
            kind = format!("{kind} (installed: {})", text(row, "installedVersion"));
        }
        ui.label(RichText::new(kind).color(tokens.text_dim));
        if flag("new") {
            ui.label(RichText::new("new").color(tokens.accent));
        }
    });
    if !text(row, "vendor").is_empty() {
        ui.label(RichText::new(text(row, "vendor")).color(tokens.text_dim));
    }
    if !text(row, "description").is_empty() {
        ui.label(text(row, "description"));
    }
    let size = row.get("size").and_then(Value::as_u64);
    let sha: String = text(row, "sha256").chars().take(SHORT_SHA).collect();
    if let Some(size) = size {
        ui.label(RichText::new(format!("{id} · {size} bytes · SHA-256 {sha}…")).small().color(tokens.text_dim));
    }
    ui.horizontal_wrapped(|ui| {
        if from_index && ui.small_button("Website").clicked() {
            events.push(Event::Run(WEBSITE.0, json!({"id": id})));
        }
        if from_index && ui.small_button("Donate").clicked() {
            events.push(Event::Run(DONATE.0, json!({"id": id})));
        }
        if from_index && ui.small_button("Preview").clicked() {
            events.push(Event::Run(ENLARGE.0, json!({"id": id})));
        }
        let installed = flag("installed");
        if from_index && !installed && ui.button("Install").clicked() {
            events.push(Event::Install(id.clone(), None));
        }
        if from_index && flag("update") && ui.button("Update").clicked() {
            events.push(Event::Install(id.clone(), Some(text(row, "version").to_string())));
        }
        if installed && ui.button("Remove").clicked() {
            events.push(Event::Engine("pluginManager.remove", json!({"id": id})));
        }
        if let Some(mut on) = row.get("enabled").and_then(Value::as_bool)
            && ui.checkbox(&mut on, "On").changed()
        {
            events.push(Event::Engine("plugin.setEnabled", json!({"id": id, "enabled": on})));
        }
    });
}

fn indexes_tab(
    ui: &mut egui::Ui,
    st: &mut ManagerState,
    tokens: &crate::theme::Tokens,
    sources: &[IndexSource],
    folders: (&str, &str),
    log_to_stderr: bool,
    events: &mut Vec<Event>,
) {
    ui.label(format!("Modules folder: {}", folders.0));
    ui.label(format!("Macros folder: {}", folders.1));
    let mut log = log_to_stderr;
    if ui.checkbox(&mut log, "Also write the extension log to standard error").changed() {
        events.push(Event::Engine("settings.set", json!({"id": "addons.useSystemConsoleForScriptOutput", "value": log})));
    }
    ui.add_space(tokens.dialog_spacing);
    ui.strong("Extension indexes");
    if sources.is_empty() {
        ui.label(RichText::new("No index yet: add a full path to an index file or an https:// URL.").color(tokens.text_dim));
    }
    for (i, source) in sources.iter().enumerate() {
        let shown = redact(&source.location);
        let report = st.indexes.iter().find(|r| text(r, "source") == shown);
        ui.horizontal_wrapped(|ui| {
            ui.label(shown.as_str());
            match report {
                Some(r) if r.get("error").is_some() => {
                    ui.label(RichText::new(text(r, "error")).color(tokens.error_text));
                }
                Some(r) => {
                    ui.label(RichText::new(format!("{} entries", r.get("entries").and_then(Value::as_u64).unwrap_or(0))).color(tokens.text_dim));
                }
                None => {}
            }
            let mut automatic = source.automatic_updates;
            if ui.checkbox(&mut automatic, "Automatic updates").changed() {
                let mut list = sources.to_vec();
                if let Some(s) = list.get_mut(i) {
                    s.automatic_updates = automatic;
                }
                events.push(Event::Sources(list));
            }
            if ui.small_button("Remove").clicked() {
                let list = sources.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, s)| s.clone()).collect();
                events.push(Event::Sources(list));
            }
        });
    }
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut st.new_index).hint_text("/path/to/index.json or https://…"));
        if ui.button("Add").clicked() && !st.new_index.trim().is_empty() {
            let mut list = sources.to_vec();
            list.extend(parse_sources(&st.new_index));
            st.new_index.clear();
            events.push(Event::Sources(list));
        }
        if ui.button("Reload").clicked() {
            events.push(Event::Reload);
        }
    });
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;
    use fontcraft_engine::extensions;

    const MACRO: &[u8] = br#"{"format": "fontcraft.macro", "version": 1, "title": "Tighten", "description": "Tightens spacing.", "steps": []}"#;

    fn entry(id: &str, url: &str, bytes: &[u8]) -> Value {
        json!({
            "id": id, "kind": "macro", "version": "1", "titles": {"en": id}, "descriptions": {"en": format!("{id} by the test foundry")},
            "vendor": "Test Foundry", "url": url, "size": bytes.len(),
            "sha256": fontcraft_engine::extension_index::checksum(bytes), "homepage": "https://example.com/home",
        })
    }

    fn index(entries: Vec<Value>) -> Vec<u8> {
        serde_json::to_vec(&json!({"format": "fontcraft.extensions", "version": 1, "entries": entries})).unwrap()
    }

    fn app_with_folders(tag: &str) -> (FontcraftApp, std::path::PathBuf) {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let data = std::env::temp_dir().join(format!("fontcraft-ui-manager-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&data).unwrap();
        let mut app = FontcraftApp::default();
        assert!(extensions::open_folders(app.session.extensions_mut(), Some(&data), false).is_empty());
        (app, data)
    }

    fn opened_ok(_: &str) -> Result<(), String> {
        Ok(())
    }

    #[test]
    fn extension_manager_filters_searches_and_installs_from_a_local_index() {
        let (mut app, data) = app_with_folders("local");
        std::fs::write(data.join("tighten.fcmacro"), MACRO).unwrap();
        let path = data.join("index.json");
        std::fs::write(&path, index(vec![entry("tighten", "tighten.fcmacro", MACRO), entry("loosen", "loosen.fcmacro", MACRO)])).unwrap();
        app.run("settings.set", &json!({"id": SOURCES_SETTING, "value": path.display().to_string()})).unwrap();

        let opened = app.run("window.pluginManager", &json!({})).unwrap();
        assert_eq!((opened["entries"].as_u64(), opened["indexes"].as_u64()), (Some(2), Some(1)));
        assert_eq!(app.run("pluginManager.filterByType", &json!({"type": "filter"})).unwrap()["count"], 0);
        assert_eq!(app.run("pluginManager.filterByType", &json!({"type": "macro"})).unwrap()["count"], 2);
        assert_eq!(app.run("pluginManager.search", &json!({"query": "TIGHT"})).unwrap()["count"], 1);
        assert!(app.run("pluginManager.filterByType", &json!({"type": "plugins"})).is_err());

        install_entry(&mut app, "tighten", None);
        assert!(data.join("Macros").join("tighten.fcmacro").exists(), "{:?}", state(&mut app).note);
        assert_eq!(app.run("pluginManager.filterByInstallState", &json!({"state": "installed"})).unwrap()["count"], 1);
        assert_eq!(app.run("pluginManager.filterByInstallState", &json!({"state": "notInstalled"})).unwrap()["count"], 0);
        app.run("pluginManager.search", &json!({"query": ""})).unwrap();
        assert_eq!(app.run("pluginManager.filterByInstallState", &json!({"state": "notInstalled"})).unwrap()["count"], 1);

        assert!(app.run("pluginManager.openWebsite", &json!({"id": "tighten"})).unwrap_err().contains("browser"));
        state(&mut app).platform.open_url = Some(opened_ok);
        assert_eq!(app.run("pluginManager.openWebsite", &json!({"id": "tighten"})).unwrap()["url"], "https://example.com/home");
        assert!(app.run("pluginManager.openDonationPage", &json!({"id": "tighten"})).unwrap_err().contains("donation"));
        assert!(app.run("pluginManager.enlargePreview", &json!({"id": "tighten"})).unwrap_err().contains("screenshot"));
    }

    /// The pretend network: URL → body.
    static NETWORK: Mutex<BTreeMap<String, Vec<u8>>> = Mutex::new(BTreeMap::new());

    fn fake_fetch(url: &str, max: usize, _language: &str) -> Result<Vec<u8>, String> {
        let body = NETWORK.lock().unwrap().get(url).cloned().ok_or_else(|| format!("{url}: not found"))?;
        if body.len() > max { Err("too large".into()) } else { Ok(body) }
    }

    fn wait_for_downloads(app: &mut FontcraftApp) {
        for _ in 0..500 {
            poll(app);
            if !state(app).downloads.busy() && state(app).downloads.with(|i| i.done.is_empty()) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the downloads did not finish");
    }

    #[test]
    fn extension_manager_downloads_https_indexes_and_entries() {
        let (mut app, data) = app_with_folders("https");
        let base = format!("https://user:pw@example.test/{}", data.file_name().unwrap().to_string_lossy());
        NETWORK.lock().unwrap().insert(format!("{base}/index.json"), index(vec![entry("tighten", "files/tighten.fcmacro", MACRO)]));
        NETWORK.lock().unwrap().insert(format!("{base}/files/tighten.fcmacro"), MACRO.to_vec());
        state(&mut app).platform.fetch = Some(fake_fetch);
        app.run("settings.set", &json!({"id": SOURCES_SETTING, "value": format!("{base}/index.json")})).unwrap();

        app.run("window.pluginManager", &json!({})).unwrap();
        wait_for_downloads(&mut app);
        let st = state(&mut app);
        assert_eq!(st.indexes[0]["source"], format!("https://example.test/{}/index.json", data.file_name().unwrap().to_string_lossy()));
        assert_eq!(st.entries.len(), 1, "{:?}", st.indexes);
        install_entry(&mut app, "tighten", None);
        wait_for_downloads(&mut app);
        assert!(data.join("Macros").join("tighten.fcmacro").exists(), "{:?}", state(&mut app).note);
        assert_eq!(state(&mut app).entries[0]["installed"], true);
    }

    #[test]
    fn extension_manager_draws_both_tabs() {
        let (mut app, data) = app_with_folders("frame");
        let path = data.join("index.json");
        std::fs::write(&path, index(vec![entry("tighten", "tighten.fcmacro", MACRO)])).unwrap();
        app.run("settings.set", &json!({"id": SOURCES_SETTING, "value": format!("manual {}", path.display())})).unwrap();
        app.run("window.pluginManager", &json!({})).unwrap();
        let ctx = egui::Context::default();
        for tab in ["extensions", "indexes"] {
            app.run("window.pluginManager", &json!({"tab": tab})).unwrap();
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                let mut pending = Vec::new();
                show(&mut app, ui, &mut pending);
            });
        }
        assert!(state(&mut app).open);
        assert_eq!(state(&mut app).indexes[0]["automaticUpdates"], false);
    }
}
