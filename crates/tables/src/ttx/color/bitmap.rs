//! TTX writer for the embedded bitmap tables: `EBLC` and `EBDT`, the colour `CBLC` and `CBDT`,
//! and the Apple `bloc` and `bdat` (M12.7).
//!
//! A location table (`EBLC CBLC bloc`) lists strikes and, in each, index subtables that give
//! where every glyph's bitmap lies in the data table (`EBDT CBDT bdat`). The data table cannot be
//! read without its location table. Images are written as hex (`rawimagedata`), the form `ttx`
//! writes by default. Ported from fontTools' `ttLib.tables` (MIT): `E_B_L_C_`, `E_B_D_T_`,
//! `C_B_L_C_`, `C_B_D_T_`, `_b_l_o_c`, `_b_d_a_t` and `BitmapGlyphMetrics`.

use std::collections::HashMap;

use crate::limits::MAX_LAYOUT_ITEMS;
use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

use super::super::core::glyph_name;
use super::super::var::{bytes_at, slice, u8_at, u16_at, u32_at};
use super::super::{XmlWriter, py_float};

const SBIT_LINE_METRICS: [&str; 12] = [
    "ascender",
    "descender",
    "widthMax",
    "caretSlopeNumerator",
    "caretSlopeDenominator",
    "caretOffset",
    "minOriginSB",
    "minAdvanceSB",
    "maxBeforeBL",
    "minAfterBL",
    "pad1",
    "pad2",
];

/// Field names of the big glyph metrics, and whether each is signed.
const BIG_METRICS: [(&str, bool); 8] = [
    ("height", false),
    ("width", false),
    ("horiBearingX", true),
    ("horiBearingY", true),
    ("horiAdvance", false),
    ("vertBearingX", true),
    ("vertBearingY", true),
    ("vertAdvance", false),
];

const SMALL_METRICS: [(&str, bool); 5] = [("height", false), ("width", false), ("BearingX", true), ("BearingY", true), ("Advance", false)];

/// The location table a data table is read with.
pub(super) fn locator(tag: Tag) -> Option<Tag> {
    match &tag {
        b"EBDT" => Some(*b"EBLC"),
        b"CBDT" => Some(*b"CBLC"),
        b"bdat" => Some(*b"bloc"),
        _ => None,
    }
}

/// One index subtable: its header, and each glyph with the byte range of its image in the data
/// table.
struct IndexSubTable {
    index_format: u16,
    image_format: u16,
    first: u16,
    last: u16,
    /// Formats 2 and 5: the size of every image and the metrics they share.
    fixed: Option<(u32, [u8; 8])>,
    glyphs: Vec<(u32, (u64, u64))>,
}

struct Strike {
    /// The 48 bytes of the bitmap size table.
    size: Vec<u8>,
    subtables: Vec<IndexSubTable>,
}

struct Location {
    version: f64,
    strikes: Vec<Strike>,
}

fn version(data: &[u8]) -> Result<f64> {
    Ok(f64::from(u32_at(data, 0)? as i32) / 65536.0)
}

fn signed(b: u8, is_signed: bool) -> String {
    if is_signed { (b as i8).to_string() } else { b.to_string() }
}

