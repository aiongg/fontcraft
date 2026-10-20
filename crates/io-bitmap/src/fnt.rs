//! Windows bitmap fonts: FNT files, versions 2 and 3.
//!
//! Written from Microsoft's published description of the Windows font file format (the font
//! file header and character table of the Windows 3 SDK reference and its knowledge-base note on
//! font files), and checked by rendering files of our own through FreeType as a black box. No
//! code that reads or writes the format was consulted.
//!
//! An FNT file is a fixed header, a character table, the face name and the glyph bitmaps. The
//! table has one entry per code from the first character to the last, plus one for a blank
//! "absolute space" glyph: a 16-bit width and an offset counted from the start of the file
//! (16 bits in version 2, 32 bits in version 3). Every glyph is as tall as the font's cell and
//! as wide as its advance. Its bitmap is stored in strips one byte wide, left to right; each
//! strip holds every row of the cell from top to bottom, the leftmost pixel in the most
//! significant bit. Codes are 8-bit, in the Windows character set the header names.
//!
//! | FNT header | [`BitmapFont`] |
//! |---|---|
//! | `dfPixHeight`, `dfAscent` | cell height; ascent, and descent = height − ascent |
//! | `dfPixHeight` − `dfInternalLeading` | [`BitmapFont::pixel_size`] |
//! | `dfPoints`, `dfHorizRes`, `dfVertRes` | point size and resolution |
//! | face name, `dfCopyright`, `dfWeight`, `dfItalic` | `FAMILY_NAME`, `COPYRIGHT`, `WEIGHT_NAME`, `SLANT` |
//! | `dfCharSet` | `CHARSET_REGISTRY`, `CHARSET_ENCODING` |
//! | `dfDefaultChar` | `DEFAULT_CHAR` |
//!
//! The ANSI character set is Windows code page 1252: its codes are read as the Unicode code
//! points they stand for, and Unicode codes are written back through the same table.

use crate::bdf::clear_padding;
use crate::{
    Bitmap, BitmapError, BitmapFont, BitmapGlyph, Import, MAX_BITMAP_SIDE, MAX_COORDINATE, MAX_FILE_BYTES, MAX_GLYPHS, MAX_STRING_BYTES, PixelBudget,
    Property, PropertyValue, Result, Warnings,
};

const VERSION_2: u16 = 0x0200;
const VERSION_3: u16 = 0x0300;
const HEADER_2: usize = 118;
const HEADER_3: usize = 148;
/// Bytes of the header a FON font directory entry repeats (`dfVersion` to `dfFace`).
pub(crate) const DIRECTORY_ENTRY_BYTES: usize = 113;
const COPYRIGHT_BYTES: usize = 60;
/// Longest face name written: a Windows face name holds 32 bytes with its terminator.
const FACE_BYTES: usize = 31;

const DFF_FIXED: u32 = 0x01;
const DFF_PROPORTIONAL: u32 = 0x02;
const DFF_ABC: u32 = 0x04 | 0x08;
const DFF_1COLOR: u32 = 0x10;
const DFF_COLOR: u32 = 0x20 | 0x40 | 0x80;

const ANSI_CHARSET: u8 = 0;
/// Windows character sets and the code pages they stand for.
const CODE_PAGES: [(u8, u16); 14] = [
    (128, 932),
    (129, 949),
    (130, 1361),
    (134, 936),
    (136, 950),
    (161, 1253),
    (162, 1254),
    (163, 1258),
    (177, 1255),
    (178, 1256),
    (186, 1257),
    (204, 1251),
    (222, 874),
    (238, 1250),
];
/// Code page 1252 at 0x80..=0x9F; the five unassigned bytes stand for the C1 control of the
/// same value.
const CP1252_HIGH: [u16; 32] = [
    0x20AC, 0x0081, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0x008D, 0x017D, 0x008F, 0x0090, 0x2018,
    0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x009D, 0x017E, 0x0178,
];
/// Weight names and the Windows weights they stand for; reading picks the nearest.
const WEIGHTS: [(&str, u16); 9] = [
    ("Thin", 100),
    ("ExtraLight", 200),
    ("Light", 300),
    ("Medium", 400),
    ("SemiBold", 600),
    ("Bold", 700),
    ("ExtraBold", 800),
    ("Black", 900),
    ("Regular", 400),
];

