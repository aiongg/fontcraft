//! Reading and writing sfnt fonts with table pass-through (M12.1a; rows
//! `cli.global.sourceOrBinary`, `roundtrip.binaryPassThroughTables`).
//!
//! [`read`] accepts a TrueType or CFF-flavoured font, one face of a collection, or a WOFF or
//! WOFF2 file (unwrapped first). Every table record is checked against the file; tables may
//! overlap (they are only read) but no tag may repeat. [`write`] lays the tables out again with
//! `fontcraft_container::sfnt::assemble`, which recomputes the directory, the checksums and
//! `head.checkSumAdjustment`.

use std::collections::BTreeMap;

use fontcraft_container::sfnt as csfnt;

use crate::limits::{MAX_COLLECTION_FONTS, MAX_SFNT, MAX_TABLES};
use crate::{Result, TablesError};

/// A table tag: four bytes, padded with spaces (`cvt `).
pub type Tag = [u8; 4];

/// The sfnt version of a single font.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Flavour {
    /// `0x00010000`: TrueType outlines, or no outlines.
    TrueType,
    /// `OTTO`: CFF or CFF2 outlines.
    Cff,
    /// `true`: TrueType outlines, the version older Apple fonts use.
    AppleTrue,
}

impl Flavour {
    /// The four bytes at the start of the font.
    pub fn sfnt_version(self) -> u32 {
        match self {
            Flavour::TrueType => csfnt::TRUETYPE,
            Flavour::Cff => csfnt::OTTO,
            Flavour::AppleTrue => csfnt::TRUE,
        }
    }

    /// The flavour an sfnt version stands for; `None` for a collection or anything else.
    pub fn from_sfnt_version(version: u32) -> Option<Self> {
        match version {
            csfnt::TRUETYPE => Some(Flavour::TrueType),
            csfnt::OTTO => Some(Flavour::Cff),
            csfnt::TRUE => Some(Flavour::AppleTrue),
            _ => None,
        }
    }
}

/// One font as its tables. `tables` holds the bytes of each table; `order` is the order the
/// tables had in the input file (or were added in), which [`TableOrder::Keep`] writes back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SfntFont {
    pub flavour: Flavour,
    pub tables: BTreeMap<Tag, Vec<u8>>,
    /// Every tag of `tables` once, in file order. Tags missing here are written after these,
    /// sorted; tags here without a table are ignored.
    pub order: Vec<Tag>,
}

impl SfntFont {
    /// An empty font of this flavour.
    pub fn new(flavour: Flavour) -> Self {
        Self { flavour, tables: BTreeMap::new(), order: Vec::new() }
    }

    /// The bytes of one table.
    pub fn get(&self, tag: Tag) -> Option<&[u8]> {
        self.tables.get(&tag).map(Vec::as_slice)
    }

    /// Adds or replaces a table. A new tag goes to the end of [`SfntFont::order`]; a replaced one
    /// keeps its place.
    pub fn insert(&mut self, tag: Tag, data: Vec<u8>) {
        if self.tables.insert(tag, data).is_none() && !self.order.contains(&tag) {
            self.order.push(tag);
        }
    }

    /// Removes a table, returning its bytes.
    pub fn remove(&mut self, tag: Tag) -> Option<Vec<u8>> {
        self.order.retain(|t| *t != tag);
        self.tables.remove(&tag)
    }

    /// The tags in file order: [`SfntFont::order`] first, then any others, sorted.
    pub fn tags(&self) -> Vec<Tag> {
        let mut out: Vec<Tag> = Vec::with_capacity(self.tables.len());
        for tag in &self.order {
            if self.tables.contains_key(tag) && !out.contains(tag) {
                out.push(*tag);
            }
        }
        for tag in self.tables.keys() {
            if !out.contains(tag) {
                out.push(*tag);
            }
        }
        out
    }

    /// The font has a `CFF ` or `CFF2` table.
    pub fn is_cff(&self) -> bool {
        self.tables.contains_key(b"CFF ") || self.tables.contains_key(b"CFF2")
    }
}

/// The order [`write`] lays tables out in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TableOrder {
    /// The specification's recommended order (fontTools' `sortedTagList`), `DSIG` last.
    #[default]
    Recommended,
    /// The input order, [`SfntFont::tags`].
    Keep,
}

/// How [`write`] lays out the font.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WriteOptions {
    pub order: TableOrder,
}

/// A tag from its text form: one to four printable ASCII characters, padded with spaces
/// (`"cvt"` is `cvt `).
pub fn parse_tag(text: &str) -> Result<Tag> {
    let bytes = text.as_bytes();
    if bytes.is_empty() || bytes.len() > 4 || !bytes.iter().all(|b| (0x20..0x7f).contains(b)) {
        return Err(TablesError::BadOptions(format!("`{text}` is not a table tag: give one to four printable ASCII characters")));
    }
    let mut tag = *b"    ";
    for (slot, b) in tag.iter_mut().zip(bytes) {
        *slot = *b;
    }
    Ok(tag)
}

/// A tag for messages: its printable ASCII bytes, others as `?`.
pub fn tag_str(tag: Tag) -> String {
    csfnt::tag_str(tag)
}

/// How many faces `bytes` holds: the font count of a collection, 1 for a single font (WOFF and
/// WOFF2 included).
pub fn face_count(bytes: &[u8]) -> Result<u32> {
    if bytes.get(..4) == Some(b"ttcf".as_slice()) {
        let count = collection_count(bytes)?;
        return u32::try_from(count).map_err(|_| TablesError::invalid("the collection's font count is out of range"));
    }
    Ok(1)
}

