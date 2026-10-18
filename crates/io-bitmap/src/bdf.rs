//! BDF: Adobe's Glyph Bitmap Distribution Format (technical note 5005), a line-based text file.
//! Grey fonts state their bits per pixel as a fourth `SIZE` value and write each row with that
//! many bits per pixel; they are read and written as `STARTFONT 2.3`.

use std::fmt::Write as _;

use crate::{
    Bitmap, BitmapError, BitmapFont, BitmapGlyph, Import, MAX_BITMAP_SIDE, MAX_COORDINATE, MAX_GLYPHS, MAX_PROPERTIES, MAX_STRING_BYTES, PixelBudget,
    Property, PropertyValue, Result, Warnings, valid_depth,
};

/// Longest line the reader accepts, in bytes. A row of the widest grey bitmap takes 2,048 hex
/// digits; property strings may be longer.
pub const MAX_LINE_BYTES: usize = MAX_STRING_BYTES + 1_024;

/// Reads a BDF file.
pub fn read_bdf(bytes: &[u8]) -> Result<Import> {
    if bytes.len() > crate::MAX_FILE_BYTES {
        return Err(BitmapError::TooLarge(bytes.len()));
    }
    if !crate::is_bdf(bytes) {
        return Err(BitmapError::NotBdf("the file does not start with STARTFONT".into()));
    }
    let text = decode(bytes);
    let mut lines = Lines { iter: text.lines(), number: 0 };
    let mut reader = Reader::default();
    reader.header(&mut lines)?;
    reader.glyphs(&mut lines)?;
    reader.finish()
}

/// BDF is ASCII, but property strings in older files are often Latin-1.
fn decode(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF".as_slice()).unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => bytes.iter().map(|b| char::from(*b)).collect(),
    }
}

struct Lines<'a> {
    iter: std::str::Lines<'a>,
    number: usize,
}

impl<'a> Lines<'a> {
    /// The next line that is neither blank nor a comment, as its keyword and the rest.
    fn next(&mut self) -> Result<Option<(usize, &'a str, &'a str)>> {
        for line in self.iter.by_ref() {
            self.number += 1;
            if line.len() > MAX_LINE_BYTES {
                return Err(error(self.number, format!("the line is longer than {MAX_LINE_BYTES} bytes")));
            }
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let (key, rest) = line.split_once(|c: char| c.is_ascii_whitespace()).unwrap_or((line, ""));
            if key == "COMMENT" {
                continue;
            }
            return Ok(Some((self.number, key, rest.trim())));
        }
        Ok(None)
    }
}

fn error(line: usize, message: String) -> BitmapError {
    BitmapError::Bdf { line, message }
}

#[derive(Default)]
struct Reader {
    name: String,
    point_size: Option<f64>,
    resolution: (u32, u32),
    depth: u8,
    /// `FONTBOUNDINGBOX`: width, height, left, bottom.
    bounds: Option<(u16, u16, i32, i32)>,
    advance: Option<i32>,
    scalable_width: Option<i32>,
    properties: Vec<Property>,
    declared_glyphs: Option<usize>,
    glyphs: Vec<BitmapGlyph>,
    ended: bool,
    warnings: Warnings,
    budget: PixelBudget,
}

