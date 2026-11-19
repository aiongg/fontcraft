//! Bitmap strikes: hand-made pixel images of the glyphs at one size, as a BDF or PCF file holds
//! them and as the `bitmap.*` commands keep them in a font.
//!
//! A strike is named by its size in pixels per em and its bits per pixel; a font holds at most one
//! strike of each pair. Each strike glyph names the font glyph it belongs to; it stays in the
//! strike when that glyph is deleted, and follows it when it is renamed. The X font properties a
//! file stated are kept as they were, so a strike imported from a BDF file is written back with
//! them.
//!
//! A strike glyph may also draw other bitmaps of its strike, placed at a pixel offset
//! ([`BitmapReference`]). References nest at most [`MAX_BITMAP_REFERENCE_DEPTH`] deep and never
//! loop; [`BitmapStrike::flattened`] draws them in for the file writers, which know only pixels.
//!
//! The limits match those of the bitmap file readers (`fontcraft-io-bitmap`), so whatever they
//! read fits a strike.

use std::collections::{HashMap, HashSet};

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
/// Most references one strike glyph holds.
pub const MAX_BITMAP_REFERENCES: usize = 64;
/// Longest chain of references from one strike glyph through others.
pub const MAX_BITMAP_REFERENCE_DEPTH: usize = 8;
/// Most bitmaps one strike glyph draws through its references, nested ones included.
pub const MAX_BITMAP_REFERENCE_PARTS: usize = 4_096;

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
    /// Other bitmaps of the same strike drawn into this one, over its own pixels.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<BitmapReference>,
}

/// A strike glyph drawn into another at a pixel offset.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BitmapReference {
    /// The strike glyph drawn.
    pub glyph: String,
    /// Pixels from the referring glyph's origin to the drawn glyph's origin, rightwards.
    #[serde(default)]
    pub x: i32,
    /// Pixels from the referring glyph's origin to the drawn glyph's origin, upwards.
    #[serde(default)]
    pub y: i32,
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
    /// A glyph with no pixels (a 0 × 0 bitmap at the origin) and no references.
    pub fn empty(glyph: impl Into<String>, advance: i32) -> Self {
        StrikeGlyph {
            glyph: glyph.into(),
            code: None,
            advance,
            scalable_width: None,
            width: 0,
            height: 0,
            left: 0,
            bottom: 0,
            data: Vec::new(),
            references: Vec::new(),
        }
    }

    /// Bytes per row at `depth` bits per pixel.
    pub fn stride(&self, depth: u8) -> usize {
        (usize::from(self.width) * usize::from(depth)).div_ceil(8)
    }

    /// The level of the pixel in column `x` of row `y` (rows from the top) at `depth` bits per
    /// pixel; 0 outside the bitmap.
    pub fn pixel(&self, depth: u8, x: u16, y: u16) -> u8 {
        let Some((at, shift, mask)) = self.locate(depth, x, y) else { return 0 };
        self.data.get(at).map_or(0, |&b| (b >> shift) & mask)
    }

    /// Sets the pixel in column `x` of row `y` (rows from the top) to `level`, cut to `depth`
    /// bits. A pixel outside the bitmap is left alone.
    pub fn set_pixel(&mut self, depth: u8, x: u16, y: u16, level: u8) {
        let Some((at, shift, mask)) = self.locate(depth, x, y) else { return };
        if let Some(b) = self.data.get_mut(at) {
            *b = (*b & !(mask << shift)) | ((level & mask) << shift);
        }
    }

    /// The byte, bit shift and level mask of a pixel; `None` outside the bitmap or at a depth a
    /// strike cannot have.
    fn locate(&self, depth: u8, x: u16, y: u16) -> Option<(usize, u32, u8)> {
        if !valid_depth(depth) || x >= self.width || y >= self.height {
            return None;
        }
        let d = usize::from(depth);
        let bit = usize::from(x) * d;
        // `d` divides 8, so the pixel never straddles a byte.
        let shift = u32::try_from(8usize.saturating_sub(d + bit % 8)).ok()?;
        let mask = u8::MAX >> (8 - d);
        Some((usize::from(y) * self.stride(depth) + bit / 8, shift, mask))
    }

    /// The bitmap's box, `(left, bottom, right, top)` in pixels up from the baseline; `None` when
    /// it has no pixels.
    pub fn pixel_box(&self) -> Option<(i64, i64, i64, i64)> {
        if self.width == 0 || self.height == 0 {
            return None;
        }
        let (left, bottom) = (i64::from(self.left), i64::from(self.bottom));
        Some((left, bottom, left + i64::from(self.width), bottom + i64::from(self.height)))
    }
}

