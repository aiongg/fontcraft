//! The command registry. Ids follow the reference app's menu structure (`file.*`, `edit.*`,
//! `glyph.*`, `path.*`…) and are listed with their planned parameters in `spec/inventory/*.toml`;
//! `cargo xtask parity` reports which inventory rows have a command here.
//!
//! To add a command: put it in the module for its menu (or a new module with a `specs()` function,
//! added to [`command_specs`]), with id, label, menu path, shortcut, a parameter description
//! agents can call it from, an `enabled` check and a `run` function, plus tests. `run` reads its
//! parameters through [`crate::params::Params`] and changes fonts only through
//! [`crate::Session::edit`], which makes the change one undo step.

mod annotation;
pub(crate) mod audit;
mod background;
mod clipboard;
mod clipboard_scale;
mod common;
pub(crate) mod compile;
mod custom_params;
mod dimensions;
mod edit;
mod file;
pub mod filter;
pub mod fit_curve;
mod font;
mod font_fields;
mod font_view;
mod glyph;
mod glyph_info;
mod glyph_search;
mod glyph_set;
mod guide;
mod inspect;
mod instances;
mod kerning;
mod kerning_groups;
mod kerning_maintain;
mod kerning_window;
mod layer;
mod layers;
mod masters;
mod measure;
mod measure_line;
mod meta;
mod metric_defs;
mod metrics_io;
mod metrics_keys;
mod new_document;
mod node;
mod path;
mod pathops;
mod project;
mod scale_upm;
mod session;
mod settings;
mod spacing;
pub mod stroke;
mod tokens;
pub mod transform;
mod transform_metrics;
mod widths;

use serde::Serialize;
use serde_json::Value;

use crate::params::TargetKind;
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
    /// The target parameters the shell fills from its selection; empty for a command that
    /// takes none. The shell greys out the menu item when the selection has none of them.
    pub targets: &'static [TargetKind],
    /// Where the shortcut works (`editView`, `textMode`, `fontView`, `dialog`, `tool:<id>`,
    /// `panel:<id>`); `None` is everywhere. Matches the inventory row's `when`.
    pub when: Option<&'static str>,
    /// The context menus that show the command (`canvas`, `canvas.node`…). Matches the
    /// inventory row's `context`.
    pub context: &'static [&'static str],
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
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    pub targets: Vec<TargetKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<&'static str>,
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    pub context: Vec<&'static str>,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
}

impl CommandSpec {
    /// Sets [`CommandSpec::targets`].
    pub const fn targets(mut self, targets: &'static [TargetKind]) -> Self {
        self.targets = targets;
        self
    }

    /// Sets [`CommandSpec::when`].
    pub const fn when(mut self, when: &'static str) -> Self {
        self.when = Some(when);
        self
    }

    /// Sets [`CommandSpec::context`].
    pub const fn context(mut self, context: &'static [&'static str]) -> Self {
        self.context = context;
        self
    }

    pub fn info(&self, s: &Session) -> CommandInfo {
        let e = (self.enabled)(s);
        CommandInfo {
            id: self.id,
            label: self.label,
            menu: self.menu.to_vec(),
            shortcut: self.shortcut,
            params: self.params,
            query: !self.journal,
            targets: self.targets.to_vec(),
            when: self.when,
            context: self.context.to_vec(),
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
        $crate::cmd::CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: $sc, params: $params, enabled: $en, run: $run, journal: true, targets: &[], when: None, context: &[] }
    };
    (query $id:literal, $label:literal, [$($m:literal),*], $sc:expr, $params:literal, $en:expr, $run:expr) => {
        $crate::cmd::CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: $sc, params: $params, enabled: $en, run: $run, journal: false, targets: &[], when: None, context: &[] }
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
        v.extend(clipboard::specs());
        v.extend(glyph::specs());
        v.extend(glyph_set::specs());
        v.extend(path::specs());
        v.extend(spacing::specs());
        v.extend(metrics_keys::specs());
        v.extend(transform_metrics::specs());
        v.extend(kerning::specs());
        v.extend(kerning_groups::specs());
        v.extend(kerning_maintain::specs());
        v.extend(kerning_window::specs());
        v.extend(metrics_io::specs());
        v.extend(font::specs());
        v.extend(compile::specs());
        v.extend(meta::specs());
        v.extend(session::specs());
        v.extend(inspect::specs());
        v.extend(layer::specs());
        v.extend(annotation::specs());
        v.extend(background::specs());
        v.extend(layers::specs());
        v.extend(transform::specs());
        v.extend(guide::specs());
        v.extend(measure::specs());
        v.extend(measure_line::specs());
        v.extend(dimensions::specs());
        v.extend(node::specs());
        v.extend(fit_curve::specs());
        v.extend(filter::specs());
        v.extend(stroke::specs());
        v.extend(pathops::specs());
        v.extend(audit::specs());
        v.extend(settings::specs());
        v.extend(glyph_info::specs());
        v.extend(font_view::specs());
        v.extend(new_document::specs());
        v.extend(glyph_search::specs());
        v.extend(custom_params::specs());
        v.extend(font_fields::specs());
        v.extend(scale_upm::specs());
        v.extend(masters::specs());
        v.extend(metric_defs::specs());
        v.extend(instances::specs());
        v.extend(clipboard_scale::specs());
        v.extend(tokens::specs());
        v.extend(project::specs());
        v
    })
}

pub fn find_command(id: &str) -> Option<&'static CommandSpec> {
    static INDEX: std::sync::OnceLock<std::collections::HashMap<&'static str, &'static CommandSpec>> = std::sync::OnceLock::new();
    INDEX.get_or_init(|| command_specs().iter().map(|c| (c.id, c)).collect()).get(id).copied()
}
