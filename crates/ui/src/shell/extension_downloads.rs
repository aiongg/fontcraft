//! The Extension Manager's network side: HTTPS extension indexes, entry files and screenshots, downloaded on the
//! host's jobs (a thread each on the desktop) through the fetch the desktop app supplies (`Options::fetch`). Without a
//! fetch (the browser build, tests that set none) only local indexes and files are read, and an entry or index on the
//! network says this build cannot download it.
//!
//! | When | What happens |
//! |---|---|
//! | `pluginManager.reloadIndexes` ran ([`after_run`]) | its report is kept for the Indexes tab; the HTTPS indexes not yet downloaded start downloading, and the automatic updates whose files are on the network are installed. A run without `fetched` (Reload) forgets the downloaded indexes first |
//! | an index arrived ([`poll`]) | `pluginManager.reloadIndexes` is posted again with every index downloaded so far, by position (`fetched`), so no URL with credentials goes into the parameters |
//! | Install or Update of an entry ([`install_entry`]) | a local file installs with `pluginManager.install {id, version}`; a file on the network is downloaded, then installed with `dataBase64`, which the engine checks against the entry's size and SHA-256 before writing. A module that was off stays off |
//! | `pluginManager.enlargePreview` ([`show_screenshot`]) | the screenshot is read or downloaded and decoded on the job, then shown in the window |
//! | the index list changed in the settings ([`step`]) | the indexes are read again while the Extension Manager or Settings › Extensions shows |
//!
//! Every engine command goes through `app.sent`, so `drain` runs it. Index URLs are shown redacted
//! (`extension_index::redact`) in every message.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};

use base64::Engine as _;
use egui::ColorImage;
use fontcraft_engine::extension_index::{Location, MAX_INDEX_BYTES, MAX_INDEXES, MAX_SCREENSHOT_BYTES, SOURCES_SETTING, parse_sources, redact};
use fontcraft_engine::imagery;
use serde_json::{Value, json};

use crate::app::{FontCraftApp, Msg};
use crate::shell::extension_manager::{Shot, find_entry};

/// A blocking HTTPS GET: the URL (which may carry credentials), the largest body accepted, and the language for
/// `Accept-Language`. The error text never holds the credentials.
pub type FetchFn = fn(&str, usize, &str) -> Result<Vec<u8>, String>;

/// The interface language sent with downloads.
const LANGUAGE: &str = "en";
/// Most downloads running at once.
pub const MAX_DOWNLOADS: usize = 8;
/// Largest entry file downloaded: the module limit.
pub const MAX_ENTRY_BYTES: usize = 32 << 20;
/// Most pixels of a screenshot shown.
pub const MAX_SCREENSHOT_PIXELS: u64 = 1 << 24;
/// Most automatic updates started by one reading of the indexes.
const MAX_UPDATES: usize = 256;
/// Longest problem text kept, in characters.
const MAX_PROBLEM_CHARS: usize = 500;

/// A download, as the running set knows it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Job {
    /// An HTTPS index, by its location in the list.
    Index(String),
    /// An entry's file, by entry id.
    Install(String),
    /// An entry's screenshot, by entry id.
    Screenshot(String),
}

/// A finished download.
enum Done {
    Index(String, Result<Vec<u8>, String>),
    /// `keep_off` puts back a module's off switch after the install.
    Install {
        id: String,
        version: String,
        keep_off: bool,
        bytes: Result<Vec<u8>, String>,
    },
    Screenshot(String, Result<Arc<ColorImage>, String>),
}

#[derive(Default)]
struct Inbox {
    running: BTreeSet<Job>,
    done: Vec<Done>,
    /// HTTPS indexes downloaded this session, by location.
    fetched: BTreeMap<String, Vec<u8>>,
    /// HTTPS indexes whose download failed since the last Reload; not tried again until then.
    failed: BTreeSet<String>,
}

/// The downloads, shared with their jobs, and the fetch that runs them. Not part of the view state: two compare equal
/// whatever they hold, and a clone shares the same downloads.
#[derive(Clone, Default)]
pub struct Downloads {
    /// The desktop app's HTTPS fetch; `None` reads local files only.
    pub fetch: Option<FetchFn>,
    inbox: Arc<Mutex<Inbox>>,
}

impl Downloads {
    fn with<T>(&self, f: impl FnOnce(&mut Inbox) -> T) -> T {
        let mut inbox = self.inbox.lock().unwrap_or_else(PoisonError::into_inner);
        f(&mut inbox)
    }

