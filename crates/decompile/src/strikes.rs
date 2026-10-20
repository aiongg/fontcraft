//! Embedded bitmap strikes read into the model (M13.55d): the `EBLC` and `EBDT` tables, or the
//! Apple `bloc` and `bdat` tables that share their layout, become the font's bitmap strikes.
//!
//! - Every image format of the data table is read. Byte-aligned rows (formats 1 and 6) are kept
//!   as they are; bit-aligned images (2, 5 and 7) are cut into rows padded to a byte, the
//!   model's layout. Composite images (8 and 9) are flattened: each component's pixels are drawn
//!   at its offset from the composite's top-left corner, rows counting down, clipped to the
//!   composite's box; where two components overlap, the darker pixel wins.
//! - A strike's size is its vertical pixels per em, its depth its bit depth, and its ascent and
//!   descent come from its horizontal line metrics. Codes, names and X properties are not part
//!   of the tables; exporting a strike as BDF takes the codes from the glyphs.
//! - A glyph listed twice in one strike keeps its first image. A strike whose size and depth
//!   repeat an earlier one, a strike at a depth or size the model has no place for, and a strike
//!   that breaks a model limit ([`fontcraft_model::strike`]) are left out with a warning; so is
//!   every image that cannot be read.
//! - The colour tables (`CBLC`, `CBDT`) are not read here.
//!
//! The work is bounded by [`MAX_ENTRIES`] index entries and [`MAX_DECODED_BYTES`] bytes of
//! images decoded for all strikes together, kept or not, and composite images by
//! [`MAX_COMPOSITE_DEPTH`] levels and [`MAX_COMPOSITE_WORK`] pixels and components drawn. Past
//! any of these totals, no strike is kept. Images that several glyphs or strikes share (the same
//! data at the same depth and size) are decoded once.

use std::collections::HashMap;
use std::sync::Arc;

use fontcraft_model::strike::{self, BitmapStrike, MAX_ALL_STRIKES_BYTES, MAX_STRIKE_BYTES, MAX_STRIKES, StrikeGlyph};
use skrifa::raw::tables::bitmap::{BigGlyphMetrics, BitmapContent, BitmapDataFormat, BitmapLocation, BitmapMetrics, BitmapSize, IndexSubtable};
use skrifa::raw::tables::ebdt::Ebdt;
use skrifa::raw::tables::eblc::Eblc;
use skrifa::raw::types::Tag;
use skrifa::raw::{FontData, FontRead, FontRef, TableProvider};

use crate::Decompiled;

/// The tables [`import`] reads, which `lib.rs` leaves out of its "tables not read" warning.
pub const TABLES: &[&str] = &["EBLC", "EBDT", "bloc", "bdat"];
/// Most index entries (glyph slots of index subtables, and the subtables themselves) read for
/// all strikes together: 64 strikes of every glyph id.
pub const MAX_ENTRIES: usize = MAX_STRIKES * 65_536;
/// Most levels of composite images inside one another.
pub const MAX_COMPOSITE_DEPTH: usize = 8;
/// Most pixels and components drawn while flattening composite images, for the whole font.
pub const MAX_COMPOSITE_WORK: u64 = 64 << 20;
/// Most bytes of images decoded for the whole font, counting the images of strikes left out and
/// every copy of a shared image: room for every strike the model keeps, and one strike more.
pub const MAX_DECODED_BYTES: usize = MAX_ALL_STRIKES_BYTES + MAX_STRIKE_BYTES;
/// Most bytes of decoded images kept for reuse by glyphs and strikes that share them.
pub const MAX_IMAGE_CACHE_BYTES: usize = MAX_STRIKE_BYTES;

/// The location and data tables, as pairs of tags.
const PAIRS: [([u8; 4], [u8; 4]); 2] = [(*b"EBLC", *b"EBDT"), (*b"bloc", *b"bdat")];

