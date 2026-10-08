//! The sfnt wrapper: the 12-byte header, the table directory, checksums, `head`'s
//! `checkSumAdjustment` and the recommended table order.
//!
//! The checksum rules and the table order are ported from fontTools (MIT): `calcChecksum`,
//! `SFNTWriter._calcMasterChecksum` and `getSearchRange` in `ttLib/sfnt.py` and `ttLib/__init__.py`,
//! and `sortedTagList` with its two order lists in `ttLib/ttFont.py`.

use crate::{Error, MAX_SFNT, MAX_TABLES, pad4, put_u16, put_u32, to_u32, u16_at, u32_at};

/// A table tag, as its four bytes.
pub type Tag = [u8; 4];

pub const HEAD: Tag = *b"head";
pub const DSIG: Tag = *b"DSIG";
pub const GLYF: Tag = *b"glyf";
pub const LOCA: Tag = *b"loca";
pub const HMTX: Tag = *b"hmtx";
const CFF: Tag = *b"CFF ";

/// The sfnt version of TrueType-flavoured fonts.
pub const TRUETYPE: u32 = 0x0001_0000;
/// The sfnt version of CFF-flavoured fonts (`OTTO`).
pub const OTTO: u32 = u32::from_be_bytes(*b"OTTO");
/// The old Apple TrueType sfnt version (`true`).
pub const TRUE: u32 = u32::from_be_bytes(*b"true");
/// The tag of a font collection (`ttcf`).
pub const TTCF: u32 = u32::from_be_bytes(*b"ttcf");

const FORMAT: &str = "sfnt";

/// Bytes 8 to 11 of `head`: `checkSumAdjustment`.
const ADJUSTMENT_AT: usize = 8;
/// The sum every font's words add up to once `checkSumAdjustment` is set.
const MAGIC: u32 = 0xB1B0_AFBA;

/// One table of an sfnt font.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Table<'a> {
    pub tag: Tag,
    pub data: &'a [u8],
}

/// An sfnt font's flavour and tables, in table directory order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Font<'a> {
    pub flavor: u32,
    pub tables: Vec<Table<'a>>,
}

impl<'a> Font<'a> {
    /// The data of the table tagged `tag`.
    pub fn get(&self, tag: Tag) -> Option<&'a [u8]> {
        self.tables.iter().find(|t| t.tag == tag).map(|t| t.data)
    }
}

/// True for the sfnt versions a single font may have.
pub fn is_font_flavor(flavor: u32) -> bool {
    matches!(flavor, TRUETYPE | OTTO | TRUE)
}

/// Reads the table directory of a single sfnt font. Collections are refused; tables may overlap
/// (they are only read), but each must lie inside `bytes`, and no tag may repeat.
pub fn parse(bytes: &[u8]) -> Result<Font<'_>, Error> {
    let flavor = u32_at(bytes, 0).ok_or_else(|| Error::invalid(FORMAT, "the header is cut short"))?;
    if flavor == TTCF {
        return Err(Error::Unsupported { format: FORMAT, what: "a font collection in a WOFF or WOFF2 file" });
    }
    if !is_font_flavor(flavor) {
        return Err(Error::invalid(FORMAT, "not a TrueType or OpenType font"));
    }
    let count = usize::from(u16_at(bytes, 4).ok_or_else(|| Error::invalid(FORMAT, "the header is cut short"))?);
    if count > MAX_TABLES {
        return Err(Error::invalid(FORMAT, "the table count is out of range"));
    }
    let mut tables: Vec<Table<'_>> = Vec::with_capacity(count);
    for i in 0..count {
        let at = 12 + 16 * i;
        let cut = || Error::invalid(FORMAT, "the table directory is cut short");
        let tag = bytes.get(at..at + 4).and_then(|t| Tag::try_from(t).ok()).ok_or_else(cut)?;
        let offset = u32_at(bytes, at + 8).ok_or_else(cut)? as usize;
        let length = u32_at(bytes, at + 12).ok_or_else(cut)? as usize;
        let data = offset
            .checked_add(length)
            .and_then(|end| bytes.get(offset..end))
            .ok_or_else(|| Error::invalid(FORMAT, format!("table '{}' lies outside the file", tag_str(tag))))?;
        if tables.iter().any(|t| t.tag == tag) {
            return Err(Error::invalid(FORMAT, format!("table '{}' is listed twice", tag_str(tag))));
        }
        tables.push(Table { tag, data });
    }
    Ok(Font { flavor, tables })
}

