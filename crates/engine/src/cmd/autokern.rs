//! Auto-kerning, Kern to Distance, group building, pair rules, vertical kerning (M13.21 fills it).
//! Rows: `kerning.autoKern`, `kerning.kernToDistance`, `kerning.buildGroups`,
//! `kerning.removePairsByRule`, `kerning.verticalFromHorizontal`.

use super::CommandSpec;

pub fn specs() -> Vec<CommandSpec> {
    // M13.21.
    Vec::new()
}
