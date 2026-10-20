//! The extension windows and their shell commands: the Extension Manager (`window.pluginManager`,
//! M13.10: index filters, search, install, remove, updates, the index list), reporter switches
//! (`view.showReporter`, M13.6), and the external-changes dialog (M13.45, unless M12's diff window
//! takes it). Module and macro menu items are built in [`super::menus_dynamic`]. (M13.6, M13.10,
//! M13.45 fill this module.)

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod dropped;
pub mod external_changes;
pub mod manager;
pub mod reporters;

use super::{Args, UiCommand};
use crate::app::FontcraftApp;
use crate::panels::Pending;

/// This module's view and window state, kept in `UiState::panels`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ExtensionsState {
    /// The reporter modules Edit View draws, by module id, in the order they were switched on
    /// (`view.showReporter`); remembered across launches.
    pub reporters: Vec<String>,
    /// The External Changes dialog (session only).
    #[serde(skip)]
    pub external: external_changes::ExternalChangesState,
    /// The Extension Manager (session only).
    #[serde(skip)]
    pub manager: manager::ManagerState,
}

pub const COMMANDS: &[UiCommand] = &[
    external_changes::COMMAND,
    manager::OPEN,
    manager::FILTER_STATE,
    manager::FILTER_TYPE,
    manager::SEARCH,
    manager::ENLARGE,
    manager::WEBSITE,
    manager::DONATE,
    reporters::COMMAND,
];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    if id == external_changes::COMMAND.0 {
        return external_changes::run(app, args);
    }
    if manager::COMMANDS.iter().any(|c| c.0 == id) {
        return manager::run(app, id, args);
    }
    if id == reporters::COMMAND.0 {
        return reporters::run(app, args);
    }
    let _ = (app, args);
    Err(format!("unknown command `{id}`"))
}

/// Draws this module's windows while they are open.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    external_changes::show(app, ui, pending);
    manager::show(app, ui, pending);
}
