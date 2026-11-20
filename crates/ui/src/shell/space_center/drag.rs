//! The Space Center preview's drags and arrow keys.
//!
//! [`SpaceTool`] is the preview canvas's tool. Its state lives in `FontCraftApp::space_preview` and reaches the panel
//! through the dock, as the edit view's active tool does. Each pass the preview hands it a [`PreviewFrame`]: where
//! every glyph of the text sits and how the preview maps to the screen. The tool records [`SpaceEdit`]s; the preview
//! turns them into commands with the model's matrix columns.
//!
//! | Input | Runs |
//! |---|---|
//! | press on a glyph, or beside every glyph | `window.spaceCenter {select}`: that glyph, or none |
//! | drag from within a grip of a glyph's left edge | `spacing.increaseLeftSidebearing`, dragged left grows |
//! | drag from within a grip of its right edge | `spacing.increaseRightSidebearing`, dragged right grows |
//! | drag from elsewhere in the glyph | `spacing.shiftOutlineRight` |
//! | the same with Alt held at the release | the command on every master layer of the glyph, as one `command.batch` |
//! | Left or Right on the selected glyph | the right margin; with Alt the left margin; with Ctrl+Cmd the outline |
//!
//! A drag runs once, at the release, so it is one undo step. Steps of the arrows: [`step`].

use craft::behavior::kurbo::{Affine, Point};
use craft::behavior::{CancelReason, CanvasView, PointerButton};
use craft::keymap::Mods;
use craft::ui::canvas::{CanvasTool, ToolAction, ToolClaims, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft::ui::primitives::Draw;
use egui::{Pos2, Rect};
use fontcraft_tools::Modifiers;
use serde_json::{Value, json};

use crate::app::Msg;
use crate::canvas::tools::bridge::pointer_mods;

/// Most glyph boxes one frame keeps for the hit test.
pub const MAX_BOXES: usize = 20_000;

/// The step of an arrow key: 100 with Shift and Cmd, 10 with Shift, else 1.
pub fn step(m: Modifiers) -> f64 {
    if m.shift && m.cmd && !m.ctrl {
        100.0
    } else if m.shift {
        10.0
    } else {
        1.0
    }
}

/// The engine command an arrow key runs on the selected glyph: Left and Right the right margin, Alt with them the left
/// margin, Ctrl+Cmd with them the outline inside the width; each side's arrow moves its edge that way. `dx` is the
/// arrow's direction on screen (negative: Left). Steps from [`step`]. `None` for Up, Down and Cmd alone.
pub fn key_command(dx: f32, m: Modifiers) -> Option<(&'static str, f64)> {
    let sign = if dx > 0.0 {
        1.0
    } else if dx < 0.0 {
        -1.0
    } else {
        return None;
    };
    let s = step(m);
    if m.ctrl && m.cmd {
        return Some(("spacing.shiftOutlineRight", sign * s));
    }
    if m.alt {
        return Some(("spacing.increaseLeftSidebearing", -sign * s));
    }
    if m.ctrl || (m.cmd && !m.shift) {
        return None;
    }
    Some(("spacing.increaseRightSidebearing", sign * s))
}

/// What a drag in the preview changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drag {
    Left,
    Right,
    Outline,
}

impl Drag {
    fn command(self) -> (&'static str, &'static str) {
        match self {
            Drag::Left => ("spacing.increaseLeftSidebearing", "Increase Left Sidebearing"),
            Drag::Right => ("spacing.increaseRightSidebearing", "Increase Right Sidebearing"),
            Drag::Outline => ("spacing.shiftOutlineRight", "Shift Outline Right"),
        }
    }
}

/// The commands a finished drag of `dx` units runs on `glyph`: one per layer of `layers` (one layer, or every master
/// layer with Alt). A left margin dragged left grows; a right margin dragged right grows. Less than half a unit runs
/// nothing.
pub fn drag_commands(drag: Drag, glyph: &str, layers: &[String], dx: f64) -> Vec<(&'static str, Value)> {
    let dx = dx.round();
    if dx == 0.0 || !dx.is_finite() {
        return Vec::new();
    }
    let amount = match drag {
        Drag::Left => -dx,
        Drag::Right | Drag::Outline => dx,
    };
    let (id, _) = drag.command();
    layers.iter().map(|layer| (id, json!({"glyphs": [glyph], "layer": layer, "amount": amount}))).collect()
}

