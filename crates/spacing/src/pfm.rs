//! PFM (Printer Font Metrics): the binary metrics file Windows used beside a Type 1 font. Read
//! only, for its pair-kerning table, its character set and its widths. Row `pfm` in
//! `spec/inventory/30-formats.toml`.
//!
//! Layout, from the Windows 3.1 Device Development Kit (Printer Drivers, chapter 2 "Printer Font
//! Metrics (PFM) Files", chapter 4 for PostScript fonts). Every number is little-endian.
//!
//! | Part | Where | What is read |
//! |---|---|---|
//! | `PFMHEADER`, 117 bytes | offset 0 | `dfVersion`, `dfCharSet`, `dfFirstChar`, `dfLastChar`, `dfFace` |
//! | width table | after the header, PCL fonts only | skipped |
//! | `PFMEXTENSION`, 30 bytes | after the header, or after the width table | the offsets of the parts below |
//! | `EXTTEXTMETRIC`, 52 bytes | `dfExtMetricsOffset` | `etmMasterUnits`, `etmKernPairs` |
//! | extent table | `dfExtentTable` | one width per code from `dfFirstChar` to `dfLastChar` |
//! | `KERNPAIR` list | `dfPairKernTable` | two character codes and a signed amount, 4 bytes each |
//!
//! The extension is found by its size field (30): first right after the header, then after a
//! width table of `dfLastChar - dfFirstChar + 2` words. The kit gives the number of pairs in
//! `etmKernPairs` and no count before the list; files made for PostScript printers put that
//! count as a word before the list as well, so a word there equal to `etmKernPairs` is skipped
//! (and, when `etmKernPairs` is 0, the word gives the count).
//!
//! Widths and kerning amounts are in `etmMasterUnits` per em (1,000 for PostScript fonts, which
//! is also what [`Pfm::units`] gives when the field is 0). Kerning names characters by code; the
//! codes mean what `dfCharSet` says ([`unicodes`]).
//!
//! Untrusted input: the file is at most [`MAX_BYTES`], every offset and every count is checked
//! against the file's length before it is used, and the face name is cut at [`MAX_FACE_BYTES`].

/// Largest PFM file, in bytes. A real one is a few kilobytes.
pub const MAX_BYTES: usize = 1024 * 1024;
/// Most kerning pairs read (the count is a 16-bit word).
pub const MAX_PAIRS: usize = 65_535;
/// Longest face name read, in bytes.
pub const MAX_FACE_BYTES: usize = 256;
/// Size of `PFMHEADER`.
pub const HEADER_LEN: usize = 117;
/// Size of `PFMEXTENSION`, which its first field states.
pub const EXTENSION_LEN: usize = 30;
/// Size of `EXTTEXTMETRIC`.
pub const EXT_METRICS_LEN: usize = 52;
/// The units per em a file means when `etmMasterUnits` is 0.
pub const DEFAULT_UNITS: u16 = 1000;
/// `dfCharSet` of a font in the Windows ANSI character set (code page 1252).
pub const CHARSET_ANSI: u8 = 0;
/// `dfCharSet` of a font with its own encoding (symbol and pi fonts).
pub const CHARSET_SYMBOL: u8 = 2;

/// A parsed PFM file: the parts FontCraft reads.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pfm {
    /// `dfVersion`, `0x0100` in files made for Windows 3.
    pub version: u16,
    /// `dfCharSet`: [`CHARSET_ANSI`], [`CHARSET_SYMBOL`], or another Windows character set.
    pub char_set: u8,
    pub first_char: u8,
    pub last_char: u8,
    /// The Windows face name (family), when the file gives a readable one.
    pub face: Option<String>,
    /// `etmMasterUnits` as stored (0 when the file leaves it unset).
    pub master_units: u16,
    /// The extent table: one width per code from `first_char` to `last_char`; empty when the
    /// file has none.
    pub widths: Vec<u16>,
    /// In file order.
    pub pairs: Vec<KernPair>,
}

impl Pfm {
    /// The units per em of widths and kerning amounts.
    pub fn units(&self) -> u16 {
        if self.master_units == 0 { DEFAULT_UNITS } else { self.master_units }
    }

    /// The width of the character `code`, from the extent table.
    pub fn width(&self, code: u8) -> Option<u16> {
        let i = usize::from(code.checked_sub(self.first_char)?);
        self.widths.get(i).copied()
    }
}

