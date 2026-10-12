//! Table actions (M12.2; rows `tables extract`, `add`, `delete`, `edit`, `copy`, `checksum`,
//! `reorder` and the engine commands `tables.extract`, `.add`, `.delete`, `.edit`, `.checksum`,
//! `.reorder`). Oracle: AFDKO sfntedit.
//!
//! [`edit`] applies the actions in sfntedit's order: extract, then delete, then add. Tables no
//! action names are kept byte for byte.

use fontcraft_container::sfnt as csfnt;

use crate::limits::{MAX_SFNT, MAX_TABLES};
use crate::sfnt::{SfntFont, TableOrder, Tag, tag_str};
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

impl EditOptions {
    /// The order to pass to [`crate::write`] for the edited font: [`TableOrder::Keep`] when
    /// [`EditOptions::order`] is given (the edited font's order already is that list), otherwise
    /// [`EditOptions::write_order`].
    pub fn effective_write_order(&self) -> TableOrder {
        if self.order.is_some() { TableOrder::Keep } else { self.write_order }
    }
}

/// The edited font and the extracted tables.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditResult {
    pub font: SfntFont,
    pub extracted: Vec<(Tag, Vec<u8>)>,
}

/// Applies `options` to `font`. Fails, changing nothing, when an extracted or deleted table is
/// missing.
pub fn edit(mut font: SfntFont, options: &EditOptions) -> Result<EditResult> {
    let named = options.extract.len().saturating_add(options.delete.len()).saturating_add(options.add.len());
    if named > MAX_TABLES.saturating_mul(3) {
        return Err(TablesError::TooLarge { what: "the number of tables named", limit: MAX_TABLES.saturating_mul(3) });
    }
    if let Some(missing) = options.extract.iter().chain(&options.delete).find(|t| !font.tables.contains_key(*t)) {
        return Err(TablesError::MissingTable(tag_str(*missing)));
    }
    let mut extracted = Vec::with_capacity(options.extract.len());
    for tag in &options.extract {
        if let Some(data) = font.get(*tag) {
            extracted.push((*tag, data.to_vec()));
        }
    }
    for tag in &options.delete {
        font.remove(*tag);
    }
    for (tag, data) in &options.add {
        if data.len() > MAX_SFNT {
            return Err(TablesError::TooLarge { what: "an added table", limit: MAX_SFNT });
        }
        font.insert(*tag, data.clone());
    }
    if font.tables.len() > MAX_TABLES {
        return Err(TablesError::TooLarge { what: "the table count", limit: MAX_TABLES });
    }
    if let Some(order) = &options.order {
        font.order = reordered(&font, order);
    }
    Ok(EditResult { font, extracted })
}

/// The tags of `font` with those of `first` (that the font has) leading in the order given, then
/// the others in their current order.
fn reordered(font: &SfntFont, first: &[Tag]) -> Vec<Tag> {
    let mut out: Vec<Tag> = Vec::with_capacity(font.tables.len());
    for tag in first {
        if font.tables.contains_key(tag) && !out.contains(tag) {
            out.push(*tag);
        }
    }
    for tag in font.tags() {
        if !out.contains(&tag) {
            out.push(tag);
        }
    }
    out
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

    /// The tables whose stored checksum is wrong.
    pub fn mismatches(&self) -> impl Iterator<Item = &ChecksumEntry> {
        self.tables.iter().filter(|t| t.stored != t.computed)
    }
}

/// Where `head.checkSumAdjustment` sits in `head`.
const ADJUSTMENT_AT: usize = 8;
/// The sum every font file comes to once `head.checkSumAdjustment` is set.
const MAGIC: u32 = 0xB1B0_AFBA;

