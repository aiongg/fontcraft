//! Size optimisation that keeps behaviour (M12.26; rows `optimize`, `font.optimize`). Oracles:
//! fontTools `otlLib.optimize`, `cffLib.width`, `varLib.varStore`. Subroutinising uses the CFF
//! subroutiniser of `fontcraft-cff` (M12.25).

use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

/// The optimisations [`optimize`] runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OptimizeOptions {
    /// Compact GPOS lookups into subtables of the best size.
    pub gpos: bool,
    /// Choose the CFF `defaultWidthX` and `nominalWidthX` that make the charstrings smallest.
    pub cff_widths: bool,
    /// Subroutinise CFF and CFF2 charstrings.
    pub subroutines: bool,
    /// Merge and sort the rows of variation stores.
    pub variation_store: bool,
}

impl Default for OptimizeOptions {
    fn default() -> Self {
        Self { gpos: true, cff_widths: true, subroutines: true, variation_store: true }
    }
}

/// The optimised font and the sizes before and after.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OptimizeResult {
    pub font: SfntFont,
    pub bytes_before: usize,
    pub bytes_after: usize,
}

/// Optimises `font`.
pub fn optimize(font: &SfntFont, options: &OptimizeOptions) -> Result<OptimizeResult> {
    // M12.26
    let _ = (font, options);
    Err(TablesError::NotBuilt("optimize"))
}