/// A kerning pair of two character codes; the amount is in [`Pfm::units`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KernPair {
    pub first: u8,
    pub second: u8,
    pub value: i16,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
    #[error("the PFM file is {0} bytes long; the limit is {MAX_BYTES}")]
    TooLarge(usize),
    #[error("the file is {0} bytes long, too short for a PFM header ({HEADER_LEN} bytes)")]
    TooShort(usize),
    #[error("the file is not a PFM file (version word {0:#06x})")]
    NotPfm(u16),
    #[error("the PFM file has no extension block where one belongs (after the header or its width table)")]
    NoExtension,
    #[error("the PFM {what} at offset {offset} ({len} bytes) runs past the end of the {size}-byte file")]
    PastEnd { what: &'static str, offset: usize, len: usize, size: usize },
}

fn u16_at(data: &[u8], at: usize) -> Option<u16> {
    let b = data.get(at..at.checked_add(2)?)?;
    Some(u16::from_le_bytes([*b.first()?, *b.get(1)?]))
}

fn u32_at(data: &[u8], at: usize) -> Option<usize> {
    let b = data.get(at..at.checked_add(4)?)?;
    let v = u32::from_le_bytes([*b.first()?, *b.get(1)?, *b.get(2)?, *b.get(3)?]);
    usize::try_from(v).ok()
}

/// `len` bytes at `offset`, or [`ReadError::PastEnd`].
fn span<'a>(data: &'a [u8], what: &'static str, offset: usize, len: usize) -> Result<&'a [u8], ReadError> {
    offset.checked_add(len).and_then(|end| data.get(offset..end)).ok_or(ReadError::PastEnd { what, offset, len, size: data.len() })
}

/// Whether `data` looks like a PFM file: long enough for the header, and a version word of
/// `0x0100`, `0x0200` or `0x0300`.
pub fn sniff(data: &[u8]) -> bool {
    data.len() >= HEADER_LEN && u16_at(data, 0).is_some_and(version_ok)
}

fn version_ok(v: u16) -> bool {
    v & 0xFF == 0 && (1..=3).contains(&(v >> 8))
}

/// Reads a PFM file.
pub fn read(data: &[u8]) -> Result<Pfm, ReadError> {
    if data.len() > MAX_BYTES {
        return Err(ReadError::TooLarge(data.len()));
    }
    let header = data.get(..HEADER_LEN).ok_or(ReadError::TooShort(data.len()))?;
    let version = u16_at(header, 0).unwrap_or_default();
    if !version_ok(version) {
        return Err(ReadError::NotPfm(version));
    }
    let byte = |at: usize| header.get(at).copied().unwrap_or_default();
    let (char_set, first_char, last_char) = (byte(85), byte(95), byte(96));
    let face = u32_at(header, 105).and_then(|at| face_name(data, at));

    let mut places = vec![HEADER_LEN];
    if last_char >= first_char {
        places.push(HEADER_LEN + 2 * (usize::from(last_char - first_char) + 2));
    }
    let ext = places
        .into_iter()
        .find(|&at| u16_at(data, at) == Some(EXTENSION_LEN as u16) && data.len() >= at + EXTENSION_LEN)
        .ok_or(ReadError::NoExtension)?;
    let field = |i: usize| u32_at(data, ext + 2 + 4 * i).unwrap_or_default();
    let (etm_at, extent_at, kern_at) = (field(0), field(1), field(3));

    let (master_units, etm_pairs) = if etm_at == 0 {
        (0, 0)
    } else {
        let etm = span(data, "extended text metrics", etm_at, EXT_METRICS_LEN)?;
        (u16_at(etm, 12).unwrap_or_default(), u16_at(etm, 48).unwrap_or_default())
    };

    let mut widths = Vec::new();
    if extent_at != 0 && last_char >= first_char {
        let n = usize::from(last_char - first_char) + 1;
        let table = span(data, "extent table", extent_at, 2 * n)?;
        widths = table.as_chunks::<2>().0.iter().map(|w| u16::from_le_bytes(*w)).collect();
    }

    // Where the pair list must end: the next part the file places after it, or the file's end.
    let parts = [etm_at, extent_at, field(2), field(4), field(5), u32_at(header, 101).unwrap_or_default(), u32_at(header, 105).unwrap_or_default()];
    let list_end = parts.into_iter().filter(|&p| p > kern_at).min().unwrap_or(data.len()).min(data.len());
    let pairs = if kern_at == 0 { Vec::new() } else { kern_pairs(data, kern_at, usize::from(etm_pairs), list_end)? };
    Ok(Pfm { version, char_set, first_char, last_char, face, master_units, widths, pairs })
}

