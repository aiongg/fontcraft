//! Gzip-compressed bitmap fonts: `.pcf.gz` and `.bdf.gz`, the form X11 font folders keep them in.
//!
//! The stream is inflated by the gzip reader of `fontcraft-container`, with every member's CRC
//! and length checked and the inflated size capped by [`MAX_GUNZIPPED_BYTES`], then read as the
//! PCF or BDF file it holds.

use fontcraft_container::gzip::{self, GunzipError};

use crate::{BitmapError, BitmapFont, Import, MAX_FILE_BYTES, PcfLayout, Result};

/// Most bytes a compressed bitmap font may inflate to: the size the readers accept uncompressed.
pub const MAX_GUNZIPPED_BYTES: usize = MAX_FILE_BYTES;

/// Whether `bytes` start like a gzip stream.
pub fn is_gzip(bytes: &[u8]) -> bool {
    bytes.starts_with(&gzip::MAGIC)
}

/// The bytes a gzip stream holds, at most [`MAX_GUNZIPPED_BYTES`].
fn inflate(bytes: &[u8]) -> Result<Vec<u8>> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(BitmapError::TooLarge(bytes.len()));
    }
    gzip::gunzip(bytes, MAX_GUNZIPPED_BYTES).map_err(|e| match e {
        GunzipError::TooLarge => BitmapError::Limit { what: "gzip-decompressed bytes", limit: MAX_GUNZIPPED_BYTES },
        GunzipError::Invalid(message) => BitmapError::Gzip(message.into()),
    })
}

/// Reads a gzip-compressed PCF or BDF file.
pub fn read_gz(bytes: &[u8]) -> Result<Import> {
    let inner = inflate(bytes)?;
    if crate::is_pcf(&inner) {
        crate::read_pcf(&inner)
    } else if crate::is_bdf(&inner) {
        crate::read_bdf(&inner)
    } else {
        Err(BitmapError::Gzip("the decompressed data is neither a PCF nor a BDF file".into()))
    }
}

/// Writes a strike as a gzip-compressed PCF file (`.pcf.gz`).
pub fn write_pcf_gz(font: &BitmapFont, layout: PcfLayout) -> Result<Vec<u8>> {
    Ok(gzip::gzip(&crate::write_pcf(font, layout)?))
}
