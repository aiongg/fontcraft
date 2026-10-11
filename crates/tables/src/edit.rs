//! Table actions (M12.2; rows `tables extract`, `add`, `delete`, `edit`, `copy`, `checksum`,
//! `reorder` and the engine commands `tables.extract`, `.add`, `.delete`, `.edit`, `.checksum`,
//! `.reorder`). Oracle: AFDKO sfntedit.
//!
//! [`edit`] applies the actions in sfntedit's order: extract, then delete, then add. Tables no
//! action names are kept byte for byte.

use crate::sfnt::{SfntFont, TableOrder, Tag};
use crate::{Result, TablesError};

/// What [`edit`] does, applied in field order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditOptions {
    /// Tables whose bytes are returned in [`EditResult::extracted`].
    pub extract: Vec<Tag>,
    /// Tables removed from the font.
    pub delete: Vec<Tag>,
    /// Tables added, or replaced when the font already has them.
    pub add: Vec<(Tag, Vec<u8>)>,
    /// The table order to write: `Some` with a full or partial tag list puts those tags first in
    /// that order (`tables reorder`); `None` keeps [`EditOptions::write_order`].
    pub order: Option<Vec<Tag>>,
    /// The order the edited font is written in.
    pub write_order: TableOrder,
}

/// The edited font and the extracted tables.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditResult {
    pub font: SfntFont,
    pub extracted: Vec<(Tag, Vec<u8>)>,
}

/// Applies `options` to `font`. Fails, changing nothing, when an extracted or deleted table is
/// missing.
pub fn edit(font: SfntFont, options: &EditOptions) -> Result<EditResult> {
    // M12.2: extract → delete → add, then the order.
    let _ = (font, options);
    Err(TablesError::NotBuilt("tables edit"))
}

/// One table's stored and computed checksum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChecksumEntry {
    pub tag: Tag,
    pub stored: u32,
    pub computed: u32,
}

/// The checksums of a font file as stored and as computed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChecksumReport {
    pub tables: Vec<ChecksumEntry>,
    /// `head.checkSumAdjustment` as stored and as it should be; `None` without `head`.
    pub adjustment: Option<(u32, u32)>,
}

impl ChecksumReport {
    /// Every stored value equals the computed one.
    pub fn ok(&self) -> bool {
        self.tables.iter().all(|t| t.stored == t.computed) && self.adjustment.is_none_or(|(s, c)| s == c)
    }
}

/// Reads the table directory of face `face` of `bytes` and checks every checksum. Fixing them is
/// [`crate::write`] of the font read back.
pub fn checksums(bytes: &[u8], face: u32) -> Result<ChecksumReport> {
    // M12.2
    let _ = (bytes, face);
    Err(TablesError::NotBuilt("tables checksum"))
}
