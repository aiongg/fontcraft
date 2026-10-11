//! `tables.importTtx`: tables from TTX text into a compiled font (M12.8; `fontcraft_tables::ttx::import`).
//!
//! Whole-file commands follow [`super::file_io`]: paths in, the input never changed, the output
//! written atomically, a refusal on the web.

use super::CommandSpec;

/// The commands of this module; empty until M12.8 lands.
pub fn specs() -> Vec<CommandSpec> {
    // M12.8
    Vec::new()
}
