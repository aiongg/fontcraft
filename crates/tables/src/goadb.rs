//! GOADB files: final name, source name and optional code point override per line, as AFDKO's
//! makeotf and `tx -gf` read them (M12.15 builds the parser; M12.40 reuses it; row
//! `compile.goadbRenaming`).

use crate::{Result, TablesError};

/// One line of a GOADB.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GoadbEntry {
    /// The name in the compiled font.
    pub final_name: String,
    /// The name in the source.
    pub source_name: String,
    /// Code points that replace the source's; empty keeps them.
    pub code_points: Vec<u32>,
}

/// A whole GOADB, in file order (which is the glyph order it asks for).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Goadb {
    pub entries: Vec<GoadbEntry>,
}

/// Parses GOADB text: lines of at most [`crate::limits::MAX_LIST_LINE`] bytes, at most
/// [`crate::limits::MAX_GLYPHS`] entries, `#` comments.
pub fn parse(text: &str) -> Result<Goadb> {
    // M12.15
    let _ = text;
    Err(TablesError::NotBuilt("GOADB reading"))
}
