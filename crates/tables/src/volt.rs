//! VOLT project text to feature code (M12.13; rows `features convert`, `layout.convertSyntax`).
//! Oracle: fontTools `voltLib.voltToFea` (MIT); a port is credited in this header and `NOTICE`.

use crate::{Result, TablesError};

/// Feature code for a VOLT source of at most [`crate::limits::MAX_FEATURE_SOURCE`] bytes, and
/// the warnings for what has no feature-code form.
pub fn volt_to_fea(volt: &str) -> Result<(String, Vec<String>)> {
    // M12.13
    let _ = volt;
    Err(TablesError::NotBuilt("VOLT conversion"))
}
