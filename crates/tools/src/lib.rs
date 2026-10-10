//! FontCraft pointer tools (L4).
//!
//! A tool turns pointer and key events into engine command invocations, selection changes and
//! view changes. It never edits the document: a drag becomes an interaction (`begin`, any number
//! of `preview`s, then `commit` or `cancel`), and the shell runs what the tool asks for through
//! the same `app.run(id, params)` every other caller uses.
//!
//! Events arrive in font units together with the view they were measured in ([`View`]), so a
//! tolerance given in screen points means the same on screen at every zoom. What a tool sees of
//! the document is a [`Scene`]: one layer, its master's guides and metric lines, and the outlines
//! of its components.
//!
//! | Module | Holds |
//! |---|---|
//! | [`hit`] | what lies under the pointer: nodes, handles, segments, anchors, components, guides |
//! | [`selection`] | the selection and the commands' target object built from it |
//! | [`snap`] | where a dragged point lands: nodes, then handles, guides, metric lines, alignment |
//! | [`select`], [`lasso`], [`hand`], [`zoom`] | the tools, as state machines |
//! | [`keymap`] | the Edit View keys these tools answer |
//! | [`toolbox`] | the active tool, temporary tools (Space, Cmd+Space) and key routing |
#![forbid(unsafe_code)]

pub mod event;
pub mod geometry;
pub mod hand;
pub mod hit;
pub mod keymap;
pub mod knife;
pub mod lasso;
pub mod output;
pub mod scene;
pub mod segment;
pub mod select;
pub mod selection;
pub mod snap;
pub mod tool;
pub mod toolbox;
pub mod view;
pub mod zoom;

pub use event::{Key, KeyEvent, Modifiers, PointerEvent, PointerKind};
pub use hit::{Hit, HitResult, hit_test};
pub use output::{Action, Invocation, Output, Overlay, OverlayKind, ViewChange};
pub use scene::Scene;
pub use selection::{GuideRef, GuideScope, Item, SelectMode, Selection};
pub use snap::{Moving, Snap, SnapKind, snap};
pub use tool::{Context, TOOLS, Tool, ToolInfo, ToolSettings, make_tool};
pub use toolbox::{Switch, Toolbox};
pub use view::View;

/// Most entries one list of a selection may hold when it is read from outside (the same cap the
/// engine puts on a command's target lists).
pub const MAX_SELECTION_LIST: usize = 100_000;
/// Most points a lasso keeps; points beyond it are dropped, which only coarsens the outline.
pub const MAX_LASSO_POINTS: usize = 10_000;
/// Most sampled points kept for the outlines of a scene's components.
pub const MAX_COMPONENT_POINTS: usize = 200_000;

#[cfg(test)]
mod tests;
