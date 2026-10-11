//! The reference's variable feature syntax lowered to what fea-rs reads (M9.11;
//! `docs/spec/12-feature-code.md` § 5): `condition` lines and `(axis:coord) value` positions.
//! Runs after token expansion and [`crate::lower::lower`], for [`crate::Target::Variable`] only.
//! `conditionset` and `variation` blocks pass through unchanged: fea-rs parses them.

use fontcraft_fea::{Axis, Diagnostic};

use crate::source_map::OffsetMap;

/// `code` with the variable syntax lowered for a font with `axes`, and the map back to `code`.
/// Diagnostics have no file; their spans are in `code`.
pub fn lower(code: &str, axes: &[Axis]) -> Result<(String, OffsetMap), Vec<Diagnostic>> {
    // M9.11: delegate to `crate::fea_variable::lower_variable_syntax`. That needs the masters'
    // locations and number values (`FeatureAxes`), which `axes` does not carry, and a way to
    // build an `OffsetMap` from `Lowered::map`.
    let _ = axes;
    Ok((code.to_string(), OffsetMap::identity()))
}
