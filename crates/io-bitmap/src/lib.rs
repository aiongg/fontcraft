//! Bitmap fonts (L3): reading and writing BDF text files and X11 PCF binary files.
//!
//! Written from public format descriptions only: Adobe's Glyph Bitmap Distribution Format
//! specification (technical note 5005, versions 2.1 and 2.2), the PCF format page of an
//! open-source font editor's public technical reference, and the files the X.Org `bdftopcf`
//! converter writes from BDF files of our own. No code that reads or writes either format was
//! consulted.
//!
//! Both formats hold one strike: the glyphs of one font at one pixel size. A file reads into a
//! [`BitmapFont`], which keeps what the strike needs (pixel size, ascent, descent, each glyph's
//! name, code, advance and pixels) and the font's X properties as they are.
//!
//! | BDF | PCF | [`BitmapFont`] |
//! |---|---|---|
//! | `FONT` | `FONT` property | [`BitmapFont::name`] |
//! | `SIZE` | `POINT_SIZE`, `RESOLUTION_X`, `RESOLUTION_Y` properties | point size, resolution, depth |
//! | `PIXEL_SIZE` property | `PIXEL_SIZE` property | [`BitmapFont::pixel_size`] |
//! | `FONT_ASCENT`, `FONT_DESCENT` properties | accelerator table, else the same properties | ascent, descent |
//! | `STARTPROPERTIES` … `ENDPROPERTIES` | properties table | [`BitmapFont::properties`] |
//! | `STARTCHAR` name | glyph names table | [`BitmapGlyph::name`] |
//! | `ENCODING` | encodings table | [`BitmapGlyph::encoding`] |
//! | `DWIDTH`, `SWIDTH` | metrics and scalable widths tables | advance, scalable width |
//! | `BBX` and `BITMAP` | metrics and bitmaps tables | [`Bitmap`] |
//!
//! Not read: vertical metrics (`DWIDTH1`, `SWIDTH1`, `VVECTOR`), PCF ink metrics, and the
//! second, non-standard code of `ENCODING -1 n`. Gzip-compressed PCF files (`.pcf.gz`) are
//! refused with a message; the user decompresses them first.
//!
//! The input is untrusted: the file size, the glyph and property counts, each bitmap's size and
//! the decoded pixels of all glyphs together are capped by the limits below, every PCF offset is
//! checked against its table, and both readers are flat loops without recursion.
#![forbid(unsafe_code)]

mod bdf;
mod pcf;
#[cfg(test)]
mod tests;

pub use bdf::{read_bdf, write_bdf};
pub use pcf::{PcfLayout, read_pcf, write_pcf};

/// Largest file either reader accepts.
pub const MAX_FILE_BYTES: usize = 16 << 20;
/// Most glyphs in one font.
pub const MAX_GLYPHS: usize = 65_536;
/// Widest and tallest bitmap, in pixels.
pub const MAX_BITMAP_SIDE: u16 = 1_024;
/// Most bytes the decoded bitmaps of one font take together. PCF glyphs may share bitmap data,
/// so this cap is what bounds the work, not the file size.
pub const MAX_TOTAL_BITMAP_BYTES: usize = 64 << 20;
/// Most properties in one font.
pub const MAX_PROPERTIES: usize = 4_096;
/// Longest glyph name, property name or property string, in bytes.
pub const MAX_STRING_BYTES: usize = 65_536;
/// Largest absolute value of a coordinate, offset, advance or width.
pub const MAX_COORDINATE: i32 = 32_767;
/// Most warnings kept; the rest are counted in the last one.
pub const MAX_WARNINGS: usize = 100;

