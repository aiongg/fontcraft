//! Font containers (L0, standalone): WOFF 1 and WOFF2 wrapped around an sfnt font (a TrueType or
//! CFF-based OpenType file), and unwrapped back to one.
//!
//! - [`woff::encode`] and [`woff::decode`]: WOFF 1.0 (W3C recommendation). Each table is
//!   compressed with zlib on its own, at level 6, and stored raw when that does not make it
//!   smaller; `head` is always stored raw.
//! - [`woff2::encode`] and [`woff2::decode`]: WOFF 2.0 (W3C recommendation). All tables go into
//!   one Brotli stream (quality 11, window 22, font mode). The `glyf`/`loca` transform and the
//!   `hmtx` transform are written when [`woff2::Options`] asks for them and read back.
//! - [`to_sfnt`]: whatever the container, the sfnt (or collection) inside.
//!
//! Byte choices (table order, padding, checksums, header versions, the metadata and private
//! blocks) follow fontTools, whose `ttLib/sfnt.py` and `ttLib/woff2.py` (MIT) these modules port.
//! WOFF2 also wraps collections (`ttcf`): each distinct table is stored once and every font lists
//! the tables it uses. WOFF 1.0 has no collection form and refuses them.
//!
//! Every input is untrusted. Counts are capped ([`MAX_TABLES`]; glyph, contour and point counts
//! by the 16-bit fields that hold them and by the data actually present), every offset and length
//! is checked against the file, tables may not overlap, and decompression stops at the size the
//! file states, which is capped by [`MAX_SFNT`] (fonts, rebuilt `glyf` tables) and [`MAX_METADATA`]
//! (metadata). Nothing here panics.
#![forbid(unsafe_code)]

mod compress;
pub mod sfnt;
pub mod woff;
pub mod woff2;

#[cfg(test)]
mod tests;

use std::borrow::Cow;

/// Largest uncompressed font accepted or produced, in bytes.
pub const MAX_SFNT: usize = 256 << 20;
/// Most tables a font or container may list.
pub const MAX_TABLES: usize = 1024;
/// Largest uncompressed extended metadata block accepted, in bytes.
pub const MAX_METADATA: usize = 16 << 20;
/// Most fonts a collection may hold, plain or in WOFF2.
pub const MAX_COLLECTION_FONTS: usize = 256;

/// Why a font could not be wrapped or unwrapped.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The input is damaged or not of the expected kind.
    #[error("{format}: {what}")]
    Invalid { format: &'static str, what: String },
    /// The input is well-formed but uses something this version does not handle.
    #[error("{format}: {what} is not supported yet")]
    Unsupported { format: &'static str, what: &'static str },
}

impl Error {
    pub(crate) fn invalid(format: &'static str, what: impl Into<String>) -> Self {
        Error::Invalid { format, what: what.into() }
    }
}

/// The optional parts of a WOFF or WOFF2 file besides the font.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Extras {
    /// `majorVersion` and `minorVersion` of the header. `None` writes the font's revision from
    /// `head` (both 0 without a `head` table).
    pub version: Option<(u16, u16)>,
    /// The extended metadata, uncompressed (an XML document). Empty or `None` writes none.
    pub metadata: Option<Vec<u8>>,
    /// The private data block, stored as is. Empty or `None` writes none.
    pub private: Option<Vec<u8>>,
}

/// An unwrapped WOFF or WOFF2 file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unwrapped {
    /// The sfnt font: tables in the order the container stores them, the table directory sorted
    /// by tag, every checksum and `head.checkSumAdjustment` computed for this layout.
    pub sfnt: Vec<u8>,
    /// `majorVersion` and `minorVersion` from the header.
    pub version: (u16, u16),
    /// The extended metadata, decompressed, when the file has any.
    pub metadata: Option<Vec<u8>>,
    /// The private data block, when the file has one.
    pub private: Option<Vec<u8>>,
}

/// What a font file is, from its first four bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A TrueType-flavoured sfnt (`0x00010000` or `true`).
    TrueType,
    /// A CFF-flavoured sfnt (`OTTO`).
    Cff,
    /// A font collection (`ttcf`).
    Collection,
    /// WOFF 1.0 (`wOFF`).
    Woff,
    /// WOFF 2.0 (`wOF2`).
    Woff2,
}

/// The kind of font `bytes` holds; `None` for anything else.
pub fn sniff(bytes: &[u8]) -> Option<Kind> {
    match bytes.get(..4)? {
        [0, 1, 0, 0] | b"true" => Some(Kind::TrueType),
        b"OTTO" => Some(Kind::Cff),
        b"ttcf" => Some(Kind::Collection),
        b"wOFF" => Some(Kind::Woff),
        b"wOF2" => Some(Kind::Woff2),
        _ => None,
    }
}

/// The sfnt (or collection) inside `bytes`: WOFF and WOFF2 are unwrapped, anything else is
/// returned unchanged.
pub fn to_sfnt(bytes: &[u8]) -> Result<Cow<'_, [u8]>, Error> {
    match sniff(bytes) {
        Some(Kind::Woff) => Ok(Cow::Owned(woff::decode(bytes)?.sfnt)),
        Some(Kind::Woff2) => Ok(Cow::Owned(woff2::decode(bytes)?.sfnt)),
        _ => Ok(Cow::Borrowed(bytes)),
    }
}

pub(crate) fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

pub(crate) fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

/// `n` rounded up to a multiple of four, or `None` on overflow.
pub(crate) fn pad4(n: usize) -> Option<usize> {
    n.checked_add(3).map(|v| v & !3)
}

/// Writes `value` big-endian at `at`; out-of-range writes do nothing (callers size the buffer).
pub(crate) fn put_u32(out: &mut [u8], at: usize, value: u32) {
    if let Some(slot) = at.checked_add(4).and_then(|end| out.get_mut(at..end)) {
        slot.copy_from_slice(&value.to_be_bytes());
    }
}

pub(crate) fn put_u16(out: &mut [u8], at: usize, value: u16) {
    if let Some(slot) = at.checked_add(2).and_then(|end| out.get_mut(at..end)) {
        slot.copy_from_slice(&value.to_be_bytes());
    }
}

/// `n` as a `u32`, or an error naming what it is.
pub(crate) fn to_u32(format: &'static str, n: usize, what: &str) -> Result<u32, Error> {
    u32::try_from(n).map_err(|_| Error::invalid(format, format!("{what} is too large")))
}
