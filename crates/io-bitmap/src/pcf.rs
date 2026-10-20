//! PCF: the X11 Portable Compiled Format, BDF's binary sibling.
//!
//! A PCF file is the magic `\x01fcp`, a table count and a table of contents (type, format, size,
//! offset; all least significant byte first), then the tables. Each table starts with its format
//! word, least significant byte first; the rest of the table uses the byte order the format
//! names. The format's low bits also say how bitmap rows are padded (1, 2, 4 or 8 bytes), the
//! bit order within a byte, and the unit whose bytes are swapped when byte and bit order differ.

use crate::bdf::{check_for_writing, clear_padding, glyph_extent, scalable_width};
use crate::{
    Bitmap, BitmapError, BitmapFont, BitmapGlyph, Import, MAX_BITMAP_SIDE, MAX_COORDINATE, MAX_FILE_BYTES, MAX_GLYPHS, MAX_PROPERTIES,
    MAX_STRING_BYTES, PixelBudget, Property, PropertyValue, Result, Warnings,
};

pub(crate) const MAGIC: &[u8] = b"\x01fcp";
/// Most entries the table of contents may list.
pub const MAX_TABLES: usize = 64;

const PROPERTIES: u32 = 1;
const ACCELERATORS: u32 = 1 << 1;
const METRICS: u32 = 1 << 2;
const BITMAPS: u32 = 1 << 3;
const ENCODINGS: u32 = 1 << 5;
const SWIDTHS: u32 = 1 << 6;
const GLYPH_NAMES: u32 = 1 << 7;
const BDF_ACCELERATORS: u32 = 1 << 8;

const COMPRESSED_METRICS: u32 = 0x100;
const BYTE_MSB: u32 = 1 << 2;
const BIT_MSB: u32 = 1 << 3;
const NO_GLYPH: u16 = 0xffff;

#[derive(Clone, Copy)]
struct Entry {
    kind: u32,
    size: usize,
    offset: usize,
}

/// Reads a PCF file.
pub fn read_pcf(bytes: &[u8]) -> Result<Import> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(BitmapError::TooLarge(bytes.len()));
    }
    if crate::is_gzip(bytes) {
        return crate::read_gz(bytes);
    }
    if !bytes.starts_with(MAGIC) {
        return Err(BitmapError::NotPcf("the file does not start with the PCF signature".into()));
    }
    let not_pcf = |message: &str| BitmapError::NotPcf(message.into());
    let count = le_u32(bytes, 4).ok_or_else(|| not_pcf("the file ends inside its header"))? as usize;
    if count > MAX_TABLES {
        return Err(BitmapError::Limit { what: "PCF tables", limit: MAX_TABLES });
    }
    let mut toc = Vec::with_capacity(count);
    for i in 0..count {
        let at = 8 + 16 * i;
        let field = |n: usize| le_u32(bytes, at + 4 * n).map(|v| v as usize);
        let (Some(kind), Some(size), Some(offset)) = (field(0), field(2), field(3)) else {
            return Err(not_pcf("the file ends inside its table of contents"));
        };
        toc.push(Entry { kind: kind as u32, size, offset });
    }
    let find = |kind: u32| toc.iter().find(|e| e.kind == kind).copied();
    let open = |kind: u32, name: &'static str| find(kind).map(|e| Table::open(bytes, e, name)).transpose();

    let mut warnings = Warnings::default();
    let mut budget = PixelBudget::default();
    let properties = match open(PROPERTIES, "properties")? {
        Some(mut t) => read_properties(&mut t)?,
        None => Vec::new(),
    };
    let mut metrics_table = open(METRICS, "metrics")?.ok_or_else(|| not_pcf("the file has no metrics table"))?;
    let metrics = read_metrics(&mut metrics_table)?;
    let mut bitmaps_table = open(BITMAPS, "bitmaps")?.ok_or_else(|| not_pcf("the file has no bitmaps table"))?;
    let bitmaps = read_bitmaps(&mut bitmaps_table, &metrics, &mut budget)?;
    let encodings = match open(ENCODINGS, "encodings")? {
        Some(mut t) => read_encodings(&mut t, metrics.len(), &mut warnings)?,
        None => vec![None; metrics.len()],
    };
    let names = match open(GLYPH_NAMES, "glyph names")? {
        Some(mut t) => read_names(&mut t)?,
        None => Vec::new(),
    };
    if !names.is_empty() && names.len() != metrics.len() {
        warnings.push(format!("the file names {} glyphs; it holds {}", names.len(), metrics.len()));
    }
    let swidths = match open(SWIDTHS, "scalable widths")? {
        Some(mut t) => read_swidths(&mut t)?,
        None => Vec::new(),
    };
    let accelerators = match open(BDF_ACCELERATORS, "BDF accelerators")? {
        Some(t) => Some(t),
        None => open(ACCELERATORS, "accelerators")?,
    };
    let lines = match accelerators {
        Some(mut t) => Some(read_accelerators(&mut t)?),
        None => None,
    };

    let mut glyphs = Vec::with_capacity(metrics.len());
    for (index, (metric, bitmap)) in metrics.iter().zip(bitmaps).enumerate() {
        let name = names.get(index).filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| format!("glyph{index}"));
        glyphs.push(BitmapGlyph {
            name,
            encoding: encodings.get(index).copied().flatten(),
            advance: metric.width,
            scalable_width: swidths.get(index).copied(),
            bitmap,
        });
    }

    let font = assemble(properties, glyphs, lines);
    Ok(Import { font, warnings: warnings.finish() })
}

