//! The TTFStems editor: a master's TT stems as rows, in the frame of the TTFZones editor
//! ([`super::ttf_zones`], which opens both and holds their shared parts). This module draws a
//! stem's row.
//!
//! | Control | Command |
//! |---|---|
//! | orientation button (↔ X, ↕ Y) | `hint.tt.setStemDefinition {orientation}`, the other one |
//! | Name, Size of a row | `hint.tt.setStemDefinition` with that key |
//! | delta button, then a cell of the Delta table | `hint.tt.setStemDefinition {deltas}` |
//! | globe button, then OK of the filter sheet | `hint.tt.setStemDefinition {filter}`; Clear sends `null` |
//! | plus | `hint.tt.addStemDefinition`, an X stem 80 units per 1000 wide |
//! | minus | `hint.tt.removeStemDefinition` on the selected row |
//! | actions menu › Get PS stems | `hint.tt.getPsStems` |
//!
//! An X stem measures a vertical stroke (it moves points along X); a Y stem measures a horizontal
//! one. The delta and globe buttons show on the main master only.
//!
//! Rows: `dialog.ttfStems` (M10.28).

use super::ttf_zones::{Control, Entry, Kind, row_id};
use crate::panels::widgets::{self, NumberEdit};
use crate::theme::Tokens;

/// The plus button's new stem, in units per 1000 units per em.
pub(super) const NEW_STEM_SIZE: f64 = 80.0;
/// The name field is this many fields wide.
const NAME_FIELDS: f32 = 2.0;

/// A stem's orientation button, Name and Size.
pub(super) fn row(ui: &mut egui::Ui, tokens: &Tokens, at: usize, entry: &Entry, controls: &mut Vec<Control>) {
    let kind = Kind::Stems;
    let w = tokens.field_width;
    let (label, hover) = if entry.horizontal {
        ("↕", "Y stem: the thickness of a horizontal stroke. Click for an X stem")
    } else {
        ("↔", "X stem: the thickness of a vertical stroke. Click for a Y stem")
    };
    if widgets::button(ui, row_id(kind, at, "orientation"), label, hover, true, false) {
        controls.push(Control::Orientation(at));
    }
    if let Some(name) = widgets::text(ui, row_id(kind, at, "name"), &entry.name, w * NAME_FIELDS) {
        controls.push(Control::Name(at, name));
    }
    if let Some(NumberEdit::Set(n)) = widgets::number(ui, row_id(kind, at, "size"), Some(entry.size), w, false) {
        controls.push(Control::Size(at, n));
    }
}