/// Reads the embedded strikes of `sfnt` into `d.font`, adding what was left out to `d.warnings`.
pub(crate) fn import(sfnt: &FontRef, d: &mut Decompiled) {
    let table = |tag: &[u8; 4]| sfnt.table_data(Tag::new(tag));
    let Some((location, data, tag)) = PAIRS.iter().find_map(|(l, t)| Some((table(l)?, table(t)?, l))) else {
        for (l, t) in PAIRS {
            if table(&l).is_some() != table(&t).is_some() {
                let (has, lacks) = if table(&l).is_some() { (l, t) } else { (t, l) };
                d.warnings.push(format!(
                    "the bitmap strikes were not read: the font has a `{}` table but no `{}`",
                    String::from_utf8_lossy(&has),
                    String::from_utf8_lossy(&lacks)
                ));
            }
        }
        return;
    };
    let tag = String::from_utf8_lossy(tag).into_owned();
    let names: Vec<String> = d.font.glyphs.iter().map(|g| g.name.clone()).collect();
    let count = sfnt.maxp().map(|m| usize::from(m.num_glyphs())).unwrap_or(0);
    if names.len() != count {
        d.warnings.push(format!("the bitmap strikes of `{tag}` were not read: the glyphs read do not match the font's glyph count"));
        return;
    }
    match read(location, data, &names, &mut d.warnings) {
        Ok(strikes) if !strikes.is_empty() => d.font.bitmap_strikes = Arc::new(strikes),
        Ok(_) => {}
        Err(message) => d.warnings.push(format!("the bitmap strikes of `{tag}` were not read: {message}")),
    }
}

/// Why reading stopped: a limit for the whole font, or a problem with one image.
enum Stop {
    Limit(String),
    Image(String),
}

/// What identifies the pixels of an image that is not a composite: its data, and the depth and
/// size it is read at.
type ImageKey = (u16, usize, usize, u8, u8, u8);

/// The work done so far, against [`MAX_ENTRIES`], [`MAX_DECODED_BYTES`] and
/// [`MAX_COMPOSITE_WORK`], and the decoded images kept for reuse.
#[derive(Default)]
struct Budget {
    entries: usize,
    work: u64,
    decoded: usize,
    cache: HashMap<ImageKey, Vec<u8>>,
    cache_bytes: usize,
}

impl Budget {
    fn entry(&mut self) -> Result<(), Stop> {
        self.entries += 1;
        if self.entries > MAX_ENTRIES {
            return Err(Stop::Limit(format!("the location table lists more than {MAX_ENTRIES} entries")));
        }
        Ok(())
    }

    fn work(&mut self, n: u64) -> Result<(), Stop> {
        self.work = self.work.saturating_add(n);
        if self.work > MAX_COMPOSITE_WORK {
            return Err(Stop::Limit(format!("the composite images draw more than {MAX_COMPOSITE_WORK} pixels")));
        }
        Ok(())
    }

    fn decoded(&mut self, n: usize) -> Result<(), Stop> {
        self.decoded = self.decoded.saturating_add(n);
        if self.decoded > MAX_DECODED_BYTES {
            return Err(Stop::Limit(format!("the bitmap images decode to more than {MAX_DECODED_BYTES} bytes")));
        }
        Ok(())
    }

    fn keep(&mut self, key: ImageKey, data: &[u8]) {
        if self.cache_bytes.saturating_add(data.len()) <= MAX_IMAGE_CACHE_BYTES {
            self.cache_bytes += data.len();
            self.cache.insert(key, data.to_vec());
        }
    }
}

