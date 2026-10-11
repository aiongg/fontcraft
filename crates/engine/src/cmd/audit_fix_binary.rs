//! Audit fixes on compiled fonts: the `tables.*` fix options `patchTable`, `dropTables`, `cleanNameRecords`, `cleanInstances`, `rebuildCmap`, `recompileCff`, `recompileLayout` (M12.61; `fontcraft_tables::patch`).
//!
//! Whole-file commands follow [`super::file_io`]: paths in, the input never changed, the output
//! written atomically, a refusal on the web.

use super::CommandSpec;

/// The commands of this module; empty until M12.61 lands.
pub fn specs() -> Vec<CommandSpec> {
    // M12.61
    Vec::new()
}