fn assemble(properties: Vec<Property>, glyphs: Vec<BitmapGlyph>, lines: Option<(i32, i32)>) -> BitmapFont {
    let integer = |name: &str| {
        properties.iter().find(|p| p.name == name).and_then(|p| match p.value {
            PropertyValue::Integer(v) => Some(v),
            PropertyValue::Text(_) => None,
        })
    };
    let clamp = |v: i64| v.clamp(-i64::from(MAX_COORDINATE), i64::from(MAX_COORDINATE)) as i32;
    let resolution = |name: &str| integer(name).filter(|v| (1..=100_000).contains(v)).map_or(72, |v| v as u32);
    let (x_res, y_res) = (resolution("RESOLUTION_X"), resolution("RESOLUTION_Y"));
    let (ascent, descent) = match lines {
        Some(lines) => lines,
        None => {
            let (a, d) = glyph_extent(&glyphs);
            (integer("FONT_ASCENT").map_or(a, clamp), integer("FONT_DESCENT").map_or(d, clamp))
        }
    };
    let decipoints = integer("POINT_SIZE").filter(|v| *v > 0 && *v <= 10 * i64::from(MAX_COORDINATE));
    let point_size = decipoints.map_or(0.0, |v| v as f64 / 10.0);
    let pixel_size = crate::pixel_size(&properties, point_size, y_res, ascent, descent);
    let point_size = if point_size > 0.0 { point_size } else { f64::from(pixel_size) * 72.0 / f64::from(y_res) };
    let name = properties
        .iter()
        .find(|p| p.name == "FONT")
        .and_then(|p| match &p.value {
            PropertyValue::Text(v) => Some(v.clone()),
            PropertyValue::Integer(_) => None,
        })
        .unwrap_or_default();
    BitmapFont { name, pixel_size, point_size, resolution: (x_res, y_res), depth: 1, ascent, descent, properties, glyphs }
}

fn le_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([*b.first()?, *b.get(1)?, *b.get(2)?, *b.get(3)?]))
}

/// One table, read from just after its format word.
struct Table<'a> {
    name: &'static str,
    data: &'a [u8],
    pos: usize,
    format: u32,
}

