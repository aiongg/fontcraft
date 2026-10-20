//! Classic Mac OS bitmap fonts: the `NFNT` and `FONT` resources of a resource fork. Reading only.
//!
//! Written from Apple's public Inside Macintosh volumes (Text: the bitmapped font record and the
//! font family record; More Macintosh Toolbox: the resource fork layout), Apple's AppleSingle and
//! AppleDouble format description, and the MacBinary II and III header descriptions. No code that
//! reads these formats was consulted.
//!
//! The resource fork is found in one of four containers: a plain file holding the fork itself (a
//! fork copied out, or a `.dfont`), an AppleSingle or AppleDouble file (its entry 2), or a
//! MacBinary II or III file (the fork after the data fork). A MacBinary file counts as one when
//! its header's checksum matches or it carries the MacBinary III signature.
//!
//! Each strike of one style is a resource. The first font family record (`FOND`) names the
//! family and lists its strikes by point size, style and resource id; the plain-style strikes are
//! read, the styled ones (bold, italic and the others) are left out with a warning. Without a
//! family record every bitmap font resource is read; an old `FONT` resource's id holds its point
//! size in its low seven bits. Mac bitmap fonts are drawn at 72 dots per inch, so a strike's
//! pixel size is its point size. Codes are Mac OS Roman and become Unicode code points.

use crate::binary::{Cursor, bit, code_name, malformed};
use crate::{
    Bitmap, BitmapError, BitmapFont, BitmapGlyph, Import, MAX_BITMAP_SIDE, MAX_FILE_BYTES, PixelBudget, Property, PropertyValue, Result, Warnings,
};

/// Most resource types read from a resource map.
pub const MAX_RESOURCE_TYPES: usize = 1_024;
/// Most bitmap font and font family resources read from one file.
pub const MAX_RESOURCES: usize = 4_096;
/// Most strikes read from one file; the rest are left out with a warning.
pub const MAX_MAC_STRIKES: usize = 64;

const APPLE_SINGLE: u32 = 0x0005_1600;
const APPLE_DOUBLE: u32 = 0x0005_1607;
const RESOURCE_FORK_ENTRY: u32 = 2;
const MACBINARY_HEADER: usize = 128;
const NFNT: [u8; 4] = *b"NFNT";
const FONT: [u8; 4] = *b"FONT";
const FOND: [u8; 4] = *b"FOND";
/// Bytes of a font family record before its font association table.
const FOND_HEADER: usize = 52;
/// An offset/width entry that marks a character the font lacks.
const NO_CHARACTER: u16 = 0xffff;
/// Mac bitmap fonts are drawn at this resolution, so their point size is their pixel size.
const MAC_DPI: u32 = 72;

/// Whether `bytes` hold a resource fork, plain or wrapped, with at least one `NFNT` or `FONT`
/// resource.
pub fn is_mac_font(bytes: &[u8]) -> bool {
    bytes.len() <= MAX_FILE_BYTES
        && fork(bytes).ok().and_then(|f| resources(f).ok()).is_some_and(|r| r.iter().any(|r| r.kind == NFNT || r.kind == FONT))
}

