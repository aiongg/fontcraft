//! The feature list (M12.12; rows `features list`, `layout.listFeatures`). Oracle: spot `-F`.

use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

/// One feature of one language system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeatureEntry {
    /// `GSUB` or `GPOS`.
    pub table: Tag,
    pub script: Tag,
    /// `dflt` for the script's default language system.
    pub language: Tag,
    pub feature: Tag,
    /// The lookup indices the feature names.
    pub lookups: Vec<u16>,
}

/// Every feature of every language system, in table, script, language and feature order.
pub fn list_features(font: &SfntFont) -> Result<Vec<FeatureEntry>> {
    // M12.12
    let _ = font;
    Err(TablesError::NotBuilt("features list"))
}
