//! The installed modules of one session: install, replace and remove ported from VectorCraft's
//! process-wide `crates/plugins/src/registry.rs` (MIT OR Apache-2.0, the same owner; see
//! `NOTICE`) into a value type the engine keeps in its `ExtensionState`, with the folder sync.
//!
//! The registry holds no files and does no I/O: the desktop app reads the Modules folder and
//! hands the bytes in ([`Registry::sync_folder`]); `plugin.installFromFile` hands one module's
//! bytes ([`Registry::install_bytes`]). A module found in the folder that the user has not turned
//! on is listed as new and off (`plugin.noManualMove`). Rows: `plugin.pluginsFolder`,
//! `plugin.loadedAtLaunch`, `plugin.installFromFile`, `pluginManager.list`, `pluginManager.remove`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub use crate::error::MAX_INSTALLED;
use crate::manifest::Kind;
use crate::runtime::{Limits, Plugin};
use crate::{Error, Result, sha256};

/// One installed module.
#[derive(Clone, Debug)]
pub struct Installed {
    pub plugin: Arc<Plugin>,
    /// Turned on: listed in menus, run by commands and the audit.
    pub enabled: bool,
    /// The file name in the Modules folder it came from; `None` for a module installed from
    /// bytes that has not been written to the folder.
    pub file: Option<String>,
    /// Found in the folder without the user turning it on (a manual copy).
    pub new: bool,
    /// SHA-256 of the module file, lowercase hex; empty for a module installed as a loaded
    /// [`Plugin`] ([`Registry::install`]).
    pub sha256: String,
}

/// What [`Registry::sync_folder`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FolderReport {
    /// Ids of the modules loaded or reloaded.
    pub loaded: Vec<String>,
    /// Ids of modules seen for the first time (installed off).
    pub new: Vec<String>,
    /// Ids of modules whose file is gone (uninstalled).
    pub removed: Vec<String>,
    /// `(file, error)` for each file that failed to load.
    pub failed: Vec<(String, String)>,
}

/// The installed modules, in install order.
#[derive(Clone, Debug, Default)]
pub struct Registry {
    modules: Vec<Installed>,
    /// Bumped on every change; menus and caches compare it.
    revision: u64,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn list(&self) -> &[Installed] {
        &self.modules
    }

    pub fn len(&self) -> usize {
        self.modules.len()
    }