/// The strikes of a location table and its data table, given the font's glyph names by glyph
/// id. `Err` when a limit for the whole font is reached.
pub fn read(location: FontData<'_>, data: FontData<'_>, names: &[String], warnings: &mut Vec<String>) -> Result<Vec<BitmapStrike>, String> {
    let eblc = Eblc::read(location).map_err(|e| format!("the location table cannot be read: {e}"))?;
    let ebdt = Ebdt::read(data).map_err(|e| format!("the data table cannot be read: {e}"))?;
    let mut budget = Budget::default();
    let mut strikes: Vec<BitmapStrike> = Vec::new();
    let mut all_bytes = 0usize;
    for size in eblc.bitmap_sizes() {
        let label = strike::label(u16::from(size.ppem_y()), size.bit_depth());
        if strikes.len() >= MAX_STRIKES {
            warnings.push(format!("the font has more than {MAX_STRIKES} bitmap strikes; the first {MAX_STRIKES} were read"));
            break;
        }
        if strikes.iter().any(|s| s.ppem == u16::from(size.ppem_y()) && s.depth == size.bit_depth()) {
            warnings.push(format!("bitmap strike {label} was left out: an earlier strike has the same size and depth"));
            continue;
        }
        let (strike, skipped) = match read_strike(&eblc, &ebdt, size, names, &mut budget) {
            Ok(read) => read,
            Err(Stop::Limit(m)) => return Err(m),
            Err(Stop::Image(m)) => {
                warnings.push(format!("bitmap strike {label} was left out: {m}"));
                continue;
            }
        };
        if skipped > 0 {
            warnings.push(format!("bitmap strike {label}: {skipped} images could not be read and were left out"));
        }
        if strike.glyphs.is_empty() {
            warnings.push(format!("bitmap strike {label} was left out: it has no image for a glyph of the font"));
            continue;
        }
        if let Err(m) = strike.check() {
            warnings.push(format!("bitmap strike {label} was left out: {m}"));
            continue;
        }
        all_bytes = all_bytes.saturating_add(strike.bytes());
        if all_bytes > MAX_ALL_STRIKES_BYTES {
            warnings
                .push(format!("the bitmap strikes take more than {MAX_ALL_STRIKES_BYTES} bytes; strike {label} and the later ones were left out"));
            break;
        }
        strikes.push(strike);
    }
    Ok(strikes)
}

/// One strike, and how many of its images could not be read.
fn read_strike(eblc: &Eblc<'_>, ebdt: &Ebdt<'_>, size: &BitmapSize, names: &[String], budget: &mut Budget) -> Result<(BitmapStrike, usize), Stop> {
    let depth = size.bit_depth();
    if !strike::valid_depth(depth) {
        return Err(Stop::Image(format!("{depth} bits per pixel; a strike has 1, 2, 4 or 8")));
    }
    let ppem = u16::from(size.ppem_y());
    if ppem == 0 {
        return Err(Stop::Image("the size is 0 pixels per em".into()));
    }
    let list = size.index_subtable_list(eblc.offset_data()).map_err(|e| Stop::Image(format!("its index subtables cannot be read: {e}")))?;
    let mut locations: HashMap<u16, BitmapLocation> = HashMap::new();
    let mut order: Vec<u16> = Vec::new();
    for record in list.index_subtable_records() {
        budget.entry()?;
        let sub = record.index_subtable(list.offset_data()).map_err(|e| Stop::Image(format!("an index subtable cannot be read: {e}")))?;
        let (first, last) = (record.first_glyph_index().to_u16(), record.last_glyph_index().to_u16());
        for (gid, location) in subtable_locations(&sub, first, last, depth, budget)? {
            if location.data_size > 0 && !locations.contains_key(&gid) {
                order.push(gid);
                locations.insert(gid, location);
            }
        }
    }
    let mut out = BitmapStrike::new(ppem, depth);
    out.ascent = i32::from(size.hori().ascender());
    out.descent = -i32::from(size.hori().descender());
    let mut bytes = 0usize;
    let mut skipped = 0usize;
    for gid in order {
        let (Some(name), Some(location)) = (names.get(usize::from(gid)), locations.get(&gid)) else {
            skipped += 1;
            continue;
        };
        let image = match image(ebdt, &locations, location, depth, 0, budget) {
            Ok(image) => image,
            Err(Stop::Limit(m)) => return Err(Stop::Limit(m)),
            Err(Stop::Image(_)) => {
                skipped += 1;
                continue;
            }
        };
        bytes = bytes.saturating_add(image.data.len());
        if bytes > MAX_STRIKE_BYTES {
            return Err(Stop::Image(format!("its bitmaps take more than {MAX_STRIKE_BYTES} bytes")));
        }
        out.glyphs.push(StrikeGlyph {
            glyph: name.clone(),
            code: None,
            advance: i32::from(image.advance),
            scalable_width: None,
            width: u16::from(image.width),
            height: u16::from(image.height),
            left: i32::from(image.left),
            bottom: i32::from(image.top) - i32::from(image.height),
            data: image.data,
        });
    }
    Ok((out, skipped))
}

