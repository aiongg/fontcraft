//! The sources a glyph interpolates from (M9.3).
//!
//! Rows: `intermediateLayer.purpose`, `intermediateLayer.inheritsEmptyCoordinates`,
//! `alternateLayer.definition`, `alternateLayer.onePerMaster`,
//! `alternateLayer.combinedWithIntermediate`, `alternateLayer.onMasterLayer`.
//!
//! Contract:
//! - The main sources are the glyph's master layers (of the designspace's masters) and its
//!   intermediate layers. An intermediate layer's location is its `coordinates`, a missing
//!   coordinate taking its master's value. Layers of masters a setting disabled are left out.
//! - Alternate layers are grouped by their distinct range sets (at most
//!   [`crate::MAX_RANGE_SETS`]). A set's sources are its alternate layers, at their masters'
//!   locations or, for a layer that is both alternate and intermediate, at its own coordinates.
//!   The reverse partners (`]…[` written as a range that excludes the interval) are separate sets.
//! - A master that has no layer in the glyph is listed in [`GlyphSources::missing_masters`];
//!   interpolation then uses a sub-model without it.
//! - Two sources at one location, a location that is not finite, and more than
//!   [`crate::MAX_SOURCES`] sources are refused.

use fontcraft_model::Font;

use crate::InterpError;
use crate::designspace::{Designspace, Location};
use crate::rules::RangeSet;

/// One source of a glyph: a layer at a location.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    /// The layer's id.
    pub layer_id: String,
    /// The master the layer belongs to.
    pub master: String,
    /// Internal coordinates, one per designspace axis.
    pub location: Location,
    /// A master layer (as opposed to an intermediate or alternate layer).
    pub is_master: bool,
}

/// The alternate layers of one range set.
#[derive(Clone, Debug, PartialEq)]
pub struct AlternateSet {
    pub ranges: RangeSet,
    pub sources: Vec<Source>,
}

/// Everything one glyph interpolates from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlyphSources {
    pub glyph: String,
    /// Master layers and intermediate layers, master layers first in master order.
    pub main: Vec<Source>,
    /// The alternate sets, in the order their first layer appears in the glyph.
    pub alternates: Vec<AlternateSet>,
    /// Ids of the designspace's masters the glyph has no layer for.
    pub missing_masters: Vec<String>,
}

impl GlyphSources {
    /// The sources of `glyph` in `designspace`.
    pub fn of(font: &Font, designspace: &Designspace, glyph: &str) -> Result<GlyphSources, InterpError> {
        // M9.3
        let _ = (font, designspace, glyph);
        Err(InterpError::NotYetImplemented("glyph sources"))
    }

    /// The sources that apply at `location`: the main sources with each alternate set that
    /// contains the location put in place of the master layers it replaces.
    pub fn at(&self, location: &Location) -> Vec<&Source> {
        // M9.4
        let _ = location;
        self.main.iter().collect()
    }

    /// Whether the glyph has intermediate layers (a source that is not at a master's location).
    pub fn has_intermediates(&self) -> bool {
        self.main.iter().any(|s| !s.is_master)
    }
}
