//! Compiling feature code, its diagnostics and the editor's queries. The result is kept in
//! [`crate::feature_state`] (M8.15).
//!
//! Commands: `font.features.compile`, `listErrors`, `applyFix`, `jumpToError`, `showErrorDetails`,
//! `evaluateToken`, `complete`, `previewName`.

use super::CommandSpec;

pub fn specs() -> Vec<CommandSpec> {
    // M8.15
    Vec::new()
}
