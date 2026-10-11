//! TrueType autohinting of glyphs in the font (`fontcraft_hinting::autohint_tt`).
//!
//! Rows: `autohint.tt.run`, `hint.tt.autohintGlyph`. Package M10.21 adds the commands; the module is registered in `cmd/mod.rs`.

use super::CommandSpec;

pub fn specs() -> Vec<CommandSpec> {
    // M10.21: the commands.
    Vec::new()
}