fn read_location(data: &[u8]) -> Result<Location> {
    let version = version(data)?;
    let count = u32_at(data, 4)?;
    let mut strikes = Vec::new();
    let mut pos = 8usize;
    for _ in 0..count {
        let size = bytes_at(data, pos, 48)?.to_vec();
        pos = pos.saturating_add(48);
        strikes.push(Strike { size, subtables: Vec::new() });
    }
    let mut items = 0usize;
    for strike in &mut strikes {
        let array = u32_at(&strike.size, 0)? as usize;
        let subtables = u32_at(&strike.size, 8)?;
        for i in 0..subtables {
            let at = array.saturating_add(8usize.saturating_mul(i as usize));
            let first = u16_at(data, at)?;
            let last = u16_at(data, at.saturating_add(2))?;
            let header = array.saturating_add(u32_at(data, at.saturating_add(4))? as usize);
            let index_format = u16_at(data, header)?;
            let image_format = u16_at(data, header.saturating_add(2))?;
            let image_offset = u64::from(u32_at(data, header.saturating_add(4))?);
            let body = slice(data, header.saturating_add(8), data.len());
            let sub = read_index_subtable(body, index_format, image_format, image_offset, first, last)?;
            items = items.saturating_add(sub.glyphs.len());
            if items > MAX_LAYOUT_ITEMS {
                return Err(TablesError::TooLarge { what: "the glyphs of a bitmap location table", limit: MAX_LAYOUT_ITEMS });
            }
            strike.subtables.push(sub);
        }
    }
    Ok(Location { version, strikes })
}

fn big_metrics(data: &[u8], pos: usize) -> Result<[u8; 8]> {
    let mut m = [0u8; 8];
    m.copy_from_slice(bytes_at(data, pos, 8)?);
    Ok(m)
}

fn read_index_subtable(body: &[u8], index_format: u16, image_format: u16, image_offset: u64, first: u16, last: u16) -> Result<IndexSubTable> {
    let mut sub = IndexSubTable { index_format, image_format, first, last, fixed: None, glyphs: Vec::new() };
    let ids = u32::from(first)..=u32::from(last);
    match index_format {
        1 | 3 => {
            let size = if index_format == 1 { 4 } else { 2 };
            // One offset more than glyphs; none when the range is empty or reversed.
            let offsets = (i64::from(last) - i64::from(first) + 2).max(0) as usize;
            let mut values = Vec::with_capacity(offsets.min(body.len()));
            for i in 0..offsets {
                let at = i.saturating_mul(size);
                let v = if size == 4 { u64::from(u32_at(body, at)?) } else { u64::from(u16_at(body, at)?) };
                values.push(v + image_offset);
            }
            for (gid, pair) in ids.zip(values.windows(2)) {
                let (Some(&start), Some(&end)) = (pair.first(), pair.get(1)) else { continue };
                // A glyph whose range is empty has no bitmap (fontTools' `removeSkipGlyphs`).
                if start < end {
                    sub.glyphs.push((gid, (start, end)));
                }
            }
            if sub.glyphs.is_empty() {
                return Err(TablesError::invalid("a bitmap index subtable lists no glyph with an image"));
            }
        }
        2 => {
            let image_size = u32_at(body, 0)?;
            sub.fixed = Some((image_size, big_metrics(body, 4)?));
            for (i, gid) in ids.enumerate() {
                let start = u64::from(image_size) * i as u64 + image_offset;
                sub.glyphs.push((gid, (start, start + u64::from(image_size))));
            }
        }
        4 => {
            let count = u32_at(body, 0)?;
            let mut pairs = Vec::new();
            for i in 0..=u64::from(count) {
                let at = usize::try_from(4 + 4 * i).unwrap_or(usize::MAX);
                pairs.push((u16_at(body, at)?, u64::from(u16_at(body, at.saturating_add(2))?) + image_offset));
            }
            for pair in pairs.windows(2) {
                let (Some(&(gid, start)), Some(&(_, end))) = (pair.first(), pair.get(1)) else { continue };
                sub.glyphs.push((u32::from(gid), (start, end)));
            }
        }
        5 => {
            let image_size = u32_at(body, 0)?;
            sub.fixed = Some((image_size, big_metrics(body, 4)?));
            let count = u32_at(body, 12)?;
            for i in 0..u64::from(count) {
                let at = usize::try_from(16 + 2 * i).unwrap_or(usize::MAX);
                let gid = u16_at(body, at)?;
                let start = u64::from(image_size) * i + image_offset;
                sub.glyphs.push((u32::from(gid), (start, start + u64::from(image_size))));
            }
        }
        _ => return Err(TablesError::invalid(format!("unknown bitmap index subtable format {index_format}"))),
    }
    Ok(sub)
}

