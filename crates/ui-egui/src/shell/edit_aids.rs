//! Edit View display aids and their View menu switches: Tunni lines, curve tension, indexes, curve
//! length, details across glyphs, ghost outline, Lock Outline, the metrics beam (M13.31); the auto-
//! meter and suggestion switches (M13.34, M13.35). The drawing is in `crate::canvas::edit_aids`,
//! `crate::canvas::autometer` and `crate::canvas::suggestions`. (M13.31, M13.33 to M13.36 fill this
//! module.)

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Args, UiCommand};
use crate::app::FontcraftApp;

/// This module's view and window state, kept in `UiState::panels`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct EditAidsState {}

pub const COMMANDS: &[UiCommand] = &[];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    let _ = (app, args);
    Err(format!("unknown command `{id}`"))
}