impl<'a> Table<'a> {
    fn open(bytes: &'a [u8], entry: Entry, name: &'static str) -> Result<Table<'a>> {
        let end = entry.offset.checked_add(entry.size);
        let data = end.and_then(|end| bytes.get(entry.offset..end));
        let data = data.ok_or_else(|| BitmapError::Pcf { table: name, message: "it reaches past the file's end".into() })?;
        let format = le_u32(data, 0).ok_or_else(|| BitmapError::Pcf { table: name, message: "it is shorter than its format word".into() })?;
        Ok(Table { name, data, pos: 4, format })
    }

    fn error(&self, message: impl Into<String>) -> BitmapError {
        BitmapError::Pcf { table: self.name, message: message.into() }
    }

    fn msb(&self) -> bool {
        self.format & BYTE_MSB != 0
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).filter(|end| *end <= self.data.len());
        let slice = end.and_then(|end| self.data.get(self.pos..end));
        let slice = slice.ok_or_else(|| self.error("it ends early"))?;
        self.pos += n;
        Ok(slice)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let slice = self.take(N)?;
        let mut out = [0u8; N];
        out.copy_from_slice(slice);
        if self.msb() {
            out.reverse();
        }
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.array::<1>()?.first().copied().unwrap_or(0))
    }

    fn i16(&mut self) -> Result<i16> {
        Ok(i16::from_le_bytes(self.array()?))
    }

    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    /// A count between 0 and `limit`.
    fn count(&mut self, what: &'static str, limit: usize) -> Result<usize> {
        let n = self.i32()?;
        let n = usize::try_from(n).map_err(|_| self.error(format!("a negative count of {what}")))?;
        if n > limit {
            return Err(BitmapError::Limit { what, limit });
        }
        Ok(n)
    }

    /// A byte length within what is left of the table.
    fn length(&mut self, what: &str) -> Result<usize> {
        let n = self.i32()?;
        let left = self.data.len().saturating_sub(self.pos);
        usize::try_from(n).ok().filter(|n| *n <= left).ok_or_else(|| self.error(format!("{what} of {n} bytes does not fit")))
    }
}

/// The NUL-terminated string at `offset` of a string pool.
fn pool_string(table: &Table<'_>, pool: &[u8], offset: i32) -> Result<String> {
    let start = usize::try_from(offset).ok();
    let rest = start.and_then(|s| pool.get(s..)).ok_or_else(|| table.error("a string offset is past its pool"))?;
    let bytes = rest.split(|b| *b == 0).next().unwrap_or(&[]);
    if bytes.len() > MAX_STRING_BYTES {
        return Err(table.error(format!("a string is longer than {MAX_STRING_BYTES} bytes")));
    }
    Ok(match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => bytes.iter().map(|b| char::from(*b)).collect(),
    })
}

fn read_properties(t: &mut Table<'_>) -> Result<Vec<Property>> {
    let count = t.count("font properties", MAX_PROPERTIES)?;
    let mut raw = Vec::with_capacity(count);
    for _ in 0..count {
        let name = t.i32()?;
        let is_string = t.u8()? != 0;
        let value = t.i32()?;
        raw.push((name, is_string, value));
    }
    if !count.is_multiple_of(4) {
        t.take(4 - count % 4)?;
    }
    let size = t.length("the string pool")?;
    let pool = t.take(size)?;
    let mut properties = Vec::with_capacity(count);
    for (name, is_string, value) in raw {
        let name = pool_string(t, pool, name)?;
        let value = if is_string { PropertyValue::Text(pool_string(t, pool, value)?) } else { PropertyValue::Integer(i64::from(value)) };
        properties.push(Property { name, value });
    }
    Ok(properties)
}

/// Horizontal metrics of one glyph, in pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Metric {
    left: i32,
    right: i32,
    width: i32,
    ascent: i32,
    descent: i32,
}

fn read_metrics(t: &mut Table<'_>) -> Result<Vec<Metric>> {
    let compressed = t.format & COMPRESSED_METRICS != 0;
    let count = if compressed { usize::from(t.u16()?) } else { t.count("glyphs in one font", MAX_GLYPHS)? };
    let mut metrics = Vec::with_capacity(count);
    for _ in 0..count {
        let metric = if compressed {
            let mut v = [0i32; 5];
            for slot in &mut v {
                *slot = i32::from(t.u8()?) - 0x80;
            }
            let [left, right, width, ascent, descent] = v;
            Metric { left, right, width, ascent, descent }
        } else {
            let mut v = [0i32; 5];
            for slot in &mut v {
                *slot = i32::from(t.i16()?);
            }
            t.u16()?; // attributes
            let [left, right, width, ascent, descent] = v;
            Metric { left, right, width, ascent, descent }
        };
        metrics.push(metric);
    }
    Ok(metrics)
}

