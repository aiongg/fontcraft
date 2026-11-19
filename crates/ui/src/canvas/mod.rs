//! The document area: a craft `Canvas` over the edit view.
//!
//! It runs every pass (the canvas is never memoized). Its body paints [`layers::LAYERS`] through the [`camera`]; the
//! active tool runs in [`active_tool`], which the canvas port fills with craft's `ToolHost` and the tools of
//! `fontcraft_tools`.

pub mod camera;
pub mod layers;

use craft::behavior::CanvasView;
use craft::ui::canvas::{Canvas, canvas_space};
use craft::ui::contract::{Bind, ShowContainer as _};
use craft::ui::runtime::env::Env;
use egui::Ui;

use self::camera::Camera;
use self::layers::PaintCx;
use crate::shell::ChromeWords;

/// What the document area reads and changes: the canvas view is view state the canvas writes back.
pub struct CanvasProps<'a> {
    pub view: &'a mut CanvasView,
    /// The current tool's id.
    pub tool: &'static str,
}

pub fn show(ui: &mut Ui, props: &mut CanvasProps<'_>) {
    let words = Env::of(ui).words::<ChromeWords>();
    let tool = props.tool;
    let _ = Canvas::new(words.document.clone()).view(Bind::bound(&mut *props.view)).show(ui, |ui, _| {
        let Some(space) = canvas_space(&Env::of(ui).view()) else { return };
        let cx = PaintCx { ui, rect: space.view.rect, camera: Camera::new(space.view) };
        layers::paint(&cx);
        active_tool(tool, &cx);
    });
}

/// The slot for the active tool: it gets the pointer and draws its feedback over the layers.
fn active_tool(_tool: &str, _cx: &PaintCx<'_>) {}
