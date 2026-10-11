//! Stem filters, on `fontcraft_pathops::stems` (M13.24 fills it). Rows: `filter.adjustStems`,
//! `filter.changeWeight`, `filter.condenseExtend`, `filter.changeXHeight`.

use super::CommandSpec;

pub fn specs() -> Vec<CommandSpec> {
    // M13.24.
    Vec::new()
}
