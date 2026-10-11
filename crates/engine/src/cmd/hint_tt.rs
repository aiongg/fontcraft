//! TrueType Snap and Stem instructions, reversal, stem reference, triple hints and rounding, validated against the touched state (`fontcraft_hinting::TouchedState`).
//!
//! Rows: `hint.tt.snapPoint`, `hint.tt.setSnapZone`, `hint.tt.addStemHint`, `hint.tt.reverse`, `hint.tt.setStemReference`, `hint.tt.makeTripleHint`, `hint.tt.setRounding`. Package M10.6 adds the commands; the module is registered in `cmd/mod.rs`.

use super::CommandSpec;

pub fn specs() -> Vec<CommandSpec> {
    // M10.6: the commands.
    Vec::new()
}
