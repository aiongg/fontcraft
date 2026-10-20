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
//! - [`resolution`] reads the pixels per inch a header states, which Add Image uses to place an
//!   image at one point per font unit.
//! - PDF and PSD are recognised and kept as references; they are not decoded here
//!   ([`ImageError::Unsupported`]). `fontcraft-io-pdf`, which builds on this crate, draws a PDF's
//!   first page; a PSD is shown as an unsupported-image marker.
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
///
/// A file this crate does not recognise is [`ImageFormat::Other`], not an error; a recognised
/// header that is cut short or states a zero size is [`ImageError::Corrupt`].
pub fn sniff(bytes: &[u8]) -> Result<ImageInfo, ImageError> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(ImageError::FileTooLarge { size: bytes.len(), limit: MAX_FILE_BYTES });
    }
    let sized = |format, (width, height): (u32, u32)| -> Result<ImageInfo, ImageError> {
        if width == 0 || height == 0 {
            return Err(ImageError::Corrupt(format!("the header states a size of {width} × {height} pixels")));
        }
        Ok(ImageInfo { format, width, height })
    };
    let unsized_ = |format| -> Result<ImageInfo, ImageError> { Ok(ImageInfo { format, width: 0, height: 0 }) };
    if bytes.starts_with(PNG_SIGNATURE) {
        sized(ImageFormat::Png, png_size(bytes)?)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        sized(ImageFormat::Jpeg, jpeg_size(bytes)?)
    } else if let Some(tiff) = Tiff::new(bytes) {
        sized(ImageFormat::Tiff, tiff_size(&tiff)?)
    } else if bytes.starts_with(b"%PDF-") {
        unsized_(ImageFormat::Pdf)
    } else if bytes.starts_with(b"8BPS") {
        unsized_(ImageFormat::Psd)
    } else if looks_like_svg(bytes) {
        unsized_(ImageFormat::Svg)
    } else {
        unsized_(ImageFormat::Other)
    }
}

/// `bytes` decoded to RGBA, refused beyond `max_pixels` (capped at [`MAX_PIXELS`]) or
/// [`MAX_SIDE`] before decoding.
pub fn decode_rgba(bytes: &[u8], max_pixels: u64) -> Result<Rgba8, ImageError> {
    use image::ImageFormat as Codec;
    let info = sniff(bytes)?;
    let codec = match info.format {
        ImageFormat::Png => Codec::Png,
        ImageFormat::Jpeg => Codec::Jpeg,
        ImageFormat::Tiff => Codec::Tiff,
        other => return Err(ImageError::Unsupported(other)),
    };
    let budget = max_pixels.min(MAX_PIXELS);
    check_size(info.width, info.height, budget)?;
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), codec);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    // Room for the widest sample type (16-bit RGBA) and the decoder's working buffers.
    limits.max_alloc = Some(budget.saturating_mul(8).saturating_add(DECODER_SLACK));
    reader.limits(limits);
    let decoded = reader.decode().map_err(|e| match e {
        image::ImageError::Limits(_) => ImageError::TooManyPixels { width: info.width, height: info.height, limit: budget },
        other => ImageError::Corrupt(other.to_string()),
    })?;
    let rgba = decoded.into_rgba8();
    let (width, height) = rgba.dimensions();
    check_size(width, height, budget)?;
    Ok(Rgba8 { width, height, pixels: rgba.into_raw() })
}

/// The resolution the file states, in pixels per inch across and up: PNG `pHYs` in pixels per
/// metre, JPEG JFIF density in dots per inch or centimetre, else its Exif resolution, TIFF
/// `XResolution` and `YResolution`. `None` when the file states none, or none that is usable
/// (an aspect ratio only, zero, or beyond 1 to 100,000 pixels per inch).
pub fn resolution(bytes: &[u8]) -> Option<(f64, f64)> {
    if bytes.len() > MAX_FILE_BYTES {
        return None;
    }
    let found = if bytes.starts_with(PNG_SIGNATURE) {
        png_resolution(bytes)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        jpeg_resolution(bytes)
    } else {
        Tiff::new(bytes).and_then(|t| tiff_resolution(&t))
    };
    let (x, y) = found?;
    let usable = |v: f64| v.is_finite() && (1.0..=100_000.0).contains(&v);
    (usable(x) && usable(y)).then_some((x, y))
}

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
/// Bytes the decoder may allocate beyond the decoded image.
const DECODER_SLACK: u64 = 64 << 20;
/// Most PNG chunks or JPEG segments walked looking for a header field.
const MAX_SEGMENTS: usize = 4_096;
const INCH_PER_METRE: f64 = 0.0254;
const CM_PER_INCH: f64 = 2.54;

