//! Display aids in front of the outline: Tunni lines, curve tension, node indexes, curve lengths,
//! details across glyphs, the ghost outline, the metrics beam. (M13.31 fills this module.)

use super::{Painting, Scene};
use crate::panels::Pending;

/// Draws this overlay for the edited layer.
pub fn draw(ui: &egui::Ui, c: &Painting, scene: &Scene, pending: &mut Pending) {
    let _ = (ui, c, scene, pending);
    // M13.31.
}
