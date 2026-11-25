//! The dialogs the app opens on the window's `DialogHost`.
//!
//! The shell's body calls [`run`] every pass with the host. Each module opens its dialog when the app state asks
//! for it, takes the answer on a later pass and turns it into messages or app state. A module adds one line to
//! [`MODULES`].
//!
//! Form dialogs over one command's parameters are specs ([`spec::DialogSpec`]): a package lists its specs and adds one
//! line to [`SPECS`]; `form.rs` keeps the open one in the app layer and `host.rs` shows it. `docs/ui.md` has the API.

pub mod about;
pub mod actions;
mod add_pixel_glyph;
pub mod after;
pub mod audit_profile;
pub mod axis_mappings;
pub mod changed_on_disk;
pub mod collection_members;
pub mod color_palettes;
mod component_name;
pub mod cross_axis;
pub mod custom_parameter_picker;
pub mod export;
pub mod export_results;
pub mod extension_indexes;
pub mod extension_manager;
pub mod external_changes;
pub mod feature_picker;
pub mod field_picker;
pub mod fields;
mod filters;
pub mod find_glyphs;
pub mod font_data;
pub mod form;
pub mod glyph_picker;
mod glyph_set;
mod guide_properties;
pub mod guide_scope;
mod host;
pub mod import;
pub mod interp;
pub mod kerning_audit;
pub mod kerning_questions;
pub mod loss_question;
pub mod master_color;
pub mod open_report;
pub mod os2_bits;
pub mod overlay_fonts;
pub mod panose;
pub mod path;
pub mod primitive_measurements;
pub mod print;
pub mod rename_by_namelist;
pub mod report_list;
pub mod review_hints;
pub mod rule_tree;
pub mod sample_strings;
pub mod settings;
pub mod settings_appearance;
pub mod shape_order;
pub mod sidebar_filters;
pub mod smart_settings;
pub mod spec;
pub mod start_window;
pub mod status_definitions;
pub mod strike_properties;
pub mod tt;
pub mod unsaved;

use craft::ui::dialog::DialogHost;
use serde_json::Value;

use self::spec::{DialogSpec, Fields};
use crate::app::FontCraftApp;

/// Opens and answers one kind of dialog.
pub type Module = fn(&mut DialogHost, &mut FontCraftApp);

/// Every dialog module.
pub const MODULES: &[Module] = &[
    unsaved::run,
    field_picker::run,
    custom_parameter_picker::run,
    feature_picker::run,
    os2_bits::run,
    panose::run,
    tt::run,
    about::run,
    host::run,
    kerning_questions::run,
    kerning_audit::run,
    overlay_fonts::run,
    start_window::run,
    settings::run,
    changed_on_disk::run,
    loss_question::run,
    collection_members::run,
    open_report::run,
    export_results::run,
    review_hints::run,
    actions::run_dialog,
    audit_profile::run,
    glyph_picker::run_dialog,
    smart_settings::run,
    sample_strings::run_dialog,
    find_glyphs::run_dialog,
    external_changes::run,
    extension_manager::run,
    axis_mappings::run_dialog,
    cross_axis::run_dialog,
    print::run_dialog,
];

/// Every package's dialog specs, one line each.
pub const SPECS: &[&[DialogSpec]] = &[
    filters::SPECS,
    glyph_set::SPECS,
    export::SPECS,
    import::SPECS,
    font_data::SPECS,
    shape_order::SPECS,
    strike_properties::SPECS,
    path::SPECS,
    guide_properties::SPECS,
    guide_scope::SPECS,
    color_palettes::SPECS,
    master_color::SPECS,
    sidebar_filters::SPECS,
    component_name::SPECS,
    interp::SPECS,
    primitive_measurements::SPECS,
    add_pixel_glyph::SPECS,
    rename_by_namelist::SPECS,
    status_definitions::SPECS,
];

/// Runs every dialog module for this pass.
pub fn run(host: &mut DialogHost, app: &mut FontCraftApp) {
    for module in MODULES {
        module(host, app);
    }
}

/// The dialog with this id: a spec or a tab of one, else one generated for a filter.
pub fn find(id: &str) -> Option<DialogSpec> {
    let specs = || SPECS.iter().flat_map(|specs| specs.iter());
    if let Some(d) = specs().find(|d| d.id == id) {
        return Some(*d);
    }
    let tab = |d: &DialogSpec| match d.fields {
        Fields::Tabs(list) => list.iter().find(|t| t.dialog.id == id).map(|t| t.dialog),
        _ => None,
    };
    if let Some(d) = specs().find_map(tab) {
        return Some(d);
    }
    filters_with_params().into_iter().find(|(command, _)| generated_id(command) == id).map(|(command, title)| generated(command, title))
}

/// The dialog of the command `command`, if it has one. Follow-up questions are not.
pub fn for_command(command: &str) -> Option<DialogSpec> {
    if let Some(d) = SPECS.iter().flat_map(|specs| specs.iter()).find(|d| d.command == command && !d.follow_up) {
        return Some(*d);
    }
    filters_with_params().into_iter().find(|(c, _)| *c == command).map(|(command, title)| generated(command, title))
}

/// Every command that has a dialog ([`for_command`]).
pub fn commands_with_dialogs() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = SPECS.iter().flat_map(|specs| specs.iter()).filter(|d| !d.follow_up).map(|d| d.command).collect();
    out.extend(filters_with_params().into_iter().map(|(command, _)| command));
    out
}

fn generated(command: &'static str, title: &'static str) -> DialogSpec {
    DialogSpec::new("", title, command, Fields::Filter)
}

/// The id of a generated filter dialog: the command's, with `filter.` turned into `dialog.filter` and the next letter
/// capitalized (`filter.roundCorners` → `dialog.filterRoundCorners`).
fn generated_id(command: &str) -> String {
    let rest = command.strip_prefix("filter.").unwrap_or(command);
    let mut chars = rest.chars();
    chars.next().map_or_else(String::new, |c| format!("dialog.filter{}{}", c.to_ascii_uppercase(), chars.as_str()))
}

/// The filter commands in the Filter menu that take parameters, with their labels.
fn filters_with_params() -> Vec<(&'static str, &'static str)> {
    let entries = filter_list();
    fontcraft_engine::command_specs()
        .iter()
        .filter(|c| c.id.starts_with("filter.") && c.menu == ["Filter"])
        .filter(|c| {
            entries.iter().any(|e| {
                e.get("command").and_then(Value::as_str) == Some(c.id) && e.get("params").and_then(Value::as_array).is_some_and(|p| !p.is_empty())
            })
        })
        .map(|c| (c.id, c.label))
        .collect()
}

/// The entries of `filter.list`. It reads no document, so a scratch session answers it and the app's session (perhaps
/// inside an interaction) is left alone.
fn filter_list() -> Vec<Value> {
    let mut scratch = fontcraft_engine::Session::new();
    let Ok(all) = scratch.execute("filter.list", &Value::Null) else { return Vec::new() };
    all.get("filters").and_then(Value::as_array).cloned().unwrap_or_default()
}

/// The `filter.list` entry of the filter command `command`.
fn filter_entry(command: &str) -> Option<Value> {
    filter_list().into_iter().find(|f| f.get("command").and_then(Value::as_str) == Some(command))
}

#[cfg(test)]
mod tests;