fn check_size(width: u32, height: u32, budget: u64) -> Result<(), ImageError> {
    if width == 0 || height == 0 {
        return Err(ImageError::Corrupt(format!("the image is {width} × {height} pixels")));
    }
    if width > MAX_SIDE || height > MAX_SIDE || u64::from(width) * u64::from(height) > budget {
        return Err(ImageError::TooManyPixels { width, height, limit: budget });
    }
    Ok(())
}

fn be16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

fn truncated(what: &str) -> ImageError {
    ImageError::Corrupt(format!("the {what} header is cut short"))
}

// ── PNG ─────────────────────────────────────────────────────────────────────────────────────────

fn png_size(b: &[u8]) -> Result<(u32, u32), ImageError> {
    if b.get(12..16) != Some(b"IHDR".as_slice()) {
        return Err(ImageError::Corrupt("the PNG file does not start with its IHDR chunk".into()));
    }
    let width = be32(b, 16).ok_or_else(|| truncated("PNG"))?;
    let height = be32(b, 20).ok_or_else(|| truncated("PNG"))?;
    Ok((width, height))
}

/// The chunks of a PNG file before its image data: `(type, data)`.
fn png_chunks(b: &[u8]) -> impl Iterator<Item = (&[u8], &[u8])> {
    let mut at = PNG_SIGNATURE.len();
    std::iter::from_fn(move || {
        let len = usize::try_from(be32(b, at)?).ok()?;
        let kind = b.get(at.checked_add(4)?..at.checked_add(8)?)?;
        let data = b.get(at + 8..(at + 8).checked_add(len)?)?;
        // Length, type, data and CRC.
        at = at.checked_add(12)?.checked_add(len)?;
        Some((kind, data))
    })
    .take(MAX_SEGMENTS)
    .take_while(|(kind, _)| *kind != b"IDAT" && *kind != b"IEND")
}

fn png_resolution(b: &[u8]) -> Option<(f64, f64)> {
    let (_, data) = png_chunks(b).find(|(kind, _)| *kind == b"pHYs")?;
    // Unit 1 is the metre; 0 states an aspect ratio only.
    if data.get(8) != Some(&1) {
        return None;
    }
    Some((f64::from(be32(data, 0)?) * INCH_PER_METRE, f64::from(be32(data, 4)?) * INCH_PER_METRE))
}

// ── JPEG ────────────────────────────────────────────────────────────────────────────────────────

/// The segments of a JPEG file up to its first frame header: `(marker, data)`, where `data`
/// follows the length field. A frame header ends the walk; so does the start of the scan data.
fn jpeg_segments(b: &[u8]) -> impl Iterator<Item = (u8, &[u8])> {
    let mut at = 2;
    let mut done = false;
    std::iter::from_fn(move || {
        if done {
            return None;
        }
        // Markers may be padded with any number of 0xFF bytes.
        while b.get(at) == Some(&0xFF) && b.get(at + 1) == Some(&0xFF) {
            at += 1;
        }
        if b.get(at) != Some(&0xFF) {
            return None;
        }
        let marker = *b.get(at + 1)?;
        if matches!(marker, 0x01 | 0xD0..=0xD8) {
            at += 2;
            return Some((marker, &[][..]));
        }
        if matches!(marker, 0xD9 | 0xDA) {
            done = true;
            return Some((marker, &[][..]));
        }
        let len = usize::from(be16(b, at + 2)?);
        if len < 2 {
            return None;
        }
        let data = b.get(at + 4..(at + 2).checked_add(len)?)?;
        at = at.checked_add(2 + len)?;
        if is_frame_header(marker) {
            done = true;
        }
        Some((marker, data))
    })
    .take(MAX_SEGMENTS)
}

/// SOF0 to SOF15, except DHT (C4), JPG (C8) and DAC (CC).
fn is_frame_header(marker: u8) -> bool {
    matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC)
}