fn metrics_element(w: &mut XmlWriter, element: &str, fields: &[(&str, bool)], bytes: &[u8]) {
    w.open(element, &[]);
    for (i, (name, is_signed)) in fields.iter().enumerate() {
        let b = bytes.get(i).copied().unwrap_or(0);
        w.simple(name, &[("value", &signed(b, *is_signed))]);
    }
    w.close(element);
}

/// Writes a location table (`EBLC`, `CBLC`, `bloc`).
pub(super) fn write_location(data: &[u8], font: &SfntFont, w: &mut XmlWriter) -> Result<()> {
    let location = read_location(data)?;
    let order = w.glyph_order(font)?;
    w.simple("header", &[("version", &py_float(location.version))]);
    for (index, strike) in location.strikes.iter().enumerate() {
        w.open("strike", &[("index", &index.to_string())]);
        w.open("bitmapSizeTable", &[]);
        for (direction, at) in [("hori", 16usize), ("vert", 28)] {
            w.open("sbitLineMetrics", &[("direction", direction)]);
            for (i, name) in SBIT_LINE_METRICS.iter().enumerate() {
                let b = u8_at(&strike.size, at + i)?;
                w.simple(name, &[("value", &signed(b, *name != "widthMax"))]);
            }
            w.close("sbitLineMetrics");
        }
        let size = &strike.size;
        let fields: [(&str, String); 7] = [
            ("colorRef", u32_at(size, 12)?.to_string()),
            ("startGlyphIndex", u16_at(size, 40)?.to_string()),
            ("endGlyphIndex", u16_at(size, 42)?.to_string()),
            ("ppemX", u8_at(size, 44)?.to_string()),
            ("ppemY", u8_at(size, 45)?.to_string()),
            ("bitDepth", u8_at(size, 46)?.to_string()),
            ("flags", (u8_at(size, 47)? as i8).to_string()),
        ];
        for (name, value) in &fields {
            w.simple(name, &[("value", value)]);
        }
        w.close("bitmapSizeTable");
        w.comment("GlyphIds are written but not read. The firstGlyphIndex and\nlastGlyphIndex values will be recalculated by the compiler.");
        for sub in &strike.subtables {
            let element = format!("eblc_index_sub_table_{}", sub.index_format);
            let attrs =
                [("imageFormat", sub.image_format.to_string()), ("firstGlyphIndex", sub.first.to_string()), ("lastGlyphIndex", sub.last.to_string())];
            let attrs: Vec<(&str, &str)> = attrs.iter().map(|(k, v)| (*k, v.as_str())).collect();
            w.open(&element, &attrs);
            if let Some((image_size, metrics)) = &sub.fixed {
                w.simple("imageSize", &[("value", &image_size.to_string())]);
                metrics_element(w, "BigGlyphMetrics", &BIG_METRICS, metrics);
            }
            for (gid, _) in &sub.glyphs {
                // Keyword attributes, which fontTools writes sorted by name.
                w.simple("glyphLoc", &[("id", &gid.to_string()), ("name", &glyph_name(&order, *gid))]);
                w.check()?;
            }
            w.close(&element);
        }
        w.close("strike");
    }
    Ok(())
}

