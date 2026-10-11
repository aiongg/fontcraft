//! The shared Font Info row: a General field or a custom parameter, with its checkbox (parameters),
//! its value editor ([`super::param_value`]), the minus button that Alt reveals, and row
//! selection (click, Cmd-click, Shift-click). Localizable fields show one sub-row per language.
//! The row draws and reports; the tab turns the report into a command (`font.info.set`,
//! `font.customParameters.set`, `font.info.removeField`…), so every change goes through
//! `app.run`.
//!
//! Rows: `fontInfo.rows.plusAddsField`, `fontInfo.rows.altRevealsMinus`,
//! `fontInfo.rows.selection`, `fontInfo.rows.localizable` (M7.15).

use fontcraft_engine::fontinfo::rows::Row;
use fontcraft_model::plist::Value;

use crate::theme::Tokens;

/// What the user did to a row this frame.
#[derive(Clone, Debug, PartialEq)]
pub enum RowAction {
    /// A new value from the editor.
    Edit(Value),
    /// The parameter's checkbox.
    SetEnabled(bool),
    /// The minus button.
    Remove,
    /// A click on the row: `extend` with Cmd, `range` with Shift.
    Select { extend: bool, range: bool },
    /// A new language for a localizable field.
    AddLanguage(String),
    /// The minus button of one language's sub-row.
    RemoveLanguage(String),
}

/// Draws one row; `selected` highlights it. Returns what the user did, if anything.
pub fn row(ui: &mut egui::Ui, tokens: &Tokens, row: &Row, selected: bool) -> Option<RowAction> {
    // M7.15: the row widget.
    let _ = (ui, tokens, row, selected);
    None
}
