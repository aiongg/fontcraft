//! The command registry. Ids follow the reference app's menu structure (`file.*`, `edit.*`,
//! `glyph.*`, `path.*`…) and are listed with their planned parameters in `spec/inventory/*.toml`;
//! `cargo xtask parity` reports which inventory rows have a command here.
//!
//! To add a command: put it in the module for its menu (or a new module with a `specs()` function,
//! added to [`COMMAND_TABLES`]), with id, label, menu path, shortcut, a parameter description
//! agents can call it from, an `enabled` check and a `run` function, plus tests. `run` reads its
//! parameters through [`crate::params::Params`] and changes fonts only through
//! [`crate::Session::edit`], which makes the change one undo step.

mod anchor;
mod anchor_context;
mod annotation;
mod attributes;
pub(crate) mod audit;
mod autohint;
mod autohint_font;
mod autohint_report;
mod autohint_tt;
mod axes;
mod background;
pub mod bitmap_edit;
mod bitmap_strikes;
mod clipboard;
mod clipboard_scale;
mod color_export;
mod color_layer;
mod common;
mod compare;
pub(crate) mod compile;
mod compile_cid;
mod compile_export;
mod compile_features;
mod compile_layout;
mod compile_masters;
mod compile_outlines;
mod compile_project;
mod compile_select;
mod component;
mod custom_params;
mod decompose;
mod dimensions;
mod edit;
mod features;
pub mod features_check;
pub(crate) mod features_compile;
mod features_entries;
mod features_find;
mod features_io;
mod file;
pub mod filter;
pub mod fit_curve;
mod font;
mod font_fields;
mod font_view;
pub(crate) mod format_warnings;
mod glyph;
mod glyph_info;
mod glyph_search;
mod glyph_set;
pub mod glyph_sources;
mod guide;
mod hint_import;
mod hint_points;
mod hint_preview;
mod hint_ps;
mod hint_tt;
mod hint_tt_defs;
mod hint_tt_more;
mod hoi;
mod image;
#[cfg(not(target_arch = "wasm32"))]
mod import_naming;
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
mod match_masters;
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
mod palette_dark;
mod path;
mod pathops;
pub mod preview;
mod project;
mod scale_upm;
mod session;
mod settings;
mod shared;
pub mod sharing;
pub mod smart;
mod snippets;
mod spacing;
mod spacing_shared;
mod special_layers;
pub mod stroke;
mod tokens;
pub mod transform;
mod transform_metrics;
mod vertical_metrics;
mod widths;
pub(crate) use widths::color_layers_follow_master;
// M12: whole-file tools and the audit services; file plumbing in `file_io`.
mod audit_fix_binary;
mod audit_fix_glyph;
mod audit_fix_info;
mod audit_fix_recalc;
mod audit_fix_source;
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
#[cfg(not(target_arch = "wasm32"))]
mod subset_source;
mod tables_file;
mod tables_import;
mod transform_file;
pub mod variation_tools;
mod variations_file;
// M13 command modules (M13.1 skeleton; each package fills its own file).
mod attach_cap;
mod attach_clipboard;
mod attach_segment;
mod autokern;
mod autospace;
mod autotrace;
mod cid;
mod component_instance;
mod construction;
mod corner;
mod corner_helpers;
pub mod cross_axis;
mod derived_glyphs;
mod detect;
mod drag_aids;
pub mod encodings;
pub mod export_profiles;
pub mod export_settings;
mod expressions;
pub mod external_changes;
mod file_helpers;
mod find_outline;
mod geometry_helpers;
mod kerning_audit;
pub mod math_base;
pub mod measurements;
mod metrics_files;
mod nonlinear;
mod overlay;
pub mod pair_list;
mod pixel;
mod plugin;
mod plugin_manager;
mod power_guides;
pub mod registry;
pub mod related;
mod script;
mod segment_reshape;
mod selections;
pub mod status;
mod stem_filters;
pub mod tension;
mod thickness;
pub mod tt_assembly;
mod tt_debug;
mod ttvm;
mod url;
mod view_export;

use serde::Serialize;
use serde_json::Value;

pub use self::file_params::{FileAccess, FileForm, FileParam};
pub use self::registry::{CommandTable, Hooks, Install, SlotInit};
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

