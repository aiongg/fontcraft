//! Subsetting gvar, HVAR, VVAR, MVAR and STAT (M12.19).

use crate::Result;
use crate::sfnt::SfntFont;

use super::Plan;

/// Writes the subset of the tables this module handles from `input` into `output`.
pub fn apply(plan: &Plan, input: &SfntFont, output: &mut SfntFont) -> Result<()> {
    // M12.19
    let _ = (plan, input, output);
    Ok(())
}
