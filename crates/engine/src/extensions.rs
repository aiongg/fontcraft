//! The session's extension state: installed modules, the macro listing, the last macro run, a
//! macro being recorded, safe mode, the folders, the command log and the reporter cache. It
//! lives on the [`crate::Session`] (per process), not in documents. Plan: `docs/plans/M13.md`
//! § 3 (where installed modules live, folder watching), § 4 › M13.3, M13.5, M13.7, M13.9.
//!
//! The desktop app polls the Modules and Macros folders on focus and every 5 seconds
//! ([`poll_folders`]): only the files' names, sizes and modification times ([`FolderStamps`])
//! are read, and the files themselves only when those changed. The folder contents reach the
//! session as bytes ([`ExtensionState::sync_modules`], [`ExtensionState::sync_macros`]), so
//! tests and the web build hand them over without a folder. In safe mode (`--safe-mode`, the
//! launch keys) modules are listed but none runs until Help › Load Extensions
//! (`app.loadExtensions`) turns them on for the session.
//!
//! The module half, for the packages built on it (M13.4 to M13.6, M13.10):
//!
//! | API | What it does |
//! |---|---|
//! | [`ExtensionState::sync_modules`] | the Modules folder's files in, the registry in line with them; manual copies new and off |
//! | [`ExtensionState::runnable`] | the module `id` if it may run now: installed, turned on, of the kind asked, not in safe mode |
//! | [`ExtensionState::set_module_enabled`] | turns a module on or off and keeps the switch for its file |
//! | [`ExtensionState::forget_module`] | uninstalls a module (its file is the caller's: [`move_out_of_folder`]) |
//! | [`ExtensionState::note_module_error`] | the last error of a module, which `plugin.info` reports |
//! | [`open_folders`], [`poll_folders`] | the desktop app's launch step and its poll |
//! | `module_switches` | the switch per file name, kept in [`SWITCHES_FILE`] in the Modules folder |
//!
//! Owners: M13.3 the module half (folder sync, switches, safe mode); M13.5 the check ids
//! (`check.plugin.<module>.<name>`); M13.7 the macro listing and polling; M13.9 the log. The
//! hook [`ExtensionState::after_command`] is complete: it hands each command to the log and the
//! recorder, whose bodies those packages write.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use fontcraft_plugins::reporter::ReporterCache;
use fontcraft_plugins::{FolderReport, Kind, Plugin, Registry};
use serde::Serialize;
use serde_json::{Value, json};

use crate::log::CommandLog;
use crate::macros::{
    FolderFile, MAX_MACRO_RUN_BYTES, MAX_MACRO_RUN_STEPS, MAX_MACRO_TEXT, MAX_RESOLVED_VALUES, Macro, Recording, file_stem, natural_cmp,
};

/// Most files listed from one extension folder (subfolders included).
pub const MAX_FOLDER_FILES: usize = 10_000;
/// Deepest subfolder of the Macros folder shown as a submenu (`script.menu.subfolders`).
pub const MAX_MACRO_FOLDER_DEPTH: usize = 8;
/// How often the desktop app polls the extension folders while it has focus, in milliseconds.
pub const FOLDER_POLL_MS: u64 = 5_000;
/// The extension of module files.
pub const MODULE_EXTENSION: &str = "wasm";
/// The Modules folder's name in the per-user data folder (`plugin.pluginsFolder`).
pub const MODULES_FOLDER: &str = "Modules";
/// The Macros folder's name beside it.
pub const MACROS_FOLDER: &str = "Macros";
/// Where removed extensions go, beside the two folders, so a removal can be taken back by hand.
pub const REMOVED_FOLDER: &str = "Removed";
/// The file in the Modules folder that keeps the switch per module file (a hidden file, so
/// folder listings skip it).
pub const SWITCHES_FILE: &str = ".switches.json";
/// Most bytes of module files read from the Modules folder in one sync.
pub const MAX_MODULES_FOLDER_BYTES: u64 = 512 << 20;
/// Largest switches file read.
const MAX_SWITCHES_BYTES: u64 = 1 << 20;
/// Longest error kept per module for `plugin.info`, in characters.
const MAX_MODULE_ERROR_CHARS: usize = 1_000;

/// One macro file in the Macros folder.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacroEntry {
    /// Path relative to the Macros folder, `/`-separated (`Spacing/Tighten.fcmacro`); the folders
    /// become submenus of the Script menu.
    pub path: String,
    /// The menu label: the macro's title, or the file name without extension.
    pub title: String,
    /// The tooltip: the macro's description.
    pub description: String,
    /// Why the file could not be read; such entries are listed greyed out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// One file's name, size and modification time, as the poll compares them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileStamp {
    /// Path relative to the folder, `/`-separated.
    pub path: String,
    pub size: u64,
    /// Modification time in milliseconds since the Unix epoch (0 when unknown).
    pub modified_ms: i64,
}