/// The pair list at `at`: `count` pairs, after a count word when the word there equals `count`
/// (or gives the count when `count` is 0). The first pair's two codes can read as a word equal
/// to `count`, so a list without the word that ends exactly at `list_end` (the next part of the
/// file, or its end) is taken as it is.
fn kern_pairs(data: &[u8], at: usize, count: usize, list_end: usize) -> Result<Vec<KernPair>, ReadError> {
    let word = u16_at(data, at).map(usize::from);
    let end = |start: usize, n: usize| start.checked_add(4 * n);
    let fits = |start: usize, n: usize| end(start, n).is_some_and(|e| e <= data.len());
    let (start, count) = match word {
        Some(w) if count == 0 => (at + 2, w),
        Some(w) if w == count && fits(at + 2, count) && end(at, count) != Some(list_end) => (at + 2, count),
        _ => (at, count),
    };
    let count = count.min(MAX_PAIRS);
    let list = span(data, "kerning pair table", start, 4 * count)?;
    Ok(list.as_chunks::<4>().0.iter().map(|&[first, second, lo, hi]| KernPair { first, second, value: i16::from_le_bytes([lo, hi]) }).collect())
}

/// The zero-terminated name at `at`, cut at [`MAX_FACE_BYTES`]; `None` for offset 0, a name
/// past the end, an empty one, or one that is not Windows-1252 text without control characters.
fn face_name(data: &[u8], at: usize) -> Option<String> {
    if at == 0 {
        return None;
    }
    let rest = data.get(at..)?;
    let bytes = rest.iter().take(MAX_FACE_BYTES).take_while(|&&b| b != 0);
    let mut name = String::new();
    for &b in bytes {
        let c = char::from_u32(cp1252(b)?)?;
        if c.is_control() {
            return None;
        }
        name.push(c);
    }
    (!name.is_empty()).then_some(name)
}

/// The Unicode value of a Windows-1252 byte; `None` for the five codes the code page leaves
/// undefined.
pub fn cp1252(code: u8) -> Option<u32> {
    const HIGH: [u32; 32] = [
        0x20AC, 0, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0, 0x017D, 0, //
        0, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0, 0x017E, 0x0178,
    ];
    match code {
        0x80..=0x9F => HIGH.get(usize::from(code - 0x80)).copied().filter(|&u| u != 0),
        _ => Some(u32::from(code)),
    }
}

