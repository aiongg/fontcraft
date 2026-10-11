//! The Exports tab: the export list (Cmd and Shift selection, drag, grey icons for inactive entries, the plus menu), the instance preview line, the instance fields with the class menus and style linking.
//!
//! Package M7.18 (`docs/plans/M7.md` § 4) draws it.

use crate::app::FontcraftApp;
use crate::panels::Pending;

/// Draws the tab's contents inside the Font Info window.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    // M7.18: the tab.
    let _ = (app, ui, pending);
}
