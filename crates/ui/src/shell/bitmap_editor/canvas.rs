//! The Bitmap Glyph Editor's pixel canvas: its view and its pointer tool. Its state lives in
//! `FontCraftApp::bitmap_canvas` and reaches the panel through the dock, as the Space Center preview's does.
//!
//! Painting hands the tool the frame it drew (`PixelFrame`): where the glyph's pixels lie on screen. The tool turns
//! the pointer into strike pixels from the glyph origin and reports presses, drags and releases as
//! [`BitmapInput`]s, which the panel posts for `shell::bitmap_editor::input`.

use craft::behavior::kurbo::{Affine, Point};
use craft::behavior::{CancelReason, CanvasView, PointerButton};
use craft::keymap::Mods;
use craft::ui::canvas::{CanvasTool, ToolAction, ToolClaims, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft::ui::primitives::Draw;
use egui::{Pos2, Rect};

use super::BitmapInput;

/// Most inputs kept between two passes; a flood of pointer samples beyond it is dropped.
const MAX_INPUTS: usize = 256;

/// Where the pixels lie: the canvas's map from its document (points) to the screen, the side of a pixel in points,
/// and the pixel position of the document's top-left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PixelFrame {
    pub to_screen: Affine,
    /// Points per pixel.
    pub cell: f64,
    /// The left edge and the top edge of the document, in pixels from the glyph origin.
    pub left: f64,
    pub top: f64,
}

impl PixelFrame {
    /// The screen point of the pixel position `(x, y)`; `None` when the map cannot place it.
    pub fn to_screen(&self, x: f64, y: f64) -> Option<Pos2> {
        let p = self.to_screen * Point::new((x - self.left) * self.cell, (self.top - y) * self.cell);
        (p.x.is_finite() && p.y.is_finite()).then(|| Pos2::new(p.x as f32, p.y as f32))
    }

    /// The pixel position under the screen point `p`; `None` when the map cannot be undone.
    pub fn to_pixels(&self, p: Pos2) -> Option<[f64; 2]> {
        if !(self.cell.is_finite() && self.cell > 0.0) || self.to_screen.determinant().abs() < f64::EPSILON {
            return None;
        }
        let d = self.to_screen.inverse() * Point::new(f64::from(p.x), f64::from(p.y));
        let at = [d.x / self.cell + self.left, self.top - d.y / self.cell];
        at.iter().all(|v| v.is_finite()).then_some(at)
    }

    /// The screen rect of the pixel `(x, y)`.
    pub fn pixel_rect(&self, x: i32, y: i32) -> Option<Rect> {
        let a = self.to_screen(f64::from(x), f64::from(y) + 1.0)?;
        let b = self.to_screen(f64::from(x) + 1.0, f64::from(y))?;
        Some(Rect::from_two_pos(a, b))
    }
}

/// The pixel canvas's view and tool.
#[derive(Clone, Debug)]
pub struct BitmapCanvas {
    /// The canvas's view: its scroll position. The editor sets the pixel size itself, so it drops any zoom, turn or
    /// mirror the canvas folds in.
    pub view: CanvasView,
    pub tool: PixelTool,
    /// The document size the view was last centred on; a new size centres it again.
    pub centred: Option<(u32, u32)>,
}

impl Default for BitmapCanvas {
    fn default() -> Self {
        BitmapCanvas { view: CanvasView::new(Rect::NOTHING, Point::ZERO, 1.0), tool: PixelTool::default(), centred: None }
    }
}

/// The pixel canvas's tool: it reports the gestures; the shell runs the stroke.
#[derive(Clone, Debug, Default)]
pub struct PixelTool {
    frame: Option<PixelFrame>,
    pressed: bool,
    inputs: Vec<BitmapInput>,
}

impl PixelTool {
    /// The frame of the coming pass, before the host runs the tool.
    pub fn set_frame(&mut self, frame: Option<PixelFrame>) {
        self.frame = frame;
    }

    /// The frame of the last pass.
    pub fn frame(&self) -> Option<PixelFrame> {
        self.frame
    }

    /// What the gestures reported since the last call.
    pub fn take_inputs(&mut self) -> Vec<BitmapInput> {
        std::mem::take(&mut self.inputs)
    }

    fn push(&mut self, input: BitmapInput) {
        if self.inputs.len() < MAX_INPUTS {
            self.inputs.push(input);
        }
    }
}

impl CanvasTool for PixelTool {
    fn press(&mut self, pointer: &ToolPointer, _cx: &mut ToolContext<'_>) -> ToolAction {
        let Some(at) = self.frame.and_then(|f| f.to_pixels(pointer.pos)) else { return ToolAction::Done };
        self.pressed = true;
        self.push(BitmapInput::Press { at, alt: pointer.mods.alt });
        ToolAction::Claimed
    }

    fn drag(&mut self, pointer: &ToolPointer, _cx: &mut ToolContext<'_>) {
        if let Some(at) = self.frame.and_then(|f| f.to_pixels(pointer.pos)).filter(|_| self.pressed) {
            self.push(BitmapInput::Move(at));
        }
    }

    fn release(&mut self, pointer: &ToolPointer, _cx: &mut ToolContext<'_>) {
        if !std::mem::take(&mut self.pressed) {
            return;
        }
        match self.frame.and_then(|f| f.to_pixels(pointer.pos)) {
            Some(at) => self.push(BitmapInput::Release(at)),
            None => self.push(BitmapInput::Cancel),
        }
    }

    fn cancel(&mut self, _reason: CancelReason, _cx: &mut ToolContext<'_>) {
        if std::mem::take(&mut self.pressed) {
            self.push(BitmapInput::Cancel);
        }
    }

    fn claims(&self) -> ToolClaims {
        ToolClaims::NONE
    }

    fn key(&mut self, _key: &ToolKey, _cx: &mut ToolContext<'_>) -> bool {
        false
    }

    fn paint(&self, _draw: &Draw<'_>, _cx: &ToolContext<'_>) {
        // The panel paints the pixels and the stroke shows through the preview of the font.
    }

    fn takes(&self, button: PointerButton, _mods: Mods) -> bool {
        button == PointerButton::Primary
    }

    fn autoscrolls(&self) -> bool {
        false
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("bitmap.setPixels")
    }
}
