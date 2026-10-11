//! Full static instances (M12.20): gvar with inferred deltas, cvar, HVAR, VVAR, MVAR, the GDEF
//! and GPOS variation data, CFF2 to CFF, `name` and `OS/2` updates.

use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

/// The static font at `location` (axis tag → user value; axes left out sit at their default).
pub fn full_instance(font: &SfntFont, location: &[(Tag, f64)]) -> Result<SfntFont> {
    // M12.20
    let _ = (font, location);
    Err(TablesError::NotBuilt("full instance"))
}
