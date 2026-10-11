//! Alternate-layer ranges (M9.3 reads them, M9.4 switches by them).
//!
//! Rows: `alternateLayer.rangeSemantics`, `alternateLayer.definition`,
//! `alternateLayer.staticExport`, `switching.threeMethods` (alternate half).
//!
//! Contract: a [`RangeSet`] is an alternate layer's `axis_rules` matched to the designspace's
//! axes: per axis an optional minimum and maximum in internal coordinates, **both bounds
//! inclusive** (a guess on `alternateLayer.rangeSemantics`; the oracles agree), a missing bound
//! open. A location is in the set when it is in every axis's range. A glyph has at most
//! [`crate::MAX_RANGE_SETS`] distinct sets. Static instances use the alternate layers of the set
//! that contains their location; when several do, the most specific one (smallest box) wins.

use fontcraft_model::AxisRule;

use crate::InterpError;
use crate::designspace::{Designspace, Location};
use crate::overlay::RegionBox;

/// An alternate layer's ranges, one entry per axis of the designspace.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RangeSet {
    /// Parallel to the designspace's axes; `AxisRule::default()` leaves an axis open.
    pub axes: Vec<AxisRule>,
}

impl RangeSet {
    /// The range set of a layer's `axis_rules` (parallel to `Font::axes`), matched to the
    /// designspace's axes. Refuses bounds that are not usable numbers and more rules than axes.
    pub fn from_rules(designspace: &Designspace, rules: &[AxisRule]) -> Result<RangeSet, InterpError> {
        // M9.3
        let _ = (designspace, rules);
        Err(InterpError::NotYetImplemented("alternate layer ranges"))
    }

    /// Whether `location` is in every axis's range, bounds included.
    pub fn contains(&self, location: &Location) -> bool {
        // M9.3
        let _ = location;
        false
    }

    /// The set as a box in normalized coordinates, for the overlay and for feature variations;
    /// open bounds become the axis ends.
    pub fn to_box(&self, designspace: &Designspace) -> Result<RegionBox, InterpError> {
        // M9.8
        let _ = designspace;
        Err(InterpError::NotYetImplemented("alternate ranges as boxes"))
    }
}
