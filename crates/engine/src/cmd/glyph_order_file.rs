//! `font.renameGlyphs` and `font.reorderGlyphs` on compiled fonts and sources (M12.15; `fontcraft_tables::rename`, `reorder`, `goadb`).
//!
//! Whole-file commands follow [`super::file_io`]: paths in, the input never changed, the output
//! written atomically, a refusal on the web.

use super::CommandSpec;

/// The commands of this module; empty until M12.15 lands.
pub fn specs() -> Vec<CommandSpec> {
    // M12.15
    Vec::new()
}
