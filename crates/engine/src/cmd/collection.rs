//! Collections and WOFF: `collection.list`, `.pack`, `.unpack`, `woff.compress`, `woff.decompress` (M12.3; `fontcraft_tables::collection`, `fontcraft_container`).
//!
//! Whole-file commands follow [`super::file_io`]: paths in, the input never changed, the output
//! written atomically, a refusal on the web.

use super::CommandSpec;

/// The commands of this module; empty until M12.3 lands.
pub fn specs() -> Vec<CommandSpec> {
    // M12.3
    Vec::new()
}
