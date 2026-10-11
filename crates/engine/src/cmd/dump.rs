//! `font.dumpTables`: TTX, JSON and hex dumps and the text views (M12.4, M12.10; `fontcraft_tables::ttx`, `fontcraft_tables::text`).
//!
//! Each package fills its own function below, so packages that share this module do not edit
//! the same lines.

use super::CommandSpec;

/// The commands of this module, in package order.
pub fn specs() -> Vec<CommandSpec> {
    let mut v = ttx_specs();
    v.extend(view_specs());
    v
}

/// `font.dumpTables` in TTX, JSON and hex (M12.4).
fn ttx_specs() -> Vec<CommandSpec> {
    // M12.4
    Vec::new()
}

/// The text views, `@font`, `@glyphs` and `--normalize` (M12.10).
fn view_specs() -> Vec<CommandSpec> {
    // M12.10
    Vec::new()
}
