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

use std::path::PathBuf;

use fontcraft_plugins::Registry;
use fontcraft_plugins::reporter::ReporterCache;
use serde::Serialize;
use serde_json::Value;

use crate::log::CommandLog;
use crate::macros::{Macro, Recording};

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
            recording.record(command, params);
        }
    }
}
