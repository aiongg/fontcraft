//! The command registry. Ids follow the reference app's menu structure (`file.*`, `edit.*`,
//! `glyph.*`, `path.*`…) and are listed with their planned parameters in `spec/inventory/*.toml`;
//! `cargo xtask parity` reports which inventory rows have a command here.
//!
//! To add a command: put it in the module for its menu (or a new module with a `specs()` function,
//! added to [`command_specs`]), with id, label, menu path, shortcut, a parameter description
//! agents can call it from, an `enabled` check and a `run` function, plus tests. `run` reads its
//! parameters through [`crate::params::Params`] and changes fonts only through
//! [`crate::Session::edit`], which makes the change one undo step.

mod edit;
mod file;
mod font;
mod glyph;
mod inspect;
mod meta;
mod path;
mod session;
mod spacing;

use serde::Serialize;
use serde_json::Value;

use crate::{Result, Session};

pub type Run = fn(&mut Session, &Value) -> Result<Value>;
pub type Enabled = fn(&Session) -> std::result::Result<(), String>;

/// One command: what menus, shortcut editors and agents need to know, and the code to run.
pub struct CommandSpec {
    pub id: &'static str,
    pub label: &'static str,
    /// Menu placement, e.g. `["Path", "Transformations"]`. Empty: not in a menu.
    pub menu: &'static [&'static str],
    /// Default shortcut in Mac notation (`Cmd+Shift+S`); the UI maps it per platform.
    pub shortcut: Option<&'static str>,
    /// Parameters and result, readable by people and agents.
    pub params: &'static str,
    pub enabled: Enabled,
    pub run: Run,
    /// Recorded in the journal and run inside a rollback snapshot. False only for queries:
    /// commands that read and change nothing, in the session or on disk.
    pub journal: bool,
}

/// What `commands.list` and the control channel return for a command.
#[derive(Clone, Debug, Serialize)]
pub struct CommandInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub menu: Vec<&'static str>,
    pub shortcut: Option<&'static str>,
    pub params: &'static str,
    pub query: bool,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
}

impl CommandSpec {
    pub fn info(&self, s: &Session) -> CommandInfo {
        let e = (self.enabled)(s);
        CommandInfo {
            id: self.id,
            label: self.label,
            menu: self.menu.to_vec(),
            shortcut: self.shortcut,
            params: self.params,
            query: !self.journal,
            enabled: e.is_ok(),
            disabled_reason: e.err(),
        }
    }
}

// ---------- enablement ----------

pub fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}
pub fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}
pub fn can_undo(s: &Session) -> std::result::Result<(), String> {
    s.active().filter(|d| !d.history.undo.is_empty()).map(|_| ()).ok_or_else(|| "nothing to undo".into())
}
pub fn can_redo(s: &Session) -> std::result::Result<(), String> {
    s.active().filter(|d| !d.history.redo.is_empty()).map(|_| ()).ok_or_else(|| "nothing to redo".into())
}

/// `cmd!(id, label, [menu…], shortcut, params, enabled, run)`; `cmd!(query …)` for commands that
/// only read.
macro_rules! cmd {
    ($id:literal, $label:literal, [$($m:literal),*], $sc:expr, $params:literal, $en:expr, $run:expr) => {
        $crate::cmd::CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: $sc, params: $params, enabled: $en, run: $run, journal: true }
    };
    (query $id:literal, $label:literal, [$($m:literal),*], $sc:expr, $params:literal, $en:expr, $run:expr) => {
        $crate::cmd::CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: $sc, params: $params, enabled: $en, run: $run, journal: false }
    };
}
pub(crate) use cmd;

/// Every command, in registry order (the order menus and listings show them in).
pub fn command_specs() -> &'static [CommandSpec] {
    static SPECS: std::sync::OnceLock<Vec<CommandSpec>> = std::sync::OnceLock::new();
    SPECS.get_or_init(|| {
        let mut v = Vec::new();
        v.extend(file::specs());
        v.extend(edit::specs());
        v.extend(glyph::specs());
        v.extend(path::specs());
        v.extend(spacing::specs());
        v.extend(font::specs());
        v.extend(meta::specs());
        v.extend(session::specs());
        v.extend(inspect::specs());
        v
    })
}

pub fn find_command(id: &str) -> Option<&'static CommandSpec> {
    static INDEX: std::sync::OnceLock<std::collections::HashMap<&'static str, &'static CommandSpec>> = std::sync::OnceLock::new();
    INDEX.get_or_init(|| command_specs().iter().map(|c| (c.id, c)).collect()).get(id).copied()
}
