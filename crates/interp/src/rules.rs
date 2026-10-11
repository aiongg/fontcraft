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

use crate::designspace::{Designspace, Location};
use crate::overlay::RegionBox;
use crate::{InterpError, MAX_AXES, MAX_COORDINATE};

/// An alternate layer's ranges, one entry per axis of the designspace.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RangeSet {
    /// Parallel to the designspace's axes; `AxisRule::default()` leaves an axis open.
    pub axes: Vec<AxisRule>,
}

impl RangeSet {
    /// The range set of a layer's `axis_rules` (parallel to `Font::axes`), matched to the
    /// designspace's axes. Refuses bounds that are not usable numbers and more rules than axes.
    /// A rule on an axis a variable font setting dropped is ignored; for the whole font, a bounded
    /// rule beyond the last axis is refused (open ones are harmless and ignored).
    pub fn from_rules(designspace: &Designspace, rules: &[AxisRule]) -> Result<RangeSet, InterpError> {
        if rules.len() > MAX_AXES {
            return Err(InterpError::LimitExceeded { what: "axis ranges on one layer", limit: MAX_AXES });
        }
        for (i, r) in rules.iter().enumerate() {
            for v in [r.min, r.max].into_iter().flatten() {
                if !v.is_finite() || v.abs() > MAX_COORDINATE {
                    return Err(InterpError::BadNumber { what: format!("the alternate range bound {v}") });
                }
            }
            let bounded = r.min.is_some() || r.max.is_some();
            if bounded && designspace.setting().is_none() && !designspace.axes().iter().any(|a| a.font_axis == i) {
                return Err(InterpError::UnknownAxis((i + 1).to_string()));
            }
        }
        Ok(RangeSet { axes: designspace.axes().iter().map(|a| rules.get(a.font_axis).copied().unwrap_or_default()).collect() })
    }

    /// Whether `location` is in every axis's range, bounds included.
    pub fn contains(&self, location: &Location) -> bool {
        self.axes.len() == location.0.len()
            && self.axes.iter().zip(&location.0).all(|(r, &v)| v.is_finite() && r.min.is_none_or(|m| v >= m) && r.max.is_none_or(|m| v <= m))
    }

    /// The set as a box in normalized coordinates, for the overlay and for feature variations;
    /// open bounds become the axis ends.
    pub fn to_box(&self, designspace: &Designspace) -> Result<RegionBox, InterpError> {
        // M9.8
        let _ = designspace;
        Err(InterpError::NotYetImplemented("alternate ranges as boxes"))
    }
}