fn jpeg_size(b: &[u8]) -> Result<(u32, u32), ImageError> {
    let (_, data) = jpeg_segments(b)
        .find(|(marker, _)| is_frame_header(*marker))
        .ok_or_else(|| ImageError::Corrupt("the JPEG file has no frame header before its image data".into()))?;
    // Precision, then height and width.
    let height = be16(data, 1).ok_or_else(|| truncated("JPEG frame"))?;
    let width = be16(data, 3).ok_or_else(|| truncated("JPEG frame"))?;
    Ok((u32::from(width), u32::from(height)))
}

fn jpeg_resolution(b: &[u8]) -> Option<(f64, f64)> {
    let mut exif = None;
    for (marker, data) in jpeg_segments(b) {
        if marker == 0xE0 && data.starts_with(b"JFIF\0") {
            let (x, y) = (f64::from(be16(data, 8)?), f64::from(be16(data, 10)?));
            match data.get(7) {
                Some(1) => return Some((x, y)),
                Some(2) => return Some((x * CM_PER_INCH, y * CM_PER_INCH)),
                _ => {}
            }
        } else if marker == 0xE1 && exif.is_none() {
            exif = data.strip_prefix(b"Exif\0\0".as_slice());
        }
    }
    tiff_resolution(&Tiff::new(exif?)?)
}

// ── TIFF (and the Exif data inside JPEG files) ──────────────────────────────────────────────────

const TAG_WIDTH: u16 = 256;
const TAG_HEIGHT: u16 = 257;
const TAG_X_RESOLUTION: u16 = 282;
const TAG_Y_RESOLUTION: u16 = 283;
const TAG_RESOLUTION_UNIT: u16 = 296;

/// A classic or BigTIFF file's header.
struct Tiff<'a> {
    b: &'a [u8],
    little: bool,
    big: bool,
}

/// One directory entry: its tag, field type, count and where its value field starts.
struct Entry {
    tag: u16,
    kind: u16,
    count: u64,
    value_at: usize,
}

impl<'a> Tiff<'a> {
    fn new(b: &'a [u8]) -> Option<Self> {
        let little = match b.get(..2)? {
            b"II" => true,
            b"MM" => false,
            _ => return None,
        };
        let mut t = Self { b, little, big: false };
        match t.u16(2)? {
            42 => {}
            43 => t.big = true,
            _ => return None,
        }
        Some(t)
    }

    fn u16(&self, at: usize) -> Option<u16> {
        let a: [u8; 2] = self.b.get(at..at.checked_add(2)?)?.try_into().ok()?;
        Some(if self.little { u16::from_le_bytes(a) } else { u16::from_be_bytes(a) })
    }

    fn u32(&self, at: usize) -> Option<u32> {
        let a: [u8; 4] = self.b.get(at..at.checked_add(4)?)?.try_into().ok()?;
        Some(if self.little { u32::from_le_bytes(a) } else { u32::from_be_bytes(a) })
    }

    fn u64(&self, at: usize) -> Option<u64> {
        let a: [u8; 8] = self.b.get(at..at.checked_add(8)?)?.try_into().ok()?;
        Some(if self.little { u64::from_le_bytes(a) } else { u64::from_be_bytes(a) })
    }

    /// An offset field: four bytes in a classic file, eight in BigTIFF.
    fn offset(&self, at: usize) -> Option<usize> {
        let o = if self.big { self.u64(at)? } else { u64::from(self.u32(at)?) };
        usize::try_from(o).ok()
    }