/// Each glyph an index subtable lists, with where its image lies in the data table. A glyph
/// whose image range is empty or does not fit an offset is listed with an empty range.
fn subtable_locations(sub: &IndexSubtable<'_>, first: u16, last: u16, depth: u8, budget: &mut Budget) -> Result<Vec<(u16, BitmapLocation)>, Stop> {
    let at = |format: u16, start: Option<usize>, end: Option<usize>, metrics: Option<BigGlyphMetrics>| {
        let (start, end) = (start.unwrap_or(0), end.unwrap_or(0));
        BitmapLocation { format, data_offset: start, data_size: end.saturating_sub(start), bit_depth: depth, metrics }
    };
    let ids = first..=last;
    let mut out = Vec::new();
    match sub {
        IndexSubtable::Format1(st) => {
            let base = st.image_data_offset() as usize;
            let offsets: Vec<usize> = st.sbit_offsets().iter().map(|o| o.get() as usize).collect();
            for (gid, pair) in ids.zip(offsets.windows(2)) {
                budget.entry()?;
                let (start, end) = (pair.first().and_then(|o| base.checked_add(*o)), pair.get(1).and_then(|o| base.checked_add(*o)));
                out.push((gid, at(st.image_format(), start, end, None)));
            }
        }
        IndexSubtable::Format3(st) => {
            let base = st.image_data_offset() as usize;
            let offsets: Vec<usize> = st.sbit_offsets().iter().map(|o| usize::from(o.get())).collect();
            for (gid, pair) in ids.zip(offsets.windows(2)) {
                budget.entry()?;
                let (start, end) = (pair.first().and_then(|o| base.checked_add(*o)), pair.get(1).and_then(|o| base.checked_add(*o)));
                out.push((gid, at(st.image_format(), start, end, None)));
            }
        }
        IndexSubtable::Format2(st) => {
            let (base, image) = (st.image_data_offset() as usize, st.image_size() as usize);
            for (i, gid) in ids.enumerate() {
                budget.entry()?;
                let start = i.checked_mul(image).and_then(|o| o.checked_add(base));
                out.push((gid, at(st.image_format(), start, start.and_then(|s| s.checked_add(image)), Some(*st.big_metrics()))));
            }
        }
        IndexSubtable::Format4(st) => {
            let base = st.image_data_offset() as usize;
            for pair in st.glyph_array().windows(2) {
                budget.entry()?;
                let (Some(this), Some(next)) = (pair.first(), pair.get(1)) else { continue };
                let start = base.checked_add(usize::from(this.sbit_offset()));
                let end = base.checked_add(usize::from(next.sbit_offset()));
                out.push((this.glyph_id().to_u16(), at(st.image_format(), start, end, None)));
            }
        }
        IndexSubtable::Format5(st) => {
            let (base, image) = (st.image_data_offset() as usize, st.image_size() as usize);
            for (i, gid) in st.glyph_array().iter().enumerate() {
                budget.entry()?;
                let start = i.checked_mul(image).and_then(|o| o.checked_add(base));
                out.push((gid.get().to_u16(), at(st.image_format(), start, start.and_then(|s| s.checked_add(image)), Some(*st.big_metrics()))));
            }
        }
    }
    Ok(out)
}

/// One image in the model's layout: rows top to bottom, each padded to a byte.
struct Image {
    width: u8,
    height: u8,
    left: i8,
    /// Pixels from the baseline up to the top row.
    top: i8,
    advance: u8,
    data: Vec<u8>,
}

