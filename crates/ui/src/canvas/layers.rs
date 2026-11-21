//! The canvas's paint layers, drawn bottom to top. A canvas package adds a layer with one line in [`LAYERS`] and its
//! paint function in its own file.

use craft::ui::contract::Tracked;
use egui::{Painter, Rect};

use super::camera::Camera;
use super::tokens::Tokens;
use super::{Display, color, hinting, overlays, paint, tools};
use crate::app::models::canvas::{CanvasModel, EditedGlyph};
use crate::app::models::compatibility::CompatibilityModel;
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
    /// Guides, measurement labels, annotations, display aids, the auto-meter and audit markers.
    pub overlays: super::overlays::Painted<'a>,
    /// The hint layers' model and state.
    pub hinting: super::hinting::HintPaint<'a>,
    /// The Master Compatibility view.
    pub compatibility: &'a Tracked<CompatibilityModel>,
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
    PaintLayer { id: "images", paint: super::images::paint },
    PaintLayer { id: "metrics", paint: paint::metrics::paint },
    PaintLayer { id: "reportersBack", paint: super::reporters::back },
    PaintLayer { id: "ttPixels", paint: hinting::pixels::backdrop },
    PaintLayer { id: "grid", paint: overlays::guides::grid },
    PaintLayer { id: "guides", paint: overlays::guides::guides },
    PaintLayer { id: "psHintBars", paint: hinting::ps::bars },
    PaintLayer { id: "textBehind", paint: super::text::paint::behind },
    PaintLayer { id: "otherLayers", paint: paint::background::paint },
    PaintLayer { id: "textColor", paint: color::paint::text },
    PaintLayer { id: "textGlyphs", paint: super::text::paint::glyphs },
    PaintLayer { id: "color", paint: color::paint::paint },
    PaintLayer { id: "ttPreviewOutlines", paint: hinting::pixels::outlines },
    PaintLayer { id: "outline", paint: paint::outline::paint },
    PaintLayer { id: "components", paint: paint::components::paint },
    PaintLayer { id: "auditHighlights", paint: overlays::audit::highlights },
    PaintLayer { id: "masterCompatibility", paint: super::compatibility::paint },
    PaintLayer { id: "selectedSegments", paint: tools::overlays::segments },
    PaintLayer { id: "nodes", paint: paint::nodes::paint },
    PaintLayer { id: "anchors", paint: paint::anchors::paint },
    PaintLayer { id: "annotations", paint: overlays::annotations::annotations },
    PaintLayer { id: "editAids", paint: overlays::aids::aids },
    PaintLayer { id: "reportersFront", paint: super::reporters::front },
    PaintLayer { id: "autometer", paint: overlays::autometer::autometer },
    PaintLayer { id: "ttNodes", paint: hinting::badges::nodes },
    PaintLayer { id: "ttBadges", paint: hinting::badges::badge_layer },
    PaintLayer { id: "ttPointNumbers", paint: hinting::badges::numbers },
    PaintLayer { id: "psHintMarks", paint: hinting::ps::marks_layer },
    PaintLayer { id: "imageSelection", paint: super::images::paint_selection },
    PaintLayer { id: "selection", paint: tools::overlays::selection },
    PaintLayer { id: "measurement", paint: overlays::guides::measurement },
    PaintLayer { id: "auditMarkers", paint: overlays::audit::markers },
    PaintLayer { id: "gradientKnobs", paint: color::knobs::paint },
    PaintLayer { id: "suggestOutlines", paint: overlays::suggest::paint },
    PaintLayer { id: "toolOverlays", paint: tools::overlays::paint },
    PaintLayer { id: "textCursor", paint: super::text::paint::cursor },
    PaintLayer { id: "measurementLine", paint: super::text::paint::measurement },
    PaintLayer { id: "textMetricsLabels", paint: super::text::paint::metrics_labels },
];

/// Paints every layer.
pub fn paint(cx: &PaintCx<'_>) {
    for layer in LAYERS {
        (layer.paint)(cx);
    }
}