fn read_bitmaps(t: &mut Table<'_>, metrics: &[Metric], budget: &mut PixelBudget) -> Result<Vec<Bitmap>> {
    let count = t.count("glyphs in one font", MAX_GLYPHS)?;
    if count != metrics.len() {
        return Err(t.error(format!("it holds {count} glyphs; the metrics table {}", metrics.len())));
    }
    let mut offsets = Vec::with_capacity(count);
    for _ in 0..count {
        let offset = t.i32()?;
        offsets.push(usize::try_from(offset).map_err(|_| t.error("a negative glyph offset"))?);
    }
    let mut sizes = [0i32; 4];
    for size in &mut sizes {
        *size = t.i32()?;
    }
    let pad_index = (t.format & 3) as usize;
    let pad = 1usize << pad_index;
    let unit = 1usize << ((t.format >> 4) & 3);
    if unit > 4 {
        return Err(t.error("the scan unit is not 1, 2 or 4 bytes"));
    }
    let size = sizes.get(pad_index).copied().unwrap_or(0);
    let left = t.data.len().saturating_sub(t.pos);
    let size = usize::try_from(size).ok().filter(|s| *s <= left).ok_or_else(|| t.error(format!("bitmap data of {size} bytes does not fit")))?;
    let mut data = t.take(size)?.to_vec();
    let bit_msb = t.format & BIT_MSB != 0;
    if !bit_msb {
        for b in &mut data {
            *b = b.reverse_bits();
        }
    }
    if t.msb() != bit_msb && unit > 1 {
        for chunk in data.chunks_exact_mut(unit) {
            chunk.reverse();
        }
    }
    let mut bitmaps = Vec::with_capacity(count);
    for (index, (metric, start)) in metrics.iter().zip(offsets).enumerate() {
        let side = |v: i32| u16::try_from(v).ok().filter(|v| *v <= MAX_BITMAP_SIDE);
        let (Some(width), Some(height)) = (side(metric.right - metric.left), side(metric.ascent + metric.descent)) else {
            return Err(BitmapError::Limit { what: "bitmap width or height in pixels", limit: usize::from(MAX_BITMAP_SIDE) });
        };
        let mut bitmap = Bitmap::blank(width, height, metric.left, -metric.descent, 1)
            .ok_or_else(|| t.error(format!("glyph {index} has a bitmap the reader cannot hold")))?;
        budget.spend(bitmap.data.len())?;
        let row = bitmap.stride();
        let source_stride = row.div_ceil(pad) * pad;
        let rows = usize::from(height);
        let end = source_stride.checked_mul(rows).and_then(|n| n.checked_add(start));
        if end.is_none_or(|end| end > data.len()) {
            return Err(t.error(format!("glyph {index}'s bitmap reaches past the bitmap data")));
        }
        for y in 0..rows {
            let from = start + y * source_stride;
            let source = data.get(from..from + row);
            let target = bitmap.data.get_mut(y * row..(y + 1) * row);
            if let (Some(source), Some(target)) = (source, target) {
                target.copy_from_slice(source);
            }
        }
        clear_padding(&mut bitmap);
        bitmaps.push(bitmap);
    }
    Ok(bitmaps)
}

