//! WOFF 1.0: the sfnt tables, each optionally zlib-compressed, behind a 44-byte header and a
//! table directory of 20-byte entries (W3C WOFF File Format 1.0).
//!
//! The writer is a port of fontTools' `SFNTWriter` with the `woff` flavour and of
//! `TTFont.save`'s table reordering (`ttLib/sfnt.py`, `ttLib/ttFont.py`; MIT): table data in the
//! recommended order, the directory sorted by tag, zlib level 6, `head` stored raw,
//! `checkSumAdjustment` computed for the sfnt the file unwraps to.

use crate::compress::{unzlib, zlib};
use crate::sfnt::{self, HEAD, Table, Tag};
use crate::{Error, Extras, MAX_METADATA, MAX_SFNT, MAX_TABLES, Unwrapped, pad4, put_u16, put_u32, to_u32, u16_at, u32_at};

const FORMAT: &str = "WOFF";
const SIGNATURE: &[u8; 4] = b"wOFF";
const HEADER: usize = 44;
const ENTRY: usize = 20;

fn bad(what: impl Into<String>) -> Error {
    Error::invalid(FORMAT, what)
}

/// One entry of a WOFF table directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WoffTable {
    pub tag: u32,
    pub checksum: u32,
    /// Where the (possibly compressed) table starts in the WOFF file.
    pub offset: u32,
    /// The table's size once inflated.
    pub length: u32,
    /// Its size in the WOFF file.
    pub compressed: u32,
}

/// The fixed-size header of a WOFF file.
struct Header {
    flavor: u32,
    length: u32,
    num_tables: usize,
    reserved: u16,
    total_sfnt_size: u32,
    version: (u16, u16),
    meta: (u32, u32, u32),
    private: (u32, u32),
}

fn header(woff: &[u8]) -> Result<Header, Error> {
    if woff.get(..4) != Some(SIGNATURE.as_slice()) {
        return Err(bad("not a WOFF file"));
    }
    let cut = || bad("the header is cut short");
    let w = |at| u32_at(woff, at).ok_or_else(cut);
    let h = |at| u16_at(woff, at).ok_or_else(cut);
    Ok(Header {
        flavor: w(4)?,
        length: w(8)?,
        num_tables: usize::from(h(12)?),
        reserved: h(14)?,
        total_sfnt_size: w(16)?,
        version: (h(20)?, h(22)?),
        meta: (w(24)?, w(28)?, w(32)?),
        private: (w(36)?, w(40)?),
    })
}

/// The table directory of a WOFF 1 file, in directory order. Only the header and the directory
/// are checked; [`decode`] checks the rest.
pub fn directory(woff: &[u8]) -> Result<Vec<WoffTable>, Error> {
    let head = header(woff)?;
    if head.num_tables > MAX_TABLES {
        return Err(bad("the table count is out of range"));
    }
    (0..head.num_tables)
        .map(|i| {
            let at = HEADER + ENTRY * i;
            let field = |k: usize| u32_at(woff, at + 4 * k).ok_or_else(|| bad("the table directory is cut short"));
            Ok(WoffTable { tag: field(0)?, offset: field(1)?, compressed: field(2)?, length: field(3)?, checksum: field(4)? })
        })
        .collect()
}

/// A region of the file that must not overlap another.
struct Span {
    start: usize,
    end: usize,
}