/// A Windows font file written from one or more strikes, with what it could not hold.
#[derive(Clone, Debug, PartialEq)]
pub struct Written {
    pub bytes: Vec<u8>,
    /// One sentence per thing left out, at most [`crate::MAX_WARNINGS`].
    pub warnings: Vec<String>,
}

fn bad(message: impl Into<String>) -> BitmapError {
    BitmapError::Fnt(message.into())
}

pub(crate) fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    bytes.get(at..at.checked_add(2)?).and_then(|b| <[u8; 2]>::try_from(b).ok()).map(u16::from_le_bytes)
}

pub(crate) fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    bytes.get(at..at.checked_add(4)?).and_then(|b| <[u8; 4]>::try_from(b).ok()).map(u32::from_le_bytes)
}

/// Whether `bytes` start like an FNT file: version 2 or 3, a whole header, and a first
/// character no later than the last.
pub fn is_fnt(bytes: &[u8]) -> bool {
    let header = match u16_at(bytes, 0) {
        Some(VERSION_2) => HEADER_2,
        Some(VERSION_3) => HEADER_3,
        _ => return false,
    };
    bytes.len() >= header && bytes.get(95) <= bytes.get(96)
}

/// Reads an FNT file.
pub fn read_fnt(bytes: &[u8]) -> Result<Import> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(BitmapError::TooLarge(bytes.len()));
    }
    let mut warnings = Warnings::default();
    let font = parse(bytes, &mut warnings, &mut PixelBudget::default())?;
    Ok(Import { font, warnings: warnings.finish() })
}