fn read_encodings(t: &mut Table<'_>, glyphs: usize, warnings: &mut Warnings) -> Result<Vec<Option<u32>>> {
    let min2 = t.i16()?;
    let max2 = t.i16()?;
    let min1 = t.i16()?;
    let max1 = t.i16()?;
    let _default_char = t.i16()?;
    let byte = |v: i16| u8::try_from(v).ok();
    let (Some(min2), Some(max2), Some(min1), Some(max1)) = (byte(min2), byte(max2), byte(min1), byte(max1)) else {
        return Err(t.error("a code range is outside 0 to 255"));
    };
    if min2 > max2 || min1 > max1 {
        return Err(t.error("a code range ends before it starts"));
    }
    let columns = u32::from(max2 - min2) + 1;
    let rows = u32::from(max1 - min1) + 1;
    let mut codes = vec![None; glyphs];
    let (mut out_of_range, mut extra) = (0usize, 0usize);
    for k in 0..columns * rows {
        let index = t.u16()?;
        if index == NO_GLYPH {
            continue;
        }
        let code = ((u32::from(min1) + k / columns) << 8) | (u32::from(min2) + k % columns);
        match codes.get_mut(usize::from(index)) {
            Some(slot) if slot.is_none() => *slot = Some(code),
            Some(_) => extra += 1,
            None => out_of_range += 1,
        }
    }
    if out_of_range > 0 {
        warnings.push(format!("{out_of_range} codes point at glyphs the font does not have"));
    }
    if extra > 0 {
        warnings.push(format!("{extra} codes point at a glyph that already has one; each glyph keeps its lowest"));
    }
    Ok(codes)
}

fn read_names(t: &mut Table<'_>) -> Result<Vec<String>> {
    let count = t.count("glyph names", MAX_GLYPHS)?;
    let mut offsets = Vec::with_capacity(count);
    for _ in 0..count {
        offsets.push(t.i32()?);
    }
    let size = t.length("the string pool")?;
    let pool = t.take(size)?;
    offsets.into_iter().map(|offset| pool_string(t, pool, offset)).collect()
}

fn read_swidths(t: &mut Table<'_>) -> Result<Vec<i32>> {
    let count = t.count("scalable widths", MAX_GLYPHS)?;
    let mut widths = Vec::with_capacity(count);
    for _ in 0..count {
        widths.push(t.i32()?);
    }
    Ok(widths)
}

/// The font's ascent and descent.
fn read_accelerators(t: &mut Table<'_>) -> Result<(i32, i32)> {
    t.take(8)?; // the flags, which follow from the metrics
    let clamp = |v: i32| v.clamp(-MAX_COORDINATE, MAX_COORDINATE);
    Ok((clamp(t.i32()?), clamp(t.i32()?)))
}

/// How [`write_pcf`] lays out a file. The default is the common X layout: rows padded to 4
/// bytes, most significant byte and bit first, compressed metrics where they fit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PcfLayout {
    /// Bytes each bitmap row is padded to: 1, 2, 4 or 8.
    pub glyph_pad: u8,
    /// Bytes swapped as one unit when byte and bit order differ: 1, 2 or 4, at most `glyph_pad`.
    pub scan_unit: u8,
    pub msb_byte_first: bool,
    pub msb_bit_first: bool,
    /// Five bytes per glyph's metrics when every value is between -128 and 127.
    pub compressed_metrics: bool,
}

impl Default for PcfLayout {
    fn default() -> Self {
        PcfLayout { glyph_pad: 4, scan_unit: 1, msb_byte_first: true, msb_bit_first: true, compressed_metrics: true }
    }
}

impl PcfLayout {
    fn format(&self, base: u32) -> u32 {
        let index = |v: u8| v.trailing_zeros() & 3;
        let mut format = base | index(self.glyph_pad) | (index(self.scan_unit) << 4);
        if self.msb_byte_first {
            format |= BYTE_MSB;
        }
        if self.msb_bit_first {
            format |= BIT_MSB;
        }
        format
    }
}

/// Writes a table's body in the layout's byte order.
struct Out {
    bytes: Vec<u8>,
    msb: bool,
}

impl Out {
    fn new(format: u32, msb: bool) -> Out {
        Out { bytes: format.to_le_bytes().to_vec(), msb }
    }

    fn put(&mut self, le: &[u8]) {
        if self.msb {
            self.bytes.extend(le.iter().rev());
        } else {
            self.bytes.extend_from_slice(le);
        }
    }

    fn i16(&mut self, v: i16) {
        self.put(&v.to_le_bytes());
    }

    fn u16(&mut self, v: u16) {
        self.put(&v.to_le_bytes());
    }

    fn i32(&mut self, v: i32) {
        self.put(&v.to_le_bytes());
    }