impl Reader {
    fn header(&mut self, lines: &mut Lines<'_>) -> Result<()> {
        self.resolution = (72, 72);
        self.depth = 1;
        match lines.next()? {
            Some((_, "STARTFONT", _)) => {}
            Some((line, key, _)) => return Err(error(line, format!("the file starts with {key}, not STARTFONT"))),
            None => return Err(BitmapError::NotBdf("the file is empty".into())),
        }
        while let Some((line, key, rest)) = lines.next()? {
            match key {
                "FONT" => self.name = text(line, rest)?,
                "SIZE" => {
                    let values = numbers(line, rest, 3)?;
                    let point_size = values.first().copied().unwrap_or(0.0);
                    let x = values.get(1).copied().unwrap_or(72.0);
                    let y = values.get(2).copied().unwrap_or(72.0);
                    if point_size <= 0.0 || point_size > f64::from(MAX_COORDINATE) {
                        return Err(error(line, format!("the point size {point_size} is out of range")));
                    }
                    self.point_size = Some(point_size);
                    self.resolution = (resolution(line, x)?, resolution(line, y)?);
                    if let Some(depth) = values.get(3) {
                        let depth = *depth as u8;
                        if !valid_depth(depth) || f64::from(depth) != values.get(3).copied().unwrap_or(0.0) {
                            return Err(error(line, "bits per pixel must be 1, 2, 4 or 8".into()));
                        }
                        self.depth = depth;
                    }
                }
                "FONTBOUNDINGBOX" => self.bounds = Some(bbx(line, rest)?),
                "SWIDTH" => self.scalable_width = Some(first_integer(line, rest)?),
                "DWIDTH" => self.advance = Some(first_integer(line, rest)?),
                "STARTPROPERTIES" => self.properties(lines, line, rest)?,
                "CHARS" => {
                    let count = rest.split_ascii_whitespace().next().and_then(|v| v.parse::<u64>().ok());
                    let count = count.ok_or_else(|| error(line, format!("CHARS needs a count, not {rest:?}")))?;
                    if count > MAX_GLYPHS as u64 {
                        return Err(BitmapError::Limit { what: "glyphs in one font", limit: MAX_GLYPHS });
                    }
                    self.declared_glyphs = Some(count as usize);
                    return Ok(());
                }
                "STARTCHAR" => return Err(error(line, "STARTCHAR before CHARS".into())),
                "ENDFONT" => {
                    self.ended = true;
                    return Ok(());
                }
                // METRICSSET, CONTENTVERSION and the vertical metrics are not kept.
                _ => {}
            }
        }
        Ok(())
    }

    fn properties(&mut self, lines: &mut Lines<'_>, line: usize, rest: &str) -> Result<()> {
        let declared = rest.split_ascii_whitespace().next().and_then(|v| v.parse::<u64>().ok());
        if declared.is_some_and(|n| n > MAX_PROPERTIES as u64) {
            return Err(BitmapError::Limit { what: "font properties", limit: MAX_PROPERTIES });
        }
        loop {
            let Some((line, key, rest)) = lines.next()? else {
                return Err(error(line, "STARTPROPERTIES has no ENDPROPERTIES".into()));
            };
            if key == "ENDPROPERTIES" {
                break;
            }
            if self.properties.len() >= MAX_PROPERTIES {
                return Err(BitmapError::Limit { what: "font properties", limit: MAX_PROPERTIES });
            }
            let name = text(line, key)?;
            self.properties.push(Property { name, value: property_value(line, rest)? });
        }
        if declared.is_some_and(|n| n != self.properties.len() as u64) {
            self.warnings.push(format!("STARTPROPERTIES states {} properties; the file holds {}", declared.unwrap_or(0), self.properties.len()));
        }
        Ok(())
    }

