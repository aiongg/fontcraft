//! The reference font's glyph drawn behind the outline (`view.setReferenceFont`). (M13.32 fills
//! this module.)

use super::{Painting, Scene};
use crate::panels::Pending;

/// Draws this overlay for the edited layer.
pub fn draw(ui: &egui::Ui, c: &Painting, scene: &Scene, pending: &mut Pending) {
    let _ = (ui, c, scene, pending);
    // M13.32.
}