    fn len32(&mut self, n: usize) -> Result<()> {
        let n = i32::try_from(n).map_err(|_| BitmapError::Write("a table is too large".into()))?;
        self.i32(n);
        Ok(())
    }
}

/// Writes `font` as a PCF file. Only black-and-white fonts can be written. The `FONT`,
/// `PIXEL_SIZE`, `POINT_SIZE`, `RESOLUTION_X` and `RESOLUTION_Y` properties are added when the
/// font lacks them, since PCF keeps the name, sizes and resolution nowhere else. Codes above
/// 65,535 cannot be encoded; those glyphs are written without a code.
pub fn write_pcf(font: &BitmapFont, layout: PcfLayout) -> Result<Vec<u8>> {
    check_for_writing(font)?;
    let fail = |message: &str| Err(BitmapError::Write(message.into()));
    if font.depth != 1 {
        return fail("PCF holds black-and-white bitmaps only");
    }
    if !matches!(layout.glyph_pad, 1 | 2 | 4 | 8) || !matches!(layout.scan_unit, 1 | 2 | 4) {
        return fail("the row padding must be 1, 2, 4 or 8 bytes and the scan unit 1, 2 or 4");
    }
    if layout.scan_unit > layout.glyph_pad {
        return fail("the scan unit cannot be larger than the row padding");
    }
    let metrics: Vec<Metric> = font
        .glyphs
        .iter()
        .map(|g| {
            let b = &g.bitmap;
            Metric { left: b.left, right: b.left + i32::from(b.width), width: g.advance, ascent: b.bottom + i32::from(b.height), descent: -b.bottom }
        })
        .collect();
    let msb = layout.msb_byte_first;
    let mut tables: Vec<(u32, u32, Vec<u8>)> = Vec::new();

    let format = layout.format(0);
    tables.push((PROPERTIES, format, properties_table(font, format, msb)?));
    let accelerators = accelerators_table(font, &metrics, format, msb);
    tables.push((ACCELERATORS, format, accelerators.clone()));

    let fits_byte = |v: i32| (-128..=127).contains(&v);
    let compressed = layout.compressed_metrics
        && metrics.len() <= i16::MAX as usize
        && metrics.iter().all(|m| [m.left, m.right, m.width, m.ascent, m.descent].into_iter().all(fits_byte));
    let metrics_format = layout.format(if compressed { COMPRESSED_METRICS } else { 0 });
    let mut out = Out::new(metrics_format, msb);
    if compressed {
        out.u16(metrics.len() as u16);
        for m in &metrics {
            for v in [m.left, m.right, m.width, m.ascent, m.descent] {
                out.bytes.push((v + 0x80) as u8);
            }
        }
    } else {
        out.len32(metrics.len())?;
        for m in &metrics {
            for v in [m.left, m.right, m.width, m.ascent, m.descent] {
                out.i16(v as i16);
            }
            out.u16(0);
        }
    }
    tables.push((METRICS, metrics_format, out.bytes));
    tables.push((BITMAPS, format, bitmaps_table(font, layout, format)?));
    tables.push((ENCODINGS, format, encodings_table(font, format, msb)));

    let mut out = Out::new(format, msb);
    out.len32(font.glyphs.len())?;
    for g in &font.glyphs {
        out.i32(g.scalable_width.unwrap_or_else(|| scalable_width(font, g.advance)));
    }
    tables.push((SWIDTHS, format, out.bytes));

    let mut out = Out::new(format, msb);
    out.len32(font.glyphs.len())?;
    let mut pool = Vec::new();
    for (index, g) in font.glyphs.iter().enumerate() {
        let name = if g.name.is_empty() { format!("glyph{index}") } else { g.name.clone() };
        out.len32(pool.len())?;
        push_string(&mut pool, &name)?;
    }
    out.len32(pool.len())?;
    out.bytes.extend_from_slice(&pool);
    tables.push((GLYPH_NAMES, format, out.bytes));
    tables.push((BDF_ACCELERATORS, format, accelerators));

    let mut file = MAGIC.to_vec();
    file.extend_from_slice(&(tables.len() as u32).to_le_bytes());
    let mut offset = 8 + 16 * tables.len();
    let mut body = Vec::new();
    for (kind, format, mut bytes) in tables {
        bytes.resize(bytes.len().div_ceil(4) * 4, 0);
        for v in [kind, format, bytes.len() as u32, offset as u32] {
            file.extend_from_slice(&v.to_le_bytes());
        }
        offset += bytes.len();
        body.extend_from_slice(&bytes);
    }
    file.extend_from_slice(&body);
    if file.len() > MAX_FILE_BYTES {
        return fail("the font is too large for a PCF file the reader accepts");
    }
    Ok(file)
}

