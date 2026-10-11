//! The extension windows and their shell commands: the Extension Manager (`window.pluginManager`,
//! M13.10: index filters, search, install, remove, updates, the index list), reporter switches
//! (`view.showReporter`, M13.6), and the external-changes dialog (M13.45, unless M12's diff window
//! takes it). Module and macro menu items are built in [`super::menus_dynamic`]. (M13.6, M13.10,
//! M13.45 fill this module.)

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Args, UiCommand};
use crate::app::FontcraftApp;
use crate::panels::Pending;

/// This module's view and window state, kept in `UiState::panels`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ExtensionsState {}

pub const COMMANDS: &[UiCommand] = &[];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    let _ = (app, args);
    Err(format!("unknown command `{id}`"))
}

/// Draws this module's windows while they are open.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let _ = (app, ui, pending);
}
