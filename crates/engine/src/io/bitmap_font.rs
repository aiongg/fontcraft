//! A bitmap font file (BDF, PCF, PK, GF or a Mac bitmap font) opened on its own as a bitmap-only
//! font (M13.55d, M13.55f).
//!
//! Every bitmap of the file becomes a glyph of the same name, in file order, with an empty
//! outline. Its code becomes the glyph's code point when the file's codes are Unicode
//! ([`BitmapFont::codes_are_unicode`]); a code an earlier glyph has is left out. The file's
//! strike holds the bitmaps, each with the code the file gave it. The font has
//! [`UNITS_PER_EM`] units per em: advance widths, the ascender and descender, and the cap and x
//! heights are the file's pixel values scaled by units per em ÷ pixels per em and rounded to
//! whole units. The cap and x heights come from the `CAP_HEIGHT` and `X_HEIGHT` properties,
//! else from the top of the `H` and `x` bitmaps. Like a compiled font, the document opens as
//! [`SourceFormat::Binary`], so Save asks for a source format. A file of several strikes (a Mac
//! font family) opens with the glyphs and metrics of its first, smallest strike; the other strikes
//! hold the bitmaps whose names that strike has.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use fontcraft_io_bitmap::{BitmapFont, BitmapGlyph, PropertyValue};
use fontcraft_model::strike::{BitmapStrike, StrikeGlyph, StrikeProperty, StrikeValue};
use fontcraft_model::{Font, Glyph, GlyphTable, Layer, MAX_GLYPHS, MAX_NAME_LEN, SourceFormat, metric};

use crate::{EngineError, Result};

/// Units per em of a font opened from a bitmap font file.
pub const UNITS_PER_EM: u16 = 1000;

/// Bytes kept free for a `.N` suffix when a name repeats: a dot and up to seven digits.
const SUFFIX_ROOM: usize = 8;

/// Whether `bytes` start like a bitmap font file.
pub(super) fn is_bitmap_font(bytes: &[u8]) -> bool {
    fontcraft_io_bitmap::is_bdf(bytes)
        || fontcraft_io_bitmap::is_pcf(bytes)
        || fontcraft_io_bitmap::is_pk(bytes)
        || fontcraft_io_bitmap::is_gf(bytes)
        || fontcraft_io_bitmap::is_mac_font(bytes)
}

/// The font a BDF or PCF file holds, with what reading it passed over.
pub(super) fn open(path: &Path, bytes: &[u8]) -> Result<(Font, Vec<String>)> {
    let bad = |message: String| EngineError::Io(format!("cannot read the bitmap font {}: {message}", path.display()));
    let mut imports = fontcraft_io_bitmap::read_all(bytes).map_err(|e| bad(e.to_string()))?.into_iter();
    let import = imports.next().ok_or_else(|| bad("it holds no strike".into()))?;
    let mut warnings = import.warnings;
    let file = import.font;
    if file.glyphs.len() > MAX_GLYPHS {
        return Err(bad(format!("it has more than {MAX_GLYPHS} glyphs")));
    }
    if file.pixel_size == 0 {
        return Err(bad("its size is 0 pixels per em".into()));
    }
    let scale = f64::from(UNITS_PER_EM) / f64::from(file.pixel_size);
    let units = |pixels: i32| (f64::from(pixels) * scale).round();

    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
    let family = [file.text_property("FAMILY_NAME").unwrap_or_default(), stem].into_iter().map(clean).find(|n| !n.trim().is_empty());
    let mut font = Font::new(family.unwrap_or_else(|| "Untitled".into()));
    font.source.format = SourceFormat::Binary;
    font.units_per_em = UNITS_PER_EM;
    let master = font.default_master()?.id.clone();

    let unicode = file.codes_are_unicode();
    let mut names = UniqueNames::default();
    let mut codes = HashSet::new();
    let mut glyphs = Vec::with_capacity(file.glyphs.len());
    let mut strike = strike_of(&file);
    // The glyph each file name went to, for the file's other strikes.
    let mut glyph_of: HashMap<String, String> = HashMap::new();
    let (mut renamed, mut repeated_codes, mut other_depth) = (0usize, 0usize, 0usize);
    for (i, g) in file.glyphs.iter().enumerate() {
        let name = names.add(&g.name, i);
        if name != g.name {
            renamed += 1;
        }
        glyph_of.entry(g.name.clone()).or_insert_with(|| name.clone());
        let mut glyph = Glyph::new(name.clone());
        if let Some(code) = g.encoding.filter(|c| unicode && char::from_u32(*c).is_some()) {
            if codes.insert(code) {
                glyph.unicodes.push(code);
            } else {
                repeated_codes += 1;
            }
        }
        glyph.layers.push(Layer::new(&master, units(g.advance.max(0))));
        glyphs.push(Arc::new(glyph));
        if g.bitmap.depth != file.depth {
            other_depth += 1;
            continue;
        }
        strike.glyphs.push(strike_glyph(name, g));
    }
    font.glyphs = GlyphTable::from_glyphs(glyphs);

    font.set_metric_pos(&master, metric::ASCENDER, units(file.ascent))?;
    font.set_metric_pos(&master, metric::DESCENDER, -units(file.descent))?;
    let height = |property: &str, code: u32| {
        let pixels = file.integer_property(property).and_then(|v| i32::try_from(v).ok());
        let top = || {
            let g = file.glyphs.iter().find(|g| g.encoding == Some(code) && unicode)?;
            Some(g.bitmap.bottom.saturating_add(i32::from(g.bitmap.height)))
        };
        pixels.or_else(top).filter(|p| p.unsigned_abs() <= file.pixel_size.into())
    };
    if let Some(cap) = height("CAP_HEIGHT", u32::from('H')) {
        font.set_metric_pos(&master, metric::CAP_HEIGHT, units(cap))?;
    }
    if let Some(x) = height("X_HEIGHT", u32::from('x')) {
        font.set_metric_pos(&master, metric::X_HEIGHT, units(x))?;
    }
    let mut strikes = vec![strike];
    let mut unmatched = 0usize;
    for other in imports.take(fontcraft_model::strike::MAX_STRIKES.saturating_sub(1)) {
        warnings.extend(other.warnings);
        let mut more = strike_of(&other.font);
        for g in other.font.glyphs.iter().filter(|g| g.bitmap.depth == other.font.depth) {
            match glyph_of.get(&g.name) {
                Some(name) if !more.glyphs.iter().any(|s| s.glyph == *name) => more.glyphs.push(strike_glyph(name.clone(), g)),
                _ => unmatched += 1,
            }
        }
        strikes.push(more);
    }
    if unmatched > 0 {
        warnings.push(format!("{unmatched} bitmaps of the file's larger strikes match no glyph of its first strike and are left out"));
    }
    font.bitmap_strikes = Arc::new(strikes);
    font.check()?;

    if renamed > 0 {
        warnings.push(format!("{renamed} glyph names were empty, repeated or not valid and were changed"));
    }
    if repeated_codes > 0 {
        warnings.push(format!("{repeated_codes} glyphs have a code an earlier glyph has; they open without a code point"));
    }
    if other_depth > 0 {
        warnings
            .push(format!("{other_depth} bitmaps have a depth other than the font's {} bits per pixel; their glyphs open without one", file.depth));
    }
    Ok((font, warnings))
}

