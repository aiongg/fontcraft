//! Settings › Extensions (the reference's Addons section, `settings.addons`): the extension
//! folders, the standard-error log switch and the list of extension indexes, drawn by the
//! Extension Manager's Indexes tab body, and a button that opens the Extension Manager.

use serde_json::json;

use crate::app::FontcraftApp;
use crate::panels::Pending;
use crate::shell::extensions::manager;

/// The section's body.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    manager::indexes_section(app, ui, pending);
    ui.add_space(app.tokens.dialog_spacing);
    if ui.button("Open Extension Manager…").clicked() {
        pending.push(("window.pluginManager", json!({})));
    }
}