/// A folder's file stamps at the last poll; the desktop app compares a new listing with it to
/// decide whether to re-read the folder.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FolderStamps(pub Vec<FileStamp>);

impl FolderStamps {
    /// Whether `now` differs from these stamps (a file added, removed, resized or touched).
    pub fn changed(&self, now: &FolderStamps) -> bool {
        self != now
    }
}

/// The budget a top-level macro run shares with every macro it runs, and the macros it has read,
/// so a macro run many times is read and parsed once. The outermost run resets it.
#[derive(Debug)]
pub struct MacroRun {
    /// Steps it may still execute.
    pub steps_left: usize,
    /// JSON values its resolved steps may still hold.
    pub values_left: usize,
    /// Bytes of macro files it may still read.
    pub bytes_left: usize,
    /// The macros read so far: by the name a step gave (`name:` and the name) and by their path
    /// in the Macros folder (`folder:` and the path).
    pub loaded: HashMap<String, LoadedMacro>,
}

impl Default for MacroRun {
    fn default() -> Self {
        Self { steps_left: MAX_MACRO_RUN_STEPS, values_left: MAX_RESOLVED_VALUES, bytes_left: MAX_MACRO_RUN_BYTES, loaded: HashMap::new() }
    }
}

/// A macro read during a run.
#[derive(Clone, Debug)]
pub struct LoadedMacro {
    /// What the self-reference check compares.
    pub key: String,
    /// Its path in the Macros folder, or its file as given.
    pub path: String,
    pub m: Arc<Macro>,
}

/// An extension index the session has read, and where from.
#[derive(Clone, Debug, Default)]
pub struct LoadedIndex {
    /// The index file's path or URL; an entry's relative `url` is a path beside a local index.
    pub source: String,
    pub index: crate::extension_index::Index,
}

/// Everything about extensions a session keeps.
#[derive(Debug, Default)]
pub struct ExtensionState {
    /// The installed modules.
    pub registry: Registry,
    /// The macros found in the Macros folder, in menu order (`script.menu.mirrorsFolder`).
    pub macros: Vec<MacroEntry>,
    /// Bumped whenever `macros` changes; menus compare it with the registry's revision.
    pub macros_revision: u64,
    /// The macro run last, for `script.rerunLast`: its path in the Macros folder (or `None` for
    /// one run from parameters) and the macro itself with the arguments it ran with.
    pub last_macro: Option<(Option<String>, Macro, Value)>,
    /// The macro being recorded (`script.recordMacro`).
    pub recording: Option<Recording>,
    /// The macros of the Macros folder that could be read, by their path in `macros`.
    pub macro_files: BTreeMap<String, Macro>,
    /// Each title of `macros` that can run, with the path of the first such macro in menu order.
    macro_titles: BTreeMap<String, String>,
    /// The macros running now, outermost first: their folder path or file (empty for one given
    /// inline). `script.run` refuses a macro already on it, and more than
    /// [`MAX_MACRO_NESTING`](crate::macros::MAX_MACRO_NESTING) of them.
    pub running_macros: Vec<String>,
    /// What the running top-level macro may still do, and the macros it has read.
    pub macro_run: MacroRun,
    /// Modules are listed but none runs (`app.launchWithoutPlugins`).
    pub safe_mode: bool,
    /// The Modules folder; `None` in the web app and in headless sessions without one.
    pub modules_dir: Option<PathBuf>,
    /// The Macros folder.
    pub macros_dir: Option<PathBuf>,
    /// The two folders' stamps at the last poll.
    pub modules_stamps: FolderStamps,
    pub macros_stamps: FolderStamps,
    /// The user's switch per module file name in the Modules folder: on, or off. A file not
    /// listed has not been seen yet, so its module is listed as new and off
    /// (`plugin.noManualMove`). Kept in [`SWITCHES_FILE`].
    pub module_switches: BTreeMap<String, bool>,
    /// The files of the Modules folder that failed to load at the last sync, with the reason.
    pub module_failures: Vec<(String, String)>,
    /// The last error of each module (a failed run or a refused output), by module id.
    pub module_errors: BTreeMap<String, String>,
    /// When the app started, in seconds since the Unix epoch (`app.info`); `None` when the
    /// shell did not say.
    pub started_at: Option<i64>,
    /// The extension indexes the session knows, first listed first: for an id in several, the
    /// first wins (`pluginManager.list`, `pluginManager.install`). The shell fills it from the
    /// user's list of indexes (M13.10).
    pub indexes: Vec<LoadedIndex>,
    /// What the shell's other surfaces are doing, by name (`control`, `mcp`), in a few words
    /// (`listening on 127.0.0.1:7985`), for `app.info`.
    pub surfaces: BTreeMap<String, String>,
    /// The commands run and module log lines.
    pub log: CommandLog,
    /// Reporter drawings.
    pub reporter_cache: ReporterCache,
}