/// Reads every plain-style strike of the first font family a Mac file holds, smallest first. The
/// first import carries the warnings about the file as a whole.
pub fn read_mac(bytes: &[u8]) -> Result<Vec<Import>> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(BitmapError::TooLarge(bytes.len()));
    }
    let all = resources(fork(bytes)?)?;
    let fonts: Vec<&Resource<'_>> = all.iter().filter(|r| r.kind == NFNT || r.kind == FONT).collect();
    if fonts.is_empty() {
        return Err(malformed("Mac font", "the file holds no NFNT or FONT bitmap font resource".into()));
    }
    let mut warnings = Warnings::default();
    let mut family = String::new();
    let mut picks: Vec<(Option<u16>, &Resource<'_>)> = Vec::new();
    let families: Vec<&Resource<'_>> = all.iter().filter(|r| r.kind == FOND).collect();
    if let Some(fond) = families.first() {
        family = fond.name.clone().unwrap_or_default();
        let (mut styled, mut missing) = (0usize, 0usize);
        for (size, style, id) in fond_entries(fond.data)? {
            if size == 0 {
                continue;
            }
            if style != 0 {
                styled += 1;
                continue;
            }
            let found = fonts.iter().find(|r| r.id == id && r.kind == NFNT).or_else(|| fonts.iter().find(|r| r.id == id));
            match found {
                Some(r) => picks.push((Some(size), *r)),
                None => missing += 1,
            }
        }
        if families.len() > 1 {
            warnings.push(format!("the file holds {} font families; only the first, {family:?}, is read", families.len()));
        }
        if styled > 0 {
            warnings.push(format!("{styled} styled strikes (bold, italic and others) are left out; only the plain style is read"));
        }
        if missing > 0 {
            warnings.push(format!("the font family lists {missing} strikes the file does not hold"));
        }
    }
    if picks.is_empty() {
        for r in &fonts {
            let size = (r.kind == FONT).then_some((r.id as u16) & 0x7f);
            if size == Some(0) {
                // A `FONT` resource of size 0 only names its family.
                if family.is_empty() {
                    family = r.name.clone().unwrap_or_default();
                }
                continue;
            }
            picks.push((size, *r));
        }
        if family.is_empty() {
            family = fonts.iter().find_map(|r| r.name.clone()).unwrap_or_default();
        }
    }
    let mut budget = PixelBudget::default();
    let mut out: Vec<Import> = Vec::new();
    let (mut repeated, mut beyond) = (0usize, 0usize);
    for (size, r) in picks {
        if out.len() >= MAX_MAC_STRIKES {
            beyond += 1;
            continue;
        }
        let font = read_nfnt(r.data, size, &family, &mut budget, &mut warnings).map_err(|e| match e {
            BitmapError::Malformed { format, message } => BitmapError::Malformed { format, message: format!("resource {}: {message}", r.id) },
            other => other,
        })?;
        if out.iter().any(|i| i.font.pixel_size == font.pixel_size) {
            repeated += 1;
            continue;
        }
        out.push(Import { font, warnings: Vec::new() });
    }
    if repeated > 0 {
        warnings.push(format!("{repeated} strikes have a size an earlier strike has; the first was kept"));
    }
    if beyond > 0 {
        warnings.push(format!("{beyond} strikes beyond the limit of {MAX_MAC_STRIKES} are left out"));
    }
    out.sort_by_key(|i| i.font.pixel_size);
    if let Some(first) = out.first_mut() {
        first.warnings = warnings.finish();
    }
    Ok(out)
}

/// The resource fork in `bytes`: an AppleSingle or AppleDouble entry, a MacBinary file's fork,
/// else the bytes themselves.
fn fork(bytes: &[u8]) -> Result<&[u8]> {
    let mut c = Cursor::new(bytes, "AppleSingle");
    let magic = c.u32().unwrap_or(0);
    if magic == APPLE_SINGLE || magic == APPLE_DOUBLE {
        c.pos = 24;
        let entries = c.u16()?;
        for _ in 0..entries {
            let (id, offset, len) = (c.u32()?, c.u32()? as usize, c.u32()? as usize);
            if id == RESOURCE_FORK_ENTRY {
                return Cursor::at(bytes, offset, "AppleSingle").take(len);
            }
        }
        return Err(c.error("the file holds no resource fork"));
    }
    Ok(macbinary_fork(bytes).unwrap_or(bytes))
}

