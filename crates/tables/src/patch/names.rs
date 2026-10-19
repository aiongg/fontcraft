//! `tables.cleanNameRecords`: white space trimmed and collapsed, empty and repeated records
//! removed, Macintosh records dropped, and the PostScript name records made to agree.
//!
//! The table is parsed and written byte by byte, so records whose encoding is not known (and
//! cannot be cleaned) and the language-tag records of format 1 keep their bytes.

use std::collections::HashMap;

use read_fonts::tables::name::MacRomanMapping;
use read_fonts::{FontData, FontRead};

use super::{be_u16, missing};
use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

const NAME: Tag = *b"name";
/// Most name records read; the count field is 16 bits.
const MAX_RECORDS: usize = 65_535;

/// What [`clean_name_records`] does besides the white space.
#[derive(Clone, Debug, PartialEq)]
pub struct NameOptions {
    /// Remove the Macintosh-platform records.
    pub drop_mac: bool,
}

impl Default for NameOptions {
    fn default() -> Self {
        Self { drop_mac: true }
    }
}

/// The result of [`clean_name_records`].
#[derive(Clone, Debug, PartialEq)]
pub struct NameCleanup {
    pub font: SfntFont,
    /// Records removed: empty, repeated or Macintosh.
    pub removed: usize,
    /// Records whose text changed.
    pub changed: usize,
}

#[derive(Clone)]
struct Record {
    platform: u16,
    encoding: u16,
    language: u16,
    id: u16,
    /// The decoded text, when the encoding is one we can write again.
    text: Option<String>,
    raw: Vec<u8>,
}

impl Record {
    fn key(&self) -> (u16, u16, u16, u16) {
        (self.platform, self.encoding, self.language, self.id)
    }

    fn utf16(&self) -> bool {
        self.platform == 0 || (self.platform == 3 && matches!(self.encoding, 0 | 1 | 10))
    }

    fn mac_roman(&self) -> bool {
        self.platform == 1 && self.encoding == 0
    }

    fn decode(&self) -> Option<String> {
        if self.utf16() {
            let (chunks, rest) = self.raw.as_chunks::<2>();
            if !rest.is_empty() {
                return None;
            }
            let units: Vec<u16> = chunks.iter().map(|c| u16::from_be_bytes(*c)).collect();
            String::from_utf16(&units).ok()
        } else if self.mac_roman() {
            Some(self.raw.iter().map(|b| MacRomanMapping.decode(*b)).collect())
        } else {
            None
        }
    }

    /// `text` in this record's encoding; `None` when a character cannot be written in it.
    fn encode(&self, text: &str) -> Option<Vec<u8>> {
        if self.utf16() {
            Some(text.encode_utf16().flat_map(u16::to_be_bytes).collect())
        } else if self.mac_roman() {
            text.chars().map(|c| MacRomanMapping.encode(c)).collect()
        } else {
            None
        }
    }
}

