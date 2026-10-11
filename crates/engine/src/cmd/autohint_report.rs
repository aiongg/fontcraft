//! The autohinter's stem and zone reports (`fontcraft_autohint::report`).
//!
//! Rows: the `autohint.ps.report*` rows. Package M10.13 adds the commands; the module is registered in `cmd/mod.rs`.

use super::CommandSpec;

pub fn specs() -> Vec<CommandSpec> {
    // M10.13: the commands.
    Vec::new()
}
