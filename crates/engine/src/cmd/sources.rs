//! Comparing and merging sources: `source.diff`, `source.merge`, `font.compareGlyphs` (M12.37).
//! A refused or stale merge writes nothing.

use super::CommandSpec;

/// Most changes one source diff lists.
#[allow(dead_code)] // M12.37
pub const MAX_SOURCE_CHANGES: usize = 1_000_000;

/// The commands of this module; empty until M12.37 lands.
pub fn specs() -> Vec<CommandSpec> {
    // M12.37
    Vec::new()
}