    fn glyphs(&mut self, lines: &mut Lines<'_>) -> Result<()> {
        if self.ended {
            return Ok(());
        }
        while let Some((line, key, rest)) = lines.next()? {
            match key {
                "STARTCHAR" => {
                    if self.glyphs.len() >= MAX_GLYPHS {
                        return Err(BitmapError::Limit { what: "glyphs in one font", limit: MAX_GLYPHS });
                    }
                    let glyph = self.glyph(lines, line, rest)?;
                    self.glyphs.push(glyph);
                }
                "ENDFONT" => {
                    self.ended = true;
                    return Ok(());
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn glyph(&mut self, lines: &mut Lines<'_>, start: usize, name: &str) -> Result<BitmapGlyph> {
        let mut name = text(start, name)?;
        if name.is_empty() {
            name = format!("glyph{}", self.glyphs.len());
        }
        let mut encoding = None;
        let mut advance = self.advance;
        let mut scalable_width = self.scalable_width;
        let mut bounds = None;
        let mut bitmap = None;
        loop {
            let Some((line, key, rest)) = lines.next()? else {
                return Err(error(start, format!("STARTCHAR {name} has no ENDCHAR")));
            };
            match key {
                "ENCODING" => {
                    let code = rest.split_ascii_whitespace().next().and_then(|v| v.parse::<i64>().ok());
                    let code = code.ok_or_else(|| error(line, format!("ENCODING needs a number, not {rest:?}")))?;
                    encoding = u32::try_from(code).ok();
                }
                "SWIDTH" => scalable_width = Some(first_integer(line, rest)?),
                "DWIDTH" => advance = Some(first_integer(line, rest)?),
                "BBX" => bounds = Some(bbx(line, rest)?),
                "BITMAP" => {
                    let Some((width, height, left, bottom)) = bounds.or(self.bounds) else {
                        return Err(error(line, format!("glyph {name} has no BBX")));
                    };
                    let mut pixels = Bitmap::blank(width, height, left, bottom, self.depth)
                        .ok_or_else(|| error(line, format!("glyph {name} has a bitmap the reader cannot hold")))?;
                    self.budget.spend(pixels.data.len())?;
                    read_rows(lines, &mut pixels, &name, line)?;
                    bitmap = Some(pixels);
                }
                "ENDCHAR" => break,
                "STARTCHAR" | "ENDFONT" => return Err(error(line, format!("{key} inside glyph {name}"))),
                _ => {}
            }
        }
        let bitmap = match bitmap {
            Some(bitmap) => bitmap,
            None => {
                let (width, height, left, bottom) = bounds.unwrap_or((0, 0, 0, 0));
                let blank = Bitmap::blank(width, height, left, bottom, self.depth)
                    .ok_or_else(|| error(start, format!("glyph {name} has a bitmap the reader cannot hold")))?;
                self.budget.spend(blank.data.len())?;
                blank
            }
        };
        let advance = match advance {
            Some(advance) => advance,
            None => {
                self.warnings.push(format!("glyph {name} has no DWIDTH; its advance is its bitmap's right edge"));
                (bitmap.left + i32::from(bitmap.width)).clamp(-MAX_COORDINATE, MAX_COORDINATE)
            }
        };
        Ok(BitmapGlyph { name, encoding, advance, scalable_width, bitmap })
    }

    fn finish(self) -> Result<Import> {
        let mut warnings = self.warnings;
        if !self.ended {
            warnings.push("the file ends without ENDFONT".into());
        }
        if let Some(declared) = self.declared_glyphs
            && declared != self.glyphs.len()
        {
            warnings.push(format!("CHARS states {declared} glyphs; the file holds {}", self.glyphs.len()));
        }
        let integer = |name: &str| {
            self.properties.iter().find(|p| p.name == name).and_then(|p| match p.value {
                PropertyValue::Integer(v) => i32::try_from(v).ok().map(|v| v.clamp(-MAX_COORDINATE, MAX_COORDINATE)),
                PropertyValue::Text(_) => None,
            })
        };
        let (bounds_ascent, bounds_descent) = match self.bounds {
            Some((_, height, _, bottom)) => (i32::from(height) + bottom, -bottom),
            None => glyph_extent(&self.glyphs),
        };
        let ascent = integer("FONT_ASCENT").unwrap_or(bounds_ascent);
        let descent = integer("FONT_DESCENT").unwrap_or(bounds_descent);
        let pixel_size = crate::pixel_size(&self.properties, self.point_size.unwrap_or(0.0), self.resolution.1, ascent, descent);
        let point_size = self.point_size.unwrap_or(f64::from(pixel_size) * 72.0 / f64::from(self.resolution.1.max(1)));
        let font = BitmapFont {
            name: self.name,
            pixel_size,
            point_size,
            resolution: self.resolution,
            depth: self.depth,
            ascent,
            descent,
            properties: self.properties,
            glyphs: self.glyphs,
        };
        Ok(Import { font, warnings: warnings.finish() })
    }
}

/// The highest top and lowest bottom of the glyphs' bitmaps, as ascent and descent.
pub(crate) fn glyph_extent(glyphs: &[BitmapGlyph]) -> (i32, i32) {
    let ascent = glyphs.iter().map(|g| g.bitmap.bottom + i32::from(g.bitmap.height)).max().unwrap_or(0);
    let descent = glyphs.iter().map(|g| -g.bitmap.bottom).max().unwrap_or(0);
    (ascent, descent)
}

fn read_rows(lines: &mut Lines<'_>, bitmap: &mut Bitmap, name: &str, start: usize) -> Result<()> {
    let stride = bitmap.stride();
    for row in 0..usize::from(bitmap.height) {
        let Some((line, key, rest)) = lines.next()? else {
            return Err(error(start, format!("glyph {name}: the file ends inside BITMAP")));
        };
        if key == "ENDCHAR" {
            return Err(error(line, format!("glyph {name}: BITMAP has {row} rows; BBX says {}", bitmap.height)));
        }
        if !rest.is_empty() {
            return Err(error(line, format!("glyph {name}: a bitmap row holds a space")));
        }
        // Rows may hold more digits than the width needs (padding); fewer are read as zeros.
        let digits = key.as_bytes();
        let start_byte = row * stride;
        for i in 0..stride {
            let high = digits.get(2 * i).map_or(Some(0), |d| hex(*d));
            let low = digits.get(2 * i + 1).map_or(Some(0), |d| hex(*d));
            let (Some(high), Some(low)) = (high, low) else {
                return Err(error(line, format!("glyph {name}: {key:?} is not a hexadecimal row")));
            };
            if let Some(b) = bitmap.data.get_mut(start_byte + i) {
                *b = (high << 4) | low;
            }
        }
        if digits.iter().skip(2 * stride).any(|d| hex(*d).is_none()) {
            return Err(error(line, format!("glyph {name}: {key:?} is not a hexadecimal row")));
        }
    }
    clear_padding(bitmap);
    Ok(())
}

/// Zeroes the bits past the last pixel of each row, so equal pixels mean equal data.
pub(crate) fn clear_padding(bitmap: &mut Bitmap) {
    let used = usize::from(bitmap.width) * usize::from(bitmap.depth) % 8;
    let stride = bitmap.stride();
    if used == 0 || stride == 0 {
        return;
    }
    let mask = 0xffu8 << (8 - used);
    for row in bitmap.data.chunks_mut(stride) {
        if let Some(last) = row.last_mut() {
            *last &= mask;
        }
    }
}

fn hex(digit: u8) -> Option<u8> {
    char::from(digit).to_digit(16).map(|d| d as u8)
}

fn text(line: usize, value: &str) -> Result<String> {
    if value.len() > MAX_STRING_BYTES {
        return Err(error(line, format!("a name or string is longer than {MAX_STRING_BYTES} bytes")));
    }
    Ok(value.to_owned())
}

/// At least `count` numbers, all finite.
fn numbers(line: usize, rest: &str, count: usize) -> Result<Vec<f64>> {
    let mut values = Vec::new();
    for word in rest.split_ascii_whitespace().take(8) {
        let value = word.parse::<f64>().ok().filter(|v| v.is_finite());
        values.push(value.ok_or_else(|| error(line, format!("{word:?} is not a number")))?);
    }
    if values.len() < count {
        return Err(error(line, format!("{count} numbers expected, not {rest:?}")));
    }
    Ok(values)
}

fn resolution(line: usize, value: f64) -> Result<u32> {
    if !(1.0..=100_000.0).contains(&value) {
        return Err(error(line, format!("the resolution {value} is out of range")));
    }
    Ok(value.round() as u32)
}

/// The first number of `rest`, rounded, within [`MAX_COORDINATE`].
fn first_integer(line: usize, rest: &str) -> Result<i32> {
    let value = numbers(line, rest, 1)?.first().copied().unwrap_or(0.0).round();
    coordinate(line, value)
}

fn coordinate(line: usize, value: f64) -> Result<i32> {
    if value.abs() > f64::from(MAX_COORDINATE) {
        return Err(error(line, format!("{value} is beyond ±{MAX_COORDINATE}")));
    }
    Ok(value as i32)
}

/// `BBX` or `FONTBOUNDINGBOX`: width, height, left, bottom.
fn bbx(line: usize, rest: &str) -> Result<(u16, u16, i32, i32)> {
    let values = numbers(line, rest, 4)?;
    let get = |i: usize| values.get(i).copied().unwrap_or(0.0).round();
    let side = |v: f64| {
        if (0.0..=f64::from(MAX_BITMAP_SIDE)).contains(&v) {
            Ok(v as u16)
        } else {
            Err(BitmapError::Limit { what: "bitmap width or height in pixels", limit: usize::from(MAX_BITMAP_SIDE) })
        }
    };
    Ok((side(get(0))?, side(get(1))?, coordinate(line, get(2))?, coordinate(line, get(3))?))
}

/// A property value: a quoted string (a doubled quote stands for one quote), an integer, or any
/// other text as it stands.
fn property_value(line: usize, rest: &str) -> Result<PropertyValue> {
    if let Some(quoted) = rest.strip_prefix('"') {
        let mut value = String::new();
        let mut chars = quoted.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                } else {
                    break;
                }
            }
            value.push(c);
        }
        return Ok(PropertyValue::Text(text(line, &value)?));
    }
    match rest.parse::<i64>() {
        Ok(v) => Ok(PropertyValue::Integer(v)),
        Err(_) => Ok(PropertyValue::Text(text(line, rest)?)),
    }
}

/// Writes `font` as a BDF file: version 2.1, or 2.3 for a grey font.
pub fn write_bdf(font: &BitmapFont) -> Result<String> {
    check_for_writing(font)?;
    if !font.point_size.is_finite() || font.point_size <= 0.0 {
        return Err(BitmapError::Write(format!("the point size {} is not a positive number", font.point_size)));
    }
    let mut out = String::new();
    let version = if font.depth == 1 { "2.1" } else { "2.3" };
    let name = if font.name.is_empty() { "unnamed" } else { font.name.as_str() };
    let _ = writeln!(out, "STARTFONT {version}");
    let _ = writeln!(out, "FONT {name}");
    let (x_res, y_res) = font.resolution;
    if font.depth == 1 {
        let _ = writeln!(out, "SIZE {} {x_res} {y_res}", font.point_size);
    } else {
        let _ = writeln!(out, "SIZE {} {x_res} {y_res} {}", font.point_size, font.depth);
    }
    let (width, height, left, bottom) = union_bounds(&font.glyphs);
    let _ = writeln!(out, "FONTBOUNDINGBOX {width} {height} {left} {bottom}");
    let has = |name: &str| font.properties.iter().any(|p| p.name == name);
    let extra = [("FONT_ASCENT", font.ascent), ("FONT_DESCENT", font.descent)];
    let extra: Vec<_> = extra.iter().filter(|(name, _)| !has(name)).collect();
    let _ = writeln!(out, "STARTPROPERTIES {}", font.properties.len() + extra.len());
    for property in &font.properties {
        match &property.value {
            PropertyValue::Integer(v) => {
                let _ = writeln!(out, "{} {v}", property.name);
            }
            PropertyValue::Text(v) => {
                let _ = writeln!(out, "{} \"{}\"", property.name, v.replace('"', "\"\""));
            }
        }
    }
    for (name, value) in extra {
        let _ = writeln!(out, "{name} {value}");
    }
    let _ = writeln!(out, "ENDPROPERTIES");
    let _ = writeln!(out, "CHARS {}", font.glyphs.len());
    for (index, glyph) in font.glyphs.iter().enumerate() {
        let b = &glyph.bitmap;
        if glyph.name.is_empty() {
            let _ = writeln!(out, "STARTCHAR glyph{index}");
        } else {
            let _ = writeln!(out, "STARTCHAR {}", glyph.name);
        }
        match glyph.encoding {
            Some(code) => {
                let _ = writeln!(out, "ENCODING {code}");
            }
            None => {
                let _ = writeln!(out, "ENCODING -1");
            }
        }
        let swidth = glyph.scalable_width.unwrap_or_else(|| scalable_width(font, glyph.advance));
        let _ = writeln!(out, "SWIDTH {swidth} 0");
        let _ = writeln!(out, "DWIDTH {} 0", glyph.advance);
        let _ = writeln!(out, "BBX {} {} {} {}", b.width, b.height, b.left, b.bottom);
        let _ = writeln!(out, "BITMAP");
        let stride = b.stride();
        for row in 0..usize::from(b.height) {
            // A row of a zero-width bitmap still takes a line.
            if stride == 0 {
                out.push_str("00");
            }
            for byte in b.data.iter().skip(row * stride).take(stride) {
                let _ = write!(out, "{byte:02X}");
            }
            out.push('\n');
        }
        let _ = writeln!(out, "ENDCHAR");
    }
    let _ = writeln!(out, "ENDFONT");
    Ok(out)
}

/// The advance in thousandths of the point size: pixels × 72,000 ÷ (points × dots per inch).
pub(crate) fn scalable_width(font: &BitmapFont, advance: i32) -> i32 {
    let denominator = font.point_size * f64::from(font.resolution.0);
    let value = (f64::from(advance) * 72_000.0 / denominator).round();
    if value.is_finite() && value.abs() <= f64::from(i32::MAX) { value as i32 } else { 0 }
}

/// The box around every glyph's bitmap: width, height, left, bottom.
pub(crate) fn union_bounds(glyphs: &[BitmapGlyph]) -> (i32, i32, i32, i32) {
    let boxes = glyphs.iter().map(|g| &g.bitmap).filter(|b| b.width > 0 && b.height > 0);
    let mut bounds: Option<(i32, i32, i32, i32)> = None;
    for b in boxes {
        let (l, bo, r, t) = (b.left, b.bottom, b.left + i32::from(b.width), b.bottom + i32::from(b.height));
        bounds = Some(match bounds {
            None => (l, bo, r, t),
            Some((l0, b0, r0, t0)) => (l0.min(l), b0.min(bo), r0.max(r), t0.max(t)),
        });
    }
    let (l, b, r, t) = bounds.unwrap_or((0, 0, 0, 0));
    (r - l, t - b, l, b)
}

/// What both writers need from a font: sizes and counts within the readers' limits, bitmaps of
/// the font's depth with the right amount of data, and names and strings a line can hold.
pub(crate) fn check_for_writing(font: &BitmapFont) -> Result<()> {
    let fail = |message: String| Err(BitmapError::Write(message));
    if !valid_depth(font.depth) {
        return fail(format!("bits per pixel {} is not 1, 2, 4 or 8", font.depth));
    }
    if font.glyphs.len() > MAX_GLYPHS {
        return fail(format!("the font has more than {MAX_GLYPHS} glyphs"));
    }
    if font.properties.len() > MAX_PROPERTIES {
        return fail(format!("the font has more than {MAX_PROPERTIES} properties"));
    }
    if !one_line(&font.name) {
        return fail("the font name holds a line break or is too long".into());
    }
    for property in &font.properties {
        let name_ok = !property.name.is_empty()
            && property.name.len() <= MAX_STRING_BYTES
            && !property.name.chars().any(|c| c.is_whitespace() || c.is_control() || c == '"');
        if !name_ok {
            return fail(format!("{:?} is not a property name", property.name));
        }
        if let PropertyValue::Text(v) = &property.value
            && !one_line(v)
        {
            return fail(format!("property {} holds a line break or is too long", property.name));
        }
    }
    let in_range = |v: i32| v.abs() <= MAX_COORDINATE;
    if !in_range(font.ascent) || !in_range(font.descent) {
        return fail("the ascent or descent is out of range".into());
    }
    for glyph in &font.glyphs {
        let b = &glyph.bitmap;
        if !one_line(&glyph.name) || glyph.name.chars().any(|c| c.is_whitespace()) {
            return fail(format!("{:?} is not a glyph name a bitmap font can hold", glyph.name));
        }
        if b.depth != font.depth {
            return fail(format!("glyph {} has {} bits per pixel; the font has {}", glyph.name, b.depth, font.depth));
        }
        if b.width > MAX_BITMAP_SIDE || b.height > MAX_BITMAP_SIDE {
            return fail(format!("glyph {}'s bitmap is larger than {MAX_BITMAP_SIDE} pixels", glyph.name));
        }
        if b.data.len() != b.stride() * usize::from(b.height) {
            return fail(format!("glyph {}'s bitmap data does not match its size", glyph.name));
        }
        let coordinates = [glyph.advance, b.left, b.bottom, b.left.saturating_add(i32::from(b.width)), b.bottom.saturating_add(i32::from(b.height))];
        if !coordinates.into_iter().all(in_range) {
            return fail(format!("glyph {}'s metrics are out of range", glyph.name));
        }
    }
    Ok(())
}

fn one_line(value: &str) -> bool {
    value.len() <= MAX_STRING_BYTES && !value.contains(['\n', '\r'])
}
