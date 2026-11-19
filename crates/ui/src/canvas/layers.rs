//! The canvas's paint layers, drawn bottom to top. A canvas package adds a layer with one line in [`LAYERS`] and its
//! paint function in its own file.

use egui::{Rect, Ui};

use super::camera::Camera;

/// What a paint layer reads.
pub struct PaintCx<'a> {
    pub ui: &'a Ui,
    /// The canvas body's rect on screen.
    pub rect: Rect,
    pub camera: Camera,
}

/// One paint layer.
pub struct PaintLayer {
    /// A stable name, for tests and the layer toggles of the View menu.
    pub id: &'static str,
    pub paint: fn(&PaintCx<'_>),
}

/// Every paint layer, bottom first.
pub const LAYERS: &[PaintLayer] = &[PaintLayer { id: "background", paint: background }];

/// The canvas's own background, which the canvas component already draws.
fn background(_: &PaintCx<'_>) {}

/// Paints every layer.
pub fn paint(cx: &PaintCx<'_>) {
    for layer in LAYERS {
        (layer.paint)(cx);
    }
}
