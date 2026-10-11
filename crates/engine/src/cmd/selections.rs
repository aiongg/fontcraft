//! The reference font and stored selections (M13.32 fills it). Rows: `view.setReferenceFont`,
//! `edit.storeSelection`, `edit.recallSelection`.

use super::CommandSpec;

pub fn specs() -> Vec<CommandSpec> {
    // M13.32.
    Vec::new()
}