/// Writes a data table (`EBDT`, `CBDT`, `bdat`), read with the location table `locator_data`.
pub(super) fn write_data(tag: Tag, data: &[u8], locator_data: &[u8], font: &SfntFont, w: &mut XmlWriter) -> Result<()> {
    let header = version(data)?;
    let location = read_location(locator_data)?;
    let order = w.glyph_order(font)?;
    let color = &tag == b"CBDT";
    // A range read once keeps the image format it was first read with, across strikes.
    let mut formats: HashMap<(u64, u64), u16> = HashMap::new();
    w.simple("header", &[("version", &py_float(header))]);
    for (index, strike) in location.strikes.iter().enumerate() {
        // Glyph name to image, in the order names first appear; a later entry for the same name
        // replaces the image and keeps the place.
        let mut glyphs: Vec<(String, u16, (u64, u64))> = Vec::new();
        let mut place: HashMap<String, usize> = HashMap::new();
        for sub in &strike.subtables {
            for (gid, range) in &sub.glyphs {
                let format = *formats.entry(*range).or_insert(sub.image_format);
                let name = glyph_name(&order, *gid);
                match place.get(&name).and_then(|i| glyphs.get_mut(*i)) {
                    Some(entry) => {
                        entry.1 = format;
                        entry.2 = *range;
                    }
                    None => {
                        place.insert(name.clone(), glyphs.len());
                        glyphs.push((name, format, *range));
                    }
                }
            }
        }
        w.open("strikedata", &[("index", &index.to_string())]);
        for (name, format, (start, end)) in &glyphs {
            let start = usize::try_from(*start).unwrap_or(usize::MAX);
            let end = usize::try_from(*end).unwrap_or(usize::MAX);
            write_glyph(slice(data, start, end), *format, color, name, &order, w)?;
            w.check()?;
        }
        w.close("strikedata");
    }
    Ok(())
}

fn write_glyph(data: &[u8], format: u16, color: bool, name: &str, order: &[String], w: &mut XmlWriter) -> Result<()> {
    let (prefix, metrics, rest): (&str, Option<(&str, &[(&str, bool)])>, usize) = match format {
        1 | 2 | 8 => ("ebdt", Some(("SmallGlyphMetrics", SMALL_METRICS.as_slice())), 5),
        5 => ("ebdt", None, 0),
        6 | 7 | 9 => ("ebdt", Some(("BigGlyphMetrics", BIG_METRICS.as_slice())), 8),
        17 if color => ("cbdt", Some(("SmallGlyphMetrics", SMALL_METRICS.as_slice())), 5),
        18 if color => ("cbdt", Some(("BigGlyphMetrics", BIG_METRICS.as_slice())), 8),
        19 if color => ("cbdt", None, 0),
        _ => return Err(TablesError::invalid(format!("unknown bitmap image format {format}"))),
    };
    let metric_bytes = bytes_at(data, 0, rest)?;
    let body = slice(data, rest, data.len());
    let element = format!("{prefix}_bitmap_format_{format}");
    match format {
        8 | 9 => {
            // Format 8 has a pad byte between its metrics and its components.
            let body = if format == 8 { slice(body, 1, body.len()) } else { body };
            let count = usize::from(u16_at(body, 0)?);
            let mut components = Vec::with_capacity(count.min(body.len()));
            for i in 0..count {
                let at = 2 + 4 * i;
                components.push((u16_at(body, at)?, u8_at(body, at + 2)? as i8, u8_at(body, at + 3)? as i8));
            }
            w.open(&element, &[("name", name)]);
            if let Some((metrics_name, fields)) = metrics {
                metrics_element(w, metrics_name, fields, metric_bytes);
            }
            w.open("components", &[]);
            for (gid, x, y) in components {
                w.open("ebdtComponent", &[("name", &glyph_name(order, u32::from(gid)))]);
                w.simple("xOffset", &[("value", &x.to_string())]);
                w.simple("yOffset", &[("value", &y.to_string())]);
                w.close("ebdtComponent");
            }
            w.close("components");
            w.close(&element);
        }
        _ => {
            let image = if format >= 17 {
                let len = u32_at(body, 0)? as usize;
                let image = slice(body, 4, body.len());
                if len > image.len() {
                    return Err(TablesError::invalid(format!("a format {format} bitmap runs past its data")));
                }
                slice(image, 0, len)
            } else {
                body
            };
            w.open(&element, &[("name", name)]);
            if let Some((metrics_name, fields)) = metrics {
                metrics_element(w, metrics_name, fields, metric_bytes);
            }
            w.open("rawimagedata", &[]);
            w.hex(image);
            w.close("rawimagedata");
            w.close(&element);
        }
    }
    Ok(())
}
