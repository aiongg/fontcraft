//! Font collections (M12.3; rows `collection list`, `pack`, `unpack`, `ttc.export`,
//! `compile.collectionSharing`). Oracles: AFDKO otf2otc and otc2otf.
//!
//! WOFF and WOFF2 compression live in `fontcraft-container`; the engine calls it directly.

use crate::sfnt::{Flavour, SfntFont, Tag};
use crate::{Result, TablesError};

/// One font of a collection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollectionMember {
    pub index: u32,
    pub flavour: Flavour,
    pub tables: Vec<Tag>,
    /// Tables whose bytes are shared with another member.
    pub shared: Vec<Tag>,
    /// Name ID 6, when the font has one.
    pub postscript_name: Option<String>,
}

/// The fonts of a collection file (a single font lists as one member).
pub fn list(bytes: &[u8]) -> Result<Vec<CollectionMember>> {
    // M12.3
    let _ = bytes;
    Err(TablesError::NotBuilt("collection list"))
}

/// How [`pack`] builds a collection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PackOptions {
    /// Store byte-identical tables once.
    pub share_tables: bool,
    /// The `ttcf` major version: 1, or 2 when a `DSIG` is kept.
    pub version: u16,
}

impl Default for PackOptions {
    fn default() -> Self {
        Self { share_tables: true, version: 1 }
    }
}

/// A collection file of `fonts` (at most [`crate::limits::MAX_COLLECTION_FONTS`]).
pub fn pack(fonts: &[SfntFont], options: &PackOptions) -> Result<Vec<u8>> {
    // M12.3
    let _ = (fonts, options);
    Err(TablesError::NotBuilt("collection pack"))
}

/// Every font of a collection file, each with its own copy of the shared tables.
pub fn unpack(bytes: &[u8]) -> Result<Vec<SfntFont>> {
    // M12.3
    let _ = bytes;
    Err(TablesError::NotBuilt("collection unpack"))
}