/// A tag for messages: its printable ASCII bytes, others as `?`.
pub fn tag_str(tag: Tag) -> String {
    tag.iter().map(|&b| if (0x20..0x7f).contains(&b) { char::from(b) } else { '?' }).collect()
}

/// The sum of `data` as big-endian 32-bit words, padded with zeros to a multiple of four.
pub fn checksum(data: &[u8]) -> u32 {
    let mut chunks = data.chunks_exact(4);
    let mut sum = (&mut chunks).fold(0u32, |sum, w| sum.wrapping_add(u32::from_be_bytes(w.try_into().unwrap_or([0; 4]))));
    let rest = chunks.remainder();
    if !rest.is_empty() {
        let mut last = [0u8; 4];
        for (slot, b) in last.iter_mut().zip(rest) {
            *slot = *b;
        }
        sum = sum.wrapping_add(u32::from_be_bytes(last));
    }
    sum
}

/// A table's checksum as the table directory records it: for `head`, with `checkSumAdjustment`
/// counted as zero.
pub fn table_checksum(tag: Tag, data: &[u8]) -> u32 {
    let sum = checksum(data);
    if tag != HEAD {
        return sum;
    }
    let adjustment = data.get(ADJUSTMENT_AT..ADJUSTMENT_AT + 4).map_or(0, checksum);
    sum.wrapping_sub(adjustment)
}

/// `searchRange`, `entrySelector` and `rangeShift` for `count` 16-byte records.
fn search_fields(count: u16) -> (u16, u16, u16) {
    let mut pow = 1u16;
    let mut shift = 0u16;
    while pow.checked_mul(2).is_some_and(|p| p <= count) {
        pow *= 2;
        shift += 1;
    }
    let range = pow.saturating_mul(16);
    (range, shift, count.saturating_mul(16).saturating_sub(range))
}

/// One record of the table directory being written.
struct Record {
    tag: Tag,
    checksum: u32,
    offset: u32,
    length: u32,
}