fn push_string(pool: &mut Vec<u8>, value: &str) -> Result<()> {
    if value.contains('\0') {
        return Err(BitmapError::Write(format!("{value:?} holds a NUL character")));
    }
    pool.extend_from_slice(value.as_bytes());
    pool.push(0);
    if pool.len() > MAX_FILE_BYTES {
        return Err(BitmapError::Write("the strings are too long".into()));
    }
    Ok(())
}

fn properties_table(font: &BitmapFont, format: u32, msb: bool) -> Result<Vec<u8>> {
    let mut properties = font.properties.clone();
    let has = |name: &str| font.properties.iter().any(|p| p.name == name);
    if !has("FONT") && !font.name.is_empty() {
        properties.push(Property { name: "FONT".into(), value: PropertyValue::Text(font.name.clone()) });
    }
    let decipoints = (font.point_size * 10.0).round();
    let extra = [
        ("PIXEL_SIZE", i64::from(font.pixel_size)),
        ("POINT_SIZE", if decipoints.is_finite() && decipoints >= 0.0 { decipoints as i64 } else { 0 }),
        ("RESOLUTION_X", i64::from(font.resolution.0)),
        ("RESOLUTION_Y", i64::from(font.resolution.1)),
    ];
    for (name, value) in extra {
        if !has(name) {
            properties.push(Property { name: name.into(), value: PropertyValue::Integer(value) });
        }
    }
    let mut out = Out::new(format, msb);
    out.len32(properties.len())?;
    let mut pool = Vec::new();
    for p in &properties {
        out.len32(pool.len())?;
        push_string(&mut pool, &p.name)?;
        match &p.value {
            PropertyValue::Integer(v) => {
                let v = i32::try_from(*v).map_err(|_| BitmapError::Write(format!("property {} is beyond a 32-bit integer", p.name)))?;
                out.bytes.push(0);
                out.i32(v);
            }
            PropertyValue::Text(v) => {
                out.bytes.push(1);
                out.len32(pool.len())?;
                push_string(&mut pool, v)?;
            }
        }
    }
    if !properties.len().is_multiple_of(4) {
        out.bytes.resize(out.bytes.len() + 4 - properties.len() % 4, 0);
    }
    out.len32(pool.len())?;
    out.bytes.extend_from_slice(&pool);
    Ok(out.bytes)
}

fn accelerators_table(font: &BitmapFont, metrics: &[Metric], format: u32, msb: bool) -> Vec<u8> {
    let first = metrics.first().copied().unwrap_or_default();
    let fold = |pick: fn(i32, i32) -> i32| {
        metrics.iter().fold(first, |a, m| Metric {
            left: pick(a.left, m.left),
            right: pick(a.right, m.right),
            width: pick(a.width, m.width),
            ascent: pick(a.ascent, m.ascent),
            descent: pick(a.descent, m.descent),
        })
    };
    let (min, max) = (fold(i32::min), fold(i32::max));
    let max_overlap = metrics.iter().map(|m| m.right - m.width).max().unwrap_or(0);
    let constant_metrics = metrics.iter().all(|m| *m == first);
    let constant_width = metrics.iter().all(|m| m.width == first.width);
    let terminal = constant_metrics && first.left == 0 && first.right == first.width && first.ascent == font.ascent && first.descent == font.descent;
    let flags = [
        u8::from(max_overlap <= min.left),
        u8::from(constant_metrics),
        u8::from(terminal),
        u8::from(constant_width),
        0, // ink inside
        0, // ink metrics
        0, // left to right
        0,
    ];
    let mut out = Out::new(format, msb);
    out.bytes.extend_from_slice(&flags);
    out.i32(font.ascent);
    out.i32(font.descent);
    out.i32(max_overlap);
    for m in [min, max] {
        for v in [m.left, m.right, m.width, m.ascent, m.descent] {
            out.i16(v as i16);
        }
        out.u16(0);
    }
    out.bytes
}