fn macbinary_fork(bytes: &[u8]) -> Option<&[u8]> {
    let header = bytes.get(..MACBINARY_HEADER)?;
    let byte = |at: usize| header.get(at).copied().unwrap_or(0xff);
    if byte(0) != 0 || byte(74) != 0 || byte(82) != 0 || !(1..=63).contains(&byte(1)) {
        return None;
    }
    let mut c = Cursor::at(header, 83, "MacBinary");
    let (data_len, fork_len) = (c.u32().ok()? as usize, c.u32().ok()? as usize);
    let signed = header.get(102..106) == Some(b"mBIN".as_slice());
    let crc = header.get(124..126).map(|b| b.iter().fold(0u16, |acc, b| (acc << 8) | u16::from(*b)));
    if !signed && crc != Some(crc16(header.get(..124)?)) {
        return None;
    }
    let secondary = usize::from(Cursor::at(header, 120, "MacBinary").u16().ok()?);
    let round = |n: usize| n.checked_next_multiple_of(MACBINARY_HEADER);
    let start = MACBINARY_HEADER.checked_add(round(secondary)?)?.checked_add(round(data_len)?)?;
    bytes.get(start..start.checked_add(fork_len)?)
}

/// The CRC-16 of a MacBinary II header (polynomial 0x1021, initial value 0).
fn crc16(bytes: &[u8]) -> u16 {
    let mut crc = 0u16;
    for b in bytes {
        crc ^= u16::from(*b) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
        }
    }
    crc
}

struct Resource<'a> {
    kind: [u8; 4],
    id: i16,
    name: Option<String>,
    data: &'a [u8],
}

/// The bitmap font and font family resources of a resource fork.
fn resources(fork: &[u8]) -> Result<Vec<Resource<'_>>> {
    const F: &str = "resource fork";
    let mut c = Cursor::new(fork, F);
    let (data_offset, map_offset, data_len, map_len) = (c.u32()? as usize, c.u32()? as usize, c.u32()? as usize, c.u32()? as usize);
    let data = Cursor::at(fork, data_offset, F).take(data_len)?;
    let map = Cursor::at(fork, map_offset, F).take(map_len)?;
    let mut m = Cursor::at(map, 24, F);
    let (type_list, name_list) = (usize::from(m.u16()?), usize::from(m.u16()?));
    let mut t = Cursor::at(map, type_list, F);
    // Counts are stored minus one, so 0xFFFF means none.
    let types = (usize::from(t.u16()?) + 1) % 0x1_0000;
    if types > MAX_RESOURCE_TYPES {
        return Err(BitmapError::Limit { what: "resource types", limit: MAX_RESOURCE_TYPES });
    }
    let mut out = Vec::new();
    for _ in 0..types {
        let kind: [u8; 4] = t.take(4)?.try_into().map_err(|_| t.error("a resource type is cut short"))?;
        let count = (usize::from(t.u16()?) + 1) % 0x1_0000;
        let refs = usize::from(t.u16()?);
        if kind != NFNT && kind != FONT && kind != FOND {
            continue;
        }
        if out.len() + count > MAX_RESOURCES {
            return Err(BitmapError::Limit { what: "font resources", limit: MAX_RESOURCES });
        }
        let mut r = Cursor::at(map, type_list.saturating_add(refs), F);
        for _ in 0..count {
            let id = r.i16()?;
            let name_offset = r.u16()?;
            let _attributes = r.u8()?;
            let offset = r.uint(3)? as usize;
            r.skip(4)?;
            let mut d = Cursor::at(data, offset, F);
            let len = d.u32()? as usize;
            let bytes = d.take(len)?;
            let name = (name_offset != 0xffff).then(|| pascal(map, name_list.saturating_add(usize::from(name_offset)))).flatten();
            out.push(Resource { kind, id, name, data: bytes });
        }
    }
    Ok(out)
}

/// A length-prefixed Mac OS Roman string.
fn pascal(bytes: &[u8], at: usize) -> Option<String> {
    let len = usize::from(*bytes.get(at)?);
    let text = bytes.get(at + 1..at + 1 + len)?;
    Some(text.iter().filter_map(|b| char::from_u32(mac_roman(*b))).filter(|c| !c.is_control()).collect())
}

/// The font association table of a font family record: point size, style and resource id.
fn fond_entries(data: &[u8]) -> Result<Vec<(u16, u16, i16)>> {
    let mut c = Cursor::at(data, FOND_HEADER, "FOND");
    let count = (usize::from(c.u16()?) + 1) % 0x1_0000;
    let mut out = Vec::with_capacity(count.min(data.len() / 6));
    for _ in 0..count {
        out.push((c.u16()?, c.u16()?, c.i16()?));
    }
    Ok(out)
}

