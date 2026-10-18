//! Palette › Smart Component (`palette.smartComponent`): the controls of the Smart Component
//! settings for the selected smart components of the edited layer, at the foot of the Palette
//! while Show Info is on and a smart component is selected. They are the window's controls
//! (`crate::dialogs::smart_component_settings::controls`) and write through the same commands.

use super::Pending;
use crate::app::FontcraftApp;
use crate::dialogs::smart_component_settings::{controls, selected};

/// The salt of the Palette's controls, for their field ids.
pub const SALT: &str = "palette";

/// Whether the section shows: a smart component is selected in the edited layer.
pub fn shown(app: &FontcraftApp) -> bool {
    selected(app).is_some()
}

pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    if let Some(t) = selected(app) {
        controls(app, ui, pending, &t, SALT);
    }
}
