//! Dependencies, related glyphs and component transform copy (M13.18 fills it). Rows:
//! `glyph.listDependencies`, `palette.relatedGlyphs` (the engine query),
//! `component.copyTransformation`.

use super::CommandSpec;

pub fn specs() -> Vec<CommandSpec> {
    // M13.18.
    Vec::new()
}