/// Reads the table directory of face `face` of `bytes` and checks every checksum. Fixing them is
/// [`crate::write`] of the font read back.
///
/// The expected adjustment is computed the way the reference tool does: the header and the
/// directory with every checksum corrected, plus the computed checksum of each table, so one
/// wrong table checksum is reported once, not also as a wrong adjustment. A WOFF or WOFF2 file
/// is checked after unwrapping.
pub fn checksums(bytes: &[u8], face: u32) -> Result<ChecksumReport> {
    if bytes.len() > MAX_SFNT {
        return Err(TablesError::TooLarge { what: "the font file", limit: MAX_SFNT });
    }
    let unwrapped = fontcraft_container::to_sfnt(bytes)?;
    let bytes: &[u8] = &unwrapped;
    let cut = || TablesError::invalid("the table directory is cut short");
    let version = u32_at(bytes, 0).ok_or_else(|| TablesError::invalid("the file is too short to be a font"))?;
    let dir = if version == csfnt::TTCF {
        let count = u32_at(bytes, 8).ok_or_else(cut)?;
        if usize::try_from(count).unwrap_or(usize::MAX) > crate::limits::MAX_COLLECTION_FONTS {
            return Err(TablesError::TooLarge { what: "the collection's font count", limit: crate::limits::MAX_COLLECTION_FONTS });
        }
        if face >= count {
            return Err(TablesError::BadOptions(format!("the collection has {count} fonts; face {face} does not exist")));
        }
        let at = usize::try_from(face).ok().and_then(|f| f.checked_mul(4)).and_then(|o| o.checked_add(12)).ok_or_else(cut)?;
        u32_at(bytes, at).ok_or_else(cut)? as usize
    } else {
        if face != 0 {
            return Err(TablesError::BadOptions(format!("the file holds one font; face {face} does not exist")));
        }
        0
    };
    let flavour = u32_at(bytes, dir).ok_or_else(cut)?;
    if !csfnt::is_font_flavor(flavour) {
        return Err(TablesError::invalid("not a TrueType or OpenType font"));
    }
    let count = usize::from(u16_at(bytes, dir.checked_add(4).ok_or_else(cut)?).ok_or_else(cut)?);
    if count > MAX_TABLES {
        return Err(TablesError::TooLarge { what: "the table count", limit: MAX_TABLES });
    }
    let dir_len = count.checked_mul(16).and_then(|n| n.checked_add(12)).ok_or_else(cut)?;
    let mut directory = bytes.get(dir..dir.checked_add(dir_len).ok_or_else(cut)?).ok_or_else(cut)?.to_vec();
    let mut report = ChecksumReport::default();
    let mut sum = 0u32;
    let mut stored_adjustment = None;
    for i in 0..count {
        let at = i.checked_mul(16).and_then(|o| o.checked_add(12)).ok_or_else(cut)?;
        let tag: Tag = directory.get(at..at.saturating_add(4)).and_then(|t| t.try_into().ok()).ok_or_else(cut)?;
        let stored = u32_at(&directory, at.saturating_add(4)).ok_or_else(cut)?;
        let offset = u32_at(&directory, at.saturating_add(8)).ok_or_else(cut)? as usize;
        let length = u32_at(&directory, at.saturating_add(12)).ok_or_else(cut)? as usize;
        let data = offset
            .checked_add(length)
            .and_then(|end| bytes.get(offset..end))
            .ok_or_else(|| TablesError::invalid(format!("table '{}' lies outside the file", tag_str(tag))))?;
        let computed = csfnt::table_checksum(tag, data);
        if tag == csfnt::HEAD {
            stored_adjustment = u32_at(data, ADJUSTMENT_AT);
        }
        if let Some(slot) = directory.get_mut(at.saturating_add(4)..at.saturating_add(8)) {
            slot.copy_from_slice(&computed.to_be_bytes());
        }
        sum = sum.wrapping_add(computed);
        report.tables.push(ChecksumEntry { tag, stored, computed });
    }
    sum = sum.wrapping_add(csfnt::checksum(&directory));
    report.adjustment = stored_adjustment.map(|stored| (stored, MAGIC.wrapping_sub(sum)));
    Ok(report)
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, clippy::string_slice)]

    use super::*;
    use crate::sfnt::{Flavour, WriteOptions, read, write};

    /// The bytes of a hex string.
    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// A `head` of 54 bytes: version 1.0, the magic number, 1000 units per em.
    fn head() -> Vec<u8> {
        let mut h = vec![0u8; 54];
        h[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        h[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
        h[18..20].copy_from_slice(&1000u16.to_be_bytes());
        h
    }

    /// Four tables written in the order zzzz, name, cvt, head (not the recommended one).
    fn sample() -> SfntFont {
        let mut font = SfntFont::new(Flavour::TrueType);
        font.insert(*b"zzzz", vec![1, 2, 3]);
        font.insert(*b"name", vec![0; 6]);
        font.insert(*b"cvt ", vec![0, 10, 0, 20]);
        font.insert(*b"head", head());
        font
    }

    // The three files below were written by a script that lays the sample out the same way, and
    // by AFDKO sfntedit 5.0.1 from it: `sfntedit -f in.ttf fixed.ttf` and
    // `sfntedit -x name -d name,zzzz -a cvt=newcvt in.ttf edited.ttf` (newcvt holds 00 05 00 06).
    const IN: &str = "00010000000400400002000063767420000a00140000005800000004686561645f1040dd0000005c000000366e616d650000000000000050000000067a7a7a7a010203000000004c00000003010203000000000000000000000a001400010000000000003cb968a25f0f3cf5000003e8000000000000000000000000000000000000000000000000000000000000000000000000";
    const FIXED: &str = "00010000000400400002000063767420000a00140000008400000004686561645f1040dd0000004c000000366e616d650000000000000088000000067a7a7a7a01020300000000900000000300010000000000003cb9680a5f0f3cf5000003e8000000000000000000000000000000000000000000000000000000000000000000000000000a0014000000000000000001020300";
    const EDITED: &str = "00010000000200200001000063767420000500060000006400000004686561645f1040dd0000002c00000036000100000000000027a657865f0f3cf5000003e800000000000000000000000000000000000000000000000000000000000000000000000000050006";

    #[test]
    fn tables_edit_matches_the_reference_tool() {
        let input = write(&sample(), &WriteOptions { order: TableOrder::Keep }).unwrap();
        assert_eq!(input, hex(IN), "the sample is laid out as the reference tool read it");

        // `tables checksum --fix` and `tables reorder`: the recommended order, fresh checksums.
        let fixed = write(&read(&input, 0).unwrap(), &WriteOptions::default()).unwrap();
        assert_eq!(fixed, hex(FIXED));

        // `tables edit`: extract, then delete, then add, whatever order the options are in.
        let options = EditOptions {
            add: vec![(*b"cvt ", vec![0, 5, 0, 6])],
            delete: vec![*b"name", *b"zzzz"],
            extract: vec![*b"name"],
            ..EditOptions::default()
        };
        let result = edit(read(&input, 0).unwrap(), &options).unwrap();
        assert_eq!(result.extracted, vec![(*b"name", vec![0u8; 6])], "a table is saved before it is deleted");
        let edited = write(&result.font, &WriteOptions { order: options.effective_write_order() }).unwrap();
        assert_eq!(edited, hex(EDITED));
        assert!(checksums(&edited, 0).unwrap().ok(), "an edited font passes tables checksum");
    }

    #[test]
    fn tables_edit_refuses_missing_tables_and_reorders() {
        let font = sample();
        let missing = EditOptions { delete: vec![*b"GPOS"], ..EditOptions::default() };
        assert_eq!(edit(font.clone(), &missing), Err(TablesError::MissingTable("GPOS".into())));

        let order = EditOptions { order: Some(vec![*b"head", *b"none", *b"cvt "]), ..EditOptions::default() };
        let result = edit(font, &order).unwrap();
        assert_eq!(result.font.order, vec![*b"head", *b"cvt ", *b"zzzz", *b"name"], "named first, the rest in file order");
        let bytes = write(&result.font, &WriteOptions { order: order.effective_write_order() }).unwrap();
        assert_eq!(read(&bytes, 0).unwrap().order, result.font.order);
    }

    #[test]
    fn tables_checksum_finds_and_fixes_bad_values() {
        let good = hex(IN);
        let report = checksums(&good, 0).unwrap();
        assert!(report.ok());
        assert_eq!(report.adjustment, Some((0x3CB9_68A2, 0x3CB9_68A2)));

        // A wrong record checksum is reported for that table only, as the reference tool does, not
        // as a wrong adjustment too.
        let mut bad = good.clone();
        bad[16..20].copy_from_slice(&1u32.to_be_bytes()); // the `cvt ` record's checksum
        let report = checksums(&bad, 0).unwrap();
        assert!(!report.ok());
        let wrong: Vec<_> = report.mismatches().collect();
        assert_eq!(wrong, vec![&ChecksumEntry { tag: *b"cvt ", stored: 1, computed: 0x000A_0014 }]);
        assert_eq!(report.adjustment.map(|(s, c)| s == c), Some(true));

        // A zeroed adjustment: the reference tool reports the value it should have.
        let mut bad = good.clone();
        bad[0x5c + 8..0x5c + 12].copy_from_slice(&[0; 4]);
        let report = checksums(&bad, 0).unwrap();
        assert_eq!(report.adjustment, Some((0, 0x3CB9_68A2)));
        assert!(report.tables.iter().all(|t| t.stored == t.computed));

        // Fixing is writing the font read back.
        let fixed = write(&read(&bad, 0).unwrap(), &WriteOptions::default()).unwrap();
        assert!(checksums(&fixed, 0).unwrap().ok());
        assert_eq!(fixed, hex(FIXED));
    }

    #[test]
    fn tables_checksum_refuses_hostile_directories() {
        let good = hex(IN);
        assert!(checksums(&good[..20], 0).is_err(), "a cut directory");
        assert!(checksums(&good, 1).is_err(), "a face that does not exist");
        let mut far = good.clone();
        far[20..24].copy_from_slice(&0xFFFF_FF00u32.to_be_bytes()); // the `cvt ` record's offset
        assert!(checksums(&far, 0).is_err(), "a table past the end");
        let mut many = good;
        many[4..6].copy_from_slice(&0xFFFFu16.to_be_bytes());
        assert!(checksums(&many, 0).is_err(), "more tables than the limit");
    }
}
