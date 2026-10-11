//! Variation utilities: coordinate mapping and `avar` building (M12.33; rows `variations map`,
//! `variations build-avar`, `variations.mapCoordinates`, `variations.buildAvar`). Oracles:
//! fontTools `varLib.avar.map` and `varLib.avar`.

use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

/// The coordinate space of a location.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Space {
    /// User coordinates (`fvar` values).
    #[default]
    User,
    /// Normalised coordinates before `avar`.
    Normalized,
    /// Normalised coordinates after `avar`.
    Mapped,
}

/// One location in all three spaces, by axis tag.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MappedLocation {
    pub user: Vec<(Tag, f64)>,
    pub normalized: Vec<(Tag, f64)>,
    pub mapped: Vec<(Tag, f64)>,
}

/// Maps each location (given in `from`) into every space. At most
/// [`crate::limits::MAX_AXES`] axes per location; values finite.
pub fn map_coordinates(font: &SfntFont, locations: &[Vec<(Tag, f64)>], from: Space) -> Result<Vec<MappedLocation>> {
    // M12.33
    let _ = (font, locations, from);
    Err(TablesError::NotBuilt("variations map"))
}

/// The segment maps of an `avar` table, version 1, and the version 2 extras.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AvarOptions {
    /// Per axis, `(from, to)` pairs in user coordinates.
    pub maps: Vec<(Tag, Vec<(f64, f64)>)>,
    /// Write `avar` version 2 even without cross-axis mappings.
    pub version2: bool,
}

/// The font with a new `avar` table.
pub fn build_avar(font: &SfntFont, options: &AvarOptions) -> Result<SfntFont> {
    // M12.33
    let _ = (font, options);
    Err(TablesError::NotBuilt("variations build-avar"))
}
