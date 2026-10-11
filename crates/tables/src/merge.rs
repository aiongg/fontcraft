//! Merging fonts (M12.23; rows `merge`, `font.merge`). Oracles: fontTools pyftmerge (font merge
//! with layout) and AFDKO mergefonts (glyph merge with a map file).

use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

/// What [`merge`] does.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MergeOptions {
    /// Glyph map (mergefonts' form): per input font, new name → name in that font. Empty merges
    /// every glyph of every font, the first font winning on a clash (pyftmerge's form).
    pub glyph_maps: Vec<Vec<(String, String)>>,
    /// Tables to drop from the result.
    pub drop_tables: Vec<Tag>,
}

/// One font made of `fonts` (2 to [`crate::limits::MAX_MERGE_FONTS`]).
pub fn merge(fonts: &[SfntFont], options: &MergeOptions) -> Result<SfntFont> {
    // M12.23
    let _ = (fonts, options);
    Err(TablesError::NotBuilt("merge"))
}
