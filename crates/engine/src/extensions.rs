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
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

use fontcraft_plugins::Registry;
use fontcraft_plugins::reporter::ReporterCache;
use serde::Serialize;
use serde_json::Value;

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

/// Parameters that name something by its id in this session only, as the app passes them: left
/// out of the log, whose lines can be copied and run again in another session.
const SESSION_PARAMS: &[(&str, &str)] = &[("file.save", "document")];

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
