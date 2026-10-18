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
pub struct EditAidsState {
    /// The snapping suggestions that are on (View › Suggest).
    pub suggest: fontcraft_tools::suggestions::Switches,
}

pub const COMMANDS: &[UiCommand] = &[
    (
        "view.suggestHandles",
        "Suggest Handles",
        &["View", "Suggest"],
        "",
        "{on?: bool} while one handle is dragged, snap it flat, upright, parallel to the segment's other handle or square to it (omitted: toggle)",
    ),
    (
        "view.suggestLines",
        "Suggest Lines",
        &["View", "Suggest"],
        "",
        "{on?: bool} while nodes are dragged, snap to the continuations of nearby straight segments and handles, perpendiculars of diagonal guides, and centrelines between facing edges (omitted: toggle)",
    ),
    (
        "view.suggestDistance",
        "Suggest Distance",
        &["View", "Suggest"],
        "",
        "{on?: bool} while nodes are dragged, snap to outlines parallel to the other segments at the master's suggestDistance (omitted: toggle)",
    ),
    (
        "view.suggestStems",
        "Suggest Stems",
        &["View", "Suggest"],
        "",
        "{on?: bool} while a straight edge is dragged, snap it one of the master's declared stems away from a facing edge (omitted: toggle)",
    ),
];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    if let Some(current) = app.state.panels.edit_aids.suggest.get_mut(id).map(|on| *on) {
        let on = args.toggle(current)?;
        if let Some(flag) = app.state.panels.edit_aids.suggest.get_mut(id) {
            *flag = on;
        }
        return Ok(serde_json::json!({"on": on}));
    }
    Err(format!("unknown command `{id}`"))
}