/// The one message a finished drag posts: its command, or a `command.batch` of them when it edits several layers, so
/// the drag is one undo step. `None` when the drag changes nothing.
pub fn drag_message(drag: Drag, glyph: &str, layers: &[String], dx: f64) -> Option<Msg> {
    let mut commands = drag_commands(drag, glyph, layers, dx);
    if commands.len() > 1 {
        let steps: Vec<Value> = commands.into_iter().map(|(id, params)| json!({"command": id, "params": params})).collect();
        return Some(Msg::run("command.batch", json!({"label": drag.command().1, "steps": steps})));
    }
    commands.pop().map(|(id, params)| Msg::run(id, params))
}

/// One glyph of the preview with a matrix column, in preview units (screen points before the view and the half turn;
/// y down).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphBox {
    pub column: usize,
    /// The advance's start and end along the row.
    pub left: f64,
    pub right: f64,
    /// The row's top and bottom.
    pub top: f64,
    pub bottom: f64,
    /// Preview units per font unit in this row.
    pub scale: f64,
}

/// Where the preview's glyphs sit in one pass.
#[derive(Clone, Debug, PartialEq)]
pub struct PreviewFrame {
    /// The glyphs with a matrix column, at their places after wrapping and right-to-left placement.
    pub boxes: Vec<GlyphBox>,
    /// Preview units to screen points: the canvas view after the half turn of Upside Down.
    pub to_screen: Affine,
    /// The selected matrix column.
    pub selected: Option<usize>,
    /// How near an edge, in screen points, a press grabs the margin.
    pub grip: f64,
}

impl Default for PreviewFrame {
    fn default() -> Self {
        PreviewFrame { boxes: Vec::new(), to_screen: Affine::IDENTITY, selected: None, grip: 1.0 }
    }
}

impl PreviewFrame {
    /// The preview point under the screen point `p`; `None` while the map cannot be inverted.
    pub fn to_preview(&self, p: Pos2) -> Option<Point> {
        let det = self.to_screen.determinant();
        if det.abs() <= f64::EPSILON || !det.is_finite() {
            return None;
        }
        let q = self.to_screen.inverse() * Point::new(f64::from(p.x), f64::from(p.y));
        (q.x.is_finite() && q.y.is_finite()).then_some(q)
    }

    /// The glyph under the screen point `p`, and that point in preview units.
    pub fn hit(&self, p: Pos2) -> Option<(GlyphBox, Point)> {
        let q = self.to_preview(p)?;
        let found = self.boxes.iter().find(|b| q.y >= b.top && q.y <= b.bottom && q.x >= b.left && q.x <= b.right)?;
        Some((*found, q))
    }

    /// What a press at `p` grabs: the glyph, the part of it, and the point in preview units. An edge within the grip
    /// wins over the outline; the left edge wins where both are near.
    pub fn grab(&self, p: Pos2) -> Option<(GlyphBox, Drag, Point)> {
        let (b, q) = self.hit(p)?;
        // Screen points per preview unit: the view only zooms and turns, so one number holds both ways.
        let per_unit = self.to_screen.determinant().abs().sqrt();
        let near = |edge: f64| (q.x - edge).abs() * per_unit <= self.grip;
        let drag = if near(b.left) {
            Drag::Left
        } else if near(b.right) {
            Drag::Right
        } else {
            Drag::Outline
        };
        Some((b, drag, q))
    }

    /// The screen rectangle of `column`'s first glyph.
    pub fn screen_rect(&self, column: usize) -> Option<Rect> {
        let b = self.boxes.iter().find(|b| b.column == column)?;
        let corner = |x: f64, y: f64| {
            let s = self.to_screen * Point::new(x, y);
            Pos2::new(s.x as f32, s.y as f32)
        };
        Some(Rect::from_two_pos(corner(b.left, b.top), corner(b.right, b.bottom)))
    }
}

/// What the tool's gestures asked for; the preview turns each into a message.
#[derive(Clone, Debug, PartialEq)]
pub enum SpaceEdit {
    /// Select a matrix column, or none.
    Select(Option<usize>),
    /// A finished drag of `dx` font units of `column`; `every` with Alt held: on every master layer.
    Drag { drag: Drag, column: usize, dx: f64, every: bool },
    /// An arrow key on `column`.
    Nudge { column: usize, command: &'static str, amount: f64 },
}

/// A drag in progress.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Dragging {
    drag: Drag,
    column: usize,
    scale: f64,
    /// Where it started, in preview units.
    start: Point,
    /// Where the pointer is, on screen.
    at: Pos2,
}

