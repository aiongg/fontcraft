//! Exported TrueType point numbers of a glyph (the compiler's point map).
//!
//! Rows: `hint.tt.getPointIndexes`. Package M10.19 adds the commands; the module is registered in `cmd/mod.rs`.

use super::CommandSpec;

pub fn specs() -> Vec<CommandSpec> {
    // M10.19: the commands.
    Vec::new()
}
