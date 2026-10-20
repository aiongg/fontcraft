//! The command registry. Ids follow the reference app's menu structure (`file.*`, `edit.*`,
//! `glyph.*`, `path.*`…) and are listed with their planned parameters in `spec/inventory/*.toml`;
//! `cargo xtask parity` reports which inventory rows have a command here.
//!
//! To add a command: put it in the module for its menu (or a new module with a `specs()` function,
//! added to [`command_specs`]), with id, label, menu path, shortcut, a parameter description
//! agents can call it from, an `enabled` check and a `run` function, plus tests. `run` reads its
//! parameters through [`crate::params::Params`] and changes fonts only through
//! [`crate::Session::edit`], which makes the change one undo step.

mod anchor;
mod anchor_context;
mod annotation;
mod attributes;
pub(crate) mod audit;
mod autohint;
mod autohint_report;
mod autohint_tt;
mod axes;
mod background;
mod bitmap_strikes;
mod clipboard;
mod clipboard_scale;
mod color_export;
mod color_layer;
mod common;
mod compare;
pub(crate) mod compile;
mod compile_cid;
mod compile_project;
mod component;
mod custom_params;
mod decompose;
mod dimensions;
mod edit;
mod features;
mod features_compile;
mod features_io;
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
mod hint_import;
mod hint_points;
mod hint_preview;
mod hint_ps;
mod hint_tt;
mod hint_tt_defs;
mod hint_tt_more;
mod image;
mod inspect;
mod instance_masters;
mod instances;
mod interpolation;
mod kerning;
mod kerning_groups;
mod kerning_maintain;
mod kerning_window;
mod layer;
mod layers;
pub mod masters;
mod measure;
mod measure_line;
mod meta;
mod metric_defs;
mod metrics_io;
mod metrics_keys;
mod new_document;
mod node;
mod numbers;
mod palette;
mod path;
mod pathops;
pub mod preview;
mod project;
mod scale_upm;
mod session;
mod settings;
pub mod smart;
mod snippets;
mod spacing;
mod special_layers;
pub mod stroke;
mod tokens;
pub mod transform;
mod transform_metrics;
mod widths;
pub(crate) use widths::color_layers_follow_master;
// M12: whole-file tools and the audit services; file plumbing in `file_io`.
mod audit_fix_binary;
mod audit_fix_glyph;
mod audit_fix_info;
mod audit_fix_recalc;
mod audit_profiles;
mod audit_reports;
pub mod audit_services;
mod collection;
mod convert_file;
mod dehint;
mod diff;
mod dump;
pub mod file_io;
mod file_params;
mod glyph_order_file;
mod instance;
mod layout_file;
mod merge_file;
mod metrics_report;
mod optimize;
mod outlines_file;
mod proof;
mod sources;
mod subset;
mod tables_file;
mod tables_import;
mod transform_file;
mod variations_file;
// M13 command modules (M13.1 skeleton; each package fills its own file).
mod attach_cap;
mod attach_segment;
mod autokern;
mod autospace;
mod autotrace;
mod cid;
mod construction;
mod corner;
mod corner_helpers;
pub mod cross_axis;
mod derived_glyphs;
mod detect;
mod drag_aids;
pub mod encodings;
pub mod export_profiles;
mod expressions;
pub mod external_changes;
mod file_helpers;
mod find_outline;
mod geometry_helpers;
pub mod math_base;
pub mod measurements;
mod metrics_files;
mod nonlinear;
pub mod pair_list;
mod pixel;
mod plugin;
mod plugin_manager;
mod power_guides;
pub mod related;
mod script;
mod selections;
pub mod status;
mod stem_filters;
pub mod tension;
mod thickness;
pub mod tt_assembly;
mod tt_debug;
mod url;

use serde::Serialize;
use serde_json::Value;