/// Reads one FNT font: a whole file, or a font resource of a FON file, whose offsets count from
/// the resource's start.
pub(crate) fn parse(bytes: &[u8], warnings: &mut Warnings, budget: &mut PixelBudget) -> Result<BitmapFont> {
    let version = u16_at(bytes, 0).ok_or_else(|| bad("the file ends inside its header"))?;
    let header = match version {
        VERSION_2 => HEADER_2,
        VERSION_3 => HEADER_3,
        other => return Err(bad(format!("version {other:#06x}; versions 2 (0x0200) and 3 (0x0300) are read"))),
    };
    if bytes.len() < header {
        return Err(bad("the file ends inside its header"));
    }
    let h8 = |at: usize| bytes.get(at).copied().unwrap_or(0);
    let h16 = |at: usize| u16_at(bytes, at).unwrap_or(0);
    let h32 = |at: usize| u32_at(bytes, at).unwrap_or(0);
    if h16(66) & 1 != 0 {
        return Err(bad("it is a vector font; only bitmap fonts are read"));
    }
    let (points, vres, hres, ascent, internal) = (h16(68), h16(70), h16(72), h16(74), h16(76));
    let (italic, weight, charset, pix_width, height) = (h8(80), h16(83), h8(85), h16(86), h16(88));
    let (average, first, last, default) = (h16(91), h8(95), h8(96), h8(97));
    if version == VERSION_3 {
        let flags = h32(118);
        if flags & DFF_ABC != 0 {
            return Err(bad("its glyphs use ABC spacing, which is not read"));
        }
        if flags & DFF_COLOR != 0 {
            return Err(bad("it is a colour font; only black-and-white fonts are read"));
        }
    }
    if last < first {
        return Err(bad(format!("its last character {last} comes before its first {first}")));
    }
    if height == 0 {
        return Err(bad("its cell height is 0 pixels"));
    }
    if height > MAX_BITMAP_SIDE {
        return Err(BitmapError::Limit { what: "FNT cell height in pixels", limit: MAX_BITMAP_SIDE.into() });
    }
    let points = if i32::from(points) > MAX_COORDINATE {
        warnings.push(format!("the point size {points} is above {MAX_COORDINATE}; {MAX_COORDINATE} is used"));
        MAX_COORDINATE as u16
    } else {
        points
    };
    let ascent = if ascent > height {
        warnings.push(format!("the ascent {ascent} is more than the cell height {height}; the cell height is used"));
        height
    } else {
        ascent
    };

    let entry = if version == VERSION_3 { 6 } else { 4 };
    let count = usize::from(last - first) + 1;
    if bytes.len() < header + entry * count {
        return Err(bad("the file ends inside its character table"));
    }
    let entries: Vec<(u16, usize)> = (0..count)
        .map(|i| {
            let at = header + entry * i;
            let offset = if version == VERSION_3 { h32(at + 2) as usize } else { usize::from(h16(at + 2)) };
            (h16(at), offset)
        })
        .collect();
    let default_index = usize::from(default);
    let default_entry = entries.get(default_index).copied();

    let ansi = charset == ANSI_CHARSET;
    let code_of = |raw: u8| if ansi { cp1252_to_unicode(raw) } else { u32::from(raw) };
    let mut glyphs = Vec::with_capacity(count);
    for (i, (width, offset)) in entries.iter().copied().enumerate() {
        let raw = u8::try_from(usize::from(first) + i).unwrap_or(u8::MAX);
        // Codes the font has no glyph for show the default character's bitmap.
        if i != default_index && Some((width, offset)) == default_entry {
            continue;
        }
        if width > MAX_BITMAP_SIDE {
            return Err(BitmapError::Limit { what: "FNT glyph width in pixels", limit: MAX_BITMAP_SIDE.into() });
        }
        let strip = usize::from(height);
        let len = usize::from(width).div_ceil(8) * strip;
        let src = match len {
            0 => &[][..],
            _ => offset
                .checked_add(len)
                .and_then(|end| bytes.get(offset..end))
                .ok_or_else(|| bad(format!("the bitmap of character {raw:#04x} lies outside the file")))?,
        };
        let mut bitmap = Bitmap::blank(width, height, 0, i32::from(ascent) - i32::from(height), 1)
            .ok_or(BitmapError::Limit { what: "FNT glyph size in pixels", limit: MAX_BITMAP_SIDE.into() })?;
        budget.spend(bitmap.data.len())?;
        let stride = bitmap.stride();
        for (column, bytes) in src.chunks(strip).enumerate() {
            for (row, byte) in bytes.iter().enumerate() {
                if let Some(b) = bitmap.data.get_mut(row * stride + column) {
                    *b = *byte;
                }
            }
        }
        clear_padding(&mut bitmap);
        let code = code_of(raw);
        let name = if ansi { format!("uni{code:04X}") } else { format!("char{raw:03}") };
        glyphs.push(BitmapGlyph { name, encoding: Some(code), advance: i32::from(width), scalable_width: None, bitmap });
    }

    let face = match h32(105) as usize {
        0 => String::new(),
        at => match bytes.get(at..) {
            Some(rest) => decode(rest),
            None => {
                warnings.push("the face name lies outside the file".into());
                String::new()
            }
        },
    };
    let copyright = decode(bytes.get(6..6 + COPYRIGHT_BYTES).unwrap_or_default());
    let (registry, encoding) = registry(charset);
    let pixel_size = if internal < height { height - internal } else { height };
    let descent = height - ascent;
    let text = |name: &str, value: String| Property { name: name.into(), value: PropertyValue::Text(value) };
    let integer = |name: &str, value: i64| Property { name: name.into(), value: PropertyValue::Integer(value) };
    let mut properties = Vec::new();
    if !face.is_empty() {
        properties.push(text("FAMILY_NAME", face.clone()));
    }
    if weight > 0 {
        let nearest = WEIGHTS.iter().min_by_key(|(_, w)| w.abs_diff(weight)).map_or("Medium", |(name, _)| name);
        properties.push(text("WEIGHT_NAME", nearest.into()));
    }
    properties.push(text("SLANT", if italic != 0 { "I" } else { "R" }.into()));
    properties.push(integer("PIXEL_SIZE", pixel_size.into()));
    properties.push(integer("POINT_SIZE", i64::from(points) * 10));
    properties.push(integer("RESOLUTION_X", hres.into()));
    properties.push(integer("RESOLUTION_Y", vres.into()));
    properties.push(text("SPACING", if pix_width > 0 { "C" } else { "P" }.into()));
    properties.push(integer("AVERAGE_WIDTH", i64::from(average) * 10));
    properties.push(text("CHARSET_REGISTRY", registry));
    properties.push(text("CHARSET_ENCODING", encoding));
    properties.push(integer("FONT_ASCENT", ascent.into()));
    properties.push(integer("FONT_DESCENT", descent.into()));
    if let Some(raw) = first.checked_add(default).filter(|raw| *raw <= last) {
        properties.push(integer("DEFAULT_CHAR", code_of(raw).into()));
    }
    if !copyright.is_empty() {
        properties.push(text("COPYRIGHT", copyright));
    }
    Ok(BitmapFont {
        name: face,
        pixel_size,
        point_size: f64::from(points),
        resolution: (hres.into(), vres.into()),
        depth: 1,
        ascent: ascent.into(),
        descent: descent.into(),
        properties,
        glyphs,
    })
}