/// Unwraps a WOFF 1 file: the sfnt with its tables in the order the file stores them, the
/// header's version, the metadata (inflated) and the private data.
pub fn decode(woff: &[u8]) -> Result<Unwrapped, Error> {
    let head = header(woff)?;
    if !sfnt::is_font_flavor(head.flavor) {
        return Err(bad("the font inside is neither TrueType nor OpenType"));
    }
    if head.num_tables == 0 || head.num_tables > MAX_TABLES {
        return Err(bad("the table count is out of range"));
    }
    if head.reserved != 0 {
        return Err(bad("the reserved header field is not zero"));
    }
    if head.length as usize != woff.len() {
        return Err(bad("the file's length differs from the length in its header"));
    }
    let total = head.total_sfnt_size as usize;
    if total > MAX_SFNT {
        return Err(bad("the font inside is too large"));
    }
    let tables = directory(woff)?;
    let directory_end = HEADER + ENTRY * tables.len();
    let mut spans = Vec::with_capacity(tables.len() + 2);
    let mut needed = 12 + 16 * tables.len();
    let mut entries: Vec<(usize, Tag, Vec<u8>)> = Vec::with_capacity(tables.len());
    for t in &tables {
        let tag = t.tag.to_be_bytes();
        let name = sfnt::tag_str(tag);
        let (offset, stored, length) = (t.offset as usize, t.compressed as usize, t.length as usize);
        needed = needed.saturating_add(pad4(length).unwrap_or(usize::MAX));
        if needed > total {
            return Err(bad("the tables are larger than the header's font size"));
        }
        if stored > length {
            return Err(bad(format!("table '{name}' is stored larger than it is")));
        }
        if offset % 4 != 0 || offset < directory_end {
            return Err(bad(format!("table '{name}' is misplaced")));
        }
        let end = offset.checked_add(stored).ok_or_else(|| bad(format!("table '{name}' lies outside the file")))?;
        let raw = woff.get(offset..end).ok_or_else(|| bad(format!("table '{name}' lies outside the file")))?;
        if entries.iter().any(|(_, seen, _)| *seen == tag) {
            return Err(bad(format!("table '{name}' is listed twice")));
        }
        let data = if stored == length { raw.to_vec() } else { unzlib(FORMAT, raw, length, &format!("table '{name}'"))? };
        spans.push(Span { start: offset, end });
        entries.push((offset, tag, data));
    }
    let (meta_offset, meta_length, meta_size) = (head.meta.0 as usize, head.meta.1 as usize, head.meta.2 as usize);
    let metadata = if meta_length == 0 {
        None
    } else {
        if meta_size > MAX_METADATA {
            return Err(bad("the metadata is too large"));
        }
        if meta_offset % 4 != 0 || meta_offset < directory_end {
            return Err(bad("the metadata is misplaced"));
        }
        let end = meta_offset.checked_add(meta_length).ok_or_else(|| bad("the metadata lies outside the file"))?;
        let raw = woff.get(meta_offset..end).ok_or_else(|| bad("the metadata lies outside the file"))?;
        spans.push(Span { start: meta_offset, end });
        Some(unzlib(FORMAT, raw, meta_size, "the metadata")?)
    };
    let (private_offset, private_length) = (head.private.0 as usize, head.private.1 as usize);
    let private = if private_length == 0 {
        None
    } else {
        if private_offset % 4 != 0 || private_offset < directory_end {
            return Err(bad("the private data is misplaced"));
        }
        let end = private_offset.checked_add(private_length).ok_or_else(|| bad("the private data lies outside the file"))?;
        let raw = woff.get(private_offset..end).ok_or_else(|| bad("the private data lies outside the file"))?;
        spans.push(Span { start: private_offset, end });
        Some(raw.to_vec())
    };
    spans.sort_by_key(|s| (s.start, s.end));
    if spans.windows(2).any(|w| matches!(w, [a, b] if b.start < a.end)) {
        return Err(bad("two blocks of the file overlap"));
    }
    entries.sort_by_key(|(offset, tag, _)| (*offset, *tag));
    let order: Vec<Table<'_>> = entries.iter().map(|(_, tag, data)| Table { tag: *tag, data }).collect();
    let sfnt = sfnt::assemble(head.flavor, &order)?;
    Ok(Unwrapped { sfnt, version: head.version, metadata, private })
}

