//! Review changes made to the open file on disk (M13.45 fills it). Rows:
//! `file.reviewExternalChanges`, `io.watchSourceOnDisk`.

use super::CommandSpec;

pub fn specs() -> Vec<CommandSpec> {
    // M13.45.
    Vec::new()
}