/// The highest level of a pixel at `depth` bits: 1 for black and white, 255 at 8 bits.
pub fn max_level(depth: u8) -> u8 {
    match depth {
        1 | 2 | 4 => (1u8 << depth) - 1,
        _ => u8::MAX,
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
            if g.references.len() > MAX_BITMAP_REFERENCES {
                return Err(format!("glyph `{}` has more than {MAX_BITMAP_REFERENCES} references", g.glyph));
            }
            for r in &g.references {
                check_name("bitmap reference", &r.glyph).map_err(|e| e.to_string())?;
                if r.glyph == g.glyph {
                    return Err(format!("glyph `{}` refers to itself", g.glyph));
                }
                if !coordinate(r.x) || !coordinate(r.y) {
                    return Err(format!("glyph `{}`: a reference offset is beyond ±{MAX_STRIKE_COORDINATE}", g.glyph));
                }
            }
        }
        self.check_references()
    }

    /// The glyphs by name.
    fn index(&self) -> HashMap<&str, &StrikeGlyph> {
        self.glyphs.iter().map(|g| (g.glyph.as_str(), g)).collect()
    }

    /// The reference rules across glyphs: no loop, at most [`MAX_BITMAP_REFERENCE_DEPTH`] levels
    /// and [`MAX_BITMAP_REFERENCE_PARTS`] bitmaps per glyph, a drawn-in box within
    /// [`MAX_STRIKE_BITMAP_SIDE`], and drawn-in bitmaps within [`MAX_STRIKE_BYTES`].
    fn check_references(&self) -> Result<(), String> {
        if self.glyphs.iter().all(|g| g.references.is_empty()) {
            return Ok(());
        }
        let index = self.index();
        let mut memo = HashMap::new();
        let mut bytes = 0usize;
        for g in &self.glyphs {
            let composite = composite_of(&index, &g.glyph, &mut memo, &mut Vec::new())?;
            let side = i64::from(MAX_STRIKE_BITMAP_SIDE);
            let size = match composite.pixel_box {
                Some((l, b, r, t)) if r - l > side || t - b > side => {
                    return Err(format!("glyph `{}` with its references drawn in would be wider or taller than {side} pixels", g.glyph));
                }
                Some((l, b, r, t)) => usize::try_from(((r - l) * i64::from(self.depth) + 7) / 8 * (t - b)).unwrap_or(usize::MAX),
                None => 0,
            };
            bytes = bytes.saturating_add(size);
            if bytes > MAX_STRIKE_BYTES {
                return Err(format!("the bitmaps with their references drawn in would take more than {MAX_STRIKE_BYTES} bytes"));
            }
        }
        Ok(())
    }

    /// `name`'s bitmap with its references drawn in, nested ones included, and no references
    /// left; `None` when the strike has no bitmap for `name`. A pixel drawn twice takes the
    /// darker level. References to glyphs the strike lacks draw nothing.
    pub fn composed(&self, name: &str) -> Result<Option<StrikeGlyph>, String> {
        let index = self.index();
        let Some(glyph) = index.get(name).copied() else { return Ok(None) };
        compose(&index, glyph, self.depth).map(Some)
    }

    /// A copy of the strike with every glyph's references drawn in; `None` when no glyph has
    /// references.
    pub fn flattened(&self) -> Result<Option<BitmapStrike>, String> {
        if self.glyphs.iter().all(|g| g.references.is_empty()) {
            return Ok(None);
        }
        self.check_references()?;
        let index = self.index();
        let mut glyphs = Vec::with_capacity(self.glyphs.len());
        for g in &self.glyphs {
            glyphs.push(if g.references.is_empty() { g.clone() } else { compose(&index, g, self.depth)? });
        }
        Ok(Some(BitmapStrike { glyphs, ..self.clone_without_glyphs() }))
    }

    /// The strike's own fields with no glyphs.
    fn clone_without_glyphs(&self) -> BitmapStrike {
        BitmapStrike {
            ppem: self.ppem,
            depth: self.depth,
            name: self.name.clone(),
            point_size: self.point_size,
            resolution: self.resolution,
            ascent: self.ascent,
            descent: self.descent,
            properties: self.properties.clone(),
            glyphs: Vec::new(),
        }
    }
}

