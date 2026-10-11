//! Imported TrueType hinting: removing what an opened hinted font brought.
//!
//! Rows: `hint.tt.removeImportedHinting`. Package M10.22 adds the commands; the module is registered in `cmd/mod.rs`.

use super::CommandSpec;

pub fn specs() -> Vec<CommandSpec> {
    // M10.22: the commands.
    Vec::new()
}
