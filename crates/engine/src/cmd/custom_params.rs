//! Custom parameters at the font, master and instance scopes: `font.customParameters.list`, `.describe`, `.add`, `.set` and `.remove`, checked through `fontcraft_fontinfo::catalog` and `fontcraft_fontinfo::value`; several targets in one undo step.
//!
//! Package M7.2 (`docs/plans/M7.md` § 4) fills [`specs`].

use super::*;

pub fn specs() -> Vec<CommandSpec> {
    // M7.2: the commands.
    Vec::new()
}
