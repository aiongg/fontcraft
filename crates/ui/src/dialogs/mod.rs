//! The dialogs the app opens on the window's `DialogHost`.
//!
//! The shell's body calls [`run`] every pass with the host. Each module opens its dialog when the app state asks
//! for it, takes the answer on a later pass and turns it into messages or app state. A module adds one line to
//! [`MODULES`].
//!
//! Form dialogs over one command's parameters are specs ([`spec::DialogSpec`]): a package lists its specs and adds one
//! line to [`SPECS`]; `form.rs` keeps the open one in the app layer and `host.rs` shows it. `docs/ui.md` has the API.

pub mod after;
pub mod fields;
mod filters;
pub mod form;
mod host;
pub mod spec;
mod unsaved;

use craft::ui::dialog::DialogHost;
use serde_json::Value;

use self::spec::{DialogSpec, Fields};
use crate::app::FontCraftApp;

/// Opens and answers one kind of dialog.
pub type Module = fn(&mut DialogHost, &mut FontCraftApp);

/// Every dialog module.
pub const MODULES: &[Module] = &[unsaved::run, host::run];

/// Every package's dialog specs, one line each.
pub const SPECS: &[&[DialogSpec]] = &[filters::SPECS];

/// Runs every dialog module for this pass.
pub fn run(host: &mut DialogHost, app: &mut FontCraftApp) {
    for module in MODULES {
        module(host, app);
    }
}

/// The dialog with this id: a spec, else one generated for a filter.
pub fn find(id: &str) -> Option<DialogSpec> {
    if let Some(d) = SPECS.iter().flat_map(|specs| specs.iter()).find(|d| d.id == id) {
        return Some(*d);
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
