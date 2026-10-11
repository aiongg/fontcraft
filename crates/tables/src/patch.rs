//! Patching table fields, for the audit's fixes on compiled fonts (M12.61; the `tables.*`
//! options `patchTable`, `cleanNameRecords`, `rebuildCmap`, `recompileCff`).

use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

/// A new value for one field.
#[derive(Clone, Debug, PartialEq)]
pub enum PatchValue {
    Number(f64),
    Text(String),
}

/// What [`patch_table`] changes.
#[derive(Clone, Debug, PartialEq)]
pub struct PatchOptions {
    pub table: Tag,
    /// Field name (as `tables show` names it) → new value.
    pub fields: Vec<(String, PatchValue)>,
}

/// The font with the fields changed; the table is rewritten, the others are kept byte for byte.
pub fn patch_table(font: &SfntFont, options: &PatchOptions) -> Result<SfntFont> {
    // M12.61
    let _ = (font, options);
    Err(TablesError::NotBuilt("table patches"))
}
