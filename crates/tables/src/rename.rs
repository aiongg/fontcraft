//! Renaming the glyphs of a compiled font (M12.15; rows `glyphs rename`, `font.renameGlyphs`).
//! Oracle: `tx -gf`. Renaming a source is the engine's job (components, kerning, groups and
//! feature code follow there).

use crate::goadb::Goadb;
use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

/// What [`rename_glyphs`] renames.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RenameOptions {
    /// Old name → new name pairs.
    pub map: Vec<(String, String)>,
    /// A GOADB to rename by (source name → final name); applied before `map`.
    pub goadb: Option<Goadb>,
    /// Write `post` version 2 names even for a CFF font (which keeps names in its charset).
    pub post_names: bool,
}

/// The renamed font.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenameResult {
    pub font: SfntFont,
    /// How many glyphs changed name.
    pub renamed: usize,
    /// Names the options mention that the font does not have.
    pub missing: Vec<String>,
}

/// Renames glyphs in `post` and, for CFF fonts, the charset.
pub fn rename_glyphs(font: SfntFont, options: &RenameOptions) -> Result<RenameResult> {
    // M12.15
    let _ = (font, options);
    Err(TablesError::NotBuilt("glyphs rename"))
}