fn cp1252_to_unicode(raw: u8) -> u32 {
    match raw.checked_sub(0x80).and_then(|i| CP1252_HIGH.get(usize::from(i))) {
        Some(c) => u32::from(*c),
        None => u32::from(raw),
    }
}

fn unicode_to_cp1252(code: u32) -> Option<u8> {
    if code < 0x80 || (0xA0..=0xFF).contains(&code) {
        return u8::try_from(code).ok();
    }
    let i = CP1252_HIGH.iter().position(|c| u32::from(*c) == code)?;
    u8::try_from(0x80 + i).ok()
}

/// Text up to the first zero byte, at most [`MAX_STRING_BYTES`], read as code page 1252
/// without control characters.
fn decode(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take(MAX_STRING_BYTES)
        .take_while(|b| **b != 0)
        .filter_map(|b| char::from_u32(cp1252_to_unicode(*b)))
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .to_string()
}

/// `text` in code page 1252, `?` for characters it lacks, at most `max` bytes.
fn encode(text: &str, max: usize) -> Vec<u8> {
    text.chars().filter(|c| !c.is_control()).map(|c| unicode_to_cp1252(c.into()).unwrap_or(b'?')).take(max).collect()
}

/// The X registry and encoding of a Windows character set. ANSI codes are read as Unicode.
fn registry(charset: u8) -> (String, String) {
    if charset == ANSI_CHARSET {
        return ("ISO10646".into(), "1".into());
    }
    match CODE_PAGES.iter().find(|(c, _)| *c == charset) {
        Some((_, page)) => ("MICROSOFT".into(), format!("CP{page}")),
        None => ("MICROSOFT".into(), format!("CHARSET{charset}")),
    }
}

/// The Windows character set a strike is written in, and whether its codes are Unicode code
/// points that go through code page 1252.
fn charset_for_writing(font: &BitmapFont) -> (u8, bool) {
    if font.codes_are_unicode() {
        return (ANSI_CHARSET, true);
    }
    let registry = font.text_property("CHARSET_REGISTRY").unwrap_or_default();
    let encoding = font.text_property("CHARSET_ENCODING").unwrap_or_default().to_ascii_uppercase();
    if !registry.eq_ignore_ascii_case("MICROSOFT") {
        return (ANSI_CHARSET, false);
    }
    if let Some(page) = encoding.strip_prefix("CP").and_then(|p| p.parse::<u16>().ok()) {
        let charset = CODE_PAGES.iter().find(|(_, p)| *p == page).map_or(ANSI_CHARSET, |(c, _)| *c);
        return (charset, false);
    }
    let charset = encoding.strip_prefix("CHARSET").and_then(|n| n.parse::<u8>().ok()).unwrap_or(ANSI_CHARSET);
    (charset, false)
}

