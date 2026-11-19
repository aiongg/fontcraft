//! The view state of Font View's sidebar, its glyph inspector and the Selection Info panel, and the sidebar paths
//! they share. The selected sidebar entries themselves live in `FontViewState::entries`, which the Font View query
//! reads.
//!
//! A sidebar path names one entry: `all`, `categories/Letter/Uppercase`, `languages/latin/de` or
//! `filters/Folder/Name`. It is text, or a list of names when a name holds `/`.

use std::collections::BTreeMap;

use serde_json::{Value, json};

/// Most sidebar entries one selection may hold (`fontView.queryGlyphs` takes no more).
pub const MAX_ENTRIES: usize = 256;
/// Most parts of a sidebar path.
pub const MAX_PATH_SEGMENTS: usize = 16;
/// Longest part of a sidebar path, in bytes.
pub const MAX_SEGMENT_BYTES: usize = 256;
/// Deepest nesting the sidebar lists (folders and groups nest at most 8 deep in the engine).
pub const MAX_DEPTH: usize = 24;
/// Most entries the sidebar lists; the rest of a larger tree is left out.
pub const MAX_ROWS: usize = 16_384;
/// Most filters and folders the Filters section holds (the engine's limit).
pub const MAX_FILTERS: usize = 1024;
/// Most open or closed states the sidebar remembers.
pub const MAX_OPEN_STATES: usize = 4096;

/// What the sidebar, the inspector and Selection Info show that no engine command changes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SidebarView {
    /// Entries opened or closed by the user, by path; the others keep their default (sections open, the rest
    /// closed).
    pub open: BTreeMap<Vec<String>, bool>,
    /// The inspector shows only its title row (`fontView.toggleInspector`).
    pub inspector_hidden: bool,
    /// The inspector shows production name, script, category and subcategory (`fontView.toggleInspectorDetails`).
    pub inspector_details: bool,
    /// The inspector's metric fields are the vertical ones (`fontView.toggleMetricsOrientation`).
    pub inspector_vertical: bool,
    /// The entry whose context menu is open.
    pub menu_row: Option<Vec<String>>,
}

/// A sidebar path from JSON: `"filters/Folder/Name"` or `["filters", "Folder", "Name"]`.
pub fn path_of(v: &Value) -> Option<Vec<String>> {
    let parts: Vec<String> = match v {
        Value::String(s) => s.split('/').map(str::to_string).collect(),
        Value::Array(items) if items.len() <= MAX_PATH_SEGMENTS => items.iter().map(|i| i.as_str().map(str::to_string)).collect::<Option<_>>()?,
        _ => return None,
    };
    let fits = !parts.is_empty() && parts.len() <= MAX_PATH_SEGMENTS && parts.iter().all(|p| !p.is_empty() && p.len() <= MAX_SEGMENT_BYTES);
    fits.then_some(parts)
}

/// A path as the query takes it: text, or a list when a part holds `/`.
pub fn path_value(path: &[String]) -> Value {
    if path.iter().any(|p| p.contains('/')) { json!(path) } else { json!(path.join("/")) }
}

/// Whether `path` is the All entry, which stands for no selection.
pub fn is_all(path: &[String]) -> bool {
    matches!(path, [only] if only == "all")
}

/// The selected entries held in `entries` (`FontViewState::entries`); none is All.
pub fn entries_of(entries: Option<&Value>) -> Vec<Vec<String>> {
    entries.and_then(Value::as_array).map(|list| list.iter().filter_map(path_of).collect()).unwrap_or_default()
}

/// `base`, or `base 2`, `base 3`…: the first name none of `taken` has.
pub fn unique_name(taken: &[String], base: &str) -> String {
    (1..=MAX_FILTERS + 1)
        .map(|n| if n == 1 { base.to_string() } else { format!("{base} {n}") })
        .find(|n| !taken.contains(n))
        .unwrap_or_else(|| base.to_string())
}

#[cfg(test)]
#[path = "sidebar_tests.rs"]
mod tests;