/// What following a glyph's references reaches: how deep they nest, how many bitmaps they draw
/// (the glyph's own included) and the box of all of them.
#[derive(Clone, Copy)]
struct Composite {
    depth: usize,
    parts: usize,
    pixel_box: Option<(i64, i64, i64, i64)>,
}

/// The [`Composite`] of `name`, memoised; `path` holds the glyphs whose references lead here.
/// A reference to a glyph the strike lacks draws nothing. The recursion is at most
/// [`MAX_BITMAP_REFERENCE_DEPTH`] + 1 calls deep.
fn composite_of<'a>(
    index: &HashMap<&'a str, &'a StrikeGlyph>,
    name: &'a str,
    memo: &mut HashMap<&'a str, Composite>,
    path: &mut Vec<&'a str>,
) -> Result<Composite, String> {
    if let Some(known) = memo.get(name) {
        return Ok(*known);
    }
    let root = path.first().copied().unwrap_or(name);
    if path.contains(&name) {
        return Err(format!("the references of glyph `{root}` lead back to `{name}`"));
    }
    if path.len() > MAX_BITMAP_REFERENCE_DEPTH {
        return Err(format!("the references of glyph `{root}` nest more than {MAX_BITMAP_REFERENCE_DEPTH} levels deep"));
    }
    let Some(glyph) = index.get(name).copied() else {
        return Ok(Composite { depth: 0, parts: 0, pixel_box: None });
    };
    let mut out = Composite { depth: 0, parts: 1, pixel_box: glyph.pixel_box() };
    path.push(name);
    for r in &glyph.references {
        let inner = composite_of(index, r.glyph.as_str(), memo, path)?;
        out.depth = out.depth.max(inner.depth.saturating_add(1));
        out.parts = out.parts.saturating_add(inner.parts);
        if let Some((l, b, rt, t)) = inner.pixel_box {
            let (x, y) = (i64::from(r.x), i64::from(r.y));
            out.pixel_box = Some(union(out.pixel_box, (l + x, b + y, rt + x, t + y)));
        }
    }
    path.pop();
    if out.depth > MAX_BITMAP_REFERENCE_DEPTH {
        return Err(format!("the references of glyph `{name}` nest more than {MAX_BITMAP_REFERENCE_DEPTH} levels deep"));
    }
    if out.parts > MAX_BITMAP_REFERENCE_PARTS {
        return Err(format!("glyph `{name}` draws more than {MAX_BITMAP_REFERENCE_PARTS} bitmaps through its references"));
    }
    memo.insert(name, out);
    Ok(out)
}

fn union(a: Option<(i64, i64, i64, i64)>, (l, b, r, t): (i64, i64, i64, i64)) -> (i64, i64, i64, i64) {
    match a {
        Some((al, ab, ar, at)) => (al.min(l), ab.min(b), ar.max(r), at.max(t)),
        None => (l, b, r, t),
    }
}

