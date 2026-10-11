//! The Space Center (`window.spaceCenter`): text with the tokens of `spaceCenter.inputSyntax`,
//! matrix entry through `fontcraft_geom::expr`, pointer and key steps, the spacing cut-off and
//! preferred suffix. The window is drawn by `crate::dialogs::space_center`. (M13.23 fills this
//! module.)

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Args, UiCommand};
use crate::app::FontcraftApp;
use crate::panels::Pending;

/// This module's view and window state, kept in `UiState::panels`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SpaceCenterState {}

pub const COMMANDS: &[UiCommand] = &[];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    let _ = (app, args);
    Err(format!("unknown command `{id}`"))
}

/// Draws this module's windows while they are open.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let _ = (app, ui, pending);
}
