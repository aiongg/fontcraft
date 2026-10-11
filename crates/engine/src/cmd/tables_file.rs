//! Table actions on compiled fonts: `tables.extract`, `.add`, `.delete`, `.edit`, `.checksum`, `.reorder` and `tables.show` (M12.2; `fontcraft_tables::edit`, `fontcraft_tables::show`).
//!
//! Whole-file commands follow [`super::file_io`]: paths in, the input never changed, the output
//! written atomically, a refusal on the web.

use super::CommandSpec;

/// The commands of this module; empty until M12.2 lands.
pub fn specs() -> Vec<CommandSpec> {
    // M12.2
    Vec::new()
}
