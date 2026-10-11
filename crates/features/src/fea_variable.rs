//! The reference's variable feature syntax lowered to plain feature-file text (M9.11).
//!
//! Rows: `featureCode.ifdefVariable`, `featureCode.ifndefVariable`,
//! `featureCode.ifdefOtherSymbols`, `featureCode.condition`, `featureCode.conditionRange`,
//! `featureCode.conditionLowerInclusive`, `featureCode.conditionMultipleAxes`,
//! `featureCode.conditionScope`, `featureCode.conditionLowering`, `featureCode.variablePos`,
//! `featureCode.variablePosMultiple`, `featureCode.variablePosFourValue`,
//! `featureCode.variablePosCoordinates`, `featureCode.variablePosLowering`,
//! `featureCode.mm.staticVersusVariable`, `switching.conditionalSubstitution`.
//!
//! Contract (`docs/plans/M9.md` § 3 › Design decisions), modelled on glyphsLib's converter
//! (Apache-2.0; credited in the header if any part is ported):
//! - `#ifdef VARIABLE` / `#ifndef VARIABLE` blocks are kept or dropped by the target; other
//!   symbols count as undefined.
//! - For [`LowerTarget::Variable`], each `condition` becomes a `conditionset` and `variation`
//!   block, its internal coordinates mapped to external ones; overlapping conditions are
//!   overlaid. Values written per location (`(wght:100 …)`) become variable scalars, number
//!   tokens per-location values.
//! - For [`LowerTarget::Static`], conditions are evaluated at the location and values resolved
//!   there, so the output is plain static feature code.
//! - Text the lowering does not touch is copied unchanged; [`Lowered::map`] lets diagnostics on
//!   the output point back into the input.
//! - Refusals (a condition inside a lookup, an unknown axis, a repeated axis, limits) are
//!   returned with their byte offset and a message; nothing panics.
//!
//! Where this meets M8: M8's `features::variable::lower(code, axes)` is the hook its assembly
//! calls for a variable target; it delegates to [`lower_variable_syntax`], converting
//! [`Lowered::map`] to M8's offset map and [`VariableSyntaxError`] to its diagnostics.

use fontcraft_model::AxisRange;

/// Most condition sets one feature text may produce.
pub const MAX_CONDITION_SETS: usize = 4_096;
/// Most alternatives one variable value may list.
pub const MAX_VALUE_ALTERNATIVES: usize = 64;
/// Most axes one condition or variable value may name.
pub const MAX_CONDITION_AXES: usize = fontcraft_model::MAX_AXIS_VALUES;
/// Longest feature text lowered, in bytes. Matches the feature-code size limit of M8's compile.
pub const MAX_CODE_BYTES: usize = 16 * 1024 * 1024;

/// An axis as the feature code sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct FeatureAxis {
    pub tag: String,
    pub name: String,
    /// Minimum, origin and maximum in internal coordinates.
    pub internal: AxisRange,
    /// Minimum, origin and maximum in external coordinates.
    pub external: AxisRange,
    /// External to internal map, `(external, internal)` sorted by external value; empty for
    /// identity.
    pub map: Vec<(f64, f64)>,
}

/// A master as the feature code sees it: its location and number values.
#[derive(Clone, Debug, PartialEq)]
pub struct FeatureMaster {
    pub name: String,
    /// Internal coordinates, parallel to [`FeatureAxes::axes`].
    pub location: Vec<f64>,
    /// The master's number values by number name.
    pub numbers: Vec<(String, f64)>,
}

/// The designspace the lowering works in.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FeatureAxes {
    pub axes: Vec<FeatureAxis>,
    pub masters: Vec<FeatureMaster>,
}

/// What the code is lowered for.
#[derive(Clone, Debug, PartialEq)]
pub enum LowerTarget {
    /// A static font at this internal location (parallel to [`FeatureAxes::axes`]).
    Static(Vec<f64>),
    /// A variable font.
    Variable,
}

/// Lowered feature text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lowered {
    pub text: String,
    /// `(output byte offset, input byte offset)` at the start of each stretch of output, sorted by
    /// output offset; empty when the text is unchanged.
    pub map: Vec<(usize, usize)>,
}

/// Why a stretch of feature text could not be lowered.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct VariableSyntaxError {
    /// Byte offset in the input.
    pub offset: usize,
    /// Length in bytes of the stretch the message is about.
    pub len: usize,
    pub message: String,
}

/// `code` with the variable syntax lowered for `target`.
pub fn lower_variable_syntax(code: &str, axes: &FeatureAxes, target: &LowerTarget) -> Result<Lowered, Vec<VariableSyntaxError>> {
    // M9.11
    let _ = (axes, target);
    Ok(Lowered { text: code.to_string(), map: Vec::new() })
}