impl ExtensionState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Changes whenever anything a menu lists changes (modules or macros).
    pub fn revision(&self) -> u64 {
        self.registry.revision().wrapping_mul(1_000_003).wrapping_add(self.macros_revision)
    }

    /// Whether modules may run now (not in safe mode).
    pub fn modules_run(&self) -> bool {
        !self.safe_mode
    }

    /// The hook `Session::execute` calls after every top-level command it runs (not previews):
    /// logs it, and hands a successful font-changing command (`journal`) other than undo and redo
    /// to the recording.
    pub fn after_command(&mut self, command: &str, params: &Value, journal: bool, result: &crate::Result<Value>) {
        let logged = without_session_params(command, params);
        self.log.push_command(command, logged.as_ref(), result.as_ref().map_err(ToString::to_string));
        if journal
            && result.is_ok()
            && !matches!(command, "edit.undo" | "edit.redo")
            && let Some(recording) = &mut self.recording
        {
            // A re-run is recorded as the macro it ran, so the recording does not depend on
            // which macro ran last.
            if matches!(command, "script.rerunLast" | "script.repeatLastAction") {
                if let Some((_, m, args)) = &self.last_macro {
                    recording.record("script.run", &serde_json::json!({ "macro": m.to_json(), "args": args }));
                }
            } else {
                recording.record(command, params);
            }
        }
    }
}

/// The module half: the Modules folder, switches, safe mode.
impl ExtensionState {
    /// Brings the installed modules in line with the files read from the Modules folder
    /// (`(file name, bytes)`, as [`read_modules_folder`] gives them) and keeps `stamps` for the
    /// next poll. A file seen for the first time is installed off, listed as new, and its switch
    /// recorded as off, so it is no longer new at the next launch; a file that is gone
    /// uninstalls its module; a file that cannot be read or loaded is kept in
    /// [`ExtensionState::module_failures`]. In safe mode the modules are listed all the same.
    pub fn sync_modules(&mut self, files: Vec<FolderFile>, stamps: FolderStamps) -> FolderReport {
        let mut failed = Vec::new();
        let mut read = Vec::new();
        for (name, bytes) in files.into_iter().take(MAX_FOLDER_FILES) {
            match bytes {
                Ok(bytes) => read.push((name, bytes)),
                Err(e) => failed.push((name, e)),
            }
        }
        let refs: Vec<(&str, &[u8])> = read.iter().map(|(name, bytes)| (name.as_str(), bytes.as_slice())).collect();
        let mut report = self.registry.sync_folder(&refs, &self.module_switches);
        for id in &report.new {
            if let Some(file) = self.registry.get(id).and_then(|m| m.file.clone()) {
                self.module_switches.insert(file, false);
            }
        }
        // Switches of files that are gone are dropped with them.
        let listed: std::collections::BTreeSet<&str> =
            read.iter().map(|(name, _)| name.as_str()).chain(failed.iter().map(|(name, _)| name.as_str())).collect();
        self.module_switches.retain(|file, _| listed.contains(file.as_str()));
        for id in &report.removed {
            self.module_errors.remove(id);
        }
        failed.extend(report.failed.iter().cloned());
        report.failed = failed.clone();
        self.module_failures = failed;
        self.modules_stamps = stamps;
        self.retain_reporter_drawings();
        report
    }

    /// The module `id` if it may run now: installed, turned on, of `kind`, and the session not
    /// in safe mode. The error says which of these failed, in words for the user.
    pub fn runnable(&self, id: &str, kind: Kind) -> Result<Arc<Plugin>, String> {
        let m = self.registry.get(id).ok_or_else(|| format!("no module {id:?} is installed"))?;
        if m.plugin.kind() != kind {
            return Err(format!("module {id:?} is a {} module, not a {} module", m.plugin.kind().name(), kind.name()));
        }
        if self.safe_mode {
            return Err("extensions are off in safe mode: Help › Load Extensions turns them on for this session".into());
        }
        if !m.enabled {
            return Err(format!("module {id:?} is turned off: turn it on in the Extension Manager"));
        }
        Ok(m.plugin.clone())
    }

    /// Turns the module `id` on or off; the switch is kept for its file in the Modules folder
    /// (written by [`save_switches`]).
    pub fn set_module_enabled(&mut self, id: &str, enabled: bool) -> Result<(), String> {
        self.registry.set_enabled(id, enabled).map_err(|e| e.to_string())?;
        if let Some(file) = self.registry.get(id).and_then(|m| m.file.clone()) {
            self.module_switches.insert(file, enabled);
        }
        Ok(())
    }

    /// Uninstalls the module `id`: it leaves the registry, its switch, its last error and its
    /// cached drawings. Returns the file it came from; moving that file out of the Modules
    /// folder is the caller's ([`move_out_of_folder`]). `None` when no such module is installed.
    pub fn forget_module(&mut self, id: &str) -> Option<Option<String>> {
        let file = self.registry.get(id)?.file.clone();
        self.registry.remove(id);
        if let Some(file) = &file {
            self.module_switches.remove(file);
        }
        self.module_errors.remove(id);
        self.retain_reporter_drawings();
        Some(file)
    }

