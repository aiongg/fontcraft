//! Font Info rows and their clipboard text.
//!
//! A row is one General field or one custom parameter of the font, a master or an instance.
//! Copy writes the selected rows as JSON (what FontCraft pastes) and as property-list text (what
//! the reference app's clipboard holds); paste reads either. Pasting replaces rows with the same
//! key, appends, or skips them ([`PasteMode`]). At most [`crate::MAX_ROWS`] rows and
//! [`crate::MAX_ROWS_BYTES`] of text.
//!
//! Rows: `font.info.copyRows`, `font.info.pasteRows` (M7.3); the Font Info window's Copy and
//! Paste (M7.15).

use fontcraft_model::Font;
use fontcraft_plist::Value;

use crate::error::RowsError;
use crate::target::Scope;

/// One row of a Font Info tab.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// The scope the row was copied from (a paste may go to another).
    pub scope: Scope,
    pub kind: RowKind,
    /// The property key or the parameter name.
    pub key: String,
    /// A parameter's value; a field's text, or for a localizable field a dictionary from language
    /// tag to text.
    pub value: Value,
    /// A parameter's checkbox; always true for a field.
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Field,
    Param,
}

/// What a paste does with a row whose key the target already has.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PasteMode {
    /// The pasted row takes the place of the first row with its key.
    #[default]
    Replace,
    /// The pasted row is added after the others.
    Append,
    /// The pasted row is left out.
    Skip,
}

impl PasteMode {
    /// `replace`, `append` or `skip`.
    pub fn from_name(name: &str) -> Option<PasteMode> {
        match name {
            "replace" => Some(PasteMode::Replace),
            "append" => Some(PasteMode::Append),
            "skip" => Some(PasteMode::Skip),
            _ => None,
        }
    }
}

/// The rows of one target: the font (`index` is ignored), or the master or instance at `index`.
/// Fields first, then parameters, each in stored order. An index past the end gives no rows.
pub fn rows_of(font: &Font, scope: Scope, index: usize) -> Vec<Row> {
    // M7.3: properties and parameters of each scope.
    let _ = (font, scope, index);
    Vec::new()
}

/// The rows as JSON text: `{"fontcraftRows": 1, "rows": [{scope, kind, key, value, enabled}]}`.
pub fn to_json(rows: &[Row]) -> String {
    // M7.3: the JSON form, values through `crate::value::to_json`.
    let _ = rows;
    String::new()
}

/// Reads [`to_json`]'s text.
pub fn from_json(text: &str) -> Result<Vec<Row>, RowsError> {
    // M7.3: with the limits.
    let _ = text;
    Err(RowsError::NotYetImplemented("reading Font Info rows"))
}

/// The rows as property-list text: parameters as `{name = …; value = …;}` dictionaries, fields as
/// `{key = …; value = …;}`, in one list.
pub fn to_plist_text(rows: &[Row]) -> String {
    // M7.3: the OpenStep form.
    let _ = rows;
    String::new()
}

/// Reads property-list rows (ours, or the reference app's clipboard text).
pub fn from_plist_text(text: &str) -> Result<Vec<Row>, RowsError> {
    // M7.3: with the limits.
    let _ = text;
    Err(RowsError::NotYetImplemented("reading Font Info rows"))
}

/// Reads either form: JSON when the text starts with `{"`, else a property list.
pub fn from_text(text: &str) -> Result<Vec<Row>, RowsError> {
    if text.len() > crate::MAX_ROWS_BYTES {
        return Err(RowsError::LimitExceeded { what: "bytes of rows", limit: crate::MAX_ROWS_BYTES });
    }
    if text.trim_start().starts_with("{\"") { from_json(text) } else { from_plist_text(text) }
}