#[derive(Debug, thiserror::Error)]
pub enum BitmapError {
    #[error("the file is too large ({0} bytes; the limit is {MAX_FILE_BYTES})")]
    TooLarge(usize),
    #[error("not a BDF file: {0}")]
    NotBdf(String),
    #[error("not a PCF file: {0}")]
    NotPcf(String),
    #[error("BDF line {line}: {message}")]
    Bdf { line: usize, message: String },
    #[error("PCF {table} table: {message}")]
    Pcf { table: &'static str, message: String },
    #[error("{what}: more than the limit of {limit}")]
    Limit { what: &'static str, limit: usize },
    #[error("cannot write the font: {0}")]
    Write(String),
}

pub type Result<T> = std::result::Result<T, BitmapError>;

/// A file read into a font, with what the reader passed over.
#[derive(Clone, Debug, PartialEq)]
pub struct Import {
    pub font: BitmapFont,
    /// One sentence per problem the reader worked around, at most [`MAX_WARNINGS`].
    pub warnings: Vec<String>,
}

/// One strike: the glyphs of a font at one pixel size.
#[derive(Clone, Debug, PartialEq)]
pub struct BitmapFont {
    /// The font's name, usually an X logical font description.
    pub name: String,
    /// Pixels per em: the size this strike is for.
    pub pixel_size: u16,
    /// The size in points the font states (BDF `SIZE`; PCF `POINT_SIZE` ÷ 10).
    pub point_size: f64,
    /// Horizontal and vertical resolution, in dots per inch.
    pub resolution: (u32, u32),
    /// Bits per pixel: 1 for black and white, 2, 4 or 8 for grey. PCF holds depth 1 only.
    pub depth: u8,
    /// Pixels above the baseline the font reserves for its lines.
    pub ascent: i32,
    /// Pixels below the baseline the font reserves for its lines.
    pub descent: i32,
    /// The X font properties as the file states them, in file order.
    pub properties: Vec<Property>,
    pub glyphs: Vec<BitmapGlyph>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Property {
    pub name: String,
    pub value: PropertyValue,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PropertyValue {
    Integer(i64),
    Text(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct BitmapGlyph {
    /// The glyph's name; `glyph<index>` when a PCF file has no names table.
    pub name: String,
    /// The glyph's code in the font's encoding (see [`BitmapFont::codes_are_unicode`]); `None`
    /// for a glyph outside it.
    pub encoding: Option<u32>,
    /// Horizontal advance, in pixels.
    pub advance: i32,
    /// Advance in thousandths of the point size, when the file states it.
    pub scalable_width: Option<i32>,
    pub bitmap: Bitmap,
}

/// A glyph's pixels and where they sit.
#[derive(Clone, Debug, PartialEq)]
pub struct Bitmap {
    pub width: u16,
    pub height: u16,
    /// Pixels from the glyph origin to the bitmap's left edge.
    pub left: i32,
    /// Pixels from the baseline up to the bitmap's bottom edge (negative below the baseline).
    pub bottom: i32,
    /// Bits per pixel: 1, 2, 4 or 8.
    pub depth: u8,
    /// Rows from top to bottom, [`Bitmap::stride`] bytes each, pixels packed from the most
    /// significant bit.
    pub data: Vec<u8>,
}

impl Bitmap {
    /// An empty (all zero) bitmap, or `None` when a side is above [`MAX_BITMAP_SIDE`] or the
    /// depth is not 1, 2, 4 or 8.
    pub fn blank(width: u16, height: u16, left: i32, bottom: i32, depth: u8) -> Option<Bitmap> {
        if width > MAX_BITMAP_SIDE || height > MAX_BITMAP_SIDE || !valid_depth(depth) {
            return None;
        }
        let len = stride(width, depth).checked_mul(usize::from(height))?;
        Some(Bitmap { width, height, left, bottom, depth, data: vec![0; len] })
    }

    /// Bytes per row.
    pub fn stride(&self) -> usize {
        stride(self.width, self.depth)
    }

    /// The value of the pixel at column `x`, row `y` (0 is the top row); 0 outside the bitmap.
    pub fn pixel(&self, x: u16, y: u16) -> u8 {
        let Some((index, shift, mask)) = self.locate(x, y) else {
            return 0;
        };
        self.data.get(index).map_or(0, |b| (b >> shift) & mask)
    }

    /// Sets a pixel; values above the depth's maximum are clipped to it. Does nothing outside
    /// the bitmap.
    pub fn set_pixel(&mut self, x: u16, y: u16, value: u8) {
        let Some((index, shift, mask)) = self.locate(x, y) else {
            return;
        };
        if let Some(b) = self.data.get_mut(index) {
            *b = (*b & !(mask << shift)) | (value.min(mask) << shift);
        }
    }

    fn locate(&self, x: u16, y: u16) -> Option<(usize, u32, u8)> {
        if x >= self.width || y >= self.height || !valid_depth(self.depth) {
            return None;
        }
        let bit = usize::from(x) * usize::from(self.depth);
        let index = usize::from(y) * self.stride() + bit / 8;
        let shift = 8 - u32::from(self.depth) - (bit % 8) as u32;
        let mask = ((1u16 << self.depth) - 1) as u8;
        Some((index, shift, mask))
    }
}

impl BitmapFont {
    /// The value of an integer property.
    pub fn integer_property(&self, name: &str) -> Option<i64> {
        self.properties.iter().find(|p| p.name == name).and_then(|p| match p.value {
            PropertyValue::Integer(v) => Some(v),
            PropertyValue::Text(_) => None,
        })
    }

    /// The value of a text property.
    pub fn text_property(&self, name: &str) -> Option<&str> {
        self.properties.iter().find(|p| p.name == name).and_then(|p| match &p.value {
            PropertyValue::Text(v) => Some(v.as_str()),
            PropertyValue::Integer(_) => None,
        })
    }

    /// Whether glyph codes are Unicode code points: the font's `CHARSET_REGISTRY` is
    /// `ISO10646`, or `ISO8859` with `CHARSET_ENCODING` `1` (whose codes are the first 256 code
    /// points), or the font names no registry. Codes of any other registry match glyphs by name
    /// only.
    pub fn codes_are_unicode(&self) -> bool {
        let Some(registry) = self.text_property("CHARSET_REGISTRY") else {
            return true;
        };
        let encoding = self.text_property("CHARSET_ENCODING").unwrap_or("");
        registry.eq_ignore_ascii_case("ISO10646") || (registry.eq_ignore_ascii_case("ISO8859") && encoding == "1")
    }
}

fn valid_depth(depth: u8) -> bool {
    matches!(depth, 1 | 2 | 4 | 8)
}

fn stride(width: u16, depth: u8) -> usize {
    (usize::from(width) * usize::from(depth)).div_ceil(8)
}

/// Whether `bytes` start like a BDF file.
pub fn is_bdf(bytes: &[u8]) -> bool {
    let b = bytes.strip_prefix(b"\xEF\xBB\xBF".as_slice()).unwrap_or(bytes);
    b.trim_ascii_start().starts_with(b"STARTFONT")
}

/// Whether `bytes` start like a PCF file.
pub fn is_pcf(bytes: &[u8]) -> bool {
    bytes.starts_with(pcf::MAGIC)
}

/// Reads a BDF or PCF file, whichever `bytes` hold.
pub fn read(bytes: &[u8]) -> Result<Import> {
    if is_pcf(bytes) {
        read_pcf(bytes)
    } else if is_bdf(bytes) {
        read_bdf(bytes)
    } else if bytes.starts_with(&[0x1f, 0x8b]) {
        Err(BitmapError::NotPcf("the file is gzip-compressed; decompress it first".into()))
    } else {
        Err(BitmapError::NotBdf("neither a BDF nor a PCF file".into()))
    }
}

/// The pixel size a font states: its `PIXEL_SIZE` property, else its point size at its vertical
/// resolution, else its ascent plus descent; at least 1.
fn pixel_size(properties: &[Property], point_size: f64, y_resolution: u32, ascent: i32, descent: i32) -> u16 {
    let property = properties.iter().find(|p| p.name == "PIXEL_SIZE").and_then(|p| match p.value {
        PropertyValue::Integer(v) => u16::try_from(v).ok().filter(|v| *v > 0),
        PropertyValue::Text(_) => None,
    });
    if let Some(size) = property {
        return size;
    }
    let from_points = (point_size * f64::from(y_resolution) / 72.0).round();
    if from_points.is_finite() && from_points >= 1.0 && from_points <= f64::from(u16::MAX) {
        return from_points as u16;
    }
    let lines = i64::from(ascent) + i64::from(descent);
    u16::try_from(lines.clamp(1, i64::from(u16::MAX))).unwrap_or(1)
}

/// Collects warnings, keeping at most [`MAX_WARNINGS`].
#[derive(Default)]
struct Warnings {
    list: Vec<String>,
    dropped: usize,
}

impl Warnings {
    fn push(&mut self, message: String) {
        if self.list.len() < MAX_WARNINGS - 1 {
            self.list.push(message);
        } else {
            self.dropped += 1;
        }
    }

    fn finish(mut self) -> Vec<String> {
        if self.dropped > 0 {
            self.list.push(format!("{} more warnings", self.dropped));
        }
        self.list
    }
}

/// Counts decoded bitmap bytes against [`MAX_TOTAL_BITMAP_BYTES`].
#[derive(Default)]
struct PixelBudget(usize);

impl PixelBudget {
    fn spend(&mut self, bytes: usize) -> Result<()> {
        self.0 = self.0.saturating_add(bytes);
        if self.0 > MAX_TOTAL_BITMAP_BYTES {
            return Err(BitmapError::Limit { what: "decoded bitmap bytes", limit: MAX_TOTAL_BITMAP_BYTES });
        }
        Ok(())
    }
}
