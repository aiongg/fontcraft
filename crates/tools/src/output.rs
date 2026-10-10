//! What a tool asks the shell to do, and what it asks the canvas to draw.

use fontcraft_geom::Point;
use serde::Serialize;
use serde_json::Value;

use crate::selection::Selection;

/// A command to run: its id and its parameters.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Invocation {
    pub id: String,
    pub params: Value,
}

impl Invocation {
    pub fn new(id: impl Into<String>, params: Value) -> Self {
        Self { id: id.into(), params }
    }
}

/// One request to the shell, in the order the shell must carry them out.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "camelCase")]
pub enum Action {
    /// Start an interaction: the engine takes a snapshot (`Session::begin`).
    Begin { label: String },
    /// Run a command on the interaction's snapshot without recording history.
    Preview { invocation: Invocation },
    /// End the interaction as one undo step, keeping the last preview.
    Commit,
    /// End the interaction and restore the snapshot.
    Cancel,
    /// Run a command on its own (one undo step if it edits).
    Run { invocation: Invocation },
    /// Replace the selection.
    Select { selection: Selection },
    /// Change the view.
    View { change: ViewChange },
}

/// A change of the Edit View camera.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ViewChange {
    /// Scroll by a distance in screen points (the content follows the pointer).
    Pan { dx: f64, dy: f64 },
    /// Multiply the scale by `factor`, keeping the font point `at` where it is on screen.
    Zoom { factor: f64, at: Point },
    /// Zoom and scroll so this rectangle in font units fills the window (`view.zoomToRect`).
    ZoomToRect { x: f64, y: f64, width: f64, height: f64 },
}

/// The answer to one event.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Output {
    pub actions: Vec<Action>,
    /// The event was used; a key that is not handled goes on to the shell's other shortcuts.
    pub handled: bool,
}

impl Output {
    /// An event that was used and asks for nothing.
    pub fn handled() -> Self {
        Self { actions: Vec::new(), handled: true }
    }

    /// An event that was used and asks for `action`.
    pub fn with(action: Action) -> Self {
        Self { actions: vec![action], handled: true }
    }

    pub fn push(&mut self, action: Action) {
        self.actions.push(action);
        self.handled = true;
    }

    /// Appends the actions of `other`.
    pub fn extend(&mut self, other: Output) {
        self.handled |= other.handled;
        self.actions.extend(other.actions);
    }
}

/// What an overlay stands for, so the canvas picks its colour and line from `theme::Tokens`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OverlayKind {
    Marquee,
    Lasso,
    ZoomRect,
    SnapTarget,
    SnapLine,
    Hover,
    /// The hovered point cannot be moved (`paths.lockedPointsHoverRed`).
    HoverLocked,
    /// The Knife tool's cutting line.
    KnifeLine,
    /// The Measurement tool's ruler; the canvas labels its crossings (`measure.ruler`).
    Ruler,
}

/// Something a tool asks the canvas to draw, in font units.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "shape", rename_all = "camelCase")]
pub enum Overlay {
    Polygon { kind: OverlayKind, points: Vec<Point> },
    Line { kind: OverlayKind, from: Point, to: Point },
    Marker { kind: OverlayKind, at: Point },
}