/// Every command module's table, in registry order: the order menus and listings show commands
/// in, which `tests/engine/command_ids.txt` pins.
///
/// **A new command module registers with two lines**: `mod my_area;` in the module list above,
/// and `my_area::specs,` at the end of this list (or after the module whose menu neighbours it
/// should follow). Then rewrite the golden list with `FONTCRAFT_BLESS=1 cargo test -p
/// fontcraft-engine --test engine registry::` and check that its diff only adds your ids.
pub const COMMAND_TABLES: &[CommandTable] = &[
    file::specs,
    compile_export::specs,
    export_settings::specs,
    edit::specs,
    clipboard::specs,
    corner::specs,
    pixel::specs,
    glyph::specs,
    decompose::specs,
    glyph_set::specs,
    path::specs,
    spacing::specs,
    vertical_metrics::specs,
    metrics_keys::specs,
    spacing_shared::specs,
    transform_metrics::specs,
    kerning::specs,
    kerning_groups::specs,
    kerning_maintain::specs,
    kerning_window::specs,
    metrics_io::specs,
    font::specs,
    compile::specs,
    meta::specs,
    session::specs,
    inspect::specs,
    layer::specs,
    annotation::specs,
    background::specs,
    layers::specs,
    transform::specs,
    guide::specs,
    measure::specs,
    measure_line::specs,
    dimensions::specs,
    node::specs,
    segment_reshape::specs,
    fit_curve::specs,
    filter::specs,
    stroke::specs,
    pathops::specs,
    audit::specs,
    settings::specs,
    sharing::specs,
    glyph_info::specs,
    font_view::specs,
    new_document::specs,
    glyph_search::specs,
    custom_params::specs,
    font_fields::specs,
    scale_upm::specs,
    masters::specs,
    metric_defs::specs,
    instances::specs,
    clipboard_scale::specs,
    tokens::specs,
    project::specs,
    features::specs,
    features_entries::specs,
    features_find::specs,
    numbers::specs,
    features_compile::specs,
    features_check::specs,
    smart::specs,
    snippets::specs,
    anchor_context::specs,
    anchor::specs,
    features_io::specs,
    axes::specs,
    special_layers::specs,
    interpolation::specs,
    match_masters::specs,
    hoi::specs,
    glyph_sources::specs,
    instance_masters::specs,
    variation_tools::specs,
    compare::specs,
    preview::specs,
    hint_ps::specs,
    hint_tt_defs::specs,
    hint_tt::specs,
    hint_tt_more::specs,
    hint_points::specs,
    hint_import::specs,
    hint_preview::specs,
    autohint::specs,
    autohint_tt::specs,
    autohint_report::specs,
    autohint_font::specs,
    palette::specs,
    palette_dark::specs,
    color_layer::specs,
    attributes::specs,
    image::specs,
    color_export::specs,
    tables_file::specs,
    tables_import::specs,
    dump::specs,
    collection::specs,
    layout_file::specs,
    glyph_order_file::specs,
    bitmap_strikes::specs,
    bitmap_edit::specs,
    metrics_report::specs,
    convert_file::specs,
    subset::specs,
    instance::specs,
    merge_file::specs,
    transform_file::specs,
    optimize::specs,
    dehint::specs,
    outlines_file::specs,
    diff::specs,
    variations_file::specs,
    proof::specs,
    sources::specs,
    audit_services::specs,
    audit_profiles::specs,
    audit_reports::specs,
    audit_fix_binary::specs,
    audit_fix_glyph::specs,
    audit_fix_info::specs,
    plugin::specs,
    plugin_manager::specs,
    script::specs,
    url::specs,
    detect::specs,
    expressions::specs,
    construction::specs,
    related::specs,
    derived_glyphs::specs,
    autospace::specs,
    autokern::specs,
    kerning_audit::specs,
    pair_list::specs,
    stem_filters::specs,
    tension::specs,
    thickness::specs,
    corner_helpers::specs,
    geometry_helpers::specs,
    find_outline::specs,
    nonlinear::specs,
    autotrace::specs,
    selections::specs,
    drag_aids::specs,
    measurements::specs,
    power_guides::specs,
    status::specs,
    attach_cap::specs,
    component_instance::specs,
    attach_segment::specs,
    export_profiles::specs,
    view_export::specs,
    file_helpers::specs,
    external_changes::specs,
    encodings::specs,
    math_base::specs,
    cross_axis::specs,
    cid::specs,
    tt_assembly::specs,
    tt_debug::specs,
    metrics_files::specs,
    component::specs,
    overlay::specs,
];

/// Installs every command table for this process; later calls do nothing. Sessions call it when
/// they are created, so a caller with a session never needs it.
pub fn install() -> Result<()> {
    registry::install(Install { commands: COMMAND_TABLES.to_vec(), slots: vec![crate::feature_state::slot_init()], ..Install::default() })
}

/// Every command, in registry order (the order menus and listings show them in). Installs the
/// command tables first when needed.
pub fn command_specs() -> &'static [CommandSpec] {
    ensure_installed();
    registry::command_specs()
}

/// The command `id`. Installs the command tables first when needed.
pub fn find_command(id: &str) -> Option<&'static CommandSpec> {
    ensure_installed();
    registry::find(id)
}

/// [`install`] where a failure cannot be returned, tried once per process. A failure is logged;
/// lookups then find nothing and [`registry::lookup`] reports the reason.
pub(crate) fn ensure_installed() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        if let Err(e) = install() {
            log::error!("{e}");
        }
    });
}
