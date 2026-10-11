//! The Masters tab: the master list (drag to reorder, multiple selection, the plus menu, minus), the General, Metrics & Alignment Zones, Stems and Custom Parameters sections, Multiple Values placeholders, the read-only Axes Coordinates and Number Values.
//!
//! Package M7.17 (`docs/plans/M7.md` § 4) draws it.

use crate::app::FontcraftApp;
use crate::panels::Pending;

/// Draws the tab's contents inside the Font Info window.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    // M7.17: the tab.
    let _ = (app, ui, pending);
}