    /// Keeps `error` as the last error of the module `id` (cut to a readable length).
    pub fn note_module_error(&mut self, id: &str, error: &str) {
        self.module_errors.insert(id.to_string(), error.chars().take(MAX_MODULE_ERROR_CHARS).collect());
    }

    /// Leaves safe mode: the modules that are turned on run again (`app.loadExtensions`).
    pub fn leave_safe_mode(&mut self) {
        self.safe_mode = false;
    }

    /// Drops reporter drawings of modules that are no longer installed.
    fn retain_reporter_drawings(&self) {
        let serials: Vec<u64> = self.registry.list().iter().map(|m| m.plugin.serial()).collect();
        self.reporter_cache.retain_modules(&serials);
    }

    /// The installed modules as `app.info` and `pluginManager.list` describe them.
    pub fn modules_json(&self) -> Vec<Value> {
        self.registry
            .list()
            .iter()
            .map(|m| {
                let man = m.plugin.manifest();
                json!({
                    "id": man.id, "name": man.name, "kind": man.kind.name(), "version": man.version, "author": man.author,
                    "enabled": m.enabled, "new": m.new, "file": m.file, "size": m.plugin.size(), "sha256": m.sha256,
                })
            })
            .collect()
    }
}

/// Whether `name` is a module file (`.wasm`, any case).
pub fn is_module_file(name: &str) -> bool {
    Path::new(name).extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case(MODULE_EXTENSION))
}

/// The stamps of the module files directly in `dir` (the Modules folder has no subfolders), at
/// most [`MAX_FOLDER_FILES`], sorted by name. Hidden files and other files are skipped; a missing
/// folder has none.
pub fn stamp_modules_folder(dir: &Path) -> FolderStamps {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return FolderStamps(out) };
    for (seen, entry) in entries.flatten().enumerate() {
        if seen >= crate::macros::MAX_FOLDER_ENTRIES || out.len() >= MAX_FOLDER_FILES {
            break;
        }
        let Ok(name) = entry.file_name().into_string() else { continue };
        if name.starts_with('.') || !is_module_file(&name) {
            continue;
        }
        let Ok(meta) = std::fs::metadata(entry.path()) else { continue };
        if !meta.is_file() {
            continue;
        }
        let modified_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
        out.push(FileStamp { path: name, size: meta.len(), modified_ms });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    FolderStamps(out)
}

/// Reads a module file, refusing one over the module size limit without reading the rest.
pub fn read_module_file(path: &Path) -> Result<Vec<u8>, String> {
    let max = fontcraft_plugins::error::MAX_MODULE_BYTES;
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    let cap = u64::try_from(max).unwrap_or(u64::MAX).saturating_add(1);
    file.take(cap).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() > max {
        return Err(format!("the file is larger than the module limit of {} MiB", max >> 20));
    }
    Ok(bytes)
}

/// Reads the module files directly in `dir`: their stamps and contents, at most
/// [`MAX_MODULES_FOLDER_BYTES`] in all and [`fontcraft_plugins::error::MAX_INSTALLED`] files.
pub fn read_modules_folder(dir: &Path) -> (FolderStamps, Vec<FolderFile>) {
    let stamps = stamp_modules_folder(dir);
    let mut left = MAX_MODULES_FOLDER_BYTES;
    let files = stamps
        .0
        .iter()
        .enumerate()
        .map(|(i, stamp)| {
            let bytes = if i >= fontcraft_plugins::error::MAX_INSTALLED {
                Err(format!("not read: at most {} modules can be installed", fontcraft_plugins::error::MAX_INSTALLED))
            } else if stamp.size > left {
                Err(format!("not read: the Modules folder holds more than {} MiB of modules", MAX_MODULES_FOLDER_BYTES >> 20))
            } else {
                read_module_file(&dir.join(&stamp.path)).inspect(|b| left = left.saturating_sub(u64::try_from(b.len()).unwrap_or(u64::MAX)))
            };
            (stamp.path.clone(), bytes)
        })
        .collect();
    (stamps, files)
}

