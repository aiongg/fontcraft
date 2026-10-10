//! One glyph data record, as a `GlyphData.xml` file holds it.

use crate::types::{Case, Category, Direction, SubCategory};

/// A database record. Built-in records come from the bundled tables; custom records come from a
/// user's `GlyphData.xml` and may also carry the five fields the published data leaves out
/// (`sort_name`, `sort_name_keep`, `decompose`, `anchors`, `marks`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Record {
    /// The nice name, the record's primary key.
    pub name: String,
    pub category: Option<Category>,
    pub sub_category: Option<SubCategory>,
    /// The code point the name maps to.
    pub unicode: Option<u32>,
    /// A compatibility or presentation-form code point; never assigned to a glyph.
    pub unicode_legacy: Option<u32>,
    /// The export name when it differs from `name`.
    pub production: Option<String>,
    /// Lowercase script id (`latin`, `arabic`, `han`…).
    pub script: Option<String>,
    pub case: Option<Case>,
    /// The direction the record states. [`crate::GlyphData::direction`] fills a missing one.
    pub direction: Option<Direction>,
    pub description: Option<String>,
    /// Other names that resolve to this record.
    pub alt_names: Vec<String>,
    pub sort_name: Option<String>,
    pub sort_name_keep: Option<String>,
    pub decompose: Option<String>,
    pub anchors: Option<String>,
    pub marks: Option<String>,
}

impl Record {
    /// The name this record exports under: its production name, or its nice name when it has
    /// none (05-font-view.md § 4.4, step 2).
    pub fn export_name(&self) -> &str {
        self.production.as_deref().unwrap_or(&self.name)
    }
}