/// The file's strike, without its glyphs.
fn strike_of(file: &BitmapFont) -> BitmapStrike {
    let mut strike = BitmapStrike::new(file.pixel_size, file.depth);
    strike.name = file.name.clone();
    strike.point_size = file.point_size;
    strike.resolution = [file.resolution.0, file.resolution.1];
    strike.ascent = file.ascent;
    strike.descent = file.descent;
    strike.properties = file
        .properties
        .iter()
        .map(|p| StrikeProperty {
            name: p.name.clone(),
            value: match &p.value {
                PropertyValue::Integer(i) => StrikeValue::Integer(*i),
                PropertyValue::Text(t) => StrikeValue::Text(t.clone()),
            },
        })
        .collect();
    strike
}

fn strike_glyph(name: String, g: &BitmapGlyph) -> StrikeGlyph {
    StrikeGlyph {
        glyph: name,
        code: g.encoding,
        advance: g.advance,
        scalable_width: g.scalable_width,
        width: g.bitmap.width,
        height: g.bitmap.height,
        left: g.bitmap.left,
        bottom: g.bitmap.bottom,
        data: g.bitmap.data.clone(),
    }
}

/// `s` without control characters, cut to [`MAX_NAME_LEN`] bytes.
fn clean(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars().filter(|c| !c.is_control()) {
        if out.len() + c.len_utf8() > MAX_NAME_LEN {
            break;
        }
        out.push(c);
    }
    out
}

/// The longest prefix of `s` of at most `max` bytes that ends on a character boundary.
fn cut(s: &str, max: usize) -> &str {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    s.get(..end).unwrap_or_default()
}

/// Glyph names made valid and unique: `glyph<index>` for a name that is empty once cleaned, and
/// `.1`, `.2`… after a repeated one. The search for one stem continues where it stopped, so
/// every repeat costs a bounded number of tries.
#[derive(Default)]
struct UniqueNames {
    seen: HashSet<String>,
    next: HashMap<String, usize>,
}

impl UniqueNames {
    fn add(&mut self, name: &str, index: usize) -> String {
        let mut base = clean(name);
        if base.trim().is_empty() {
            base = format!("glyph{index}");
        }
        if self.seen.insert(base.clone()) {
            return base;
        }
        let stem = cut(&base, MAX_NAME_LEN - SUFFIX_ROOM).to_string();
        let n = self.next.entry(stem.clone()).or_insert(1);
        loop {
            let name = format!("{stem}.{n}");
            *n += 1;
            if self.seen.insert(name.clone()) {
                return name;
            }
        }
    }
}
