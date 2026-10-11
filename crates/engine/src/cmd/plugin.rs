//! Module commands: install from a file or bytes, remove, list, switch on and off, run a filter,
//! module info, and `app.info` (M13.3 fills it). Rows: `plugin.installFromFile`,
//! `pluginManager.install`, `pluginManager.remove`, `pluginManager.list`, `plugin.uninstall`,
//! `plugin.runFilter`, `plugin.info`, `app.launchWithoutPlugins`, `plugin.lockedPathsAndGlyphs`,
//! `app.info`. The registry and folder state are in `crate::extensions`; the host is
//! `fontcraft_plugins`.

use super::CommandSpec;

pub fn specs() -> Vec<CommandSpec> {
    // M13.3.
    Vec::new()
}