fn put8(out: &mut [u8], at: usize, value: u8) {
    if let Some(b) = out.get_mut(at) {
        *b = value;
    }
}

fn put16(out: &mut [u8], at: usize, value: u16) {
    if let Some(b) = out.get_mut(at..at + 2) {
        b.copy_from_slice(&value.to_le_bytes());
    }
}

fn put32(out: &mut [u8], at: usize, value: u32) {
    if let Some(b) = out.get_mut(at..at + 4) {
        b.copy_from_slice(&value.to_le_bytes());
    }
}

fn clamp16(value: i64) -> u16 {
    u16::try_from(value.clamp(0, i64::from(u16::MAX))).unwrap_or(u16::MAX)
}

/// Writes a black-and-white strike as an FNT file, version 3.
///
/// A glyph goes to the 8-bit code its own code stands for in the strike's Windows character set
/// (code page 1252 for Unicode codes); glyphs without one, and later glyphs with a code an
/// earlier one has, are left out with a warning. The cell spans every glyph's pixels and the
/// strike's ascent and descent; each glyph is as wide as its advance, and pixels outside the
/// advance are left out with a warning. Codes between the first and last with no glyph show
/// the default character.
pub fn write_fnt(font: &BitmapFont) -> Result<Written> {
    let mut warnings = Warnings::default();
    let bytes = build(font, &mut warnings)?;
    Ok(Written { bytes, warnings: warnings.finish() })
}