/// Reads face `face` of `bytes` (0 for a single font). WOFF and WOFF2 files are unwrapped first.
pub fn read(bytes: &[u8], face: u32) -> Result<SfntFont> {
    if bytes.len() > MAX_SFNT {
        return Err(TablesError::TooLarge { what: "the font file", limit: MAX_SFNT });
    }
    let unwrapped = fontcraft_container::to_sfnt(bytes)?;
    let bytes: &[u8] = &unwrapped;
    let version = u32_at(bytes, 0).ok_or_else(|| TablesError::invalid("the file is too short to be a font"))?;
    let dir = if version == csfnt::TTCF {
        let count = collection_count(bytes)?;
        let index = usize::try_from(face).unwrap_or(usize::MAX);
        if index >= count {
            return Err(TablesError::BadOptions(format!("the collection has {count} fonts; face {face} does not exist")));
        }
        let at = index.checked_mul(4).and_then(|o| o.checked_add(12)).ok_or_else(|| TablesError::invalid("face index out of range"))?;
        u32_at(bytes, at).ok_or_else(|| TablesError::invalid("the collection header is cut short"))? as usize
    } else {
        if face != 0 {
            return Err(TablesError::BadOptions(format!("the file holds one font; face {face} does not exist")));
        }
        0
    };
    read_directory(bytes, dir)
}

/// The table directory at `dir`; table offsets count from the start of `bytes`.
fn read_directory(bytes: &[u8], dir: usize) -> Result<SfntFont> {
    let cut = || TablesError::invalid("the table directory is cut short");
    let version = u32_at(bytes, dir).ok_or_else(cut)?;
    let flavour = Flavour::from_sfnt_version(version).ok_or_else(|| TablesError::invalid("not a TrueType or OpenType font"))?;
    let count = usize::from(u16_at(bytes, dir.checked_add(4).ok_or_else(cut)?).ok_or_else(cut)?);
    if count > MAX_TABLES {
        return Err(TablesError::TooLarge { what: "the table count", limit: MAX_TABLES });
    }
    let mut font = SfntFont::new(flavour);
    for i in 0..count {
        let at = i.checked_mul(16).and_then(|o| o.checked_add(dir)).and_then(|o| o.checked_add(12)).ok_or_else(cut)?;
        let tag: Tag = bytes.get(at..at.saturating_add(4)).and_then(|t| t.try_into().ok()).ok_or_else(cut)?;
        let offset = u32_at(bytes, at.saturating_add(8)).ok_or_else(cut)? as usize;
        let length = u32_at(bytes, at.saturating_add(12)).ok_or_else(cut)? as usize;
        let data = offset
            .checked_add(length)
            .and_then(|end| bytes.get(offset..end))
            .ok_or_else(|| TablesError::invalid(format!("table '{}' lies outside the file", tag_str(tag))))?;
        if font.tables.contains_key(&tag) {
            return Err(TablesError::invalid(format!("table '{}' is listed twice", tag_str(tag))));
        }
        font.insert(tag, data.to_vec());
    }
    // The directory is sorted by tag; the file order is the order of the offsets.
    let mut by_offset: Vec<(usize, Tag)> = Vec::with_capacity(count);
    for i in 0..count {
        let at = dir.saturating_add(12).saturating_add(i.saturating_mul(16));
        if let (Some(tag), Some(offset)) =
            (bytes.get(at..at.saturating_add(4)).and_then(|t| Tag::try_from(t).ok()), u32_at(bytes, at.saturating_add(8)))
        {
            by_offset.push((offset as usize, tag));
        }
    }
    by_offset.sort();
    font.order = by_offset.into_iter().map(|(_, t)| t).collect();
    Ok(font)
}

/// Writes `font` as an sfnt file: tables in the order `options` asks for, each padded to four
/// bytes, the directory sorted by tag, checksums and `head.checkSumAdjustment` recomputed.
pub fn write(font: &SfntFont, options: &WriteOptions) -> Result<Vec<u8>> {
    if font.tables.len() > MAX_TABLES {
        return Err(TablesError::TooLarge { what: "the table count", limit: MAX_TABLES });
    }
    let total = font.tables.values().try_fold(0usize, |sum, t| sum.checked_add(t.len()));
    if total.is_none_or(|n| n > MAX_SFNT) {
        return Err(TablesError::TooLarge { what: "the font", limit: MAX_SFNT });
    }
    let tables: Vec<csfnt::Table<'_>> =
        font.tags().into_iter().filter_map(|tag| font.tables.get(&tag).map(|data| csfnt::Table { tag, data: data.as_slice() })).collect();
    let laid_out = match options.order {
        TableOrder::Recommended => csfnt::recommended_order(&tables),
        TableOrder::Keep => tables,
    };
    Ok(csfnt::assemble(font.flavour.sfnt_version(), &laid_out)?)
}

fn collection_count(bytes: &[u8]) -> Result<usize> {
    let count = u32_at(bytes, 8).ok_or_else(|| TablesError::invalid("the collection header is cut short"))? as usize;
    if count > MAX_COLLECTION_FONTS {
        return Err(TablesError::TooLarge { what: "the collection's font count", limit: MAX_COLLECTION_FONTS });
    }
    Ok(count)
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}