/// One bitmap font record (Inside Macintosh: Text, the font record).
fn read_nfnt(data: &[u8], size: Option<u16>, family: &str, budget: &mut PixelBudget, warnings: &mut Warnings) -> Result<BitmapFont> {
    let mut c = Cursor::new(data, "NFNT");
    let font_type = c.u16()?;
    let (first, last) = (c.i16()?, c.i16()?);
    let _widest = c.i16()?;
    let kern_max = c.i16()?;
    let n_descent = c.i16()?;
    let _rect_width = c.i16()?;
    let rect_height = c.i16()?;
    let ow_location = c.u16()?;
    let (ascent, descent, _leading) = (c.i16()?, c.i16()?, c.i16()?);
    let row_words = c.u16()?;
    if (font_type >> 2) & 3 != 0 {
        return Err(c.error("it is a grey or colour font; only black-and-white Mac bitmap fonts are read"));
    }
    if first < 0 || last > 255 || first > last {
        return Err(c.error(format!("its character range {first}..{last} is not within 0..255")));
    }
    let Some(height) = u16::try_from(rect_height).ok().filter(|h| *h <= MAX_BITMAP_SIDE) else {
        return Err(c.error(format!("its font rectangle is {rect_height} pixels tall; the limit is {MAX_BITMAP_SIDE}")));
    };
    let row_bytes = usize::from(row_words) * 2;
    let image = c.take(row_bytes * usize::from(height))?;
    let count = usize::from((last - first) as u16) + 1;
    // One entry per character, one for the missing-character glyph and one that ends the table.
    let entries = count + 2;
    let locations = (0..entries).map(|_| c.u16()).collect::<Result<Vec<u16>>>()?;
    // A positive nDescent is the high word of the table's offset, in words from byte 16.
    let high = if n_descent > 0 { u64::from(n_descent as u16) << 16 } else { 0 };
    let ow_at = usize::try_from(16 + 2 * (high | u64::from(ow_location))).unwrap_or(usize::MAX);
    let mut ow = Cursor::at(data, ow_at, "NFNT");
    let offset_widths = (0..entries).map(|_| ow.u16()).collect::<Result<Vec<u16>>>()?;
    let row_bits = row_bytes * 8;
    let mut glyphs = Vec::with_capacity(count + 1);
    for i in 0..=count {
        let (Some(&entry), Some(&start), Some(&end)) = (offset_widths.get(i), locations.get(i), locations.get(i + 1)) else { break };
        if entry == NO_CHARACTER {
            continue;
        }
        let (start, end) = (usize::from(start), usize::from(end));
        if end < start || end > row_bits || end - start > usize::from(MAX_BITMAP_SIDE) {
            return Err(c.error(format!("character {i} past the first: its image location is out of range")));
        }
        let width = (end - start) as u16;
        budget.spend(crate::stride(width, 1).saturating_mul(usize::from(height)))?;
        let left = i32::from(kern_max) + i32::from(entry >> 8);
        let bottom = i32::from(ascent) - i32::from(height);
        let mut bitmap = Bitmap::blank(width, height, left, bottom, 1).ok_or_else(|| c.error("a character's box is too large"))?;
        for y in 0..height {
            for x in 0..width {
                if bit(image, usize::from(y) * row_bits + start + usize::from(x)) {
                    bitmap.set_pixel(x, y, 1);
                }
            }
        }
        let (encoding, name) = if i < count {
            let code = mac_roman((first as usize + i) as u8);
            (Some(code), code_name(code, true))
        } else {
            (None, ".notdef".to_string())
        };
        glyphs.push(BitmapGlyph { name, encoding, advance: i32::from(entry & 0xff), scalable_width: None, bitmap: trim_rows(bitmap) });
    }
    let pixel_size = match size {
        Some(size) => size,
        None => {
            let lines = (i32::from(ascent) + i32::from(descent)).clamp(1, i32::from(u16::MAX)) as u16;
            warnings.push(format!("a strike without a font family record is taken to be {lines} pixels per em, its ascent plus descent"));
            lines
        }
    };
    let properties =
        if family.is_empty() { Vec::new() } else { vec![Property { name: "FAMILY_NAME".into(), value: PropertyValue::Text(family.to_string()) }] };
    Ok(BitmapFont {
        name: family.to_string(),
        pixel_size,
        point_size: f64::from(pixel_size),
        resolution: (MAC_DPI, MAC_DPI),
        depth: 1,
        ascent: i32::from(ascent),
        descent: i32::from(descent),
        properties,
        glyphs,
    })
}

