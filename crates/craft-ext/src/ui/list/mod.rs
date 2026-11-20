//! Pieces for `craft::ui::list` (craft-ui-widgets, `src/list/`) and the collection contract behind it.

mod edits;
mod strike;

pub use edits::{CollectionEdits, EditClaim, EditCommand, claim_edits, deliver, focused_claim, take_edits};
pub use strike::{Strike, StrikeRows, strike_line};
