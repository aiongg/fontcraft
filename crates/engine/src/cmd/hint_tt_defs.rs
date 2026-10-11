//! TT zone and stem definitions (TTFZones, TTFStems): Get PS zones and stems, add, remove, set.
//!
//! Rows: `hint.tt.getPsZones`, `hint.tt.addZone`, `hint.tt.removeZone`, `hint.tt.setZone`, `hint.tt.getPsStems`, `hint.tt.addStemDefinition`, `hint.tt.removeStemDefinition`, `hint.tt.setStemDefinition`. Package M10.5 adds the commands; the module is registered in `cmd/mod.rs`.

use super::CommandSpec;

pub fn specs() -> Vec<CommandSpec> {
    // M10.5: the commands.
    Vec::new()
}
