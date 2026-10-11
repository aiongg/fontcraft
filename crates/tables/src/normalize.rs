//! The layout normaliser (M12.11; rows `features normalize`, `layout.normalize`). Oracle: fontc's
//! otl-normalizer. Kerning, mark attachment and cursive attachment are printed as flat rules
//! sorted by glyph name, so two fonts that behave alike print alike whatever their lookups.

use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

/// What [`normalize`] prints.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NormalizeOptions {
    /// `GPOS`, `GSUB` or `GDEF`; empty prints every layout table the font has.
    pub tables: Vec<Tag>,
    /// The location variable deltas are resolved at, by axis tag in user coordinates; empty is the
    /// default location.
    pub location: Vec<(Tag, f64)>,
}

/// The normalised layout rules as text.
pub fn normalize(font: &SfntFont, options: &NormalizeOptions) -> Result<String> {
    // M12.11
    let _ = (font, options);
    Err(TablesError::NotBuilt("features normalize"))
}
