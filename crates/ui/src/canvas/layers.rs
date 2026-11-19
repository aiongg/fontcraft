//! The canvas's paint layers, drawn bottom to top. A canvas package adds a layer with one line in [`LAYERS`] and its
//! paint function in its own file.

use craft::ui::contract::Tracked;
use egui::{Painter, Rect};

use super::camera::Camera;
use super::tokens::Tokens;
use super::{Display, paint};
use crate::app::models::canvas::{CanvasModel, EditedGlyph};

/// What a paint layer reads.
pub struct PaintCx<'a> {
    /// The painter clipped to the canvas body.
    pub painter: &'a Painter,
    /// The canvas body's rect on screen.
    pub rect: Rect,
    pub camera: Camera,
    pub model: &'a Tracked<CanvasModel>,
    pub tokens: &'a Tokens,
    pub display: Display,
    /// The background is the side being edited.
    pub edit_background: bool,
    pub words: &'a paint::CanvasWords,
}

impl PaintCx<'_> {
    /// The edited glyph, when there is one.
    pub fn glyph(&self) -> Option<&EditedGlyph> {
        self.model.glyph.as_ref()
    }
}

/// One paint layer.
pub struct PaintLayer {
    /// A stable name, for tests and the layer toggles of the View menu.
    pub id: &'static str,
    pub paint: fn(&PaintCx<'_>),
}

/// Every paint layer, bottom first.
pub const LAYERS: &[PaintLayer] = &[
    PaintLayer { id: "background", paint: paint::background::tint },
    PaintLayer { id: "status", paint: paint::status::paint },
    PaintLayer { id: "metrics", paint: paint::metrics::paint },
    PaintLayer { id: "otherLayers", paint: paint::background::paint },
    PaintLayer { id: "outline", paint: paint::outline::paint },
    PaintLayer { id: "components", paint: paint::components::paint },
    PaintLayer { id: "nodes", paint: paint::nodes::paint },
    PaintLayer { id: "anchors", paint: paint::anchors::paint },
];

/// Paints every layer.
pub fn paint(cx: &PaintCx<'_>) {
    for layer in LAYERS {
        (layer.paint)(cx);
    }
}
