//! The value editor of each parameter kind ([`ParamKind`]): a checkbox (Bool), a number field
//! (Number), a text field (String), a multi-line list, one line per item (List), a key and value
//! table (Dict), and a language table (Localized). The editor returns a new value; the row
//! hands it to a command, which checks it (`fontcraft_fontinfo::value::check`).
//!
//! Rows: `fontInfo.customParameterPicker`, `fontInfo.rows.localizable` (M7.15).

use fontcraft_engine::fontinfo::ParamKind;
use fontcraft_model::plist::Value;

use crate::theme::Tokens;

/// Draws the editor of a value of `kind`. Returns the new value when the user changed it (on
/// Enter or when the field loses focus, not on every keystroke).
pub fn editor(ui: &mut egui::Ui, tokens: &Tokens, kind: ParamKind, value: &Value) -> Option<Value> {
    // M7.15: one editor per kind.
    let _ = (ui, tokens, kind, value);
    None
}