pub use self::file_params::{FileAccess, FileForm, FileParam};
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
    /// The parameters that name files or folders. Every parameter the command reads a path
    /// from is listed: surfaces that restrict file access check only these.
    pub files: &'static [FileParam],
    /// Writes to disk: through a write parameter in [`CommandSpec::files`], `inPlace`, or a
    /// place the parameters do not name (the document's own file, a stored destination).
    pub writes_files: bool,
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
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    pub file_params: Vec<FileParam>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub writes_files: bool,
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

    /// Sets [`CommandSpec::files`]; pass `const { &[…] }`.
    pub const fn files(mut self, files: &'static [FileParam]) -> Self {
        self.files = files;
        self
    }

    /// Sets [`CommandSpec::writes_files`].
    pub const fn writes_files(mut self) -> Self {
        self.writes_files = true;
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
            file_params: self.files.to_vec(),
            writes_files: self.writes_files,
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
        $crate::cmd::CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: $sc, params: $params, enabled: $en, run: $run, journal: true, targets: &[], when: None, context: &[], files: &[], writes_files: false }
    };
    (query $id:literal, $label:literal, [$($m:literal),*], $sc:expr, $params:literal, $en:expr, $run:expr) => {
        $crate::cmd::CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: $sc, params: $params, enabled: $en, run: $run, journal: false, targets: &[], when: None, context: &[], files: &[], writes_files: false }
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
        v.extend(corner::specs());
        v.extend(pixel::specs());
        v.extend(glyph::specs());
        v.extend(decompose::specs());
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
        v.extend(features::specs());
        v.extend(numbers::specs());
        v.extend(features_compile::specs());
        v.extend(smart::specs());
        v.extend(snippets::specs());
        v.extend(anchor_context::specs());
        v.extend(anchor::specs());
        v.extend(features_io::specs());
        v.extend(axes::specs());
        v.extend(special_layers::specs());
        v.extend(interpolation::specs());
        v.extend(instance_masters::specs());
        v.extend(compare::specs());
        v.extend(preview::specs());
        v.extend(hint_ps::specs());
        v.extend(hint_tt_defs::specs());
        v.extend(hint_tt::specs());
        v.extend(hint_tt_more::specs());
        v.extend(hint_points::specs());
        v.extend(hint_import::specs());
        v.extend(hint_preview::specs());
        v.extend(autohint::specs());
        v.extend(autohint_tt::specs());
        v.extend(autohint_report::specs());
        v.extend(palette::specs());
        v.extend(color_layer::specs());
        v.extend(attributes::specs());
        v.extend(image::specs());
        v.extend(color_export::specs());
        v.extend(tables_file::specs());
        v.extend(tables_import::specs());
        v.extend(dump::specs());
        v.extend(collection::specs());
        v.extend(layout_file::specs());
        v.extend(glyph_order_file::specs());
        v.extend(bitmap_strikes::specs());
        v.extend(metrics_report::specs());
        v.extend(convert_file::specs());
        v.extend(subset::specs());
        v.extend(instance::specs());
        v.extend(merge_file::specs());
        v.extend(transform_file::specs());
        v.extend(optimize::specs());
        v.extend(dehint::specs());
        v.extend(outlines_file::specs());
        v.extend(diff::specs());
        v.extend(variations_file::specs());
        v.extend(proof::specs());
        v.extend(sources::specs());
        v.extend(audit_services::specs());
        v.extend(audit_profiles::specs());
        v.extend(audit_reports::specs());
        v.extend(audit_fix_binary::specs());
        v.extend(audit_fix_glyph::specs());
        v.extend(audit_fix_info::specs());
        v.extend(plugin::specs());
        v.extend(plugin_manager::specs());
        v.extend(script::specs());
        v.extend(url::specs());
        v.extend(detect::specs());
        v.extend(expressions::specs());
        v.extend(construction::specs());
        v.extend(related::specs());
        v.extend(derived_glyphs::specs());
        v.extend(autospace::specs());
        v.extend(autokern::specs());
        v.extend(pair_list::specs());
        v.extend(stem_filters::specs());
        v.extend(tension::specs());
        v.extend(thickness::specs());
        v.extend(corner_helpers::specs());
        v.extend(geometry_helpers::specs());
        v.extend(find_outline::specs());
        v.extend(nonlinear::specs());
        v.extend(autotrace::specs());
        v.extend(selections::specs());
        v.extend(drag_aids::specs());
        v.extend(measurements::specs());
        v.extend(power_guides::specs());
        v.extend(status::specs());
        v.extend(attach_cap::specs());
        v.extend(attach_segment::specs());
        v.extend(export_profiles::specs());
        v.extend(file_helpers::specs());
        v.extend(external_changes::specs());
        v.extend(encodings::specs());
        v.extend(math_base::specs());
        v.extend(cross_axis::specs());
        v.extend(cid::specs());
        v.extend(tt_assembly::specs());
        v.extend(tt_debug::specs());
        v.extend(metrics_files::specs());
        v.extend(component::specs());
        v
    })
}

pub fn find_command(id: &str) -> Option<&'static CommandSpec> {
    static INDEX: std::sync::OnceLock<std::collections::HashMap<&'static str, &'static CommandSpec>> = std::sync::OnceLock::new();
    INDEX.get_or_init(|| command_specs().iter().map(|c| (c.id, c)).collect()).get(id).copied()
}
