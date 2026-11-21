//! Pieces for `craft::ui::text` (craft-ui-widgets, `src/text/`) and the text part behind its fields (craft-ui-contract,
//! `src/contract/text.rs`, `src/primitives/text_adapter.rs`).
//!
//! The text part keeps its widget, layout and selection to itself, so these stand-ins reach the field's egui text
//! widget by the id craft derives for it ([`text_widget`]) and work on what that widget stores and paints.

mod highlight;

pub use highlight::{Highlight, HighlightText, MAX_HIGHLIGHT_BYTES, MAX_HIGHLIGHTS, recoloured};

use craft::behavior::PartKey;
use craft::ui::runtime::memory;
use egui::{Context, Id};

/// The key of the text part of craft's text and number fields (craft-ui-widgets, `src/text/mod.rs`).
const TEXT_PART: PartKey = PartKey::new("text");

/// The id of the egui text widget of the field whose component id is `field` (its response's id): the id craft's text
/// adapter gives the widget of the text part (`text_adapter::text_id`), which holds key focus and the selection.
pub fn text_widget(field: Id) -> Id {
    memory::sub(field, ("text", TEXT_PART))
}

/// Whether the field whose component id is `field` has key focus in its text.
pub fn text_focused(ctx: &Context, field: Id) -> bool {
    let widget = text_widget(field);
    ctx.memory(|memory| memory.has_focus(widget))
}
