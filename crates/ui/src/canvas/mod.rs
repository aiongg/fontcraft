//! The document area: a craft `Canvas` over the edit view.
//!
//! It runs every pass (the canvas is never memoized). The canvas folds the wheel, pinch, middle-button pan and its
//! view commands into the view and writes it back to `ViewState::canvas`. Its body paints [`layers::LAYERS`] through
//! the [`camera`], then craft's `ToolHost` runs the active tool ([`ActiveTool`]) on the canvas's pass: the tool gets
//! the background press, hover and keys and paints its feedback over the layers.

pub mod annotation_editor;
pub mod camera;
pub mod color;
pub mod compatibility;
pub mod hinting;
pub mod images;
pub mod layers;
pub mod overlays;
pub mod paint;
pub mod reporters;
pub mod text;
pub mod tokens;
pub mod tools;

use craft::behavior::CanvasView;
use craft::behavior::kurbo::Size;
use craft::ui::canvas::{Canvas, CanvasTool, NoTool, ToolHost, canvas_space, tool_pass};
use craft::ui::contract::{Bind, ShowContainer as _, Tracked};
use craft::ui::runtime::env::Env;
use egui::Ui;

use self::camera::Camera;
use self::layers::PaintCx;
use self::tokens::Tokens;
use crate::app::Msg;
use crate::app::models::canvas::CanvasModel;
use crate::app::models::text::TextModel;
use crate::shell::ChromeWords;

/// The View menu's display toggles (`view.show*`, `shell::view`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Display {
    pub nodes: bool,
    pub nodes_in_background: bool,
    pub extra_nodes: bool,
    pub metrics: bool,
    pub metrics_names: bool,
    pub background: bool,
    pub anchors: bool,
    pub bounding_box: bool,
    pub guides: bool,
    pub annotations: bool,
    pub fill_preview: bool,
    pub info: bool,
    /// The edited layer's PostScript hints (`view.showHints`).
    pub hints: bool,
    /// Every source layer of the edited glyph beside it (`view.showMasterCompatibility`).
    pub master_compatibility: bool,
    /// View › Show Image: placed images show (a side with no paths and no components shows its image regardless).
    pub image: bool,
}

impl Default for Display {
    fn default() -> Self {
        Display {
            nodes: true,
            nodes_in_background: false,
            extra_nodes: false,
            metrics: true,
            metrics_names: false,
            background: true,
            anchors: true,
            bounding_box: false,
            guides: true,
            annotations: true,
            fill_preview: true,
            info: true,
            hints: false,
            master_compatibility: false,
            image: true,
        }
    }
}

/// The tool in the canvas's active-tool slot: a craft `CanvasTool` that also hands over the engine commands its
/// gestures made. [`tools::bridge::ToolBridge`] fills the slot.
pub trait ActiveTool: CanvasTool {
    /// The messages the tool's gestures posted since the last call.
    fn take_sent(&mut self) -> Vec<Msg> {
        Vec::new()
    }

    /// The camera of the coming pass, before the host runs the tool: font units to screen points.
    fn set_camera(&mut self, _camera: Camera) {}

    /// The chosen tool's id, before each event the tool posts is handled: some tools take more keys than others.
    fn set_tool(&mut self, _tool: &'static str) {}
}

impl ActiveTool for NoTool {}

/// What the document area reads and changes. The canvas is never memoized, so its props may hold the active tool
/// (`dyn`), which panel props may not.
pub struct CanvasProps<'a> {
    /// The canvas view: view state the canvas writes back.
    pub view: &'a mut CanvasView,
    pub model: &'a Tracked<CanvasModel>,
    pub tokens: &'a Tokens,
    pub display: Display,
    /// The background is the side being edited.
    pub edit_background: bool,
    /// The selection and the active tool's overlays.
    pub tools: &'a tools::ToolView,
    /// The edit view's text, laid out around the edited glyph.
    pub text: &'a Tracked<TextModel>,
    /// The Master Compatibility view.
    pub compatibility: &'a Tracked<crate::app::models::compatibility::CompatibilityModel>,
    /// The active-tool slot.
    pub tool: &'a mut dyn ActiveTool,
    /// What the overlay layers paint.
    pub overlays: &'a overlays::OverlayData,
    /// The text note being typed, with its field over the note.
    pub note: &'a mut Option<annotation_editor::NoteEdit>,
    /// The hint layers' model and state.
    pub hinting: hinting::HintPaint<'a>,
}

/// Shows the canvas; what the active tool's gestures posted goes to `sent`.
pub fn show(ui: &mut Ui, props: &mut CanvasProps<'_>, sent: &mut Vec<Msg>) {
    let words = Env::of(ui).words::<ChromeWords>();
    let paint_words = Env::of(ui).words::<paint::CanvasWords>();
    let overlay_words = Env::of(ui).words::<overlays::OverlayWords>();
    let glyph = props.model.glyph.as_ref();
    // The document is the edited glyph's advance box from the ascender to the descender.
    let document = glyph.map(|g| Size::new(g.width.max(1.0), (g.ascender - g.descender).max(1.0)));
    let top = glyph.map_or(0.0, |g| g.ascender);
    let mut canvas = Canvas::new(words.document.clone()).view(Bind::bound(&mut *props.view));
    if let Some(size) = document {
        canvas = canvas.document(size);
    }
    let (model, tokens, display, edit_background, tool_view, text, hinting) =
        (props.model, props.tokens, props.display, props.edit_background, props.tools, props.text, props.hinting);
    let overlay_data = props.overlays;
    let note = &mut *props.note;
    let compatibility = props.compatibility;
    let tool = &mut *props.tool;
    let mut reply = None;
    let _ = canvas.show(ui, |ui, body| {
        let Some(space) = canvas_space(&Env::of(ui).view()) else { return };
        let painter = ui.painter_at(space.view.rect);
        let camera = Camera::new(space.view, top);
        let cx = PaintCx {
            painter: &painter,
            rect: space.view.rect,
            camera,
            model,
            tokens,
            display,
            edit_background,
            words: &paint_words,
            tools: tool_view,
            text,
            overlays: overlays::Painted { data: overlay_data, words: &overlay_words },
            hinting,
            compatibility,
        };
        layers::paint(&cx);
        if let Some(edit) = note.as_mut() {
            let at = model.glyph.as_ref().and_then(|g| g.layer.info.annotations.get(edit.index)).map(|a| fontcraft_geom::Point::new(a.x, a.y));
            if let Some(at) = at.map(|p| camera.to_screen(p)).filter(|p| p.x.is_finite() && p.y.is_finite()) {
                annotation_editor::show(ui, annotation_editor::EditorProps { note: edit, at });
            }
        }
        tool.set_camera(camera);
        if let Some(pass) = tool_pass(&body.env) {
            reply = Some(ToolHost::new().run(ui, &pass, &mut *tool));
        }
    });
    // A secondary press the tool did not take, or the Menu key, opens the context menu (`shell::context_menu`).
    if let Some(at) = reply.as_ref().and_then(|r| r.context) {
        let params = serde_json::json!({ "x": at.at.x, "y": at.at.y, "alt": at.mods.alt });
        sent.push(Msg::Shell(craft::ui::runtime::commands::CommandRef::with_params("contextMenu.open", params)));
    }
    if let Some(view) = reply.and_then(|r| r.view) {
        *props.view = view;
    }
    sent.extend(props.tool.take_sent());
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests;
