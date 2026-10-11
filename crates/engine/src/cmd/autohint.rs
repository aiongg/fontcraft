//! PostScript autohinting of glyphs in the font (`fontcraft-autohint`).
//!
//! Rows: `autohint.ps.run`, `hint.ps.autohintGlyph`. Package M10.15 adds the commands; the module is registered in `cmd/mod.rs`.

use super::CommandSpec;

pub fn specs() -> Vec<CommandSpec> {
    // M10.15: the commands.
    Vec::new()
}