/// The Unicode values a character code of a font in `char_set` may stand for, most likely
/// first: a symbol font's code `c` is U+F000 + `c` (where such fonts put their characters in a
/// cmap) or `c` itself; any other character set is read as Windows-1252.
pub fn unicodes(char_set: u8, code: u8) -> Vec<u32> {
    if char_set == CHARSET_SYMBOL { vec![0xF000 + u32::from(code), u32::from(code)] } else { cp1252(code).into_iter().collect() }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    /// A PFM file for PostScript printers: header, extension, extended metrics, the face name,
    /// an extent table for codes 32–126, and `pairs` after a count word when `count_word`.
    fn build(pairs: &[(u8, u8, i16)], count_word: bool) -> Vec<u8> {
        let mut d = vec![0u8; HEADER_LEN];
        d[0..2].copy_from_slice(&0x0100u16.to_le_bytes());
        d[85] = CHARSET_ANSI;
        d[95] = 32;
        d[96] = 126;
        let ext = d.len();
        d.extend_from_slice(&[0u8; EXTENSION_LEN]);
        d[ext..ext + 2].copy_from_slice(&(EXTENSION_LEN as u16).to_le_bytes());
        let etm = d.len();
        d.extend_from_slice(&[0u8; EXT_METRICS_LEN]);
        d[etm..etm + 2].copy_from_slice(&(EXT_METRICS_LEN as u16).to_le_bytes());
        d[etm + 12..etm + 14].copy_from_slice(&1000u16.to_le_bytes());
        d[etm + 48..etm + 50].copy_from_slice(&(pairs.len() as u16).to_le_bytes());
        let face = d.len();
        d.extend_from_slice(b"Test Sans\0");
        let extent = d.len();
        for code in 32u16..=126 {
            d.extend_from_slice(&(code * 5).to_le_bytes());
        }
        let kern = d.len();
        if count_word {
            d.extend_from_slice(&(pairs.len() as u16).to_le_bytes());
        }
        for &(a, b, v) in pairs {
            d.extend_from_slice(&[a, b]);
            d.extend_from_slice(&v.to_le_bytes());
        }
        let size = d.len() as u32;
        d[2..6].copy_from_slice(&size.to_le_bytes());
        d[105..109].copy_from_slice(&(face as u32).to_le_bytes());
        for (i, at) in [(0, etm), (1, extent), (3, kern)] {
            d[ext + 2 + 4 * i..ext + 6 + 4 * i].copy_from_slice(&(at as u32).to_le_bytes());
        }
        d
    }

    #[test]
    fn reads_pairs_with_and_without_count_word() {
        let pairs = [(b'A', b'V', -80), (b'T', b'o', -45), (0x93, b'A', -10)];
        for count_word in [true, false] {
            let pfm = read(&build(&pairs, count_word)).unwrap();
            assert_eq!(pfm.face.as_deref(), Some("Test Sans"));
            assert_eq!(pfm.units(), 1000);
            assert_eq!(pfm.width(b'A'), Some(65 * 5));
            assert_eq!(pfm.width(31), None);
            let got: Vec<_> = pfm.pairs.iter().map(|p| (p.first, p.second, p.value)).collect();
            assert_eq!(got, pairs, "count word: {count_word}");
        }
        assert_eq!(unicodes(CHARSET_ANSI, 0x93), vec![0x201C]);
        assert_eq!(unicodes(CHARSET_ANSI, 0x81), Vec::<u32>::new());
        assert_eq!(unicodes(CHARSET_SYMBOL, 0x41), vec![0xF041, 0x41]);
    }

    #[test]
    fn a_first_pair_that_reads_as_the_count_is_kept_when_a_part_follows_the_list() {
        // The first pair's codes (3, 0) read as the word 3, the number of pairs. A track kerning
        // table follows the list, so the list does not end at the file's end.
        let pairs = [(3, 0, -10), (b'A', b'V', -80), (b'T', b'o', -45)];
        for count_word in [false, true] {
            let mut d = build(&pairs, count_word);
            let track = d.len() as u32;
            d.extend_from_slice(&[0, 0]);
            d[HEADER_LEN + 2 + 16..HEADER_LEN + 6 + 16].copy_from_slice(&track.to_le_bytes());
            let got: Vec<_> = read(&d).unwrap().pairs.iter().map(|p| (p.first, p.second, p.value)).collect();
            assert_eq!(got, pairs, "count word: {count_word}");
        }
    }

    #[test]
    fn offsets_and_counts_past_the_end_are_refused() {
        let good = build(&[(b'A', b'V', -80)], true);
        // The pair count claims more pairs than the file holds.
        let mut d = good.clone();
        let etm = HEADER_LEN + EXTENSION_LEN;
        d[etm + 48..etm + 50].copy_from_slice(&60_000u16.to_le_bytes());
        assert!(matches!(read(&d), Err(ReadError::PastEnd { .. })), "{:?}", read(&d));
        // Every offset of the extension pointing past the end, or near the end of the address space.
        for field in [0usize, 1, 3] {
            for offset in [d.len() as u32, u32::MAX, u32::MAX - 1] {
                let mut d = good.clone();
                let at = HEADER_LEN + 2 + 4 * field;
                d[at..at + 4].copy_from_slice(&offset.to_le_bytes());
                assert!(matches!(read(&d), Err(ReadError::PastEnd { .. })), "field {field}, offset {offset}");
            }
        }
        // A face name past the end is dropped, not followed.
        let mut d = good.clone();
        d[105..109].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(read(&d).unwrap().face, None);
        // Every truncation of a good file fails cleanly or reads.
        for n in 0..good.len() {
            let _ = read(&good[..n]);
        }
        assert_eq!(read(&good[..100]), Err(ReadError::TooShort(100)));
        assert_eq!(read(&[0u8; 200]), Err(ReadError::NotPfm(0)));
        let mut no_ext = good.clone();
        no_ext[HEADER_LEN] = 0;
        assert_eq!(read(&no_ext), Err(ReadError::NoExtension));
        assert!(matches!(read(&vec![0u8; MAX_BYTES + 1]), Err(ReadError::TooLarge(_))));
    }
}
