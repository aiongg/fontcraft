//! Small caps, subscripts and superscripts derived from existing glyphs (M13.19 fills it). Rows:
//! `glyph.addSmallCaps`, `glyph.addSubSuperscripts`. Stem compensation uses
//! `fontcraft_pathops::stems`.

use super::CommandSpec;

pub fn specs() -> Vec<CommandSpec> {
    // M13.19.
    Vec::new()
}