fn image(
    ebdt: &Ebdt<'_>,
    locations: &HashMap<u16, BitmapLocation>,
    location: &BitmapLocation,
    depth: u8,
    level: usize,
    budget: &mut Budget,
) -> Result<Image, Stop> {
    let read = ebdt.data(location).map_err(|e| Stop::Image(e.to_string()))?;
    let (height, width, left, top, advance) = match &read.metrics {
        BitmapMetrics::Small(m) => (m.height(), m.width(), m.bearing_x(), m.bearing_y(), m.advance()),
        BitmapMetrics::Big(m) => (m.height(), m.width(), m.hori_bearing_x(), m.hori_bearing_y(), m.hori_advance()),
    };
    let stride = row_stride(width, depth);
    let len = stride * usize::from(height);
    // Every image is charged, composite or not and decoded or reused, so strikes left out and
    // shared images still count toward the font's total.
    budget.decoded(len)?;
    let key: ImageKey = (location.format, location.data_offset, location.data_size, depth, width, height);
    if !matches!(read.content, BitmapContent::Composite(_))
        && let Some(data) = budget.cache.get(&key)
    {
        return Ok(Image { width, height, left, top, advance, data: data.clone() });
    }
    let data = match read.content {
        BitmapContent::Data(BitmapDataFormat::ByteAligned, bytes) if bytes.len() == len => {
            let data = bytes.to_vec();
            budget.keep(key, &data);
            data
        }
        BitmapContent::Data(BitmapDataFormat::BitAligned, bytes) => {
            let mut out = vec![0u8; len];
            for y in 0..usize::from(height) {
                for x in 0..usize::from(width) {
                    let bit = (y * usize::from(width) + x) * usize::from(depth);
                    set_pixel(&mut out, stride, depth, x, y, pixel_at(bytes, bit, depth));
                }
            }
            budget.keep(key, &out);
            out
        }
        BitmapContent::Composite(components) => {
            if level >= MAX_COMPOSITE_DEPTH {
                return Err(Stop::Image(format!("composite images nest more than {MAX_COMPOSITE_DEPTH} deep")));
            }
            let mut out = vec![0u8; len];
            for c in components {
                budget.work(1)?;
                let part = locations.get(&(c.glyph_id().to_u16())).ok_or_else(|| Stop::Image("a component has no image".into()))?;
                let part = image(ebdt, locations, part, depth, level + 1, budget)?;
                budget.work(u64::from(part.width) * u64::from(part.height))?;
                let part_stride = row_stride(part.width, depth);
                for y in 0..usize::from(part.height) {
                    for x in 0..usize::from(part.width) {
                        let (tx, ty) = (x as i64 + i64::from(c.x_offset()), y as i64 + i64::from(c.y_offset()));
                        if tx < 0 || ty < 0 || tx >= i64::from(width) || ty >= i64::from(height) {
                            continue;
                        }
                        let value = pixel_at(&part.data, y * part_stride * 8 + x * usize::from(depth), depth);
                        let (tx, ty) = (tx as usize, ty as usize);
                        let have = pixel_at(&out, ty * stride * 8 + tx * usize::from(depth), depth);
                        set_pixel(&mut out, stride, depth, tx, ty, value.max(have));
                    }
                }
            }
            out
        }
        _ => return Err(Stop::Image("an image the format does not allow".into())),
    };
    Ok(Image { width, height, left, top, advance, data })
}

fn row_stride(width: u8, depth: u8) -> usize {
    (usize::from(width) * usize::from(depth)).div_ceil(8)
}

/// The `depth`-bit value that starts at bit `bit` of `data`, counting from the most significant
/// bit of the first byte; 0 past the end. `depth` divides 8 and `bit` is a multiple of it, so a
/// value never spans two bytes.
fn pixel_at(data: &[u8], bit: usize, depth: u8) -> u8 {
    let mask = ((1u16 << depth) - 1) as u8;
    let shift = 8 - u32::from(depth) - (bit % 8) as u32;
    data.get(bit / 8).map_or(0, |b| (b >> shift) & mask)
}

fn set_pixel(data: &mut [u8], stride: usize, depth: u8, x: usize, y: usize, value: u8) {
    let bit = x * usize::from(depth);
    let mask = ((1u16 << depth) - 1) as u8;
    let shift = 8 - u32::from(depth) - (bit % 8) as u32;
    if let Some(b) = data.get_mut(y * stride + bit / 8) {
        *b = (*b & !(mask << shift)) | ((value & mask) << shift);
    }
}

#[cfg(test)]
mod tests;
