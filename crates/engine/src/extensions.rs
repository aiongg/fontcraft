//! The session's extension state: installed modules, the macro listing, the last macro run, a
//! macro being recorded, safe mode, the folders, the command log and the reporter cache. It
//! lives on the [`crate::Session`] (per process), not in documents. Plan: `docs/plans/M13.md`
//! § 3 (where installed modules live, folder watching), § 4 › M13.3, M13.5, M13.7, M13.9.
//!
//! The engine does no folder I/O here: the desktop app polls the Modules and Macros folders on
//! focus and every 5 seconds ([`FolderStamps`]), and when they changed reads the files and hands
//! the bytes to `ExtensionState::sync_modules` (M13.3) and `ExtensionState::sync_macros`
//! (M13.7). In safe mode (`--safe-mode`, the launch keys) modules are listed but none runs until
//! Help › Load Extensions turns them on for the session.
//!
//! Owners: M13.3 the module half (folder sync, switches, safe mode); M13.5 the check ids
//! (`check.plugin.<module>.<name>`); M13.7 the macro listing and polling; M13.9 the log. The
//! hook [`ExtensionState::after_command`] is complete: it hands each command to the log and the
//! recorder, whose bodies those packages write.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::path::PathBuf;

use fontcraft_plugins::Registry;
use fontcraft_plugins::reporter::ReporterCache;
use serde::Serialize;
use serde_json::Value;

use crate::log::CommandLog;
use crate::macros::{FolderFile, MAX_MACRO_TEXT, Macro, Recording, file_stem, natural_cmp};

/// Most files listed from one extension folder (subfolders included).
pub const MAX_FOLDER_FILES: usize = 10_000;
/// Deepest subfolder of the Macros folder shown as a submenu (`script.menu.subfolders`).
pub const MAX_MACRO_FOLDER_DEPTH: usize = 8;
/// How often the desktop app polls the extension folders while it has focus, in milliseconds.
pub const FOLDER_POLL_MS: u64 = 5_000;

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
    /// The macros running now, outermost first: their folder path or file (empty for one given
    /// inline). `script.run` refuses a macro already on it, and more than
    /// [`MAX_MACRO_NESTING`](crate::macros::MAX_MACRO_NESTING) of them.
    pub running_macros: Vec<String>,
    /// Modules are listed but none runs (`app.launchWithoutPlugins`).
    pub safe_mode: bool,
    /// The Modules folder; `None` in the web app and in headless sessions without one.
    pub modules_dir: Option<PathBuf>,
    /// The Macros folder.
    pub macros_dir: Option<PathBuf>,
    /// The two folders' stamps at the last poll.
    pub modules_stamps: FolderStamps,
    pub macros_stamps: FolderStamps,
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
        self.log.push_command(command, params, result.as_ref().map_err(ToString::to_string));
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
        self.macros_revision = self.macros_revision.wrapping_add(1);
    }

    /// The macro of the Macros folder that `name` names: its path (`Tools/Tighten.fcmacro`), its
    /// path without the extension, or its menu title (the first in menu order).
    pub fn find_macro(&self, name: &str) -> Option<(&str, &Macro)> {
        let by_path = |p: &str| self.macro_files.get_key_value(p).map(|(k, m)| (k.as_str(), m));
        by_path(name)
            .or_else(|| by_path(&format!("{name}.{}", crate::macros::MACRO_EXTENSION)))
            .or_else(|| self.macros.iter().filter(|e| e.error.is_none() && e.title == name).find_map(|e| by_path(&e.path)))
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