/// Wraps an sfnt font in WOFF 1.0. Collections are refused.
pub fn encode(sfnt_bytes: &[u8], extras: &Extras) -> Result<Vec<u8>, Error> {
    let font = sfnt::parse(sfnt_bytes)?;
    if font.tables.is_empty() {
        return Err(bad("the font has no tables"));
    }
    let order = sfnt::recommended_order(&font.tables);
    let adjustment = sfnt::checksum_adjustment(font.flavor, &order)?;
    let head = font.get(HEAD).map(|h| sfnt::with_adjustment(h, adjustment)).transpose()?;
    let count = order.len();
    let mut out = vec![0u8; HEADER + ENTRY * count];
    let mut records: Vec<(Tag, usize, usize, usize, u32)> = Vec::with_capacity(count);
    let mut total_sfnt = 12 + 16 * count;
    for t in &order {
        let data = match (&head, t.tag == HEAD) {
            (Some(h), true) => h.as_slice(),
            _ => t.data,
        };
        let packed = if t.tag == HEAD { None } else { Some(zlib(data)).filter(|z| z.len() < data.len()) };
        let stored = packed.as_deref().unwrap_or(data);
        let offset = out.len();
        out.extend_from_slice(stored);
        out.resize(pad4(out.len()).ok_or_else(|| bad("the font is too large"))?, 0);
        records.push((t.tag, offset, stored.len(), data.len(), sfnt::table_checksum(t.tag, data)));
        total_sfnt = total_sfnt.saturating_add(pad4(data.len()).unwrap_or(usize::MAX));
    }
    if total_sfnt > MAX_SFNT {
        return Err(bad("the font is too large"));
    }
    records.sort_by_key(|r| r.0);
    for (i, (tag, offset, stored, length, checksum)) in records.iter().enumerate() {
        let at = HEADER + ENTRY * i;
        put_u32(&mut out, at, u32::from_be_bytes(*tag));
        put_u32(&mut out, at + 4, to_u32(FORMAT, *offset, "the file")?);
        put_u32(&mut out, at + 8, to_u32(FORMAT, *stored, "a table")?);
        put_u32(&mut out, at + 12, to_u32(FORMAT, *length, "a table")?);
        put_u32(&mut out, at + 16, *checksum);
    }
    let (mut meta_offset, mut meta_length, mut meta_size) = (0, 0, 0);
    if let Some(meta) = extras.metadata.as_deref().filter(|m| !m.is_empty()) {
        if meta.len() > MAX_METADATA {
            return Err(bad("the metadata is too large"));
        }
        let packed = zlib(meta);
        meta_offset = out.len();
        meta_length = packed.len();
        meta_size = meta.len();
        out.extend_from_slice(&packed);
    }
    let (mut private_offset, mut private_length) = (0, 0);
    if let Some(private) = extras.private.as_deref().filter(|p| !p.is_empty()) {
        out.resize(pad4(out.len()).ok_or_else(|| bad("the file is too large"))?, 0);
        private_offset = out.len();
        private_length = private.len();
        out.extend_from_slice(private);
    }
    let version = extras.version.unwrap_or_else(|| revision(font.get(HEAD)));
    let length = to_u32(FORMAT, out.len(), "the file")?;
    if let Some(sig) = out.get_mut(..4) {
        sig.copy_from_slice(SIGNATURE);
    }
    put_u32(&mut out, 4, font.flavor);
    put_u32(&mut out, 8, length);
    put_u16(&mut out, 12, u16::try_from(count).map_err(|_| bad("too many tables"))?);
    put_u32(&mut out, 16, to_u32(FORMAT, total_sfnt, "the font")?);
    put_u16(&mut out, 20, version.0);
    put_u16(&mut out, 22, version.1);
    put_u32(&mut out, 24, to_u32(FORMAT, meta_offset, "the file")?);
    put_u32(&mut out, 28, to_u32(FORMAT, meta_length, "the metadata")?);
    put_u32(&mut out, 32, to_u32(FORMAT, meta_size, "the metadata")?);
    put_u32(&mut out, 36, to_u32(FORMAT, private_offset, "the file")?);
    put_u32(&mut out, 40, to_u32(FORMAT, private_length, "the private data")?);
    Ok(out)
}

/// The font revision from `head` (bytes 4 to 7) as a major and a minor number, or (0, 0).
pub(crate) fn revision(head: Option<&[u8]>) -> (u16, u16) {
    head.and_then(|h| Some((u16_at(h, 4)?, u16_at(h, 6)?))).unwrap_or((0, 0))
}
