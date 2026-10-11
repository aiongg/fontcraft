//! Feature entries: prefixes, classes and features of Font Info › Features, and Update (M8.14).
//!
//! Commands: `font.features.list`, `get`, `listAvailable`, `addPrefix`, `addClass`, `addFeature`,
//! `remove`, `rename`, `reorder`, `setCode`, `setAutomatic`, `setNotes`, `update`, `updateEntry`,
//! `setLabel`, `removeLabel`, `openSpec`, `expandCode`. Entry data and Update logic live in
//! `fontcraft_features` (`auto`, `assemble`, `registry`).

use super::CommandSpec;

pub fn specs() -> Vec<CommandSpec> {
    // M8.14
    Vec::new()
}
