//! The installed modules of one session (M13.2 ports the install, replace and remove logic from
//! VectorCraft's process-wide `registry.rs` into this value type; M13.3 adds the folder sync).
//!
//! The registry holds no files and does no I/O: the desktop app reads the Modules folder and
//! hands the bytes in ([`Registry::sync_folder`]); `plugin.installFromFile` hands one module's
//! bytes ([`Registry::install_bytes`]). A module found in the folder that the user has not turned
//! on is listed as new and off (`plugin.noManualMove`). Rows: `plugin.pluginsFolder`,
//! `plugin.loadedAtLaunch`, `plugin.installFromFile`, `pluginManager.list`, `pluginManager.remove`.

use std::collections::BTreeMap;
use std::sync::Arc;

pub use crate::error::MAX_INSTALLED;
use crate::manifest::Kind;
use crate::runtime::{Limits, Plugin};
use crate::{Error, Result};

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
        let _ = (plugin, file);
        // M13.2.
        Err(Error::Unsupported("installing modules".into()))
    }

    /// Loads `bytes` under the default limits and installs the module ([`Registry::install`]).
    pub fn install_bytes(&mut self, bytes: &[u8], file: Option<String>) -> Result<&Installed> {
        let plugin = Plugin::load(bytes, Limits::default())?;
        self.install(plugin, file)
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
    /// user's switch per file name from the settings. A file not in `known` is installed off and
    /// reported as new; a module whose file is gone is removed; a file that fails to load is
    /// reported and skipped. Modules installed without a file are kept.
    pub fn sync_folder(&mut self, files: &[(&str, &[u8])], known: &BTreeMap<String, bool>) -> FolderReport {
        let _ = (files, known);
        // M13.3.
        FolderReport::default()
    }
}
