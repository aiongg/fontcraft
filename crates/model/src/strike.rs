//! Bitmap strikes: hand-made pixel images of the glyphs at one size, as a BDF or PCF file holds
//! them and as the `bitmap.*` commands keep them in a font.
//!
//! A strike is named by its size in pixels per em and its bits per pixel; a font holds at most one
//! strike of each pair. Each strike glyph names the font glyph it belongs to; it stays in the
//! strike when that glyph is deleted, and follows it when it is renamed. The X font properties a
//! file stated are kept as they were, so a strike imported from a BDF file is written back with
//! them.
//!
//! The limits match those of the bitmap file readers (`fontcraft-io-bitmap`), so whatever they
//! read fits a strike.

use std::collections::HashSet;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{MAX_GLYPHS, ModelError, check_name};

/// Most strikes in one font.
pub const MAX_STRIKES: usize = 64;
/// Widest and tallest strike bitmap, in pixels.
pub const MAX_STRIKE_BITMAP_SIDE: u16 = 1_024;
/// Most bytes the bitmaps of one strike take together.
pub const MAX_STRIKE_BYTES: usize = 64 << 20;
/// Most bytes the bitmaps of all strikes of a font take together.
pub const MAX_ALL_STRIKES_BYTES: usize = 256 << 20;
/// Most properties in one strike.
pub const MAX_STRIKE_PROPERTIES: usize = 4_096;
/// Longest strike name, property name or property text, in bytes.
pub const MAX_STRIKE_STRING: usize = 65_536;
/// Largest absolute value of a strike's ascent, descent, or a glyph's advance or offset, in
/// pixels.
pub const MAX_STRIKE_COORDINATE: i32 = 32_767;

/// The glyphs of a font drawn at one pixel size.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BitmapStrike {
    /// Pixels per em.
    pub ppem: u16,
    /// Bits per pixel: 1 for black and white, 2, 4 or 8 for grey.
    #[serde(default = "one_bit")]
    pub depth: u8,
    /// The name a bitmap font file gives the strike (an X logical font description).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// The size in points the strike states; 0 when unknown.
    #[serde(default)]
    pub point_size: f64,
    /// Horizontal and vertical resolution in dots per inch; 0 when unknown.
    #[serde(default)]
    pub resolution: [u32; 2],
    /// Pixels above the baseline the strike reserves for its lines.
    #[serde(default)]
    pub ascent: i32,
    /// Pixels below the baseline the strike reserves for its lines.
    #[serde(default)]
    pub descent: i32,
    /// The X font properties of the file the strike came from, in file order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub properties: Vec<StrikeProperty>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub glyphs: Vec<StrikeGlyph>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StrikeProperty {
    pub name: String,
    pub value: StrikeValue,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StrikeValue {
    Integer(i64),
    Text(String),
}

/// One glyph's pixels in a strike.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StrikeGlyph {
    /// The font glyph this bitmap draws.
    pub glyph: String,
    /// The code the glyph had in the file it came from, written back on export.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<u32>,
    /// Horizontal advance, in pixels.
    pub advance: i32,
    /// Advance in thousandths of the point size, when the file stated it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scalable_width: Option<i32>,
    pub width: u16,
    pub height: u16,
    /// Pixels from the glyph origin to the bitmap's left edge.
    #[serde(default)]
    pub left: i32,
    /// Pixels from the baseline up to the bitmap's bottom edge.
    #[serde(default)]
    pub bottom: i32,
    /// Rows from top to bottom, [`StrikeGlyph::stride`] bytes each at the strike's depth, pixels
    /// packed from the most significant bit. Base64 text in the native file.
    #[serde(serialize_with = "to_base64", deserialize_with = "from_base64")]
    pub data: Vec<u8>,
}

fn one_bit() -> u8 {
    1
}

fn to_base64<S: Serializer>(data: &[u8], s: S) -> Result<S::Ok, S::Error> {
    use base64::Engine as _;
    s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(data))
}

fn from_base64<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
    use base64::Engine as _;
    let text = String::deserialize(d)?;
    // Base64 is 4 bytes of text per 3 of data: refuse before decoding what could not fit.
    if text.len() / 4 * 3 > MAX_STRIKE_BYTES {
        return Err(<D::Error as serde::de::Error>::custom(format!("a strike bitmap is larger than {MAX_STRIKE_BYTES} bytes")));
    }
    base64::engine::general_purpose::STANDARD
        .decode(text.as_bytes())
        .map_err(|e| <D::Error as serde::de::Error>::custom(format!("a strike bitmap: {e}")))
}

impl StrikeGlyph {
    /// Bytes per row at `depth` bits per pixel.
    pub fn stride(&self, depth: u8) -> usize {
        (usize::from(self.width) * usize::from(depth)).div_ceil(8)
    }
}

impl BitmapStrike {
    /// An empty strike of `ppem` pixels per em at `depth` bits per pixel.
    pub fn new(ppem: u16, depth: u8) -> Self {
        BitmapStrike {
            ppem,
            depth,
            name: String::new(),
            point_size: 0.0,
            resolution: [0, 0],
            ascent: 0,
            descent: 0,
            properties: Vec::new(),
            glyphs: Vec::new(),
        }
    }

    /// `12` for a black-and-white strike, `12@4` for a grey one: how messages name a strike.
    pub fn label(&self) -> String {
        label(self.ppem, self.depth)
    }

    pub fn glyph(&self, name: &str) -> Option<&StrikeGlyph> {
        self.glyphs.iter().find(|g| g.glyph == name)
    }

