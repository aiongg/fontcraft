//! The subsetter (M12.17–M12.19; rows `subset`, `font.subset`, `compile.subsetOnExport`). A port
//! of fontTools' pyftsubset (MIT), table by table, with its option names; HarfBuzz's subsetter is
//! the second oracle.
//!
//! [`subset`] resolves the selection into a [`Plan`] ([`closure`]), then each table module
//! writes its tables into the output: [`core`] (M12.17), [`layout`] (M12.18), [`var`] and
//! [`other`] (M12.19). Each module has `apply(plan, input, output) -> Result<()>` and leaves the
//! tables it does not handle alone.

pub mod closure;
pub mod core;
pub mod layout;
pub mod other;
pub mod var;

use std::collections::{BTreeMap, BTreeSet};

use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

/// What to keep. The selections add up; an empty selection keeps `.notdef` only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubsetOptions {
    /// Characters to keep (within [`crate::limits::MAX_SUBSET_TEXT`]).
    pub text: String,
    /// Code points to keep (at most [`crate::limits::MAX_CODE_POINTS`]).
    pub unicodes: Vec<u32>,
    /// Glyph names to keep.
    pub glyphs: Vec<String>,
    /// Glyph ids to keep.
    pub gids: Vec<u32>,
    /// Layout features to keep; `None` keeps pyftsubset's default list of 64 features, `["*"]`
    /// keeps all.
    pub layout_features: Option<Vec<String>>,
    /// Keep glyph ids: dropped glyphs become empty instead of being removed.
    pub retain_gids: bool,
    /// Keep TrueType instructions and CFF hints.
    pub hinting: bool,
    /// Keep the outline of `.notdef`.
    pub notdef_outline: bool,
    /// Name IDs to keep; `None` keeps pyftsubset's default (0 to 6).
    pub name_ids: Option<Vec<u16>>,
    /// Tables to drop besides pyftsubset's default list.
    pub drop_tables: Vec<Tag>,
}

impl Default for SubsetOptions {
    fn default() -> Self {
        Self {
            text: String::new(),
            unicodes: Vec::new(),
            glyphs: Vec::new(),
            gids: Vec::new(),
            layout_features: None,
            retain_gids: false,
            hinting: true,
            notdef_outline: true,
            name_ids: None,
            drop_tables: Vec::new(),
        }
    }
}

/// The resolved selection the table modules work from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// Glyph ids kept, after the closure, in the input's numbering.
    pub glyphs: BTreeSet<u32>,
    /// Input glyph id → output glyph id.
    pub glyph_map: BTreeMap<u32, u32>,
    /// Code points kept → input glyph id.
    pub cmap: BTreeMap<u32, u32>,
    /// Layout feature tags kept.
    pub features: BTreeSet<Tag>,
    pub options: SubsetOptions,
}

/// The subset font and what it kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubsetResult {
    pub font: SfntFont,
    /// Input glyph id → output glyph id.
    pub glyph_map: BTreeMap<u32, u32>,
    /// Names of the kept glyphs in output order.
    pub retained: Vec<String>,
}

/// Subsets `font`.
pub fn subset(font: &SfntFont, options: &SubsetOptions) -> Result<SubsetResult> {
    // M12.17: closure, then core, layout, var and other in turn.
    let _ = (font, options);
    Err(TablesError::NotBuilt("subset"))
}
