//! PostScript hint commands (`hint.ps.*`): add horizontal and vertical hints, ghosts, edge values, node links, delete, clear, list, the font hints and their check.
//!
//! Rows: `hint.ps.addHorizontalHint`, `hint.ps.addVerticalHint`, `hint.ps.makeGhostHint`, `hint.ps.setGhostDirection`, `hint.ps.setHintValues`, `hint.ps.attachHintToNode`, `hint.ps.deleteHints`, `hint.ps.clearGlyphHints`, `hint.ps.listHints`, `hint.ps.getFontHints`, `hint.ps.validateFontHints`. Package M10.4 adds the commands; the module is registered in `cmd/mod.rs`.

use super::CommandSpec;

pub fn specs() -> Vec<CommandSpec> {
    // M10.4: the commands.
    Vec::new()
}