/// The switches kept in the Modules folder `dir`: `{"version": 1, "modules": {file: bool}}`. A
/// missing file has none; an unreadable one is an error and is left as it is.
pub fn read_switches(dir: &Path) -> Result<BTreeMap<String, bool>, String> {
    let path = dir.join(SWITCHES_FILE);
    let file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let mut bytes = Vec::new();
    file.take(MAX_SWITCHES_BYTES.saturating_add(1)).read_to_end(&mut bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_SWITCHES_BYTES {
        return Err(format!("{}: larger than {} KiB", path.display(), MAX_SWITCHES_BYTES >> 10));
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    let modules = value.get("modules").and_then(Value::as_object).ok_or_else(|| format!("{}: no `modules` object", path.display()))?;
    Ok(modules
        .iter()
        .filter(|(file, _)| is_module_file(file) && !file.contains(['/', '\\']))
        .filter_map(|(file, on)| on.as_bool().map(|on| (file.clone(), on)))
        .take(MAX_FOLDER_FILES)
        .collect())
}

/// Writes `switches` to the Modules folder `dir` (creating it when missing).
pub fn save_switches(dir: &Path, switches: &BTreeMap<String, bool>) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let text = serde_json::to_string_pretty(&json!({"version": 1, "modules": switches})).map_err(|e| e.to_string())?;
    write_file(&dir.join(SWITCHES_FILE), text.as_bytes())
}

/// Writes `bytes` to `path` in one step where the platform allows it.
pub fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    #[cfg(not(target_arch = "wasm32"))]
    let written = fontcraft_format::write_atomic(path, bytes).map_err(|e| e.to_string());
    // The web build has no folders; a plain write keeps it compiling.
    #[cfg(target_arch = "wasm32")]
    let written = std::fs::write(path, bytes).map_err(|e| e.to_string());
    written.map_err(|e| format!("{}: {e}", path.display()))
}

/// Moves `file` (a path relative to `dir`) into the [`REMOVED_FOLDER`] beside `dir`, under a free name
/// (`name 2.wasm`, `name 3.wasm`…). Returns where it went.
pub fn move_out_of_folder(dir: &Path, file: &str) -> Result<PathBuf, String> {
    let from = dir.join(file);
    let removed = dir.parent().unwrap_or(dir).join(REMOVED_FOLDER);
    std::fs::create_dir_all(&removed).map_err(|e| format!("{}: {e}", removed.display()))?;
    // A macro in a subfolder lands directly in the Removed folder.
    let name = file.rsplit(['/', '\\']).next().unwrap_or(file);
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (name, String::new()),
    };
    let mut to = removed.join(name);
    let mut n = 2u32;
    while to.exists() {
        if n > 10_000 {
            return Err(format!("{} holds too many copies of {file}", removed.display()));
        }
        to = removed.join(format!("{stem} {n}{ext}"));
        n += 1;
    }
    std::fs::rename(&from, &to).map_err(|e| format!("cannot move {} to {}: {e}", from.display(), to.display()))?;
    Ok(to)
}

/// Re-reads the session's Modules folder when its files changed since the last poll; `None`
/// when nothing changed or the session has none. Switches of files seen for the first time
/// are written back to the folder.
pub fn poll_modules(ext: &mut ExtensionState) -> Option<Result<FolderReport, String>> {
    let dir = ext.modules_dir.clone()?;
    if !ext.modules_stamps.changed(&stamp_modules_folder(&dir)) {
        return None;
    }
    let (stamps, files) = read_modules_folder(&dir);
    let switches = ext.module_switches.clone();
    let report = ext.sync_modules(files, stamps);
    if ext.module_switches != switches
        && let Err(e) = save_switches(&dir, &ext.module_switches)
    {
        return Some(Err(e));
    }
    Some(Ok(report))
}

/// Sets the session's extension folders inside the per-user data folder `data_dir`
/// ([`MODULES_FOLDER`], [`MACROS_FOLDER`]) and reads the module switches; `safe_mode` keeps
/// every module from running. The folders are read by the first [`poll_folders`]. Returns the
/// problems found, one line each.
pub fn open_folders(ext: &mut ExtensionState, data_dir: Option<&Path>, safe_mode: bool) -> Vec<String> {
    ext.safe_mode = safe_mode;
    ext.modules_dir = data_dir.map(|d| d.join(MODULES_FOLDER));
    ext.macros_dir = data_dir.map(|d| d.join(MACROS_FOLDER));
    let mut problems = Vec::new();
    if let Some(dir) = &ext.modules_dir {
        match read_switches(dir) {
            Ok(switches) => ext.module_switches = switches,
            Err(e) => problems.push(format!("module switches: {e}")),
        }
    }
    problems
}

/// Polls both extension folders (the desktop app calls it on focus and every
/// [`FOLDER_POLL_MS`]); returns what the user should hear about: files that failed to load and
/// switches that could not be saved, one line each.
pub fn poll_folders(ext: &mut ExtensionState) -> Vec<String> {
    let mut problems = Vec::new();
    crate::macros::poll(ext);
    match poll_modules(ext) {
        Some(Ok(report)) => {
            for (file, e) in &report.failed {
                problems.push(format!("module {file}: {e}"));
            }
            for id in &report.new {
                problems.push(format!("new module {id} found in the Modules folder: it stays off until you turn it on"));
            }
        }
        Some(Err(e)) => problems.push(format!("module switches: {e}")),
        None => {}
    }
    problems
}

/// Parameters that name something by its id in this session only, as the app passes them, or
/// that may hold index bytes and credentials: left out of the log, whose lines can be copied and
/// run again in another session.
const SESSION_PARAMS: &[(&str, &str)] =
    &[("file.save", "document"), ("pluginManager.reloadIndexes", "fetched"), ("pluginManager.reloadIndexes", "indexes")];

