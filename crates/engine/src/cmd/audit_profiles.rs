//! Audit profiles, settings, the ignore list and the result cache: `audit.clearCache`, `audit.profile.show`, `.save`, `.delete`, `.setDefault`, `audit.config.get`, `.set`, `audit.ignoreIssue`, `.unignoreIssue`, `.listIgnored` (M12.59).
//!
//! Whole-file commands follow [`super::file_io`]: paths in, the input never changed, the output
//! written atomically, a refusal on the web.

use super::CommandSpec;

/// The commands of this module; empty until M12.59 lands.
pub fn specs() -> Vec<CommandSpec> {
    // M12.59
    Vec::new()
}
