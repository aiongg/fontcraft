//! Fix commands for glyphs and outlines that the audit calls, one undo step each (M12.62; the `own = true` rows of `21-qa-checks.toml`).
//!
//! Whole-file commands follow [`super::file_io`]: paths in, the input never changed, the output
//! written atomically, a refusal on the web.

use super::CommandSpec;

/// The commands of this module; empty until M12.62 lands.
pub fn specs() -> Vec<CommandSpec> {
    // M12.62
    Vec::new()
}