fn bitmaps_table(font: &BitmapFont, layout: PcfLayout, format: u32) -> Result<Vec<u8>> {
    let pad = usize::from(layout.glyph_pad);
    let source_stride = |b: &Bitmap, pad: usize| b.stride().div_ceil(pad) * pad;
    let mut sizes = [0usize; 4];
    for (i, size) in sizes.iter_mut().enumerate() {
        let p = 1usize << i;
        *size = font.glyphs.iter().map(|g| source_stride(&g.bitmap, p) * usize::from(g.bitmap.height)).sum();
    }
    let total = sizes.get(layout.glyph_pad.trailing_zeros() as usize).copied().unwrap_or(0);
    if total > MAX_FILE_BYTES {
        return Err(BitmapError::Write("the bitmaps are too large for a PCF file the reader accepts".into()));
    }
    let mut data = Vec::with_capacity(total);
    let mut offsets = Vec::with_capacity(font.glyphs.len());
    for g in &font.glyphs {
        let b = &g.bitmap;
        offsets.push(data.len());
        let stride = b.stride();
        let padded = source_stride(b, pad);
        for y in 0..usize::from(b.height) {
            let row = b.data.get(y * stride..(y + 1) * stride).unwrap_or(&[]);
            data.extend_from_slice(row);
            data.resize(data.len() + padded - row.len(), 0);
        }
    }
    let unit = usize::from(layout.scan_unit);
    if layout.msb_byte_first != layout.msb_bit_first && unit > 1 {
        for chunk in data.chunks_exact_mut(unit) {
            chunk.reverse();
        }
    }
    if !layout.msb_bit_first {
        for b in &mut data {
            *b = b.reverse_bits();
        }
    }
    let mut out = Out::new(format, layout.msb_byte_first);
    out.len32(font.glyphs.len())?;
    for offset in offsets {
        out.len32(offset)?;
    }
    for size in sizes {
        out.len32(size)?;
    }
    out.bytes.extend_from_slice(&data);
    Ok(out.bytes)
}

fn encodings_table(font: &BitmapFont, format: u32, msb: bool) -> Vec<u8> {
    let codes: Vec<(u16, u16)> = font
        .glyphs
        .iter()
        .enumerate()
        .filter_map(|(i, g)| Some((u16::try_from(g.encoding?).ok()?, u16::try_from(i).ok().filter(|i| *i != NO_GLYPH)?)))
        .collect();
    let low = |c: u16| c & 0xff;
    let high = |c: u16| c >> 8;
    let range = |f: &dyn Fn(u16) -> u16| {
        let min = codes.iter().map(|(c, _)| f(*c)).min().unwrap_or(0);
        let max = codes.iter().map(|(c, _)| f(*c)).max().unwrap_or(0);
        (min, max)
    };
    let (min2, max2) = range(&low);
    let (min1, max1) = range(&high);
    let columns = usize::from(max2 - min2) + 1;
    let rows = usize::from(max1 - min1) + 1;
    let mut cells = vec![NO_GLYPH; columns * rows];
    for (code, glyph) in codes {
        let cell = usize::from(high(code) - min1) * columns + usize::from(low(code) - min2);
        if let Some(slot) = cells.get_mut(cell)
            && *slot == NO_GLYPH
        {
            *slot = glyph;
        }
    }
    let default_char = font.integer_property("DEFAULT_CHAR").and_then(|v| u16::try_from(v).ok()).unwrap_or(NO_GLYPH);
    let mut out = Out::new(format, msb);
    for v in [min2, max2, min1, max1] {
        out.i16(v as i16);
    }
    out.u16(default_char);
    for cell in cells {
        out.u16(cell);
    }
    out.bytes
}