/// `params` without the [`SESSION_PARAMS`] of `command`.
fn without_session_params<'a>(command: &str, params: &'a Value) -> std::borrow::Cow<'a, Value> {
    let keys: Vec<&str> = SESSION_PARAMS.iter().filter(|(c, k)| *c == command && params.get(k).is_some()).map(|(_, k)| *k).collect();
    if keys.is_empty() {
        return std::borrow::Cow::Borrowed(params);
    }
    let mut kept = params.clone();
    if let Some(map) = kept.as_object_mut() {
        for k in keys {
            map.remove(k);
        }
    }
    std::borrow::Cow::Owned(kept)
}

/// The macro half: the Macros folder's listing.
impl ExtensionState {
    /// Replaces the macro listing with the files read from the Macros folder (paths relative to
    /// it, `/`-separated) and keeps `stamps` for the next poll. Files that cannot be read or
    /// parsed are listed with their error. Returns whether the listing changed.
    pub fn sync_macros(&mut self, files: Vec<FolderFile>, stamps: FolderStamps) -> bool {
        let mut entries = Vec::new();
        let mut bodies = BTreeMap::new();
        for (path, bytes) in files.into_iter().take(MAX_FOLDER_FILES) {
            if path.is_empty() || path.len() > MAX_MACRO_TEXT || bodies.contains_key(&path) || entries.iter().any(|e: &MacroEntry| e.path == path) {
                continue;
            }
            let stem = file_stem(&path).to_string();
            match bytes.and_then(|b| Macro::parse(&b).map_err(|e| e.to_string())) {
                Ok(m) => {
                    let title = if m.title.trim().is_empty() { stem } else { m.title.trim().to_string() };
                    entries.push(MacroEntry { path: path.clone(), title, description: m.description.clone(), error: None });
                    bodies.insert(path, m);
                }
                Err(error) => entries.push(MacroEntry { path, title: stem, description: String::new(), error: Some(error) }),
            }
        }
        entries.sort_by(menu_order);
        self.macros_stamps = stamps;
        let changed = entries != self.macros || bodies != self.macro_files;
        if changed {
            self.macros = entries;
            self.macro_files = bodies;
            self.index_titles();
            self.macros_revision = self.macros_revision.wrapping_add(1);
        }
        changed
    }

    /// Adds or replaces one macro of the Macros folder, as saving it there does; the next poll
    /// reads the folder again anyway.
    pub fn put_macro(&mut self, path: String, m: Macro) {
        let stem = file_stem(&path).to_string();
        let title = if m.title.trim().is_empty() { stem } else { m.title.trim().to_string() };
        self.macros.retain(|e| e.path != path);
        self.macros.push(MacroEntry { path: path.clone(), title, description: m.description.clone(), error: None });
        self.macros.sort_by(menu_order);
        self.macro_files.insert(path, m);
        self.index_titles();
        self.macros_revision = self.macros_revision.wrapping_add(1);
    }

    /// Drops one macro of the Macros folder from the listing, as removing its file does; the
    /// next poll reads the folder again anyway. Returns whether it was listed.
    pub fn forget_macro(&mut self, path: &str) -> bool {
        let before = self.macros.len();
        self.macros.retain(|e| e.path != path);
        let removed = self.macro_files.remove(path).is_some() || self.macros.len() != before;
        if removed {
            self.index_titles();
            self.macros_revision = self.macros_revision.wrapping_add(1);
        }
        removed
    }

    fn index_titles(&mut self) {
        self.macro_titles.clear();
        for e in self.macros.iter().filter(|e| e.error.is_none()) {
            self.macro_titles.entry(e.title.clone()).or_insert_with(|| e.path.clone());
        }
    }

    /// The macro of the Macros folder that `name` names: its path (`Tools/Tighten.fcmacro`), its
    /// path without the extension, or its menu title (the first in menu order).
    pub fn find_macro(&self, name: &str) -> Option<(&str, &Macro)> {
        let by_path = |p: &str| self.macro_files.get_key_value(p).map(|(k, m)| (k.as_str(), m));
        by_path(name)
            .or_else(|| by_path(&format!("{name}.{}", crate::macros::MACRO_EXTENSION)))
            .or_else(|| self.macro_titles.get(name).and_then(|path| by_path(path)))
    }
}

/// The Script menu's order: folders (submenus) before items at each level, then folder names and
/// item titles in [`natural_cmp`] order.
pub fn menu_order(a: &MacroEntry, b: &MacroEntry) -> Ordering {
    let folders = |e: &MacroEntry| -> Vec<String> {
        let mut parts: Vec<String> = e.path.split('/').map(str::to_string).collect();
        parts.pop();
        parts
    };
    let (fa, fb) = (folders(a), folders(b));
    for i in 0..=fa.len().max(fb.len()) {
        match (fa.get(i), fb.get(i)) {
            (Some(x), Some(y)) => {
                let order = natural_cmp(x, y);
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(_), None) => return Ordering::Less,
            (None, Some(_)) => return Ordering::Greater,
            (None, None) => break,
        }
    }
    natural_cmp(&a.title, &b.title).then_with(|| a.path.cmp(&b.path))
}

