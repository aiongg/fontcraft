//! Reporter module drawings: background primitives behind the outline, foreground primitives in
//! front, from the reporter cache in the session's extension state. (M13.6 fills this module.)

use super::{Painting, Scene};
use crate::panels::Pending;

/// Draws the shown reporters' background primitives.
pub fn draw_back(ui: &egui::Ui, c: &Painting, scene: &Scene, pending: &mut Pending) {
    let _ = (ui, c, scene, pending);
    // M13.6.
}

/// Draws the shown reporters' foreground primitives.
pub fn draw_front(ui: &egui::Ui, c: &Painting, scene: &Scene, pending: &mut Pending) {
    let _ = (ui, c, scene, pending);
    // M13.6.
}
