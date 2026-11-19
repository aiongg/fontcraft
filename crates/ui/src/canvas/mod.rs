//! The document area: a craft `Canvas` over the edit view.
//!
//! It runs every pass (the canvas is never memoized). The canvas folds the wheel, pinch, middle-button pan and its
//! view commands into the view and writes it back to `ViewState::canvas`. Its body paints [`layers::LAYERS`] through
//! the [`camera`], then craft's `ToolHost` runs the active tool ([`ActiveTool`]) on the canvas's pass: the tool gets
//! the background press, hover and keys and paints its feedback over the layers.

pub mod camera;
pub mod layers;
pub mod paint;
pub mod tokens;

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
        }
    }
}

/// The tool in the canvas's active-tool slot: a craft `CanvasTool` that also hands over the engine commands its
/// gestures made. The tool bridge over `fontcraft_tools` fills the slot; until then it holds [`NoTool`].
pub trait ActiveTool: CanvasTool {
    /// The messages the tool's gestures posted since the last call.
    fn take_sent(&mut self) -> Vec<Msg> {
        Vec::new()
    }
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
    /// The active-tool slot.
    pub tool: &'a mut dyn ActiveTool,
}

/// Shows the canvas; what the active tool's gestures posted goes to `sent`.
pub fn show(ui: &mut Ui, props: &mut CanvasProps<'_>, sent: &mut Vec<Msg>) {
    let words = Env::of(ui).words::<ChromeWords>();
    let paint_words = Env::of(ui).words::<paint::CanvasWords>();
    let glyph = props.model.glyph.as_ref();
    // The document is the edited glyph's advance box from the ascender to the descender.
    let document = glyph.map(|g| Size::new(g.width.max(1.0), (g.ascender - g.descender).max(1.0)));
    let top = glyph.map_or(0.0, |g| g.ascender);
    let mut canvas = Canvas::new(words.document.clone()).view(Bind::bound(&mut *props.view));
    if let Some(size) = document {
        canvas = canvas.document(size);
    }
    let (model, tokens, display, edit_background) = (props.model, props.tokens, props.display, props.edit_background);
    let tool = &mut *props.tool;
    let mut reply = None;
    let _ = canvas.show(ui, |ui, body| {
        let Some(space) = canvas_space(&Env::of(ui).view()) else { return };
        let painter = ui.painter_at(space.view.rect);
        let camera = Camera::new(space.view, top);
        let cx = PaintCx { painter: &painter, rect: space.view.rect, camera, model, tokens, display, edit_background, words: &paint_words };
        layers::paint(&cx);
        if let Some(pass) = tool_pass(&body.env) {
            reply = Some(ToolHost::new().run(ui, &pass, &mut *tool));
        }
    });
    if let Some(view) = reply.and_then(|r| r.view) {
        *props.view = view;
    }
    sent.extend(props.tool.take_sent());
}
