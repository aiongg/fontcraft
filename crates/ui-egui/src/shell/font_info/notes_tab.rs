//! The Notes tab: the font note, with Markdown highlighting, through `font.note.get` and `font.note.set`.
//!
//! Package M7.16 (`docs/plans/M7.md` § 4) draws it.

use crate::app::FontcraftApp;
use crate::panels::Pending;

/// Draws the tab's contents inside the Font Info window.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    // M7.16: the tab.
    let _ = (app, ui, pending);
}
