//! The Command Console (`window.macroPanel`): a one-line command input with completion of command
//! ids and glyph names, the command log (`fontcraft_engine::log`) and module log lines; history
//! with Up and Down; the setting that also writes the log to standard error
//! (`addons.useSystemConsoleForScriptOutput`). The panel is drawn by
//! `crate::panels::command_console`. (M13.9 fills this module.)

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Args, UiCommand};
use crate::app::FontcraftApp;
use crate::panels::Pending;

/// This module's view and window state, kept in `UiState::panels`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CommandConsoleState {}

pub const COMMANDS: &[UiCommand] = &[];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    let _ = (app, args);
    Err(format!("unknown command `{id}`"))
}

/// Draws this module's windows while they are open.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let _ = (app, ui, pending);
}