/// The header and the table directory (records sorted by tag) of a font whose tables lie in the
/// order given, each padded to four bytes after the directory.
fn header_and_directory(flavor: u32, tables: &[Table<'_>]) -> Result<Vec<u8>, Error> {
    let count = u16::try_from(tables.len()).ok().filter(|&n| usize::from(n) <= MAX_TABLES);
    let count = count.ok_or_else(|| Error::invalid(FORMAT, "too many tables"))?;
    let mut offset = 12 + 16 * tables.len();
    let mut records = Vec::with_capacity(tables.len());
    for t in tables {
        let length = to_u32(FORMAT, t.data.len(), "a table")?;
        records.push(Record { tag: t.tag, checksum: table_checksum(t.tag, t.data), offset: to_u32(FORMAT, offset, "the font")?, length });
        offset = pad4(offset).and_then(|o| o.checked_add(t.data.len())).and_then(pad4).unwrap_or(usize::MAX);
        if offset > MAX_SFNT {
            return Err(Error::invalid(FORMAT, "the font is too large"));
        }
    }
    records.sort_by_key(|r| r.tag);
    let (range, selector, shift) = search_fields(count);
    let mut out = vec![0u8; 12 + 16 * records.len()];
    put_u32(&mut out, 0, flavor);
    put_u16(&mut out, 4, count);
    put_u16(&mut out, 6, range);
    put_u16(&mut out, 8, selector);
    put_u16(&mut out, 10, shift);
    for (i, r) in records.iter().enumerate() {
        let at = 12 + 16 * i;
        if let Some(slot) = out.get_mut(at..at + 4) {
            slot.copy_from_slice(&r.tag);
        }
        put_u32(&mut out, at + 4, r.checksum);
        put_u32(&mut out, at + 8, r.offset);
        put_u32(&mut out, at + 12, r.length);
    }
    Ok(out)
}

/// `head.checkSumAdjustment` for the font made of `tables` laid out in the order given.
pub fn checksum_adjustment(flavor: u32, tables: &[Table<'_>]) -> Result<u32, Error> {
    let directory = header_and_directory(flavor, tables)?;
    let sum = tables.iter().fold(checksum(&directory), |sum, t| sum.wrapping_add(table_checksum(t.tag, t.data)));
    Ok(MAGIC.wrapping_sub(sum))
}

/// Puts `adjustment` into a copy of `head`. Refuses a `head` too short to hold it.
pub fn with_adjustment(head: &[u8], adjustment: u32) -> Result<Vec<u8>, Error> {
    if head.len() < ADJUSTMENT_AT + 4 {
        return Err(Error::invalid(FORMAT, "the head table is too short"));
    }
    let mut out = head.to_vec();
    put_u32(&mut out, ADJUSTMENT_AT, adjustment);
    Ok(out)
}

/// The sfnt font made of `tables`: their data in the order given, each padded to four bytes, the
/// directory sorted by tag, checksums and `head.checkSumAdjustment` computed for this layout.
pub fn assemble(flavor: u32, tables: &[Table<'_>]) -> Result<Vec<u8>, Error> {
    let adjustment = checksum_adjustment(flavor, tables)?;
    let mut out = header_and_directory(flavor, tables)?;
    for t in tables {
        let start = out.len();
        out.extend_from_slice(t.data);
        if t.tag == HEAD {
            if t.data.len() < ADJUSTMENT_AT + 4 {
                return Err(Error::invalid(FORMAT, "the head table is too short"));
            }
            put_u32(&mut out, start + ADJUSTMENT_AT, adjustment);
        }
        let padded = pad4(out.len()).unwrap_or(out.len());
        out.resize(padded, 0);
    }
    Ok(out)
}

/// fontTools' recommended table order for TrueType-flavoured fonts (OpenType 1.4).
const TRUETYPE_ORDER: [&[u8; 4]; 19] = [
    b"head", b"hhea", b"maxp", b"OS/2", b"hmtx", b"LTSH", b"VDMX", b"hdmx", b"cmap", b"fpgm", b"prep", b"cvt ", b"loca", b"glyf", b"kern", b"name",
    b"post", b"gasp", b"PCLT",
];
/// fontTools' recommended table order for fonts with a `CFF ` table.
const CFF_ORDER: [&[u8; 4]; 8] = [b"head", b"hhea", b"maxp", b"OS/2", b"name", b"cmap", b"post", b"CFF "];

/// `tables` in fontTools' recommended order (its `sortedTagList`): the tags of the order list for
/// the font's kind first, in that order, then the others sorted by tag, with `DSIG` last.
pub fn recommended_order<'a>(tables: &[Table<'a>]) -> Vec<Table<'a>> {
    let mut rest: Vec<Table<'a>> = tables.to_vec();
    rest.sort_by_key(|t| t.tag);
    if let Some(i) = rest.iter().position(|t| t.tag == DSIG) {
        let dsig = rest.remove(i);
        rest.push(dsig);
    }
    let order: &[&[u8; 4]] = if rest.iter().any(|t| t.tag == CFF) { &CFF_ORDER } else { &TRUETYPE_ORDER };
    let mut out = Vec::with_capacity(rest.len());
    for tag in order {
        if let Some(i) = rest.iter().position(|t| &t.tag == *tag) {
            out.push(rest.remove(i));
        }
    }
    out.extend(rest);
    out
}