/// The preview canvas's view and tool, in `FontCraftApp::space_preview`.
#[derive(Clone, Debug)]
pub struct SpacePreview {
    /// The preview canvas's view: its scroll position. The preview shows glyphs at the Space Center's size, so the
    /// preview drops any zoom, turn or mirror the canvas folds in.
    pub view: CanvasView,
    pub tool: SpaceTool,
}

impl Default for SpacePreview {
    fn default() -> Self {
        SpacePreview { view: CanvasView::new(Rect::NOTHING, Point::ZERO, 1.0), tool: SpaceTool::default() }
    }
}

/// The preview canvas's tool.
#[derive(Clone, Debug, Default)]
pub struct SpaceTool {
    frame: PreviewFrame,
    dragging: Option<Dragging>,
    edits: Vec<SpaceEdit>,
}

impl SpaceTool {
    /// The glyph places of the coming pass, before the host runs the tool.
    pub fn set_frame(&mut self, frame: PreviewFrame) {
        self.frame = frame;
    }

    /// The frame of the last pass.
    pub fn frame(&self) -> &PreviewFrame {
        &self.frame
    }

    /// What the gestures asked for since the last call.
    pub fn take_edits(&mut self) -> Vec<SpaceEdit> {
        std::mem::take(&mut self.edits)
    }

    /// Where a drag in progress has got to, on screen.
    pub fn drag_at(&self) -> Option<Pos2> {
        self.dragging.map(|d| d.at)
    }

    fn push(&mut self, edit: SpaceEdit) {
        if self.edits.len() < MAX_BOXES {
            self.edits.push(edit);
        }
    }
}

impl CanvasTool for SpaceTool {
    fn press(&mut self, pointer: &ToolPointer, _cx: &mut ToolContext<'_>) -> ToolAction {
        let grabbed = self.frame.grab(pointer.pos);
        let select = grabbed.map(|(b, _, _)| b.column);
        if select != self.frame.selected {
            self.frame.selected = select;
            self.push(SpaceEdit::Select(select));
        }
        let Some((b, drag, start)) = grabbed else { return ToolAction::Done };
        self.dragging = Some(Dragging { drag, column: b.column, scale: b.scale, start, at: pointer.pos });
        ToolAction::Claimed
    }

    fn drag(&mut self, pointer: &ToolPointer, _cx: &mut ToolContext<'_>) {
        if let Some(d) = self.dragging.as_mut() {
            d.at = pointer.pos;
        }
    }

    fn release(&mut self, pointer: &ToolPointer, _cx: &mut ToolContext<'_>) {
        let Some(d) = self.dragging.take() else { return };
        let Some(end) = self.frame.to_preview(pointer.pos) else { return };
        if d.scale <= f64::EPSILON || !d.scale.is_finite() {
            return;
        }
        // In font units: the half turn is undone by the frame's map, so a row turned or not moves the edge the way it
        // was dragged.
        let dx = (end.x - d.start.x) / d.scale;
        self.push(SpaceEdit::Drag { drag: d.drag, column: d.column, dx, every: pointer.mods.alt });
    }

    fn cancel(&mut self, _reason: CancelReason, _cx: &mut ToolContext<'_>) {
        self.dragging = None;
    }

    fn claims(&self) -> ToolClaims {
        ToolClaims::NONE
    }

    fn key(&mut self, key: &ToolKey, _cx: &mut ToolContext<'_>) -> bool {
        let ToolKey::Nudge { direction, mods, .. } = key else { return false };
        let Some(column) = self.frame.selected else { return false };
        let Some((command, amount)) = key_command(direction.x, key_mods(*mods)) else { return false };
        self.push(SpaceEdit::Nudge { column, command, amount });
        true
    }

    fn paint(&self, _draw: &Draw<'_>, _cx: &ToolContext<'_>) {
        // The preview paints the drag's line itself (`panels::space_center::preview`).
    }

    fn takes(&self, button: PointerButton, _mods: Mods) -> bool {
        button == PointerButton::Primary
    }