/// The bitmap without its blank rows above and below; a blank bitmap becomes 0 × 0.
fn trim_rows(b: Bitmap) -> Bitmap {
    let stride = b.stride();
    let blank = |y: usize| b.data.get(y * stride..(y + 1) * stride).is_none_or(|row| row.iter().all(|v| *v == 0));
    let rows = usize::from(b.height);
    let Some(top) = (0..rows).find(|y| !blank(*y)) else {
        return Bitmap { width: 0, height: 0, left: b.left, bottom: 0, depth: 1, data: Vec::new() };
    };
    let last = (0..rows).rev().find(|y| !blank(*y)).unwrap_or(top);
    let data = b.data.get(top * stride..(last + 1) * stride).map(<[u8]>::to_vec).unwrap_or_default();
    let height = (last + 1 - top) as u16;
    let bottom = b.bottom + (rows - 1 - last) as i32;
    Bitmap { width: b.width, height, left: b.left, bottom, depth: 1, data }
}

/// The Unicode code point of a Mac OS Roman byte.
fn mac_roman(byte: u8) -> u32 {
    match byte.checked_sub(0x80) {
        None => u32::from(byte),
        Some(high) => u32::from(MAC_ROMAN_HIGH.get(usize::from(high)).copied().unwrap_or(0xfffd)),
    }
}