    /// Bytes the strike's bitmaps take.
    pub fn bytes(&self) -> usize {
        self.glyphs.iter().fold(0usize, |n, g| n.saturating_add(g.data.len()))
    }

    /// Every invariant of one strike; `Err` names the first problem.
    pub fn check(&self) -> Result<(), String> {
        if self.ppem == 0 {
            return Err("the size is 0 pixels per em".into());
        }
        if !valid_depth(self.depth) {
            return Err(format!("{} bits per pixel; a strike has 1, 2, 4 or 8", self.depth));
        }
        if !self.point_size.is_finite() || self.point_size < 0.0 || self.point_size > f64::from(MAX_STRIKE_COORDINATE) {
            return Err("the point size is not a number or is out of range".into());
        }
        if self.name.len() > MAX_STRIKE_STRING {
            return Err(format!("the name is longer than {MAX_STRIKE_STRING} bytes"));
        }
        if !coordinate(self.ascent) || !coordinate(self.descent) {
            return Err(format!("the ascent or descent is beyond ±{MAX_STRIKE_COORDINATE}"));
        }
        if self.properties.len() > MAX_STRIKE_PROPERTIES {
            return Err(format!("more than {MAX_STRIKE_PROPERTIES} properties"));
        }
        for p in &self.properties {
            let text = match &p.value {
                StrikeValue::Text(t) => t.len(),
                StrikeValue::Integer(_) => 0,
            };
            if p.name.is_empty() || p.name.len() > MAX_STRIKE_STRING || text > MAX_STRIKE_STRING {
                return Err(format!("a property name is empty, or a name or text is longer than {MAX_STRIKE_STRING} bytes"));
            }
        }
        if self.glyphs.len() > MAX_GLYPHS {
            return Err(format!("more than {MAX_GLYPHS} glyphs"));
        }
        let mut names = HashSet::with_capacity(self.glyphs.len());
        let mut bytes = 0usize;
        for g in &self.glyphs {
            check_name("strike glyph", &g.glyph).map_err(|e| e.to_string())?;
            if !names.insert(g.glyph.as_str()) {
                return Err(format!("glyph `{}` has two bitmaps", g.glyph));
            }
            if g.width > MAX_STRIKE_BITMAP_SIDE || g.height > MAX_STRIKE_BITMAP_SIDE {
                return Err(format!("glyph `{}`: the bitmap is wider or taller than {MAX_STRIKE_BITMAP_SIDE} pixels", g.glyph));
            }
            if !coordinate(g.advance) || !coordinate(g.left) || !coordinate(g.bottom) || !g.scalable_width.is_none_or(coordinate) {
                return Err(format!("glyph `{}`: an advance or offset is beyond ±{MAX_STRIKE_COORDINATE}", g.glyph));
            }
            if g.code.is_some_and(|c| c > 0x7FFF_FFFF) {
                return Err(format!("glyph `{}`: the code is out of range", g.glyph));
            }
            let want = g.stride(self.depth).saturating_mul(usize::from(g.height));
            if g.data.len() != want {
                return Err(format!("glyph `{}`: the bitmap holds {} bytes, its size needs {want}", g.glyph, g.data.len()));
            }
            bytes = bytes.saturating_add(want);
            if bytes > MAX_STRIKE_BYTES {
                return Err(format!("the bitmaps take more than {MAX_STRIKE_BYTES} bytes"));
            }
        }
        Ok(())
    }
}

/// `12`, or `12@4` for a grey strike.
pub fn label(ppem: u16, depth: u8) -> String {
    if depth == 1 { ppem.to_string() } else { format!("{ppem}@{depth}") }
}

pub fn valid_depth(depth: u8) -> bool {
    matches!(depth, 1 | 2 | 4 | 8)
}

fn coordinate(v: i32) -> bool {
    (-MAX_STRIKE_COORDINATE..=MAX_STRIKE_COORDINATE).contains(&v)
}

/// The checks on a font's strikes: each strike's own ([`BitmapStrike::check`]), at most one
/// strike per size and depth, and the limits on their number and bytes.
pub(crate) fn check_strikes(strikes: &[BitmapStrike]) -> Result<(), ModelError> {
    let bad = |strike: String, message: String| ModelError::BadStrike { strike, message };
    if strikes.len() > MAX_STRIKES {
        return Err(bad("set".into(), format!("the font holds more than {MAX_STRIKES} strikes")));
    }
    let mut seen = HashSet::with_capacity(strikes.len());
    let mut bytes = 0usize;
    for s in strikes {
        s.check().map_err(|m| bad(s.label(), m))?;
        if !seen.insert((s.ppem, s.depth)) {
            return Err(bad(s.label(), "the font has two strikes of this size and depth".into()));
        }
        bytes = bytes.saturating_add(s.bytes());
        if bytes > MAX_ALL_STRIKES_BYTES {
            return Err(bad(s.label(), format!("the strikes take more than {MAX_ALL_STRIKES_BYTES} bytes together")));
        }
    }
    Ok(())
}

/// Moves strike glyphs to the new names of renamed glyphs.
pub(crate) fn rename_in_strikes(strikes: &mut Vec<BitmapStrike>, renames: &std::collections::HashMap<&str, &str>) {
    for s in strikes.iter_mut() {
        for g in s.glyphs.iter_mut() {
            if let Some(new) = renames.get(g.glyph.as_str()) {
                g.glyph = (*new).to_string();
            }
        }
    }
}