    /// The entries of the first image file directory, at most [`MAX_TIFF_IFD_ENTRIES`].
    fn first_directory(&self) -> Result<Vec<Entry>, ImageError> {
        let cut = || truncated("TIFF");
        let ifd = self.offset(if self.big { 8 } else { 4 }).ok_or_else(cut)?;
        let (count, first, size) = if self.big {
            (self.u64(ifd).ok_or_else(cut)?, ifd.checked_add(8).ok_or_else(cut)?, 20)
        } else {
            (u64::from(self.u16(ifd).ok_or_else(cut)?), ifd.checked_add(2).ok_or_else(cut)?, 12)
        };
        let count = usize::try_from(count).unwrap_or(usize::MAX);
        if count > MAX_TIFF_IFD_ENTRIES {
            return Err(ImageError::Corrupt(format!("the TIFF directory has {count} entries; at most {MAX_TIFF_IFD_ENTRIES} are read")));
        }
        (0..count)
            .map(|i| {
                // The whole entry must be in the file, which bounds every offset below.
                let at = first.checked_add(i * size).filter(|at| at.saturating_add(size) <= self.b.len()).ok_or_else(cut)?;
                let tag = self.u16(at).ok_or_else(cut)?;
                let kind = self.u16(at + 2).ok_or_else(cut)?;
                let (count, value_at) = if self.big { (self.u64(at + 4), at + 12) } else { (self.u32(at + 4).map(u64::from), at + 8) };
                Ok(Entry { tag, kind, count: count.ok_or_else(cut)?, value_at })
            })
            .collect()
    }

    /// A SHORT, LONG or LONG8 value.
    fn uint(&self, e: &Entry) -> Option<u64> {
        match e.kind {
            3 => self.u16(e.value_at).map(u64::from),
            4 => self.u32(e.value_at).map(u64::from),
            16 => self.u64(e.value_at),
            _ => None,
        }
    }

    /// A RATIONAL value: stored inline in BigTIFF, at an offset in a classic file.
    fn rational(&self, e: &Entry) -> Option<f64> {
        if e.kind != 5 || e.count == 0 {
            return None;
        }
        let at = if self.big { e.value_at } else { self.offset(e.value_at)? };
        let (num, den) = (self.u32(at)?, self.u32(at.checked_add(4)?)?);
        (den != 0).then_some(f64::from(num) / f64::from(den))
    }
}

fn tiff_size(t: &Tiff) -> Result<(u32, u32), ImageError> {
    let entries = t.first_directory()?;
    let get = |tag: u16, name: &str| -> Result<u32, ImageError> {
        let e = entries.iter().find(|e| e.tag == tag).ok_or_else(|| ImageError::Corrupt(format!("the TIFF image states no {name}")))?;
        let v = t.uint(e).ok_or_else(|| ImageError::Corrupt(format!("the TIFF image's {name} cannot be read")))?;
        // Beyond u32 is beyond every limit; report it as the largest size.
        Ok(u32::try_from(v).unwrap_or(u32::MAX))
    };
    Ok((get(TAG_WIDTH, "width")?, get(TAG_HEIGHT, "height")?))
}

fn tiff_resolution(t: &Tiff) -> Option<(f64, f64)> {
    let entries = t.first_directory().ok()?;
    let find = |tag: u16| entries.iter().find(|e| e.tag == tag);
    let x = t.rational(find(TAG_X_RESOLUTION)?)?;
    let y = find(TAG_Y_RESOLUTION).and_then(|e| t.rational(e)).unwrap_or(x);
    // Unit 2 (the default) is the inch, 3 the centimetre, 1 states an aspect ratio only.
    match find(TAG_RESOLUTION_UNIT).and_then(|e| t.uint(e)).unwrap_or(2) {
        2 => Some((x, y)),
        3 => Some((x * CM_PER_INCH, y * CM_PER_INCH)),
        _ => None,
    }
}

// ── SVG ─────────────────────────────────────────────────────────────────────────────────────────

/// Whether the file's first kilobyte holds an `<svg` element after an optional XML declaration,
/// doctype or comments.
fn looks_like_svg(b: &[u8]) -> bool {
    let head = b.get(..b.len().min(1024)).unwrap_or_default();
    let head = head.strip_prefix(b"\xEF\xBB\xBF".as_slice()).unwrap_or(head);
    let text = String::from_utf8_lossy(head);
    let text = text.trim_start();
    text.starts_with('<') && text.contains("<svg")
}