    fn autoscrolls(&self) -> bool {
        false
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Space Center's Left Margin, Advance Width and Right Margin fields")
    }
}

/// The modifiers of an arrow key as [`key_command`] reads them: off the Mac, Control is the command key and also
/// counts as Ctrl, so Ctrl with an arrow moves the outline there.
fn key_mods(m: Mods) -> Modifiers {
    pointer_mods(m)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn mods(shift: bool, alt: bool, cmd: bool, ctrl: bool) -> Modifiers {
        Modifiers { shift, alt, cmd, ctrl }
    }

    /// The arrows pick the margin by their modifiers and step by 1, 10 or 100.
    #[test]
    fn arrows_pick_the_margin_and_the_step() {
        let none = Modifiers::default();
        assert_eq!(key_command(1.0, none), Some(("spacing.increaseRightSidebearing", 1.0)));
        assert_eq!(key_command(-1.0, none), Some(("spacing.increaseRightSidebearing", -1.0)));
        assert_eq!(key_command(-1.0, mods(false, true, false, false)), Some(("spacing.increaseLeftSidebearing", 1.0)));
        assert_eq!(key_command(1.0, mods(true, true, false, false)), Some(("spacing.increaseLeftSidebearing", -10.0)));
        assert_eq!(key_command(1.0, mods(false, false, true, true)), Some(("spacing.shiftOutlineRight", 1.0)));
        assert_eq!(key_command(1.0, mods(true, false, true, false)), Some(("spacing.increaseRightSidebearing", 100.0)));
        assert_eq!(key_command(1.0, mods(false, false, true, false)), None, "Cmd alone is not a Space Center key");
        assert_eq!(key_command(0.0, none), None, "Up and Down change nothing");
    }

    /// A drag runs one command, or one batch over every master layer; less than half a unit runs nothing.
    #[test]
    fn a_drag_is_one_message() {
        let one = vec!["m01".to_string()];
        assert_eq!(
            drag_message(Drag::Left, "n", &one, -12.4),
            Some(Msg::run("spacing.increaseLeftSidebearing", json!({"glyphs": ["n"], "layer": "m01", "amount": 12.0})))
        );
        let every = vec!["m01".to_string(), "m02".to_string()];
        let Some(Msg::Run(batch)) = drag_message(Drag::Outline, "n", &every, 5.0) else { panic!("a batch") };
        assert_eq!(batch.id, "command.batch");
        assert_eq!(batch.params["steps"].as_array().map(Vec::len), Some(2));
        assert_eq!(batch.params["steps"][1]["params"]["layer"], "m02");
        assert_eq!(drag_message(Drag::Right, "n", &one, 0.3), None);
    }

    /// The hit test finds the glyph under a point through the half turn and the view, and an edge within the grip
    /// grabs the margin.
    #[test]
    fn the_hit_test_reads_through_the_turn() {
        let boxes = vec![
            GlyphBox { column: 0, left: 10.0, right: 60.0, top: 0.0, bottom: 80.0, scale: 0.1 },
            // A wrapped second row.
            GlyphBox { column: 1, left: 10.0, right: 70.0, top: 80.0, bottom: 160.0, scale: 0.1 },
        ];
        let mut frame = PreviewFrame { boxes, to_screen: Affine::translate((100.0, 50.0)), selected: None, grip: 3.0 };
        let (b, drag, _) = frame.grab(Pos2::new(130.0, 60.0)).unwrap();
        assert_eq!((b.column, drag), (0, Drag::Outline));
        assert_eq!(frame.grab(Pos2::new(112.0, 60.0)).map(|g| g.1), Some(Drag::Left));
        assert_eq!(frame.grab(Pos2::new(169.0, 150.0)).map(|g| (g.0.column, g.1)), Some((1, Drag::Right)));
        assert!(frame.grab(Pos2::new(300.0, 60.0)).is_none());
        // Upside down: the preview turns half a circle about (100, 100); the first glyph's left edge is on the right.
        frame.to_screen = Affine::new([-1.0, 0.0, 0.0, -1.0, 200.0, 200.0]);
        let (b, drag, _) = frame.grab(Pos2::new(189.0, 180.0)).unwrap();
        assert_eq!((b.column, drag), (0, Drag::Left));
    }
}
