//! The overlay of alternate-layer boxes into non-overlapping feature-variation records (M9.8).
//!
//! Rows: `compile.featureVariations`, `alternateLayer.variableExport`.
//!
//! Contract: a port of fontTools' `overlayFeatureVariations` (MIT; credit goes in the file header
//! and `NOTICE` with the port). The input is a list of rules, each a set of boxes (any of which
//! applies) and a payload; the output is a list of records whose boxes do not overlap except in
//! the order given, each with the payloads of every rule that applies there, in input order. The
//! most specific region comes first. The work is bounded by a budget
//! ([`crate::MAX_OVERLAY_WORK`]) and the output by [`crate::MAX_OVERLAY_RECORDS`]; past either the
//! overlay stops with [`InterpError::LimitExceeded`].

use crate::InterpError;

/// The range of one axis in a [`RegionBox`], in normalized coordinates, both ends included.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxisSpan {
    /// Index into the designspace's axes.
    pub axis: usize,
    pub min: f64,
    pub max: f64,
}

/// A box in normalized coordinates: the axes it constrains, sorted by axis index; an axis not
/// listed is unconstrained.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RegionBox(pub Vec<AxisSpan>);

/// One rule of the overlay's input: where it applies and what it carries (for feature variations,
/// the substitutions of one alternate set).
#[derive(Clone, Debug, PartialEq)]
pub struct OverlayRule<T> {
    /// Any of these boxes; empty applies nowhere.
    pub boxes: Vec<RegionBox>,
    pub payload: T,
}

/// One record of the overlay's output.
#[derive(Clone, Debug, PartialEq)]
pub struct OverlayRecord<T> {
    pub region: RegionBox,
    /// The payloads of every rule that applies in `region`, in input order.
    pub payloads: Vec<T>,
}

/// The non-overlapping records of `rules`, most specific first, within `budget` steps of work.
pub fn overlay<T: Clone>(rules: &[OverlayRule<T>], budget: usize) -> Result<Vec<OverlayRecord<T>>, InterpError> {
    // M9.8
    let _ = (rules, budget);
    Err(InterpError::NotYetImplemented("the overlay of alternate ranges"))
}
