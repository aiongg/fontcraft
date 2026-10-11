//! Comparing two fonts table by table (M12.32; rows `diff`, `font.diff`). Oracles: AFDKO
//! sfntdiff, `fonttools diff`.

use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

/// How table contents are compared.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DiffMode {
    /// Presence and bytes, with the first differing offset (sfntdiff's default).
    #[default]
    Bytes,
    /// Bytes, with a hex listing of the differences.
    Hex,
    /// The TTX text of each table, line by line.
    Text,
    /// The layout normaliser's output (`--normalize compiler`).
    NormalizeCompiler,
    /// The normalised dump (`--normalize dump`).
    NormalizeDump,
}

/// What [`diff`] compares.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiffOptions {
    pub mode: DiffMode,
    /// Tables to compare; empty compares all.
    pub tables: Vec<Tag>,
    /// Tables to leave out.
    pub skip: Vec<Tag>,
    /// List every difference instead of stopping at [`crate::limits::MAX_DIFF_RECORDS`] records
    /// and [`crate::limits::MAX_DIFF_HEX`] hex bytes per table.
    pub all: bool,
}

/// The verdict for one table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffStatus {
    Same,
    Different,
    OnlyFirst,
    OnlySecond,
}

/// One table's comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableDiff {
    pub tag: Tag,
    pub status: DiffStatus,
    pub first_offset: Option<usize>,
    /// Difference lines in the mode's form.
    pub lines: Vec<String>,
    /// Lines left out by the limits.
    pub truncated: usize,
}

/// Compares `a` with `b`, one entry per table of either, in tag order.
pub fn diff(a: &SfntFont, b: &SfntFont, options: &DiffOptions) -> Result<Vec<TableDiff>> {
    // M12.32
    let _ = (a, b, options);
    Err(TablesError::NotBuilt("diff"))
}
