//! The container of the built-in tables: a small header and a zlib stream holding named sections.
//!
//! ```text
//! b"FCGD"  u16 format version  u16 0  u32 payload length  zlib stream (the payload)
//! payload: u32 section count, then per section [u8; 4] tag, u32 offset, u32 length, then the data
//! ```
//!
//! All integers are little-endian. Every read is bounds-checked: a damaged blob gives an error or
//! `None`, never a panic.

use crate::Error;

pub const MAGIC: &[u8; 4] = b"FCGD";
/// Bumped whenever a section's layout changes; the reader refuses other versions.
pub const FORMAT_VERSION: u16 = 1;
/// Largest payload the reader inflates.
pub const MAX_PAYLOAD: usize = 64 << 20;
/// Most sections in one payload.
pub const MAX_SECTIONS: usize = 64;

pub fn u8_at(b: &[u8], at: usize) -> Option<u8> {
    b.get(at).copied()
}

pub fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    let s = b.get(at..at.checked_add(2)?)?;
    Some(u16::from_le_bytes([*s.first()?, *s.get(1)?]))
}

pub fn u24_at(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(3)?)?;
    Some(u32::from_le_bytes([*s.first()?, *s.get(1)?, *s.get(2)?, 0]))
}

pub fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([*s.first()?, *s.get(1)?, *s.get(2)?, *s.get(3)?]))
}

/// A section's tag and its bytes' range in the payload.
pub type SectionRange = ([u8; 4], std::ops::Range<usize>);

/// A decoded payload: the bytes and where each section is.
pub struct Payload {
    data: Vec<u8>,
    sections: Vec<([u8; 4], usize, usize)>,
}

impl Payload {
    /// Checks the header, inflates the payload and reads the section directory.
    pub fn open(blob: &[u8]) -> Result<Payload, Error> {
        let bad = |m: &str| Error::Blob(m.to_string());
        if blob.get(..4) != Some(MAGIC.as_slice()) {
            return Err(bad("not a glyph data blob"));
        }
        let version = u16_at(blob, 4).ok_or_else(|| bad("truncated header"))?;
        if version != FORMAT_VERSION {
            return Err(Error::Blob(format!("format version {version}, expected {FORMAT_VERSION}")));
        }
        let len = u32_at(blob, 8).ok_or_else(|| bad("truncated header"))? as usize;
        if len > MAX_PAYLOAD {
            return Err(bad("payload too large"));
        }
        let stream = blob.get(12..).ok_or_else(|| bad("truncated header"))?;
        let data = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(stream, len).map_err(|e| Error::Blob(format!("inflate: {e:?}")))?;
        if data.len() != len {
            return Err(bad("payload length differs from the header"));
        }
        Payload::from_data(data)
    }

    /// The payload bytes and the byte range of each section.
    pub fn into_parts(self) -> (Vec<u8>, Vec<SectionRange>) {
        let ranges = self.sections.iter().map(|(t, o, l)| (*t, *o..o + l)).collect();
        (self.data, ranges)
    }

    fn from_data(data: Vec<u8>) -> Result<Payload, Error> {
        let bad = |m: &str| Error::Blob(m.to_string());
        let count = u32_at(&data, 0).ok_or_else(|| bad("truncated section directory"))? as usize;
        if count > MAX_SECTIONS {
            return Err(bad("too many sections"));
        }
        let mut sections = Vec::with_capacity(count);
        for i in 0..count {
            let at = 4 + i * 12;
            let tag = data.get(at..at + 4).ok_or_else(|| bad("truncated section directory"))?;
            let tag = [
                tag.first().copied().unwrap_or(0),
                tag.get(1).copied().unwrap_or(0),
                tag.get(2).copied().unwrap_or(0),
                tag.get(3).copied().unwrap_or(0),
            ];
            let offset = u32_at(&data, at + 4).ok_or_else(|| bad("truncated section directory"))? as usize;
            let length = u32_at(&data, at + 8).ok_or_else(|| bad("truncated section directory"))? as usize;
            if offset.checked_add(length).is_none_or(|end| end > data.len()) {
                return Err(Error::Blob(format!("section {} lies outside the payload", String::from_utf8_lossy(&tag))));
            }
            sections.push((tag, offset, length));
        }
        Ok(Payload { data, sections })
    }

    /// The bytes of a section, if present.
    pub fn section(&self, tag: &[u8; 4]) -> Option<&[u8]> {
        let (_, offset, length) = self.sections.iter().find(|(t, _, _)| t == tag)?;
        self.data.get(*offset..offset.checked_add(*length)?)
    }

    /// A section that must exist and hold whole elements of `size` bytes.
    pub fn table(&self, tag: &[u8; 4], size: usize) -> Result<&[u8], Error> {
        let s = self.section(tag).ok_or_else(|| Error::Blob(format!("missing section {}", String::from_utf8_lossy(tag))))?;
        if size == 0 || s.len() % size != 0 {
            return Err(Error::Blob(format!("section {} is not a whole number of {size}-byte entries", String::from_utf8_lossy(tag))));
        }
        Ok(s)
    }
}

/// Builds a payload from sections and wraps it in the container.
#[derive(Default)]
pub struct Writer {
    sections: Vec<([u8; 4], Vec<u8>)>,
}

impl Writer {
    pub fn add(&mut self, tag: &[u8; 4], data: Vec<u8>) {
        self.sections.push((*tag, data));
    }

    /// The container bytes. Fails when the payload would break the reader's limits.
    pub fn finish(self) -> Result<Vec<u8>, String> {
        if self.sections.len() > MAX_SECTIONS {
            return Err("too many sections".into());
        }
        let mut payload = Vec::new();
        payload.extend_from_slice(&(self.sections.len() as u32).to_le_bytes());
        let mut offset = 4 + 12 * self.sections.len();
        for (tag, data) in &self.sections {
            payload.extend_from_slice(tag);
            payload.extend_from_slice(&(offset as u32).to_le_bytes());
            payload.extend_from_slice(&(data.len() as u32).to_le_bytes());
            offset += data.len();
        }
        for (_, data) in &self.sections {
            payload.extend_from_slice(data);
        }
        if payload.len() > MAX_PAYLOAD {
            return Err("payload too large".into());
        }
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&miniz_oxide::deflate::compress_to_vec_zlib(&payload, 10));
        Ok(out)
    }
}

pub fn push_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

pub fn push_u24(out: &mut Vec<u8>, v: u32) -> Result<(), String> {
    if v > 0xFF_FFFF {
        return Err(format!("{v} does not fit in 24 bits"));
    }
    out.extend_from_slice(v.to_le_bytes().get(..3).unwrap_or(&[]));
    Ok(())
}

pub fn push_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_damage() {
        let mut w = Writer::default();
        w.add(b"ABCD", vec![1, 2, 3, 4, 5, 6]);
        w.add(b"EFGH", vec![]);
        let blob = w.finish().unwrap();
        let p = Payload::open(&blob).unwrap();
        assert_eq!(p.section(b"ABCD"), Some(&[1u8, 2, 3, 4, 5, 6][..]));
        assert_eq!(p.table(b"ABCD", 3).unwrap().len(), 6);
        assert!(p.table(b"ABCD", 4).is_err());
        assert!(p.table(b"ZZZZ", 1).is_err());
        // Every truncation and every single-byte change fails cleanly or still reads.
        for cut in 0..blob.len() {
            let _ = Payload::open(&blob[..cut]);
        }
        for i in 0..blob.len() {
            let mut b = blob.clone();
            b[i] ^= 0xFF;
            let _ = Payload::open(&b);
        }
        assert!(Payload::open(&[]).is_err());
    }
}
