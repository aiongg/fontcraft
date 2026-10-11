//! The Actions dialog (`dialog.actions`): any command that takes `glyphs` run over a glyph scope
//! and layers with a before-and-after preview, and a sidebar that builds a macro. The dialog is
//! drawn by `crate::dialogs::actions`. (M13.8 fills this module.)

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Args, UiCommand};
use crate::app::FontcraftApp;
use crate::panels::Pending;

/// This module's view and window state, kept in `UiState::panels`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ActionsState {}

pub const COMMANDS: &[UiCommand] = &[];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    let _ = (app, args);
    Err(format!("unknown command `{id}`"))
}

/// Draws this module's windows while they are open.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let _ = (app, ui, pending);
}
