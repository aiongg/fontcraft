//! `tables show` (M12.2; rows `tables show`, `tables.show`): a tree of a table's fields built
//! from read-fonts' field traversal, with glyph ids shown as names when the font has them.

use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

/// What [`show`] shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShowOptions {
    /// Tables to show; empty shows every table.
    pub tables: Vec<Tag>,
    /// Deepest level of the tree; `None` shows everything (still capped by
    /// [`crate::limits::MAX_SHOW_NODES`]).
    pub depth: Option<usize>,
    /// Show glyph ids as glyph names where the font names its glyphs.
    pub glyph_names: bool,
}

/// One field or record of a table.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShowNode {
    /// The field name, or the record index as text.
    pub name: String,
    /// The value as text; `None` for a record or array.
    pub value: Option<String>,
    /// Byte offset of the field from the start of the table, when known.
    pub offset: Option<usize>,
    pub children: Vec<ShowNode>,
}

/// One table's tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShownTable {
    pub tag: Tag,
    pub length: usize,
    /// The fields; a table read-fonts does not know is one node with the length.
    pub root: ShowNode,
    /// More nodes existed than the limit allowed.
    pub truncated: bool,
}

/// The trees of the tables `options` names.
pub fn show(font: &SfntFont, options: &ShowOptions) -> Result<Vec<ShownTable>> {
    // M12.2
    let _ = (font, options);
    Err(TablesError::NotBuilt("tables show"))
}