// ── Check modules in the audit (M13.5) ──────────────────────────────────────────────────────────
//
// `audit.run` runs every turned-on check module on the layers it examines, next to the built-in
// layer checks. A module's findings join the result document under the check ids
// `check.plugin.<module id>.<finding name>`, with no fix; the canvas markers read them like any
// other result. A module that fails on a layer is reported once, as a check with status `error`,
// and is not run on the remaining layers of that run.

pub use module_checks::{
    MAX_MODULE_CHECK_IDS, MAX_MODULE_FINDINGS_PER_RUN, MODULE_CHECK_GROUP, MODULE_CHECK_PREFIX, MODULE_FINDING_CODE, ModuleChecks, ModuleRun,
    module_check_id, names_module_checks,
};

mod module_checks {
    use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
    use std::sync::{Arc, Mutex, OnceLock, PoisonError};

    use fontcraft_audit::{Finding as AuditFinding, Location, Severity as AuditSeverity, matches_pattern};
    use fontcraft_model::{Font, Glyph, Layer};
    use fontcraft_plugins::check::{Severity as ModuleSeverity, parse_findings};
    use fontcraft_plugins::layer_json::LayerInput;
    use fontcraft_plugins::{Kind, Plugin};
    use serde_json::{Value, json};

    use super::ExtensionState;

    /// The start of every check id a check module reports under.
    pub const MODULE_CHECK_PREFIX: &str = "check.plugin.";
    /// The audit group of module checks (`groups: ["plugin"]`).
    pub const MODULE_CHECK_GROUP: &str = "plugin";
    /// The `code` of every module finding: the finding's name is already part of its check id.
    pub const MODULE_FINDING_CODE: &str = "module";
    /// Most distinct module check ids one process names. Each is kept for the life of the process,
    /// like the built-in ids, so a module that invents a new name on every run cannot grow memory
    /// without bound.
    pub const MAX_MODULE_CHECK_IDS: usize = 65_536;
    /// Most module findings one audit run keeps; the rest are counted in `summary.truncated`.
    pub const MAX_MODULE_FINDINGS_PER_RUN: usize = 100_000;

    static MODULE_CHECK_IDS: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();

