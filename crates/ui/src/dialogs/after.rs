//! When a dialog opens without `dialog.open`: from a command's menu item, and as the question a command's result asks.
//!
//! A command the person fires (menu item, shortcut) with no parameters opens its dialog instead of running
//! ([`fired`]). A follow-up question ([`DialogSpec::after`]) opens after its command ran in `drain` or from the OK of a
//! run-on-OK dialog, when the result calls for it ([`after_run`]).

use serde_json::{Map, Value};

use super::spec::DialogSpec;
use crate::app::FontCraftApp;

/// The follow-up dialog that `command`, run with `request` and returning `result`, asks, with the parameters to open
/// it with.
pub fn after(command: &str, request: &Value, result: &Value) -> Option<(DialogSpec, Map<String, Value>)> {
    super::SPECS.iter().flat_map(|specs| specs.iter()).find_map(|d| {
        let after = d.after.filter(|a| a.command == command)?;
        (after.params)(request, result).map(|given| (*d, given))
    })
}

/// Opens the question the run of `command` asks, unless a dialog is open. A dialog that cannot open leaves its
/// reason in the status line.
pub fn after_run(app: &mut FontCraftApp, command: &str, request: &Value, result: &Value) {
    if app.dialogs.open.is_some() {
        return;
    }
    if let Some((spec, given)) = after(command, request, result)
        && let Err(e) = super::form::open(app, &spec, &given)
    {
        app.status = e;
    }
}

/// A command the person fired: one that has a dialog and no parameters opens the dialog and returns true; any other
/// returns false and runs.
pub fn fired(app: &mut FontCraftApp, id: &str, params: &Value) -> bool {
    let empty = params.is_null() || params.as_object().is_some_and(Map::is_empty);
    let Some(spec) = empty.then(|| super::for_command(id)).flatten() else { return false };
    if let Err(e) = super::form::open(app, &spec, &Map::new()) {
        app.status = e;
    }
    true
}