/// Mac OS Roman 0x80..=0xFF as Unicode (the 0xDB euro sign is the 8.5 and later mapping).
const MAC_ROMAN_HIGH: [u16; 128] = [
    0x00C4, 0x00C5, 0x00C7, 0x00C9, 0x00D1, 0x00D6, 0x00DC, 0x00E1, 0x00E0, 0x00E2, 0x00E4, 0x00E3, 0x00E5, 0x00E7, 0x00E9, 0x00E8, //
    0x00EA, 0x00EB, 0x00ED, 0x00EC, 0x00EE, 0x00EF, 0x00F1, 0x00F3, 0x00F2, 0x00F4, 0x00F6, 0x00F5, 0x00FA, 0x00F9, 0x00FB, 0x00FC, //
    0x2020, 0x00B0, 0x00A2, 0x00A3, 0x00A7, 0x2022, 0x00B6, 0x00DF, 0x00AE, 0x00A9, 0x2122, 0x00B4, 0x00A8, 0x2260, 0x00C6, 0x00D8, //
    0x221E, 0x00B1, 0x2264, 0x2265, 0x00A5, 0x00B5, 0x2202, 0x2211, 0x220F, 0x03C0, 0x222B, 0x00AA, 0x00BA, 0x03A9, 0x00E6, 0x00F8, //
    0x00BF, 0x00A1, 0x00AC, 0x221A, 0x0192, 0x2248, 0x2206, 0x00AB, 0x00BB, 0x2026, 0x00A0, 0x00C0, 0x00C3, 0x00D5, 0x0152, 0x0153, //
    0x2013, 0x2014, 0x201C, 0x201D, 0x2018, 0x2019, 0x00F7, 0x25CA, 0x00FF, 0x0178, 0x2044, 0x20AC, 0x2039, 0x203A, 0xFB01, 0xFB02, //
    0x2021, 0x00B7, 0x201A, 0x201E, 0x2030, 0x00C2, 0x00CA, 0x00C1, 0x00CB, 0x00C8, 0x00CD, 0x00CE, 0x00CF, 0x00CC, 0x00D3, 0x00D4, //
    0xF8FF, 0x00D2, 0x00DA, 0x00DB, 0x00D9, 0x0131, 0x02C6, 0x02DC, 0x00AF, 0x02D8, 0x02D9, 0x02DA, 0x00B8, 0x02DD, 0x02DB, 0x02C7, //
];

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(b: &Bitmap) -> Vec<String> {
        (0..b.height).map(|y| (0..b.width).map(|x| if b.pixel(x, y) != 0 { '#' } else { '.' }).collect()).collect()
    }

    /// One character of a test font: its image rows (each the font rectangle's height; `None` for
    /// a character the font lacks), image offset from `kern_max` and advance.
    type Char<'a> = (Option<&'a [&'a str]>, u8, u8);

    /// An NFNT record with characters from `first`; the last of `chars` is the missing-character
    /// glyph. Ascent 3 and descent 1 make the font rectangle 4 rows tall; kernMax is -1.
    fn nfnt(first: i16, chars: &[Char<'_>]) -> Vec<u8> {
        let (ascent, descent, height) = (3i16, 1i16, 4usize);
        let mut locations = vec![0u16];
        for (image, ..) in chars {
            let width = image.and_then(|rows| rows.first()).map_or(0, |r| r.len()) as u16;
            locations.push(locations.last().unwrap() + width);
        }
        let row_bits = usize::from(*locations.last().unwrap());
        let row_words = row_bits.div_ceil(16);
        let mut image = vec![0u8; row_words * 2 * height];
        for (i, (rows, ..)) in chars.iter().enumerate() {
            for (y, row) in rows.unwrap_or(&[]).iter().enumerate() {
                for (x, c) in row.chars().enumerate() {
                    if c == '#' {
                        let bit = y * row_words * 16 + usize::from(locations[i]) + x;
                        image[bit / 8] |= 0x80 >> (bit % 8);
                    }
                }
            }
        }
        let last = first + chars.len() as i16 - 2;
        let mut out = Vec::new();
        let ow_location = (26 + image.len() + 2 * locations.len() - 16) / 2;
        for v in [
            0x9000u16,
            first as u16,
            last as u16,
            8,
            (-1i16) as u16,
            0,
            8,
            height as u16,
            ow_location as u16,
            ascent as u16,
            descent as u16,
            0,
            row_words as u16,
        ] {
            out.extend_from_slice(&v.to_be_bytes());
        }
        out.extend_from_slice(&image);
        for l in &locations {
            out.extend_from_slice(&l.to_be_bytes());
        }
        for (rows, offset, advance) in chars {
            let entry = if rows.is_some() { (u16::from(*offset) << 8) | u16::from(*advance) } else { NO_CHARACTER };
            out.extend_from_slice(&entry.to_be_bytes());
        }
        out.extend_from_slice(&NO_CHARACTER.to_be_bytes());
        out
    }

    /// A resource fork holding `resources` (type, id, name, data), each type's in the order given.
    fn fork_of(resources: &[([u8; 4], i16, Option<&str>, Vec<u8>)]) -> Vec<u8> {
        let mut data = Vec::new();
        let mut offsets = Vec::new();
        for (.., bytes) in resources {
            offsets.push(data.len());
            data.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
            data.extend_from_slice(bytes);
        }
        let mut kinds: Vec<[u8; 4]> = Vec::new();
        for (kind, ..) in resources {
            if !kinds.contains(kind) {
                kinds.push(*kind);
            }
        }
        let mut names = Vec::new();
        let mut type_list = ((kinds.len() as u16).wrapping_sub(1)).to_be_bytes().to_vec();
        let mut refs = Vec::new();
        let refs_start = 2 + 8 * kinds.len();
        for kind in &kinds {
            let members: Vec<usize> = (0..resources.len()).filter(|i| resources[*i].0 == *kind).collect();
            type_list.extend_from_slice(kind);
            type_list.extend_from_slice(&(members.len() as u16 - 1).to_be_bytes());
            type_list.extend_from_slice(&((refs_start + refs.len()) as u16).to_be_bytes());
            for i in members {
                let (_, id, name, _) = &resources[i];
                refs.extend_from_slice(&id.to_be_bytes());
                match name {
                    Some(n) => {
                        refs.extend_from_slice(&(names.len() as u16).to_be_bytes());
                        names.push(n.len() as u8);
                        names.extend_from_slice(n.as_bytes());
                    }
                    None => refs.extend_from_slice(&0xffffu16.to_be_bytes()),
                }
                refs.push(0);
                refs.extend_from_slice(&(offsets[i] as u32).to_be_bytes()[1..]);
                refs.extend_from_slice(&[0; 4]);
            }
        }
        let type_list_offset = 28usize;
        let name_list_offset = type_list_offset + type_list.len() + refs.len();
        let mut map = vec![0u8; 24];
        map.extend_from_slice(&(type_list_offset as u16).to_be_bytes());
        map.extend_from_slice(&(name_list_offset as u16).to_be_bytes());
        map.extend_from_slice(&type_list);
        map.extend_from_slice(&refs);
        map.extend_from_slice(&names);
        let data_offset = 256usize;
        let map_offset = data_offset + data.len();
        let mut out = Vec::new();
        for v in [data_offset, map_offset, data.len(), map.len()] {
            out.extend_from_slice(&(v as u32).to_be_bytes());
        }
        out.resize(data_offset, 0);
        out.extend_from_slice(&data);
        out.extend_from_slice(&map);
        out
    }

    /// A family record with font association entries (size, style, id).
    fn fond(entries: &[(u16, u16, i16)]) -> Vec<u8> {
        let mut out = vec![0u8; FOND_HEADER];
        out.extend_from_slice(&(entries.len() as u16 - 1).to_be_bytes());
        for (size, style, id) in entries {
            for v in [*size, *style, *id as u16] {
                out.extend_from_slice(&v.to_be_bytes());
            }
        }
        out
    }

    const A: &[&str] = &[".#.", "#.#", "###", "..."];
    const BAR: &[&str] = &["#", "#", "#", "#"];
    const BOX: &[&str] = &["##", "##", "##", ".."];

    /// A family of a plain 9-point strike (codes 0x41..=0x43, B absent), a bold one left out, and
    /// a plain 12-point one holding code 0x80 (Ä in Mac OS Roman).
    fn family_fork() -> Vec<u8> {
        let nine = nfnt(0x41, &[(Some(A), 1, 4), (None, 0, 0), (Some(BAR), 2, 3), (Some(BOX), 0, 3)]);
        let bold = nfnt(0x41, &[(Some(BOX), 0, 4), (Some(BOX), 0, 3)]);
        let twelve = nfnt(0x80, &[(Some(A), 1, 5), (Some(BOX), 0, 3)]);
        fork_of(&[
            (FOND, 128, Some("Tiny"), fond(&[(9, 0, 1000), (9, 1, 1001), (12, 0, 1002)])),
            (NFNT, 1002, None, twelve),
            (NFNT, 1000, None, nine),
            (NFNT, 1001, None, bold),
        ])
    }

    #[test]
    fn mac_reads_the_plain_strikes_of_a_family() {
        let bytes = family_fork();
        assert!(is_mac_font(&bytes));
        let strikes = read_mac(&bytes).unwrap();
        assert_eq!(strikes.iter().map(|s| s.font.pixel_size).collect::<Vec<_>>(), [9, 12]);
        assert_eq!(strikes[0].warnings.len(), 1, "{:?}", strikes[0].warnings);
        let nine = &strikes[0].font;
        assert_eq!(nine.text_property("FAMILY_NAME"), Some("Tiny"));
        assert_eq!((nine.ascent, nine.descent, nine.resolution), (3, 1, (72, 72)));
        let names: Vec<&str> = nine.glyphs.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["A", "C", ".notdef"]);
        let a = &nine.glyphs[0];
        assert_eq!(rows(&a.bitmap), [".#.", "#.#", "###"]);
        // kernMax -1 plus offset 1; the blank bottom row is trimmed, so the box sits on the baseline.
        assert_eq!((a.bitmap.left, a.bitmap.bottom, a.advance, a.encoding), (0, 0, 4, Some(0x41)));
        let c = &nine.glyphs[1];
        assert_eq!((c.bitmap.left, c.bitmap.bottom, c.bitmap.height), (1, -1, 4));
        assert_eq!(nine.glyphs[2].encoding, None);
        let twelve = &strikes[1].font;
        assert_eq!(twelve.glyphs[0].encoding, Some(0xC4));
        assert_eq!(twelve.glyphs[0].name, "uni00C4");
    }

    #[test]
    fn mac_reads_wrapped_forks() {
        let fork = family_fork();
        // AppleDouble: magic, version 2, 16 filler bytes, one entry (id 2) pointing at the fork.
        let mut double = Vec::new();
        double.extend_from_slice(&APPLE_DOUBLE.to_be_bytes());
        double.extend_from_slice(&0x0002_0000u32.to_be_bytes());
        double.extend_from_slice(&[0; 16]);
        double.extend_from_slice(&1u16.to_be_bytes());
        for v in [2u32, 38, fork.len() as u32] {
            double.extend_from_slice(&v.to_be_bytes());
        }
        double.extend_from_slice(&fork);
        // MacBinary II: a 128-byte header with a 4-byte data fork, then the fork at byte 256.
        let mut binary = vec![0u8; 128];
        binary[1] = 4;
        binary[2..6].copy_from_slice(b"Tiny");
        binary[83..87].copy_from_slice(&4u32.to_be_bytes());
        binary[87..91].copy_from_slice(&(fork.len() as u32).to_be_bytes());
        binary[122] = 129;
        binary[123] = 129;
        let crc = crc16(&binary[..124]);
        binary[124..126].copy_from_slice(&crc.to_be_bytes());
        binary.extend_from_slice(b"data");
        binary.resize(256, 0);
        binary.extend_from_slice(&fork);
        for bytes in [double, binary] {
            assert!(is_mac_font(&bytes));
            let strikes = read_mac(&bytes).unwrap();
            assert_eq!(strikes.len(), 2);
            assert_eq!(strikes[0].font.glyphs.len(), 3);
        }
    }

    #[test]
    fn mac_refuses_hostile_records() {
        // A location table entry past the end of the image rows.
        let mut record = nfnt(0x41, &[(Some(A), 1, 4), (Some(BOX), 0, 3)]);
        let locations = 26 + 2 * 4; // one 16-bit word per row, four rows
        record[locations + 2..locations + 4].copy_from_slice(&500u16.to_be_bytes());
        let bytes = fork_of(&[(NFNT, 1, None, record)]);
        let err = read_mac(&bytes).unwrap_err().to_string();
        assert!(err.contains("out of range"), "{err}");
        // An offset/width table beyond the record.
        let mut record = nfnt(0x41, &[(Some(A), 1, 4), (Some(BOX), 0, 3)]);
        record[16..18].copy_from_slice(&0x7fffu16.to_be_bytes());
        assert!(read_mac(&fork_of(&[(NFNT, 1, None, record)])).is_err());
        // A font rectangle taller than the limit.
        let mut record = nfnt(0x41, &[(Some(A), 1, 4), (Some(BOX), 0, 3)]);
        record[14..16].copy_from_slice(&5000u16.to_be_bytes());
        assert!(read_mac(&fork_of(&[(NFNT, 1, None, record)])).is_err());
        // A resource map whose data offset lies past the end of the file.
        let mut bytes = family_fork();
        bytes[0..4].copy_from_slice(&0xffff_fff0u32.to_be_bytes());
        assert!(!is_mac_font(&bytes));
        assert!(read_mac(&bytes).is_err());
        // Plain text is not a resource fork.
        assert!(!is_mac_font(b"STARTFONT 2.1\nFONT x\n"));
    }
}
