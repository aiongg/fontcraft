//! The canvas's paint layers, drawn bottom to top. A canvas package adds a layer with one line in [`LAYERS`] and its
//! paint function in its own file.

use craft::ui::contract::Tracked;
use egui::{Painter, Rect};

use super::camera::Camera;
use super::tokens::Tokens;
use super::{Display, paint, tools};
use crate::app::models::canvas::{CanvasModel, EditedGlyph};
use crate::app::models::text::TextModel;

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
    /// The selection and the active tool's overlays.
    pub tools: &'a super::tools::ToolView,
    /// The edit view's text around the edited glyph.
    pub text: &'a Tracked<TextModel>,
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
    PaintLayer { id: "textBehind", paint: super::text::paint::behind },
    PaintLayer { id: "otherLayers", paint: paint::background::paint },
    PaintLayer { id: "textGlyphs", paint: super::text::paint::glyphs },
    PaintLayer { id: "outline", paint: paint::outline::paint },
    PaintLayer { id: "components", paint: paint::components::paint },
    PaintLayer { id: "selectedSegments", paint: tools::overlays::segments },
    PaintLayer { id: "nodes", paint: paint::nodes::paint },
    PaintLayer { id: "anchors", paint: paint::anchors::paint },
    PaintLayer { id: "selection", paint: tools::overlays::selection },
    PaintLayer { id: "toolOverlays", paint: tools::overlays::paint },
    PaintLayer { id: "textCursor", paint: super::text::paint::cursor },
    PaintLayer { id: "measurementLine", paint: super::text::paint::measurement },
];

/// Paints every layer.
pub fn paint(cx: &PaintCx<'_>) {
    for layer in LAYERS {
        (layer.paint)(cx);
    }
}
