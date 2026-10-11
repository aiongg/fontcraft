//! Decompiling layout tables to feature code (M12.12; rows `features decompile`,
//! `layout.decompileFeatures`). The text compiled back by feaLib gives tables that normalise equal
//! to the original.

use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

/// What [`decompile`] writes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecompileOptions {
    /// `GSUB`, `GPOS`, `GDEF`; empty decompiles every layout table the font has.
    pub tables: Vec<Tag>,
    /// Write glyph classes as named classes when a set of glyphs repeats.
    pub named_classes: bool,
}

/// The font's layout as feature code.
pub fn decompile(font: &SfntFont, options: &DecompileOptions) -> Result<String> {
    // M12.12
    let _ = (font, options);
    Err(TablesError::NotBuilt("features decompile"))
}