    /// Whether any download is running.
    pub fn busy(&self) -> bool {
        self.with(|i| !i.running.is_empty())
    }

    /// Whether the HTTPS index at `location` is downloading.
    pub fn index_running(&self, location: &str) -> bool {
        self.with(|i| i.running.contains(&Job::Index(location.to_string())))
    }

    /// Whether the entry `id`'s file is downloading.
    pub fn install_running(&self, id: &str) -> bool {
        self.with(|i| i.running.contains(&Job::Install(id.to_string())))
    }
}

impl PartialEq for Downloads {
    fn eq(&self, _: &Downloads) -> bool {
        true
    }
}

impl std::fmt::Debug for Downloads {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Downloads(fetch: {}, busy: {})", self.fetch.is_some(), self.busy())
    }
}

/// The text of the user's index list.
pub fn sources_setting(app: &FontCraftApp) -> String {
    app.session.settings().get(SOURCES_SETTING).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

/// Puts `text` under the Extension Manager's lists and in the status line.
fn problem(app: &mut FontCraftApp, text: String) {
    let text: String = text.chars().take(MAX_PROBLEM_CHARS).collect();
    app.status = text.clone();
    let view = &mut app.view.extensions.manager;
    view.problem = Some(text);
    view.touch();
}

/// The parameters of `pluginManager.reloadIndexes` with the HTTPS indexes downloaded so far, by position in the
/// list, and records the list they were made for.
pub fn reload_params(app: &mut FontCraftApp) -> Value {
    let text = sources_setting(app);
    let sources = parse_sources(&text);
    let fetched: Vec<Value> = app.view.extensions.downloads.with(|i| {
        sources
            .iter()
            .take(MAX_INDEXES)
            .map(|s| i.fetched.get(&s.location).map_or(Value::Null, |b| json!(base64::engine::general_purpose::STANDARD.encode(b))))
            .collect()
    });
    app.view.extensions.manager.seen_sources = Some(text);
    json!({ "fetched": fetched })
}

/// Posts a reading of the indexes with the HTTPS ones downloaded so far.
pub fn queue_reload(app: &mut FontCraftApp) {
    let params = reload_params(app);
    app.sent.push(Msg::run("pluginManager.reloadIndexes", params));
}

/// Takes finished downloads, then reads the indexes again when the index list changed while a window shows it.
/// Runs every pass, so downloads finish while the windows are closed too.
pub fn step(app: &mut FontCraftApp) {
    poll(app);
    let settings = &app.view.settings;
    let shown = app.view.extensions.manager.open || (settings.open && crate::shell::settings::current(settings) == "addons");
    if shown && app.view.extensions.manager.seen_sources.as_deref() != Some(sources_setting(app).as_str()) {
        queue_reload(app);
    }
}

/// After an engine command ran: a reading of the indexes keeps its report and starts the downloads it calls for.
pub fn after_run(app: &mut FontCraftApp, id: &str, params: &Value, reply: &Value) {
    if id != "pluginManager.reloadIndexes" {
        return;
    }
    let reported = reply.get("indexes").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let view = &mut app.view.extensions.manager;
    view.reported = reported
        .iter()
        .take(MAX_INDEXES)
        .map(|r| {
            let source = r.get("source").and_then(Value::as_str).unwrap_or_default().to_string();
            let entries = r.get("entries").and_then(Value::as_u64);
            let error = r.get("error").and_then(Value::as_str).map(|e| e.chars().take(MAX_PROBLEM_CHARS).collect());
            (source, entries, error)
        })
        .collect();
    view.invalidate();
    view.touch();
    if let Some(f) = reply.get("failed").and_then(Value::as_array).and_then(|f| f.first()) {
        let text = |key: &str| f.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
        problem(app, format!("{}: {}", text("id"), text("error")));
    }
    // A list given in the parameters is not the user's: its indexes are not downloaded.
    if params.get("indexes").is_some() {
        return;
    }
    if params.get("fetched").is_none() {
        app.view.extensions.downloads.with(|i| {
            i.fetched.clear();
            i.failed.clear();
        });
    }
    let sources = parse_sources(&sources_setting(app));
    for source in sources.iter().take(MAX_INDEXES) {
        let location = &source.location;
        let wanted = app.view.extensions.downloads.with(|i| !i.fetched.contains_key(location) && !i.failed.contains(location));
        if wanted && let Ok(Location::Https(url)) = Location::of_source(location) {
            start_index(app, location, &url);
        }
    }
    if app.session.extensions().safe_mode {
        return;
    }
    let updates = reply.get("updates").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let network: Vec<(String, String)> = updates
        .iter()
        .filter(|u| u.get("automatic") == Some(&json!(true)) && u.get("network") == Some(&json!(true)) && u.get("installed") != Some(&json!(true)))
        .filter_map(|u| Some((u.get("id")?.as_str()?.to_string(), u.get("version")?.as_str()?.to_string())))
        .take(MAX_UPDATES)
        .collect();
    for (id, version) in network {
        install_entry(app, &id, Some(&version));
    }
}

/// Installs the entry `id` (in `version`, else the first listed): at once when its file is local, else after
/// downloading it. A module that is off stays off.
pub fn install_entry(app: &mut FontCraftApp, id: &str, version: Option<&str>) {
    let found = app.session.extensions().indexes.iter().find_map(|l| l.index.find(id, version).map(|e| (e.clone(), l.source.clone())));
    let Some((entry, index)) = found else {
        problem(app, format!("no extension index offers {id:?}"));
        return;
    };
    let keep_off = app.session.extensions().registry.get(id).is_some_and(|m| !m.enabled);
    match Location::of_source(&index).map(|l| l.resolve(&entry.url)) {
        Ok(Location::Https(url)) => {
            let Some(fetch) = network(app, &url) else { return };
            let max = usize::try_from(entry.size).unwrap_or(MAX_ENTRY_BYTES).min(MAX_ENTRY_BYTES);
            let (id, version) = (entry.id.clone(), entry.version.clone());
            spawn(app, Job::Install(entry.id), move || Done::Install { id, version, keep_off, bytes: fetch(&url, max, LANGUAGE) });
        }
        Ok(Location::Local(_)) => post_install(app, &entry.id, &entry.version, None, keep_off),
        Err(e) => problem(app, e),
    }
}

/// Posts the install of `id` (with the downloaded bytes when there are some) and, for a module that was off, its
/// switch back off.
fn post_install(app: &mut FontCraftApp, id: &str, version: &str, bytes: Option<Vec<u8>>, keep_off: bool) {
    let mut params = json!({ "id": id, "version": version });
    if let (Some(bytes), Some(o)) = (bytes, params.as_object_mut()) {
        o.insert("dataBase64".into(), json!(base64::engine::general_purpose::STANDARD.encode(bytes)));
    }
    app.sent.push(Msg::run("pluginManager.install", params));
    if keep_off {
        app.sent.push(Msg::run("plugin.setEnabled", json!({ "id": id, "enabled": false })));
    }
    app.view.extensions.manager.invalidate();
}

/// Reads or downloads the screenshot of the entry `id` and shows it in the window when it is decoded.
pub fn show_screenshot(app: &mut FontCraftApp, id: &str) -> Result<(), String> {
    let (entry, index) = find_entry(&app.session, id).ok_or_else(|| format!("no extension index offers {id:?}"))?;
    let shot = entry.screenshot.as_deref().ok_or_else(|| format!("{id:?} has no screenshot"))?;
    let location = Location::of_source(&index)?.resolve(shot);
    let fetch = app.view.extensions.downloads.fetch;
    if let Location::Https(url) = &location
        && fetch.is_none()
    {
        return Err(format!("{} is on the network, and this build cannot download", redact(url)));
    }
    let view = &mut app.view.extensions.manager;
    view.preview = Some(Shot { id: id.to_string(), ..Shot::default() });
    view.touch();
    let id = id.to_string();
    let job = Job::Screenshot(id.clone());
    spawn(app, job, move || {
        let bytes = match (&location, fetch) {
            (Location::Https(url), Some(fetch)) => fetch(url, MAX_SCREENSHOT_BYTES, LANGUAGE),
            (Location::Https(url), None) => Err(format!("{} is on the network, and this build cannot download", redact(url))),
            (Location::Local(path), _) => fontcraft_engine::extensions::read_module_file(path).map_err(|e| format!("{}: {e}", path.display())),
        };
        let image = bytes.and_then(|b| fontcraft_engine::guard::catch_panic(|| decode_screenshot(&b)).unwrap_or_else(Err));
        Done::Screenshot(id, image)
    });
    Ok(())
}

/// The screenshot's pixels, refused beyond [`MAX_SCREENSHOT_BYTES`] or [`MAX_SCREENSHOT_PIXELS`].
fn decode_screenshot(bytes: &[u8]) -> Result<Arc<ColorImage>, String> {
    if bytes.len() > MAX_SCREENSHOT_BYTES {
        return Err(format!("the screenshot is larger than {} MiB", MAX_SCREENSHOT_BYTES >> 20));
    }
    let image = imagery::decode_rgba(bytes, MAX_SCREENSHOT_PIXELS).map_err(|e| format!("the screenshot cannot be shown: {e}"))?;
    let (w, h) = (usize::try_from(image.width).map_err(|e| e.to_string())?, usize::try_from(image.height).map_err(|e| e.to_string())?);
    if w.checked_mul(h).and_then(|n| n.checked_mul(4)) != Some(image.pixels.len()) {
        return Err("the screenshot cannot be shown: its pixels do not match its size".into());
    }
    Ok(Arc::new(ColorImage::from_rgba_unmultiplied([w, h], &image.pixels)))
}

/// The fetch, or `None` after saying that this build cannot download `url`.
fn network(app: &mut FontCraftApp, url: &str) -> Option<FetchFn> {
    let fetch = app.view.extensions.downloads.fetch;
    if fetch.is_none() {
        problem(app, format!("{} is on the network, and this build cannot download", redact(url)));
    }
    fetch
}

fn start_index(app: &mut FontCraftApp, location: &str, url: &str) {
    let Some(fetch) = network(app, url) else { return };
    let (location, url) = (location.to_string(), url.to_string());
    spawn(app, Job::Index(location.clone()), move || Done::Index(location, fetch(&url, MAX_INDEX_BYTES, LANGUAGE)));
}

/// Runs `work` on the host's jobs unless the same job runs already or [`MAX_DOWNLOADS`] run; the window wakes when it
/// is done.
fn spawn(app: &mut FontCraftApp, job: Job, work: impl FnOnce() -> Done + Send + 'static) {
    let downloads = app.view.extensions.downloads.clone();
    let admitted = downloads.with(|i| {
        if i.running.contains(&job) || i.running.len() >= MAX_DOWNLOADS {
            return false;
        }
        i.running.insert(job.clone());
        true
    });
    if !admitted {
        if !matches!(job, Job::Index(_)) && !downloads.with(|i| i.running.contains(&job)) {
            problem(app, format!("{MAX_DOWNLOADS} downloads are running; try again when one has finished"));
        }
        return;
    }
    let wake = app.window.clone();
    let run = move || {
        let done = work();
        downloads.with(|i| {
            i.running.remove(&job);
            i.done.push(done);
        });
        if let Some(ctx) = wake {
            ctx.request_repaint();
        }
    };
    app.host.jobs().spawn("extension download", Box::new(run));
    app.view.extensions.manager.touch();
}

/// Handles the downloads that finished: keeps arrived indexes and reads the indexes again, installs downloaded entry
/// files, shows screenshots. Returns whether anything finished.
pub fn poll(app: &mut FontCraftApp) -> bool {
    let done = app.view.extensions.downloads.with(|i| std::mem::take(&mut i.done));
    if done.is_empty() {
        return false;
    }
    let mut arrived = false;
    for d in done {
        match d {
            Done::Index(location, Ok(bytes)) => {
                app.view.extensions.downloads.with(|i| i.fetched.insert(location, bytes));
                arrived = true;
            }
            Done::Index(location, Err(e)) => {
                app.view.extensions.downloads.with(|i| i.failed.insert(location.clone()));
                problem(app, format!("{}: {e}", redact(&location)));
            }
            Done::Install { id, version, keep_off, bytes } => match bytes {
                Ok(bytes) => post_install(app, &id, &version, Some(bytes), keep_off),
                Err(e) => problem(app, format!("{id}: {e}")),
            },
            Done::Screenshot(id, image) => {
                let view = &mut app.view.extensions.manager;
                if let Some(shot) = view.preview.as_mut().filter(|s| s.id == id) {
                    match image {
                        Ok(image) => shot.image = Some(image),
                        Err(e) => shot.error = Some(e),
                    }
                    shot.key = shot.key.wrapping_add(1);
                }
            }
        }
    }
    app.view.extensions.manager.touch();
    if arrived {
        queue_reload(app);
    }
    true
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use std::sync::Arc;

    use craft::host::{Host, InlineJobs};
    use craft::ui::runtime::commands::CommandRef;
    use fontcraft_engine::Session;
    use fontcraft_engine::extensions;

    use super::*;

    const MACRO: &[u8] = br#"{"format": "fontcraft.macro", "version": 1, "title": "Tighten", "description": "Tightens spacing.", "steps": []}"#;
    const BASE: &str = "https://user:secret@example.test/ext";

    fn index(entries: Vec<Value>) -> Vec<u8> {
        serde_json::to_vec(&json!({"format": "fontcraft.extensions", "version": 1, "entries": entries})).unwrap()
    }

    fn entry(id: &str, url: &str, bytes: &[u8]) -> Value {
        json!({
            "id": id, "kind": "macro", "version": "1", "titles": {"en": id}, "descriptions": {"en": "a test entry"}, "vendor": "Test Foundry", "url": url,
            "size": bytes.len(), "sha256": fontcraft_engine::extension_index::checksum(bytes), "screenshot": "shot.png",
        })
    }

    /// The pretend network: the index, one entry file and a screenshot that is not an image. No request leaves the
    /// process.
    fn fake_fetch(url: &str, max: usize, _: &str) -> Result<Vec<u8>, String> {
        let body = match url.strip_prefix(BASE) {
            Some("/index.json") => index(vec![entry("tighten", "files/tighten.fcmacro", MACRO), entry("huge", "files/huge.fcmacro", &[1; 64])]),
            Some("/files/tighten.fcmacro") => MACRO.to_vec(),
            Some("/shot.png") => b"not a picture".to_vec(),
            _ => return Err(format!("{}: not found", redact(url))),
        };
        if body.len() > max { Err("too large".into()) } else { Ok(body) }
    }

    fn app_with_folders(tag: &str) -> (FontCraftApp, std::path::PathBuf) {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let data = std::env::temp_dir().join(format!("fontcraft-ui-next-downloads-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&data).unwrap();
        let host = Host::builder(crate::app::APP).jobs(Arc::new(InlineJobs)).build();
        let mut app = FontCraftApp::new(Session::new(), Arc::new(host));
        assert!(extensions::open_folders(app.session.extensions_mut(), Some(&data), false).is_empty());
        (app, data)
    }

    /// Opening the window downloads the HTTPS index, shows its source without the password, and installs an entry
    /// whose file is downloaded and checked by the engine; a bad screenshot reports why.
    #[test]
    fn extension_downloads_fetch_https_indexes_entries_and_screenshots() {
        let (mut app, data) = app_with_folders("https");
        app.view.extensions.downloads.fetch = Some(fake_fetch);
        app.run_now(Msg::run("settings.set", json!({"id": SOURCES_SETTING, "value": format!("{BASE}/index.json")})), false).unwrap();
        app.run_now(Msg::Shell(CommandRef::with_params("window.pluginManager", json!({"tab": "indexes"}))), false).unwrap();
        for _ in 0..4 {
            step(&mut app);
            app.drain();
            crate::shell::extension_manager::refresh(&mut app);
        }
        let view = &app.view.extensions.manager;
        assert_eq!(view.reported.len(), 1);
        assert_eq!(view.reported[0].0, "https://example.test/ext/index.json", "the password is never shown");
        assert_eq!(view.reported[0].1, Some(2), "{:?}", view.reported);
        assert!(view.indexes.rows.iter().all(|r| !r.shown.contains("secret")));
        assert!(view.entries.iter().any(|e| e.id == "tighten"), "{:?}", view.entries);

        install_entry(&mut app, "tighten", None);
        step(&mut app);
        app.drain();
        assert!(data.join("Macros").join("tighten.fcmacro").exists(), "{:?} {}", app.view.extensions.manager.problem, app.status);

        install_entry(&mut app, "huge", None);
        step(&mut app);
        app.drain();
        assert!(app.view.extensions.manager.problem.as_deref().unwrap_or_default().contains("huge"), "a missing file is reported");

        show_screenshot(&mut app, "tighten").unwrap();
        step(&mut app);
        let shot = app.view.extensions.manager.preview.clone().unwrap();
        assert!(shot.image.is_none() && shot.error.as_deref().unwrap_or_default().contains("cannot be shown"), "{shot:?}");

        app.view.extensions.downloads.fetch = None;
        install_entry(&mut app, "tighten", None);
        assert!(app.view.extensions.manager.problem.as_deref().unwrap_or_default().contains("cannot download"));
        assert!(!app.view.extensions.manager.problem.as_deref().unwrap_or_default().contains("secret"));
        let _ = std::fs::remove_dir_all(&data);
    }
}
