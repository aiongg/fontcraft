//! Snapping suggestions while dragging: handle directions, lines, distances, stems. (M13.35 fills
//! this module.)

use super::{Painting, Scene};
use crate::panels::Pending;

/// Draws this overlay for the edited layer.
pub fn draw(ui: &egui::Ui, c: &Painting, scene: &Scene, pending: &mut Pending) {
    let _ = (ui, c, scene, pending);
    // M13.35.
}
