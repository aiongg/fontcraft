//! Subsetting `cmap`, `glyf`, `loca`, `CFF `, `CFF2`, `hmtx`, `vmtx`, `post`, `name`, `OS/2`, `maxp` and the hinting tables (M12.17).

use crate::Result;
use crate::sfnt::SfntFont;

use super::Plan;

/// Writes the subset of the tables this module handles from `input` into `output`.
pub fn apply(plan: &Plan, input: &SfntFont, output: &mut SfntFont) -> Result<()> {
    // M12.17
    let _ = (plan, input, output);
    Ok(())
}