/// Trims the text and turns every run of white space (line breaks included) into one space.
fn clean(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The CFF font name, when the font has a `CFF ` table with exactly one.
fn cff_name(font: &SfntFont) -> Option<String> {
    let cff = read_fonts::tables::cff::Cff::read(FontData::new(font.get(*b"CFF ")?)).ok()?;
    if cff.names().count() != 1 {
        return None;
    }
    cff.name(0).map(|b| String::from_utf8_lossy(b).into_owned())
}

fn parse(data: &[u8]) -> Result<(u16, Vec<Record>, Vec<Vec<u8>>)> {
    let bad = || TablesError::invalid("the name table is damaged");
    let format = be_u16(data, 0).ok_or_else(bad)?;
    let count = usize::from(be_u16(data, 2).ok_or_else(bad)?).min(MAX_RECORDS);
    let storage = usize::from(be_u16(data, 4).ok_or_else(bad)?);
    let string = |len: u16, off: u16| -> Result<Vec<u8>> {
        let start = storage.checked_add(usize::from(off)).ok_or_else(bad)?;
        let end = start.checked_add(usize::from(len)).ok_or_else(bad)?;
        Ok(data.get(start..end).ok_or_else(bad)?.to_vec())
    };
    let mut records = Vec::with_capacity(count);
    for i in 0..count {
        let at = 6usize.saturating_add(i.saturating_mul(12));
        let field = |k: usize| be_u16(data, at.saturating_add(k * 2)).ok_or_else(bad);
        let raw = string(field(4)?, field(5)?)?;
        let mut r = Record { platform: field(0)?, encoding: field(1)?, language: field(2)?, id: field(3)?, text: None, raw };
        r.text = r.decode();
        records.push(r);
    }
    let mut tags = Vec::new();
    if format == 1 {
        let at = 6usize.saturating_add(count.saturating_mul(12));
        let tag_count = usize::from(be_u16(data, at).ok_or_else(bad)?);
        for i in 0..tag_count {
            let rec = at.saturating_add(2).saturating_add(i.saturating_mul(4));
            tags.push(string(be_u16(data, rec).ok_or_else(bad)?, be_u16(data, rec.saturating_add(2)).ok_or_else(bad)?)?);
        }
    }
    Ok((format, records, tags))
}

fn write(format: u16, records: &[Record], tags: &[Vec<u8>]) -> Result<Vec<u8>> {
    let too_big = || TablesError::TooLarge { what: "the name table's string storage", limit: 65_535 };
    let header = 6 + records.len() * 12 + if format == 1 { 2 + tags.len() * 4 } else { 0 };
    let storage_offset = u16::try_from(header).map_err(|_| too_big())?;
    let mut storage: Vec<u8> = Vec::new();
    let mut placed: HashMap<Vec<u8>, u16> = HashMap::new();
    let mut place = |bytes: &[u8]| -> Result<(u16, u16)> {
        let len = u16::try_from(bytes.len()).map_err(|_| too_big())?;
        if let Some(off) = placed.get(bytes) {
            return Ok((len, *off));
        }
        let off = u16::try_from(storage.len()).map_err(|_| too_big())?;
        storage.extend_from_slice(bytes);
        u16::try_from(storage.len()).map_err(|_| too_big())?;
        placed.insert(bytes.to_vec(), off);
        Ok((len, off))
    };
    let mut out = Vec::with_capacity(header);
    out.extend(format.to_be_bytes());
    out.extend(u16::try_from(records.len()).map_err(|_| too_big())?.to_be_bytes());
    out.extend(storage_offset.to_be_bytes());
    for r in records {
        let (len, off) = place(&r.raw)?;
        for v in [r.platform, r.encoding, r.language, r.id, len, off] {
            out.extend(v.to_be_bytes());
        }
    }
    if format == 1 {
        out.extend(u16::try_from(tags.len()).map_err(|_| too_big())?.to_be_bytes());
        for t in tags {
            let (len, off) = place(t)?;
            out.extend(len.to_be_bytes());
            out.extend(off.to_be_bytes());
        }
    }
    out.extend(storage);
    Ok(out)
}

/// The font with its name records cleaned.
pub fn clean_name_records(font: &SfntFont, options: &NameOptions) -> Result<NameCleanup> {
    let data = font.get(NAME).ok_or_else(|| missing(NAME))?;
    let (format, records, tags) = parse(data)?;
    let before = records.len();
    let mut changed = 0usize;
    let mut kept: Vec<Record> = Vec::with_capacity(records.len());
    for mut r in records {
        if options.drop_mac && r.platform == 1 {
            continue;
        }
        if let Some(text) = &r.text {
            let cleaned = clean(text);
            if cleaned.is_empty() {
                continue;
            }
            if cleaned != *text
                && let Some(raw) = r.encode(&cleaned)
            {
                r.raw = raw;
                r.text = Some(cleaned);
                changed = changed.saturating_add(1);
            }
        }
        if kept.iter().any(|k| k.key() == r.key()) {
            continue;
        }
        kept.push(r);
    }
    // One PostScript name: the CFF font name, else the Windows English record, else the first.
    let ps = cff_name(font)
        .or_else(|| kept.iter().find(|r| r.id == 6 && r.platform == 3 && r.language == 0x409).and_then(|r| r.text.clone()))
        .or_else(|| kept.iter().find(|r| r.id == 6).and_then(|r| r.text.clone()));
    if let Some(ps) = ps {
        for r in kept.iter_mut().filter(|r| r.id == 6) {
            if r.text.as_deref() != Some(ps.as_str())
                && let Some(raw) = r.encode(&ps)
            {
                r.raw = raw;
                r.text = Some(ps.clone());
                changed = changed.saturating_add(1);
            }
        }
    }
    kept.sort_by_key(Record::key);
    let removed = before.saturating_sub(kept.len());
    let mut out = font.clone();
    if removed > 0 || changed > 0 {
        out.insert(NAME, write(format, &kept, &tags)?);
    }
    Ok(NameCleanup { font: out, removed, changed })
}
