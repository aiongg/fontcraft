//! A bitmap font file (BDF, PCF, either gzip-compressed, Windows FNT or FON) opened on its own as
//! a bitmap-only font (M13.55d, M13.55e).
//!
//! The glyphs come from the file's first font; each further font of a FON file becomes another
//! strike, holding the bitmaps whose names the first font has.
//!
//! Every bitmap of the file becomes a glyph of the same name, in file order, with an empty
//! outline. Its code becomes the glyph's code point when the file's codes are Unicode
//! ([`BitmapFont::codes_are_unicode`]); a code an earlier glyph has is left out. The file's
//! strike holds the bitmaps, each with the code the file gave it. The font has
//! [`UNITS_PER_EM`] units per em: advance widths, the ascender and descender, and the cap and x
//! heights are the file's pixel values scaled by units per em ÷ pixels per em and rounded to
//! whole units. The cap and x heights come from the `CAP_HEIGHT` and `X_HEIGHT` properties,
//! else from the top of the `H` and `x` bitmaps. Like a compiled font, the document opens as
//! [`SourceFormat::Binary`], so Save asks for a source format.

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

/// Whether `bytes` start like a BDF, PCF, FNT or FON file.
pub(super) fn is_bitmap_font(bytes: &[u8]) -> bool {
    use fontcraft_io_bitmap as io;
    io::is_bdf(bytes) || io::is_pcf(bytes) || io::is_fnt(bytes) || io::is_fon(bytes)
}

/// Whether `bytes` are gzip data in a file named `.pcf.gz` or `.bdf.gz`. Other gzip data is a
/// compressed FontCraft document.
pub(super) fn is_gzipped_bitmap_font(path: &Path, bytes: &[u8]) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_ascii_lowercase();
    fontcraft_io_bitmap::is_gzip(bytes) && (name.ends_with(".pcf.gz") || name.ends_with(".bdf.gz"))
}

/// The font a bitmap font file holds, with what reading it passed over.
pub(super) fn open(path: &Path, bytes: &[u8]) -> Result<(Font, Vec<String>)> {
    let bad = |message: String| EngineError::Io(format!("cannot read the bitmap font {}: {message}", path.display()));
    let mut imports = fontcraft_io_bitmap::read_all(bytes).map_err(|e| bad(e.to_string()))?.into_iter();
    let import = imports.next().ok_or_else(|| bad("it holds no fonts".into()))?;
    let others: Vec<_> = imports.collect();
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
    let stem = [".pcf", ".bdf", ".PCF", ".BDF"].iter().find_map(|x| stem.strip_suffix(x)).unwrap_or(stem);
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
    let (mut renamed, mut repeated_codes, mut other_depth) = (0usize, 0usize, 0usize);
    for (i, g) in file.glyphs.iter().enumerate() {
        let name = names.add(&g.name, i);
        if name != g.name {
            renamed += 1;
        }
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
    for other in others {
        warnings.extend(other.warnings);
        let file = other.font;
        if file.pixel_size == 0 || strikes.iter().any(|s| s.ppem == file.pixel_size && s.depth == file.depth) {
            warnings.push(format!("a further font of {} pixels per em was left out: the font has a strike of that size", file.pixel_size));
            continue;
        }
        let mut more = strike_of(&file);
        let mut left_out = 0usize;
        for g in &file.glyphs {
            if g.bitmap.depth == file.depth && names.seen.contains(&g.name) {
                more.glyphs.push(strike_glyph(g.name.clone(), g));
            } else {
                left_out += 1;
            }
        }
        if left_out > 0 {
            warnings.push(format!("{left_out} bitmaps of the {} pixel font have no glyph of the same name and were left out", file.pixel_size));
        }
        strikes.push(more);
    }
    strikes.sort_by_key(|s| (s.ppem, s.depth));
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
