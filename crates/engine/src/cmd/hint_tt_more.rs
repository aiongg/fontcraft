//! TrueType Shift, Interpolate and Delta instructions, deleting and listing instructions.
//!
//! Rows: `hint.tt.shiftPoints`, `hint.tt.interpolate`, `hint.tt.addDelta`, `hint.tt.setDeltas`, `hint.tt.deleteInstructions`, `hint.tt.removeHints`, `hint.tt.listInstructions`. Package M10.7 adds the commands; the module is registered in `cmd/mod.rs`.

use super::CommandSpec;

pub fn specs() -> Vec<CommandSpec> {
    // M10.7: the commands.
    Vec::new()
}
