//! Cap, segment and brush interface: the highlight and selection of attachments, the pickers
//! (`crate::dialogs::attach_picker`), the Info boxes (`crate::panels::info_box_attach`) and the
//! edit-glyph arrows (`segmentComponent.editGlyph`, `brush.editGlyph`). (M13.42 fills this module.)

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Args, UiCommand};
use crate::app::FontcraftApp;

/// This module's view and window state, kept in `UiState::panels`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AttachState {}

pub const COMMANDS: &[UiCommand] = &[];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    let _ = (app, args);
    Err(format!("unknown command `{id}`"))
}
