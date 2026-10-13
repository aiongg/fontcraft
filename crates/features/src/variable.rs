//! The reference's variable feature syntax lowered for the assembly (M9.11;
//! `docs/spec/12-feature-code.md` § 5): `condition` lines and `(axis:coord) value` positions,
//! through [`crate::fea_variable`]. Runs after token expansion and [`crate::lower::lower`], for
//! every target: the variable font gets feature variations and variable values, a static
//! target the values at its location and the rules whose conditions hold there.

use fontcraft_fea::{Diagnostic, Severity};

use crate::fea_variable::VariableSyntax;
use crate::source_map::OffsetMap;

/// `code` with the variable syntax lowered by `syntax`, and the map back to `code`.
/// Diagnostics have no file; their spans are in `code`.
pub fn lower(code: &str, syntax: &mut VariableSyntax) -> Result<(String, OffsetMap), Vec<Diagnostic>> {
    match syntax.lower(code) {
        Ok(lowered) => Ok((lowered.text, lowered.map)),
        Err(errors) => {
            Err(errors.into_iter().map(|e| Diagnostic::at("", e.offset..e.offset.saturating_add(e.len), Severity::Error, e.message)).collect())
        }
    }
}