    pub fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }

    /// Changes with every install, removal, reload or switch.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn get(&self, id: &str) -> Option<&Installed> {
        self.modules.iter().find(|m| m.plugin.id() == id)
    }

    /// The turned-on modules of `kind`, in install order.
    pub fn enabled_of_kind(&self, kind: Kind) -> impl Iterator<Item = &Installed> {
        self.modules.iter().filter(move |m| m.enabled && m.plugin.kind() == kind)
    }

    /// Installs `plugin`, turned on, replacing a module with the same id. Refused past
    /// [`MAX_INSTALLED`].
    pub fn install(&mut self, plugin: Plugin, file: Option<String>) -> Result<&Installed> {
        self.put(Installed { plugin: Arc::new(plugin), enabled: true, file, new: false, sha256: String::new() })
    }

    /// Loads `bytes` under the default limits and installs the module ([`Registry::install`]),
    /// keeping the file's checksum.
    pub fn install_bytes(&mut self, bytes: &[u8], file: Option<String>) -> Result<&Installed> {
        let plugin = Plugin::load(bytes, Limits::default())?;
        self.put(Installed { plugin: Arc::new(plugin), enabled: true, file, new: false, sha256: sha256::hex(bytes) })
    }

    fn put(&mut self, entry: Installed) -> Result<&Installed> {
        let at = match self.modules.iter().position(|m| m.plugin.id() == entry.plugin.id()) {
            Some(at) => {
                let slot = self.modules.get_mut(at).ok_or_else(|| Error::NotFound(entry.plugin.id().to_string()))?;
                *slot = entry;
                at
            }
            None if self.modules.len() >= MAX_INSTALLED => {
                return Err(Error::TooMany(MAX_INSTALLED));
            }
            None => {
                self.modules.push(entry);
                self.modules.len() - 1
            }
        };
        self.revision = self.revision.wrapping_add(1);
        self.modules.get(at).ok_or_else(|| Error::NotFound(String::new()))
    }

    /// The module installed from the folder file `file`.
    pub fn by_file(&self, file: &str) -> Option<&Installed> {
        self.modules.iter().find(|m| m.file.as_deref() == Some(file))
    }

    /// Records that the module `id` now lives in the folder file `file` (after the engine wrote
    /// it there), with the file's checksum, so the next folder sync keeps it as it is.
    pub fn set_file(&mut self, id: &str, file: Option<String>, sha256: String) -> Result<()> {
        let m = self.modules.iter_mut().find(|m| m.plugin.id() == id).ok_or_else(|| Error::NotFound(id.to_string()))?;
        m.file = file;
        m.sha256 = sha256;
        self.revision = self.revision.wrapping_add(1);
        Ok(())
    }

    /// Removes the module `id`; false when none is installed.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.modules.len();
        self.modules.retain(|m| m.plugin.id() != id);
        let removed = self.modules.len() != before;
        if removed {
            self.revision = self.revision.wrapping_add(1);
        }
        removed
    }

    /// Turns the module `id` on or off; it stops being new.
    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> Result<()> {
        let m = self.modules.iter_mut().find(|m| m.plugin.id() == id).ok_or_else(|| Error::NotFound(id.to_string()))?;
        m.enabled = enabled;
        m.new = false;
        self.revision = self.revision.wrapping_add(1);
        Ok(())
    }

    /// Brings the registry in line with the Modules folder: `files` are `(file name, bytes)` of
    /// every `.wasm` in it (the caller reads them, capped at the module limit); `known` is the
    /// user's switch per file name. A file not in `known` is installed off and reported as new;
    /// a module whose file is gone is removed; a file that fails to load is reported and skipped,
    /// and the module it held before is removed. Modules installed without a file are kept, unless
    /// a folder file holds the same id: that copy takes their place and keeps their switch.
    ///
    /// A file whose checksum matches the module already installed from it is not loaded again,
    /// so a sync after an unrelated change is cheap. A second file with the id of a module
    /// installed from another file in the folder is refused (the first one keeps the id). A
    /// changed file keeps its module's switch.
    pub fn sync_folder(&mut self, files: &[(&str, &[u8])], known: &BTreeMap<String, bool>) -> FolderReport {
        let mut report = FolderReport::default();
        let listed: BTreeSet<&str> = files.iter().map(|(name, _)| *name).collect();
        let before = self.modules.len();
        self.modules.retain(|m| match &m.file {
            Some(file) if !listed.contains(file.as_str()) => {
                report.removed.push(m.plugin.id().to_string());
                false
            }
            _ => true,
        });
        let mut changed = self.modules.len() != before;
        let mut seen = BTreeSet::new();
        for (name, bytes) in files {
            if !seen.insert(*name) {
                continue;
            }
            let hash = sha256::hex(bytes);
            if self.by_file(name).is_some_and(|m| m.sha256 == hash) {
                continue;
            }
            let plugin = match Plugin::load(bytes, Limits::default()) {
                Ok(plugin) => plugin,
                Err(e) => {
                    changed |= self.drop_file(name, &mut report);
                    report.failed.push(((*name).to_string(), e.to_string()));
                    continue;
                }
            };
            let id = plugin.id().to_string();
            if let Some(other) = self.get(&id).and_then(|m| m.file.clone()).filter(|f| f.as_str() != *name) {
                changed |= self.drop_file(name, &mut report);
                report.failed.push(((*name).to_string(), format!("module {id:?} is already installed from {other}")));
                continue;
            }
            // The switch of what this file replaces: the module it held, or one with the same id
            // installed without a file.
            let old = self.by_file(name).or_else(|| self.get(&id));
            let (enabled, new) = match old {
                Some(m) => (m.enabled, m.new),
                None => match known.get(*name) {
                    Some(on) => (*on, false),
                    None => (false, true),
                },
            };
            let slot = self.modules.iter().position(|m| m.file.as_deref() == Some(*name) || (m.file.is_none() && m.plugin.id() == id));
            if slot.is_none() && self.modules.len() >= MAX_INSTALLED {
                report.failed.push(((*name).to_string(), Error::TooMany(MAX_INSTALLED).to_string()));
                continue;
            }
            if let Some(old_id) = self.by_file(name).map(|m| m.plugin.id().to_string()).filter(|old| *old != id) {
                report.removed.push(old_id);
            }
            let entry = Installed { plugin: Arc::new(plugin), enabled, file: Some((*name).to_string()), new, sha256: hash };
            match slot.and_then(|at| self.modules.get_mut(at)) {
                Some(slot) => *slot = entry,
                None => self.modules.push(entry),
            }
            // What the new entry replaces and did not take the slot of: the file's old module
            // under another id, or a file-less module with this id.
            let mut kept_one = false;
            self.modules.retain(|m| {
                let same_file = m.file.as_deref() == Some(*name);
                if m.plugin.id() != id {
                    return !same_file;
                }
                let keep = !kept_one && same_file;
                kept_one |= keep;
                keep
            });
            if new {
                report.new.push(id.clone());
            }
            report.loaded.push(id);
            changed = true;
        }
        if changed {
            self.revision = self.revision.wrapping_add(1);
        }
        report
    }

    /// Removes the module installed from `file`, reporting it; whether one was.
    fn drop_file(&mut self, file: &str, report: &mut FolderReport) -> bool {
        let Some(id) = self.by_file(file).map(|m| m.plugin.id().to_string()) else { return false };
        self.modules.retain(|m| m.file.as_deref() != Some(file));
        report.removed.push(id);
        true
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::Registry;
    use crate::wat;

    fn wasm(text: &str) -> Vec<u8> {
        ::wat::parse_str(text).unwrap()
    }

    #[test]
    fn folder_sync_lists_manual_copies_as_new_and_off_and_follows_the_folder() {
        let shift = wasm(&wat::shift_filter("org.test.shift"));
        let bbox = wasm(&wat::bbox_reporter("org.test.bbox"));
        let known = BTreeMap::from([("shift.wasm".to_string(), true)]);
        let mut r = Registry::new();
        let report = r.sync_folder(&[("shift.wasm", &shift), ("bbox.wasm", &bbox), ("junk.wasm", b"junk")], &known);
        assert_eq!(report.loaded, ["org.test.shift", "org.test.bbox"]);
        assert_eq!(report.new, ["org.test.bbox"]);
        assert_eq!(report.failed.len(), 1);
        let (on, copied) = (r.get("org.test.shift").unwrap(), r.get("org.test.bbox").unwrap());
        assert!(on.enabled && !on.new && on.sha256.len() == 64);
        assert!(!copied.enabled && copied.new, "a manual copy stays off until turned on");

        // Nothing changed: nothing is loaded again and the revision stays.
        let revision = r.revision();
        let serial = r.get("org.test.shift").unwrap().plugin.serial();
        let report = r.sync_folder(&[("shift.wasm", &shift), ("bbox.wasm", &bbox)], &known);
        assert!(report.loaded.is_empty() && report.removed.is_empty());
        assert_eq!((r.revision(), r.get("org.test.shift").unwrap().plugin.serial()), (revision, serial));

        // A second file with a taken id is refused; a deleted file uninstalls its module.
        let report = r.sync_folder(&[("bbox.wasm", &bbox), ("copy.wasm", &shift)], &known);
        assert_eq!(report.removed, ["org.test.shift"]);
        assert_eq!(r.get("org.test.shift").map(|m| m.file.clone()), Some(Some("copy.wasm".to_string())));
        let report = r.sync_folder(&[("bbox.wasm", &bbox), ("copy.wasm", &shift), ("again.wasm", &shift)], &known);
        assert!(report.failed.iter().any(|(f, e)| f == "again.wasm" && e.contains("copy.wasm")), "{report:?}");
        assert_eq!(r.len(), 2);
    }
}
