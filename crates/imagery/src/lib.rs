//! FontCraft image decoding (standalone, builds for wasm): what an image file is and how large,
//! read from its header alone, then PNG, JPEG and TIFF decoded to RGBA within named limits, and
//! RGBA encoded as PNG. Plan: `docs/plans/M11.md` § 3 › Image decoding and § 4 › M11.12; rows
//! `image-png`, `image-jpeg`, `image-tiff`, `image.supportedFormats`.
//!
//! Contract:
//! - [`sniff`] reads the header only and decodes nothing. Every decode checks the sniffed size
//!   against [`MAX_SIDE`] and the caller's pixel budget (at most [`MAX_PIXELS`]) first, so a file
//!   that claims a huge size fails before any allocation.
//! - Input is at most [`MAX_FILE_BYTES`]; TIFF reads the first page only and at most
//!   [`MAX_TIFF_IFD_ENTRIES`] directory entries.
//! - PDF and PSD are recognised and kept as references; they are not decoded
//!   ([`ImageError::Unsupported`]), and the editor draws an unsupported-image marker.
//! - Backed by the `image` crate with its `png`, `jpeg` and `tiff` decoders only. Nothing here
//!   panics on input: errors are [`ImageError`].
#![forbid(unsafe_code)]

/// Largest image file read for display or export.
pub const MAX_FILE_BYTES: usize = 64 << 20;
/// Largest width or height decoded, in pixels.
pub const MAX_SIDE: u32 = 16_384;
/// Most pixels decoded from one image.
pub const MAX_PIXELS: u64 = 1 << 26;
/// Most entries read from a TIFF image file directory.
pub const MAX_TIFF_IFD_ENTRIES: usize = 1_024;

/// An image file's format, from its first bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Tiff,
    Pdf,
    Psd,
    Svg,
    Other,
}

impl ImageFormat {
    /// Whether [`decode_rgba`] decodes this format.
    pub fn decodable(self) -> bool {
        matches!(self, ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Tiff)
    }

    /// The format a file name's extension suggests (`a.PNG` → [`ImageFormat::Png`]); for file
    /// choosers and messages, never trusted over [`sniff`].
    pub fn from_extension(name: &str) -> ImageFormat {
        let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
        match ext.as_str() {
            "png" => ImageFormat::Png,
            "jpg" | "jpeg" => ImageFormat::Jpeg,
            "tif" | "tiff" => ImageFormat::Tiff,
            "pdf" => ImageFormat::Pdf,
            "psd" => ImageFormat::Psd,
            "svg" => ImageFormat::Svg,
            _ => ImageFormat::Other,
        }
    }
}

/// What [`sniff`] learns from a header. `width` and `height` are 0 for formats whose size is not
/// read (PDF, PSD, SVG, other).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageInfo {
    pub format: ImageFormat,
    pub width: u32,
    pub height: u32,
}

/// A decoded image: `width × height` pixels, rows top to bottom, four bytes each (red, green,
/// blue, alpha, not premultiplied).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rgba8 {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ImageError {
    #[error("the image file is {size} bytes; at most {limit} are read")]
    FileTooLarge { size: usize, limit: usize },
    #[error("the image is {width} × {height} pixels; at most {limit} pixels are decoded")]
    TooManyPixels { width: u32, height: u32, limit: u64 },
    #[error("{0:?} images are not decoded")]
    Unsupported(ImageFormat),
    #[error("the image cannot be read: {0}")]
    Corrupt(String),
    #[error("the image cannot be written: {0}")]
    Encode(String),
    /// A function of this crate whose M11 package has not been written yet.
    #[error("{0} is not available yet")]
    NotYetImplemented(&'static str),
}

/// The format and pixel size of `bytes`, from the header only.
pub fn sniff(bytes: &[u8]) -> Result<ImageInfo, ImageError> {
    // M11.12
    let _ = bytes;
    Err(ImageError::NotYetImplemented("reading image headers"))
}

/// `bytes` decoded to RGBA, refused beyond `max_pixels` (capped at [`MAX_PIXELS`]) or
/// [`MAX_SIDE`] before decoding.
pub fn decode_rgba(bytes: &[u8], max_pixels: u64) -> Result<Rgba8, ImageError> {
    // M11.12
    let _ = (bytes, max_pixels);
    Err(ImageError::NotYetImplemented("decoding images"))
}

/// `image` as a PNG file (TIFF to PNG for sbix, bitmaps in SVG documents).
pub fn to_png(image: &Rgba8) -> Result<Vec<u8>, ImageError> {
    // M11.8
    let _ = image;
    Err(ImageError::NotYetImplemented("writing PNG images"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_suggest_formats_and_placeholders_fail() {
        assert_eq!(ImageFormat::from_extension("sun 48.PNG"), ImageFormat::Png);
        assert_eq!(ImageFormat::from_extension("noext"), ImageFormat::Other);
        assert!(!ImageFormat::Pdf.decodable());
        assert!(matches!(decode_rgba(&[], MAX_PIXELS), Err(_)));
    }
}