pub(crate) fn build(font: &BitmapFont, warnings: &mut Warnings) -> Result<Vec<u8>> {
    let fail = |message: String| BitmapError::Write(message);
    if font.depth != 1 {
        return Err(fail(format!("Windows bitmap fonts hold black-and-white strikes only; this one has {} bits per pixel", font.depth)));
    }
    if font.glyphs.len() > MAX_GLYPHS {
        return Err(fail(format!("the font has more than {MAX_GLYPHS} glyphs")));
    }
    let (charset, from_unicode) = charset_for_writing(font);
    let to_raw = |code: u32| if from_unicode { unicode_to_cp1252(code) } else { u8::try_from(code).ok() };
    let mut slots: Vec<Option<&BitmapGlyph>> = vec![None; 256];
    let (mut no_code, mut no_code_count, mut repeated) = (Vec::new(), 0usize, 0usize);
    for glyph in &font.glyphs {
        let b = &glyph.bitmap;
        if b.width > MAX_BITMAP_SIDE || b.height > MAX_BITMAP_SIDE || b.depth != 1 {
            return Err(fail(format!("the bitmap of {} is larger than {MAX_BITMAP_SIDE} pixels or not black and white", glyph.name)));
        }
        match glyph.encoding.and_then(to_raw).and_then(|raw| slots.get_mut(usize::from(raw))) {
            Some(slot) if slot.is_none() => *slot = Some(glyph),
            Some(_) => repeated += 1,
            None => {
                no_code_count += 1;
                if no_code.len() < 10 {
                    no_code.push(glyph.name.as_str());
                }
            }
        }
    }
    let present = |raw: usize| slots.get(raw).copied().flatten();
    let (Some(first), Some(last)) = ((0..256).find(|r| present(*r).is_some()), (0..256).rev().find(|r| present(*r).is_some())) else {
        return Err(fail("no glyph has a code the strike's Windows character set holds".into()));
    };
    if no_code_count > 0 {
        let more = if no_code_count > no_code.len() { "…" } else { "" };
        warnings.push(format!("{no_code_count} glyphs have no code in the Windows character set and were left out: {}{more}", no_code.join(", ")));
    }
    if repeated > 0 {
        warnings.push(format!("{repeated} glyphs have a code an earlier glyph has and were left out"));
    }
    let chosen: Vec<&BitmapGlyph> = (first..=last).filter_map(present).collect();

    let clamp = |v: i64| v.clamp(0, i64::from(MAX_COORDINATE));
    let (mut ascent, mut descent) = (clamp(font.ascent.into()), clamp(font.descent.into()));
    for g in &chosen {
        let b = &g.bitmap;
        if b.width > 0 && b.height > 0 {
            ascent = ascent.max(clamp(i64::from(b.bottom) + i64::from(b.height)));
            descent = descent.max(clamp(-i64::from(b.bottom)));
        }
    }
    if ascent + descent == 0 {
        ascent = 1;
    }
    let cell = ascent + descent;
    if cell > i64::from(MAX_BITMAP_SIDE) {
        return Err(fail(format!("the strike is {cell} pixels tall from its highest pixel to its lowest; the limit is {MAX_BITMAP_SIDE}")));
    }
    let strip = usize::try_from(cell).unwrap_or(1);

    // Each glyph's width and its bitmap in byte-wide strips.
    let mut images: Vec<Option<(u16, Vec<u8>)>> = vec![None; 256];
    let mut clipped = 0usize;
    for (raw, slot) in images.iter_mut().enumerate() {
        let Some(g) = present(raw) else { continue };
        if g.advance > i32::from(MAX_BITMAP_SIDE) {
            return Err(fail(format!("{} advances {} pixels; the limit is {MAX_BITMAP_SIDE}", g.name, g.advance)));
        }
        let width = u16::try_from(g.advance.max(0)).unwrap_or(0);
        let mut data = vec![0u8; usize::from(width).div_ceil(8) * strip];
        let b = &g.bitmap;
        let top = ascent - (i64::from(b.bottom) + i64::from(b.height));
        let mut lost = false;
        for y in 0..b.height {
            for x in 0..b.width {
                if b.pixel(x, y) == 0 {
                    continue;
                }
                let (cx, cy) = (i64::from(b.left) + i64::from(x), top + i64::from(y));
                match (usize::try_from(cx), usize::try_from(cy)) {
                    (Ok(cx), Ok(cy)) if cx < usize::from(width) && cy < strip => {
                        if let Some(byte) = data.get_mut(cx / 8 * strip + cy) {
                            *byte |= 0x80 >> (cx % 8);
                        }
                    }
                    _ => lost = true,
                }
            }
        }
        clipped += usize::from(lost);
        *slot = Some((width, data));
    }
    if clipped > 0 {
        warnings.push(format!("{clipped} glyphs have pixels outside their advance width; those pixels were left out"));
    }

    let widths: Vec<u16> = images.iter().flatten().map(|(w, _)| *w).collect();
    let max_width = widths.iter().copied().max().unwrap_or(0);
    let fixed = widths.iter().all(|w| *w == max_width);
    let average = match images.get(usize::from(b'x')).and_then(Option::as_ref) {
        Some((w, _)) => *w,
        None => {
            let sum: u64 = widths.iter().map(|w| u64::from(*w)).sum();
            u16::try_from(sum.div_ceil(widths.len().max(1) as u64)).unwrap_or(max_width)
        }
    };
    let default_raw = font
        .integer_property("DEFAULT_CHAR")
        .and_then(|c| u32::try_from(c).ok())
        .and_then(to_raw)
        .map(usize::from)
        .filter(|r| present(*r).is_some())
        .or_else(|| present(usize::from(b'?')).map(|_| usize::from(b'?')))
        .unwrap_or(first);
    let break_raw = if (first..=last).contains(&0x20) { 0x20 } else { first };

    let family = font.text_property("FAMILY_NAME").map(str::trim).filter(|f| !f.is_empty()).unwrap_or(font.name.trim());
    let mut face = encode(family, FACE_BYTES);
    if face.is_empty() {
        face = b"Untitled".to_vec();
    }
    face.push(0);

    let count = last.saturating_sub(first) + 2;
    let face_at = HEADER_3 + 6 * count;
    let bits_at = face_at + face.len();
    let mut bits = Vec::new();
    let mut table = Vec::with_capacity(6 * count);
    let mut offsets = vec![None; 256];
    for raw in first..=last {
        if let Some(Some((width, data))) = images.get(raw) {
            let offset = bits_at + bits.len();
            bits.extend_from_slice(data);
            if let Some(o) = offsets.get_mut(raw) {
                *o = Some((*width, offset));
            }
        }
    }
    let default_entry = offsets.get(default_raw).copied().flatten().unwrap_or((0, bits_at));
    let blank = (average, bits_at + bits.len());
    bits.resize(bits.len() + usize::from(average).div_ceil(8) * strip, 0);
    for raw in first..=last + 1 {
        let (width, offset) = if raw > last { blank } else { offsets.get(raw).copied().flatten().unwrap_or(default_entry) };
        table.extend_from_slice(&width.to_le_bytes());
        table.extend_from_slice(&u32::try_from(offset).unwrap_or(u32::MAX).to_le_bytes());
    }
    let size = bits_at + bits.len();
    if size > MAX_FILE_BYTES {
        return Err(fail(format!("the font would take {size} bytes; the limit is {MAX_FILE_BYTES}")));
    }

    let mut out = vec![0u8; HEADER_3];
    put16(&mut out, 0, VERSION_3);
    put32(&mut out, 2, u32::try_from(size).unwrap_or(u32::MAX));
    let copyright = encode(font.text_property("COPYRIGHT").unwrap_or_default(), COPYRIGHT_BYTES);
    if let Some(field) = out.get_mut(6..6 + copyright.len()) {
        field.copy_from_slice(&copyright);
    }
    let (hres, vres) = (clamp16(font.resolution.0.into()), clamp16(font.resolution.1.into()));
    let (hres, vres) = (if hres == 0 { 96 } else { hres }, if vres == 0 { 96 } else { vres });
    let points = if font.point_size.is_finite() && font.point_size >= 0.5 {
        font.point_size.round()
    } else {
        (f64::from(font.pixel_size) * 72.0 / f64::from(vres)).round()
    };
    put16(&mut out, 68, clamp16(points.clamp(1.0, 65_535.0) as i64));
    put16(&mut out, 70, vres);
    put16(&mut out, 72, hres);
    put16(&mut out, 74, clamp16(ascent));
    put16(&mut out, 76, clamp16(cell - i64::from(font.pixel_size)));
    let slant = font.text_property("SLANT").unwrap_or_default();
    put8(&mut out, 80, u8::from(slant.eq_ignore_ascii_case("I") || slant.eq_ignore_ascii_case("O")));
    let weight = font.text_property("WEIGHT_NAME").and_then(|w| WEIGHTS.iter().find(|(name, _)| name.eq_ignore_ascii_case(w.trim())));
    put16(&mut out, 83, weight.map_or(400, |(_, w)| *w));
    put8(&mut out, 85, charset);
    put16(&mut out, 86, if fixed { max_width } else { 0 });
    put16(&mut out, 88, clamp16(cell));
    put8(&mut out, 90, u8::from(!fixed));
    put16(&mut out, 91, average);
    put16(&mut out, 93, max_width);
    put8(&mut out, 95, u8::try_from(first).unwrap_or(0));
    put8(&mut out, 96, u8::try_from(last).unwrap_or(u8::MAX));
    put8(&mut out, 97, u8::try_from(default_raw.saturating_sub(first)).unwrap_or(0));
    put8(&mut out, 98, u8::try_from(break_raw.saturating_sub(first)).unwrap_or(0));
    let strip_bytes: u64 = widths.iter().map(|w| u64::from(*w)).sum::<u64>().div_ceil(16) * 2;
    put16(&mut out, 99, clamp16(i64::try_from(strip_bytes).unwrap_or(i64::MAX)));
    put32(&mut out, 105, u32::try_from(face_at).unwrap_or(0));
    put32(&mut out, 113, u32::try_from(bits_at).unwrap_or(0));
    put32(&mut out, 118, DFF_1COLOR | if fixed { DFF_FIXED } else { DFF_PROPORTIONAL });
    out.extend_from_slice(&table);
    out.extend_from_slice(&face);
    out.extend_from_slice(&bits);
    Ok(out)
}
