//! The Text tool: the canvas's active tool in text mode.
//!
//! It posts the press, drags, release and double-click as the tool bridge does (`Msg::Tool`, in the edited glyph's
//! font units), and takes the canvas's whole plain keyboard: letters type instead of choosing tools, the deletes,
//! arrows, Home and End edit the text. Keys with Cmd or Ctrl stay the app's shortcuts. `drain` hands the events to
//! [`super::input`].

use craft::behavior::{CancelReason, PointerButton};
use craft::keymap::Mods;
use craft::ui::canvas::{CanvasTool, ToolAction, ToolClaims, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft::ui::primitives::Draw;

use crate::app::Msg;
use crate::canvas::ActiveTool;
use crate::canvas::camera::Camera;
use crate::canvas::tools::bridge::ToolBridge;

/// The Text tool over the bridge's event translation.
#[derive(Debug, Default)]
pub struct TextTool {
    bridge: ToolBridge,
}

impl CanvasTool for TextTool {
    fn each_pass(&mut self, cx: &mut ToolContext<'_>) {
        self.bridge.each_pass(cx);
    }

    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        self.bridge.press(pointer, cx)
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        self.bridge.drag(pointer, cx);
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        self.bridge.release(pointer, cx);
    }

    fn cancel(&mut self, reason: CancelReason, cx: &mut ToolContext<'_>) {
        self.bridge.cancel(reason, cx);
    }

    fn claims(&self) -> ToolClaims {
        ToolClaims::NONE.keyboard().deletes()
    }

    fn key(&mut self, key: &ToolKey, cx: &mut ToolContext<'_>) -> bool {
        self.bridge.key(key, cx)
    }

    fn paint(&self, _draw: &Draw<'_>, _cx: &ToolContext<'_>) {
        // The cursor and the selection are canvas layers (`super::paint`).
    }

    fn takes(&self, button: PointerButton, _mods: Mods) -> bool {
        button == PointerButton::Primary
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the edit view's text commands (editView.setCursor, editView.insertText)")
    }
}

impl ActiveTool for TextTool {
    fn take_sent(&mut self) -> Vec<Msg> {
        self.bridge.take_sent()
    }

    fn set_camera(&mut self, camera: Camera) {
        self.bridge.set_camera(camera);
    }
}