    /// The check id of the finding `name` of module `module`: `check.plugin.<module>.<name>`. `None`
    /// once the process has named [`MAX_MODULE_CHECK_IDS`] distinct ids.
    pub fn module_check_id(module: &str, name: &str) -> Option<&'static str> {
        let id = format!("{MODULE_CHECK_PREFIX}{module}.{name}");
        let mut ids = MODULE_CHECK_IDS.get_or_init(Default::default).lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(&known) = ids.get(id.as_str()) {
            return Some(known);
        }
        if ids.len() >= MAX_MODULE_CHECK_IDS {
            return None;
        }
        let kept: &'static str = Box::leak(id.into_boxed_str());
        ids.insert(kept);
        Some(kept)
    }

    /// Whether the check id or `prefix*` pattern `pattern` can match a module check id, so the audit
    /// accepts it although no built-in check matches.
    pub fn names_module_checks(pattern: &str) -> bool {
        match pattern.strip_suffix('*') {
            Some(prefix) => prefix.starts_with(MODULE_CHECK_PREFIX) || MODULE_CHECK_PREFIX.starts_with(prefix),
            None => pattern.starts_with(MODULE_CHECK_PREFIX),
        }
    }

    /// The check modules one audit run uses, with the `checks` and `exclude` patterns that narrow
    /// their findings.
    #[derive(Clone, Debug, Default)]
    pub struct ModuleChecks {
        /// Each module with its parameters (the manifest's defaults) as JSON.
        modules: Vec<(Arc<Plugin>, Vec<u8>)>,
        include: Vec<String>,
        exclude: Vec<String>,
    }

    impl ModuleChecks {
        /// The turned-on check modules of `extensions` (none in safe mode) whose checks `include` and
        /// `exclude` (check ids and `prefix*` patterns, as `audit.run` takes them) leave room for.
        pub fn select(extensions: &ExtensionState, include: &[&str], exclude: &[&str]) -> ModuleChecks {
            if !extensions.modules_run() {
                return ModuleChecks::default();
            }
            let modules = extensions
                .registry
                .enabled_of_kind(Kind::Check)
                .filter(|m| {
                    let own = format!("{MODULE_CHECK_PREFIX}{}.", m.plugin.id());
                    let may_match = |pat: &&str| match pat.strip_suffix('*') {
                        Some(prefix) => prefix.starts_with(&own) || own.starts_with(prefix),
                        None => pat.starts_with(&own),
                    };
                    let all_excluded = exclude.iter().any(|pat| pat.strip_suffix('*').is_some_and(|prefix| own.starts_with(prefix)));
                    (include.is_empty() || include.iter().any(may_match)) && !all_excluded
                })
                .filter_map(|m| {
                    let params = serde_json::to_vec(&Value::Object(m.plugin.manifest().defaults())).ok()?;
                    Some((m.plugin.clone(), params))
                })
                .collect();
            ModuleChecks {
                modules,
                include: include.iter().map(|s| (*s).to_string()).collect(),
                exclude: exclude.iter().map(|s| (*s).to_string()).collect(),
            }
        }

        pub fn is_empty(&self) -> bool {
            self.modules.is_empty()
        }

        fn selects(&self, check: &str) -> bool {
            (self.include.is_empty() || self.include.iter().any(|pat| matches_pattern(check, pat)))
                && !self.exclude.iter().any(|pat| matches_pattern(check, pat))
        }

        /// Runs each module on `layer` of `glyph` and adds its findings to `out`; a module that
        /// failed earlier in the run is not run again.
        pub fn run_layer(&self, font: &Font, glyph: &Glyph, layer: &Layer, state: &mut ModuleRun, out: &mut Vec<AuditFinding>) {
            for (plugin, params) in &self.modules {
                let module = plugin.id();
                if state.errors.contains_key(module) {
                    continue;
                }
                state.ran.insert(module.to_string());
                let found = LayerInput::from_layer(font, glyph, layer)
                    .and_then(|input| input.to_json())
                    .and_then(|input| plugin.call(&input, params))
                    .and_then(|output| parse_findings(&output));
                let findings = match found {
                    Ok(findings) => findings,
                    Err(e) => {
                        state.errors.insert(module.to_string(), format!("glyph {}, layer {}: {e}", glyph.name, layer.id));
                        continue;
                    }
                };
                for f in findings {
                    let Some(check) = module_check_id(module, &f.name) else {
                        state.errors.insert(module.to_string(), format!("more than {MAX_MODULE_CHECK_IDS} module check names in this session"));
                        break;
                    };
                    if !self.selects(check) {
                        continue;
                    }
                    if state.kept >= MAX_MODULE_FINDINGS_PER_RUN {
                        state.truncated = state.truncated.saturating_add(1);
                        continue;
                    }
                    state.kept += 1;
                    state.modules_of.insert(check, module.to_string());
                    out.push(AuditFinding {
                        check,
                        code: MODULE_FINDING_CODE,
                        severity: match f.severity {
                            ModuleSeverity::Error => AuditSeverity::Fail,
                            ModuleSeverity::Warning => AuditSeverity::Warn,
                            ModuleSeverity::Info => AuditSeverity::Info,
                        },
                        message: f.message,
                        location: Location { glyph: glyph.name.clone(), layer: layer.id.clone(), point: f.position, ..Location::default() },
                        data: BTreeMap::new(),
                        fix: None,
                    });
                }
            }
        }
    }

    /// What the check modules did in one audit run.
    #[derive(Debug, Default)]
    pub struct ModuleRun {
        /// The modules that ran on at least one layer.
        ran: BTreeSet<String>,
        /// The first failure of each module that failed.
        errors: BTreeMap<String, String>,
        /// The module behind each check id reported.
        modules_of: BTreeMap<&'static str, String>,
        kept: usize,
        /// Findings dropped past [`MAX_MODULE_FINDINGS_PER_RUN`].
        pub truncated: usize,
    }

    impl ModuleRun {
        /// The result document's `checks` entries for the modules: each module check id with results
        /// (`results` and the worst severity as `status`, from `results`, the counts of the listed
        /// results by check id), each module that failed (`check.plugin.<module>`, status `error`,
        /// with `message`), and, with `include_passed`, each module that ran and reported nothing
        /// (status `pass`). Returns the entries, the passes and the errors.
        pub fn entries(&self, results: &HashMap<&str, (usize, AuditSeverity)>, include_passed: bool) -> (Vec<Value>, usize, usize) {
            let mut out = Vec::new();
            let mut reported: BTreeSet<&str> = BTreeSet::new();
            for (check, module) in &self.modules_of {
                if let Some((n, worst)) = results.get(check) {
                    reported.insert(module.as_str());
                    out.push(json!({"id": check, "status": worst.as_str(), "results": n}));
                }
            }
            let (mut passed, mut errors) = (0, 0);
            for module in &self.ran {
                let id = format!("{MODULE_CHECK_PREFIX}{module}");
                if let Some(message) = self.errors.get(module) {
                    errors += 1;
                    out.push(json!({"id": id, "status": "error", "results": 0, "message": message}));
                } else if !reported.contains(module.as_str()) {
                    passed += 1;
                    if include_passed {
                        out.push(json!({"id": id, "status": "pass", "results": 0}));
                    }
                }
            }
            (out, passed, errors)
        }
    }
}
