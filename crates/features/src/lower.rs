//! The reference's additions to the feature language, rewritten to standard syntax (M8.4;
//! `docs/spec/12-feature-code.md` § 4).
//!
//! Rows: `featureCode.delete`, `.deleteContextual`, `.deleteLowering`, `.standaloneLookup`,
//! `.standaloneRedundant`, `.standaloneLowering`, `.optionalLookupEndName`,
//! `.multipleLanguages`, `.multipleLanguagesCompiler`, `.multipleSubstitutionClasses`,
//! `.multipleSubstitutionSingletons`, `.lookupflagReplaces`, `.emptyLookupflag`; the static
//! half of `featureCode.ifdefVariable` and `.ifndefVariable`.
//!
//! One pass over the text, without recursion, braces and brackets at most
//! [`fontcraft_fea::MAX_NESTING_DEPTH`] deep. For a static target `#ifdef VARIABLE` blocks are
//! removed and `#ifndef VARIABLE` blocks kept; for the variable target the reverse. Any other
//! symbol, or `#else`, is an error at the directive. Runs after token expansion.

use fontcraft_fea::Diagnostic;

use crate::source_map::OffsetMap;

/// `code` with the extensions lowered, and the map from the result back to `code`. Diagnostics
/// have no file; their spans are in `code`.
pub fn lower(code: &str, variable: bool) -> Result<(String, OffsetMap), Vec<Diagnostic>> {
    // M8.4
    let _ = variable;
    Ok((code.to_string(), OffsetMap::identity()))
}