/// `glyph` with its references drawn in, at `depth` bits per pixel.
fn compose(index: &HashMap<&str, &StrikeGlyph>, glyph: &StrikeGlyph, depth: u8) -> Result<StrikeGlyph, String> {
    // Every bitmap drawn, with its origin's offset from `glyph`'s.
    let mut parts: Vec<(&StrikeGlyph, i64, i64)> = vec![(glyph, 0, 0)];
    let mut stack: Vec<(&StrikeGlyph, i64, i64, usize)> = vec![(glyph, 0, 0, 0)];
    while let Some((g, dx, dy, level)) = stack.pop() {
        for r in &g.references {
            let Some(inner) = index.get(r.glyph.as_str()).copied() else { continue };
            if level >= MAX_BITMAP_REFERENCE_DEPTH {
                return Err(format!("the references of glyph `{}` nest more than {MAX_BITMAP_REFERENCE_DEPTH} levels deep", glyph.glyph));
            }
            if parts.len() >= MAX_BITMAP_REFERENCE_PARTS {
                return Err(format!("glyph `{}` draws more than {MAX_BITMAP_REFERENCE_PARTS} bitmaps through its references", glyph.glyph));
            }
            let (x, y) = (dx + i64::from(r.x), dy + i64::from(r.y));
            parts.push((inner, x, y));
            stack.push((inner, x, y, level + 1));
        }
    }
    let mut pixel_box = None;
    for (g, dx, dy) in &parts {
        if let Some((l, b, r, t)) = g.pixel_box() {
            pixel_box = Some(union(pixel_box, (l + dx, b + dy, r + dx, t + dy)));
        }
    }
    let mut out = StrikeGlyph { references: Vec::new(), data: Vec::new(), width: 0, height: 0, ..glyph.clone() };
    let Some((left, bottom, right, top)) = pixel_box else { return Ok(out) };
    let side = i64::from(MAX_STRIKE_BITMAP_SIDE);
    if right - left > side || top - bottom > side || !coordinate_i64(left) || !coordinate_i64(bottom) {
        return Err(format!(
            "glyph `{}` with its references drawn in would be wider or taller than {side} pixels, or placed too far away",
            glyph.glyph
        ));
    }
    // Within 1..=1024 by the test above.
    out.width = u16::try_from(right - left).unwrap_or(0);
    out.height = u16::try_from(top - bottom).unwrap_or(0);
    out.left = i32::try_from(left).unwrap_or(0);
    out.bottom = i32::try_from(bottom).unwrap_or(0);
    out.data = vec![0; out.stride(depth) * usize::from(out.height)];
    for (g, dx, dy) in parts {
        for row in 0..g.height {
            for col in 0..g.width {
                let level = g.pixel(depth, col, row);
                if level == 0 {
                    continue;
                }
                let x = i64::from(g.left) + dx + i64::from(col) - left;
                let y = top - (i64::from(g.bottom) + dy + i64::from(g.height) - i64::from(row));
                let (Ok(x), Ok(y)) = (u16::try_from(x), u16::try_from(y)) else { continue };
                if level > out.pixel(depth, x, y) {
                    out.set_pixel(depth, x, y, level);
                }
            }
        }
    }
    Ok(out)
}

fn coordinate_i64(v: i64) -> bool {
    (-i64::from(MAX_STRIKE_COORDINATE)..=i64::from(MAX_STRIKE_COORDINATE)).contains(&v)
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

/// Moves strike glyphs, and the references to them, to the new names of renamed glyphs.
pub(crate) fn rename_in_strikes(strikes: &mut [BitmapStrike], renames: &std::collections::HashMap<&str, &str>) {
    for s in strikes.iter_mut() {
        for g in s.glyphs.iter_mut() {
            if let Some(new) = renames.get(g.glyph.as_str()) {
                g.glyph = (*new).to_string();
            }
            for r in g.references.iter_mut() {
                if let Some(new) = renames.get(r.glyph.as_str()) {
                    r.glyph = (*new).to_string();
                }
            }
        }
    }
}
