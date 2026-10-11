//! Partial instances (M12.21): some axes pinned, others limited to a range.

use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

use super::AxisLimit;

/// The variable font with `limits` applied; at least one axis keeps a range.
pub fn partial_instance(font: &SfntFont, limits: &[(Tag, AxisLimit)]) -> Result<SfntFont> {
    // M12.21
    let _ = (font, limits);
    Err(TablesError::NotBuilt("partial instance"))
}