/// `image` as a PNG file (TIFF to PNG for sbix, bitmaps in SVG documents). Refused when the
/// pixel buffer does not hold `width × height` RGBA pixels or the size is beyond [`MAX_SIDE`]
/// and [`MAX_PIXELS`].
pub fn to_png(image: &Rgba8) -> Result<Vec<u8>, ImageError> {
    use image::ImageEncoder;
    check_size(image.width, image.height, MAX_PIXELS)?;
    let expected = u64::from(image.width) * u64::from(image.height) * 4;
    if u64::try_from(image.pixels.len()).ok() != Some(expected) {
        return Err(ImageError::Encode(format!(
            "{} bytes of pixels for a {} × {} image, which needs {expected}",
            image.pixels.len(),
            image.width,
            image.height
        )));
    }
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(&image.pixels, image.width, image.height, image::ExtendedColorType::Rgba8)
        .map_err(|e| ImageError::Encode(e.to_string()))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, ImageFormat as Codec, RgbImage, RgbaImage};

    fn encode(image: DynamicImage, codec: Codec) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        image.write_to(&mut out, codec).unwrap();
        out.into_inner()
    }

    /// A 3 × 2 image with every pixel different, alpha included.
    fn sample() -> RgbaImage {
        RgbaImage::from_fn(3, 2, |x, y| image::Rgba([x as u8 * 80, y as u8 * 120, 7, 255 - x as u8 * 50]))
    }

    /// A little-endian TIFF header and first directory with `entries` as (tag, type, count,
    /// value), each value inline; rationals in `tail` are reached by offsets the caller computes.
    fn tiff(entries: &[(u16, u16, u32, u32)], tail: &[u8]) -> Vec<u8> {
        let mut b = b"II*\0".to_vec();
        b.extend(8u32.to_le_bytes());
        b.extend((entries.len() as u16).to_le_bytes());
        for &(tag, kind, count, value) in entries {
            b.extend(tag.to_le_bytes());
            b.extend(kind.to_le_bytes());
            b.extend(count.to_le_bytes());
            b.extend(value.to_le_bytes());
        }
        b.extend(0u32.to_le_bytes());
        b.extend(tail);
        b
    }

    /// Where `tiff` puts the tail for `n` entries.
    fn tiff_tail_at(n: usize) -> u32 {
        (8 + 2 + 12 * n + 4) as u32
    }

    #[test]
    fn png_jpeg_and_tiff_are_sniffed_and_decoded() {
        let png = encode(DynamicImage::ImageRgba8(sample()), Codec::Png);
        assert_eq!(sniff(&png).unwrap(), ImageInfo { format: ImageFormat::Png, width: 3, height: 2 });
        let decoded = decode_rgba(&png, MAX_PIXELS).unwrap();
        assert_eq!((decoded.width, decoded.height), (3, 2));
        assert_eq!(decoded.pixels, sample().into_raw());

        let tif = encode(DynamicImage::ImageRgba8(sample()), Codec::Tiff);
        assert_eq!(sniff(&tif).unwrap(), ImageInfo { format: ImageFormat::Tiff, width: 3, height: 2 });
        assert_eq!(decode_rgba(&tif, MAX_PIXELS).unwrap().pixels, sample().into_raw());

        let jpg = encode(DynamicImage::ImageRgb8(RgbImage::from_pixel(16, 8, image::Rgb([200, 40, 40]))), Codec::Jpeg);
        assert_eq!(sniff(&jpg).unwrap(), ImageInfo { format: ImageFormat::Jpeg, width: 16, height: 8 });
        let decoded = decode_rgba(&jpg, MAX_PIXELS).unwrap();
        assert_eq!((decoded.width, decoded.height, decoded.pixels.len()), (16, 8, 16 * 8 * 4));
        assert_eq!(decoded.pixels.get(3), Some(&255), "JPEG decodes opaque");

        // The caller's pixel budget is checked before decoding.
        assert_eq!(decode_rgba(&png, 5), Err(ImageError::TooManyPixels { width: 3, height: 2, limit: 5 }));
    }

    #[test]
    fn tiff_converts_to_png_with_the_same_pixels() {
        let tif = encode(DynamicImage::ImageRgba8(sample()), Codec::Tiff);
        let png = to_png(&decode_rgba(&tif, MAX_PIXELS).unwrap()).unwrap();
        assert_eq!(sniff(&png).unwrap(), ImageInfo { format: ImageFormat::Png, width: 3, height: 2 });
        assert_eq!(decode_rgba(&png, MAX_PIXELS).unwrap().pixels, sample().into_raw());

        // A buffer that does not match its stated size is refused, not encoded.
        let short = Rgba8 { width: 3, height: 2, pixels: vec![0; 20] };
        assert!(matches!(to_png(&short), Err(ImageError::Encode(_))));
        let huge = Rgba8 { width: MAX_SIDE + 1, height: 1, pixels: Vec::new() };
        assert!(matches!(to_png(&huge), Err(ImageError::TooManyPixels { .. })));
    }

    #[test]
    fn references_are_recognised_but_not_decoded() {
        for (bytes, format) in [
            (&b"%PDF-1.7\n"[..], ImageFormat::Pdf),
            (&b"8BPS\0\x01"[..], ImageFormat::Psd),
            (&b"\xEF\xBB\xBF<?xml version=\"1.0\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\"/>"[..], ImageFormat::Svg),
            (&b"GIF89a"[..], ImageFormat::Other),
            (&b""[..], ImageFormat::Other),
        ] {
            assert_eq!(sniff(bytes).unwrap(), ImageInfo { format, width: 0, height: 0 });
            assert_eq!(decode_rgba(bytes, MAX_PIXELS), Err(ImageError::Unsupported(format)));
        }
        assert_eq!(ImageFormat::from_extension("sun 48.PNG"), ImageFormat::Png);
        assert_eq!(ImageFormat::from_extension("noext"), ImageFormat::Other);
        assert!(!ImageFormat::Pdf.decodable());
    }

    #[test]
    fn hostile_files_fail_before_decoding() {
        // A PNG whose header claims 100,000 × 100,000 pixels and holds no image data.
        let mut huge = PNG_SIGNATURE.to_vec();
        huge.extend(13u32.to_be_bytes());
        huge.extend(b"IHDR");
        huge.extend(100_000u32.to_be_bytes());
        huge.extend(100_000u32.to_be_bytes());
        huge.extend([8, 6, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(sniff(&huge).unwrap(), ImageInfo { format: ImageFormat::Png, width: 100_000, height: 100_000 });
        assert_eq!(decode_rgba(&huge, MAX_PIXELS), Err(ImageError::TooManyPixels { width: 100_000, height: 100_000, limit: MAX_PIXELS }));
        // Within the side limit, beyond the pixel limit.
        let wide = tiff(&[(TAG_WIDTH, 4, 1, 10_000), (TAG_HEIGHT, 4, 1, 10_000)], &[]);
        assert!(matches!(decode_rgba(&wide, MAX_PIXELS), Err(ImageError::TooManyPixels { .. })));

        // A TIFF bomb: a directory claiming 65,535 entries.
        let mut bomb = b"II*\0".to_vec();
        bomb.extend(8u32.to_le_bytes());
        bomb.extend(u16::MAX.to_le_bytes());
        assert!(matches!(sniff(&bomb), Err(ImageError::Corrupt(_))));
        // A directory offset beyond the file, and one at the end of the address space.
        let mut far = b"MM\0*".to_vec();
        far.extend(u32::MAX.to_be_bytes());
        assert!(matches!(sniff(&far), Err(ImageError::Corrupt(_))));
        let mut big = b"II+\0\x08\0\0\0".to_vec();
        big.extend(u64::MAX.to_le_bytes());
        assert!(matches!(sniff(&big), Err(ImageError::Corrupt(_))));

        // A truncated JPEG: cut before its frame header, and cut in the middle of its data.
        let jpg = encode(DynamicImage::ImageRgb8(RgbImage::from_fn(64, 64, |x, y| image::Rgb([x as u8 * 4, y as u8 * 4, 9]))), Codec::Jpeg);
        assert!(matches!(sniff(&jpg[..4]), Err(ImageError::Corrupt(_))));
        match decode_rgba(&jpg[..jpg.len() / 2], MAX_PIXELS) {
            Err(ImageError::Corrupt(_)) => {}
            Ok(partial) => assert_eq!((partial.width, partial.height), (64, 64)),
            other => panic!("truncated JPEG: {other:?}"),
        }
        // A cut PNG header, and a zero size.
        assert!(matches!(sniff(&huge[..18]), Err(ImageError::Corrupt(_))));
        let zero = tiff(&[(TAG_WIDTH, 3, 1, 0), (TAG_HEIGHT, 3, 1, 5)], &[]);
        assert!(matches!(sniff(&zero), Err(ImageError::Corrupt(_))));
        // Every prefix of a valid file is refused or read, never a panic.
        let png = encode(DynamicImage::ImageRgba8(sample()), Codec::Png);
        for cut in 0..png.len() {
            let _ = decode_rgba(&png[..cut], MAX_PIXELS);
            let _ = resolution(&png[..cut]);
        }
    }

    #[test]
    fn resolution_comes_from_png_jpeg_and_tiff_headers() {
        // PNG pHYs: 5,669 pixels per metre is 144 pixels per inch.
        let png = encode(DynamicImage::ImageRgba8(sample()), Codec::Png);
        assert_eq!(resolution(&png), None, "no pHYs chunk");
        let mut phys = png.get(..33).unwrap().to_vec();
        phys.extend(9u32.to_be_bytes());
        phys.extend(b"pHYs");
        phys.extend(5_669u32.to_be_bytes());
        phys.extend(5_669u32.to_be_bytes());
        phys.extend([1, 0, 0, 0, 0]);
        phys.extend(png.get(33..).unwrap());
        let (x, y) = resolution(&phys).unwrap();
        assert!((x - 144.0).abs() < 0.01 && (y - 144.0).abs() < 0.01, "{x} {y}");
        assert_eq!(sniff(&phys).unwrap().width, 3);

        // JPEG JFIF in dots per inch, then in dots per centimetre.
        let jfif = |units: u8, density: u16| {
            let mut b = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 16];
            b.extend(b"JFIF\0\x01\x02");
            b.push(units);
            b.extend(density.to_be_bytes());
            b.extend(density.to_be_bytes());
            b.extend([0, 0]);
            b.extend([0xFF, 0xC0, 0, 11, 8, 0, 20, 0, 10, 1, 1, 0x11, 0]);
            b
        };
        assert_eq!(resolution(&jfif(1, 300)), Some((300.0, 300.0)));
        assert_eq!(resolution(&jfif(2, 100)), Some((254.0, 254.0)));
        assert_eq!(resolution(&jfif(0, 1)), None, "an aspect ratio only");
        assert_eq!(sniff(&jfif(1, 300)).unwrap(), ImageInfo { format: ImageFormat::Jpeg, width: 10, height: 20 });

        // TIFF: 144/1 by 72/1 pixels per inch; then the same in centimetres.
        let n = 5;
        let at = tiff_tail_at(n);
        let mut rationals = Vec::new();
        for v in [144u32, 1, 72, 1] {
            rationals.extend(v.to_le_bytes());
        }
        let file = |unit: u32| {
            tiff(
                &[
                    (TAG_WIDTH, 3, 1, 4),
                    (TAG_HEIGHT, 3, 1, 3),
                    (TAG_X_RESOLUTION, 5, 1, at),
                    (TAG_Y_RESOLUTION, 5, 1, at + 8),
                    (TAG_RESOLUTION_UNIT, 3, 1, unit),
                ],
                &rationals,
            )
        };
        assert_eq!(resolution(&file(2)), Some((144.0, 72.0)));
        assert_eq!(resolution(&file(3)), Some((144.0 * 2.54, 72.0 * 2.54)));
        assert_eq!(resolution(&file(1)), None);
        assert_eq!(sniff(&file(2)).unwrap(), ImageInfo { format: ImageFormat::Tiff, width: 4, height: 3 });
    }

    /// The decoding speed target of the M11 plan (§ 5): a 1024 × 1024 PNG in under 20 ms. Run
    /// with `cargo test --release -p fontcraft-imagery -- --ignored decode_speed`.
    #[test]
    #[ignore = "a measure, run in release"]
    fn decode_speed_of_a_1024_png() {
        let img = RgbaImage::from_fn(1024, 1024, |x, y| image::Rgba([(x ^ y) as u8, (x * 3) as u8, (y * 5) as u8, 255]));
        let png = encode(DynamicImage::ImageRgba8(img), Codec::Png);
        let runs = 10;
        let start = std::time::Instant::now();
        for _ in 0..runs {
            decode_rgba(&png, MAX_PIXELS).unwrap();
        }
        let each = start.elapsed() / runs;
        println!("decoding a 1024 × 1024 PNG: {each:?}");
        assert!(each.as_millis() < 20, "{each:?}");
    }
}
