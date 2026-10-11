//! Audit reports and export: `audit.export`, `audit.report.menuNames`, `.metrics`, `.panose`, `.featureLanguages`, `audit.validatorKnownIssues` (M12.60).
//!
//! Whole-file commands follow [`super::file_io`]: paths in, the input never changed, the output
//! written atomically, a refusal on the web.

use super::CommandSpec;

/// The commands of this module; empty until M12.60 lands.
pub fn specs() -> Vec<CommandSpec> {
    // M12.60
    Vec::new()
}
