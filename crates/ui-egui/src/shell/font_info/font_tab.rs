//! The Font tab: Family Name with its character warning, Units per Em with the scale button, Version with the stepper, Creation Date with the update button, the General fields, the read-only Axes and the font's Custom Parameters.
//!
//! Package M7.16 (`docs/plans/M7.md` § 4) draws it.

use crate::app::FontcraftApp;
use crate::panels::Pending;

/// Draws the tab's contents inside the Font Info window.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    // M7.16: the tab.
    let _ = (app, ui, pending);
}
