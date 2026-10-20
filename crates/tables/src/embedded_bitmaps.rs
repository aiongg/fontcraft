//! The model's bitmap strikes as compiled tables (M13.55c): `EBDT` and `EBLC` for a font, added
//! to a compiled font or made into a bitmap-only sfnt.
//!
//! Choices, after the OpenType specification's `EBLC` and `EBDT` chapters:
//!
//! - Every image is format 1: small glyph metrics, then the rows top to bottom, each padded to a
//!   byte, pixels from the most significant bit. That is the model's own layout at every depth
//!   (1, 2, 4 or 8 bits), so the pixels are copied as they are. Small and big metrics hold the
//!   same horizontal ranges, so big metrics would add nothing.
//! - A strike's glyphs, in glyph order, are split into index subtables wherever more than
//!   [`MAX_GAP`] glyphs without a bitmap lie between two with one; inside a subtable those glyphs
//!   take an empty image. A subtable is format 3 (16-bit offsets) when its images take at most
//!   65,535 bytes, else format 1 (32-bit offsets).
//! - The vertical line metrics are zero and the strike flags say horizontal metrics only.
//!
//! The layout (data table first, each image once in subtable order; header, size tables, then
//! per strike its subtable array and subtables) is ported from fontTools' `E_B_D_T_` and
//! `E_B_L_C_` compile (MIT), so fontTools writes the same bytes from the same strikes.

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};

use fontcraft_model::strike::{BitmapStrike, StrikeGlyph};

use crate::limits::{MAX_BITMAP_SUBTABLES, MAX_SFNT};
use crate::sfnt::{Flavour, SfntFont, Tag};
use crate::{Result, TablesError};

/// Most glyphs without a bitmap between two glyphs with one that still share an index
/// subtable. An empty entry costs 2 bytes in a format 3 subtable; a new subtable costs 18 or
/// more.
pub const MAX_GAP: u32 = 8;
/// Most glyph names a result lists as left out.
pub const MAX_LISTED: usize = 100;
/// Largest size an embedded strike can state, in pixels per em (`ppemX` is one byte).
pub const MAX_EMBEDDED_PPEM: u16 = 255;

const VERSION: [u8; 4] = [0, 2, 0, 0];
const HEADER: usize = 8;
const SIZE_TABLE: usize = 48;
/// `bitmapSizeTable.flags`: the strike has horizontal metrics.
const HORIZONTAL_METRICS: u8 = 1;

/// The tables of a bitmap-only sfnt besides `glyf`, `loca`, `EBDT` and `EBLC`.
const BITMAP_ONLY_TABLES: [Tag; 8] = [*b"head", *b"hhea", *b"hmtx", *b"maxp", *b"cmap", *b"name", *b"OS/2", *b"post"];

/// `EBDT` and `EBLC` for some strikes, and what went into them.
#[derive(Clone, Debug, PartialEq)]
pub struct BitmapTables {
    pub ebdt: Vec<u8>,
    pub eblc: Vec<u8>,
    /// The strikes written, in table order.
    pub strikes: Vec<WrittenStrike>,
    /// Strikes left out because none of their bitmaps belongs to a glyph of the font.
    pub empty: Vec<String>,
    /// Names of strike glyphs the font has no glyph for, each once, at most [`MAX_LISTED`].
    pub left_out: Vec<String>,
    /// How many names were left out, listed or not.
    pub left_out_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WrittenStrike {
    pub label: String,
    pub glyphs: usize,
}

/// Glyph ids of a compiled font by glyph name, and by code point for glyphs whose name the font
/// does not keep.
#[derive(Clone, Debug, Default)]
pub struct GlyphIds {
    by_name: HashMap<String, u16>,
    by_code: HashMap<u32, u16>,
}

impl GlyphIds {
    /// The ids of a glyph order, such as the one a compile reports.
    pub fn from_order(order: &[String]) -> Self {
        let mut by_name = HashMap::with_capacity(order.len());
        for (i, name) in order.iter().enumerate() {
            if let Ok(id) = u16::try_from(i) {
                by_name.entry(name.clone()).or_insert(id);
            }
        }
        GlyphIds { by_name, by_code: HashMap::new() }
    }

    /// The ids of a compiled font: the names fontTools gives its glyphs, and its Unicode `cmap`
    /// subtables.
    pub fn from_font(font: &SfntFont) -> Result<Self> {
        let order = crate::ttx::glyph_order(font)?;
        let mut ids = Self::from_order(&order);
        if let Some(data) = font.get(*b"cmap")
            && let Ok((_, subtables)) = crate::ttx::core::cmap::parse(data)
        {
            for st in subtables.iter().filter(|s| s.is_unicode()) {
                for &(code, gid) in st.map() {
                    if let Ok(gid) = u16::try_from(gid) {
                        ids.by_code.entry(code).or_insert(gid);
                    }
                }
            }
        }
        Ok(ids)
    }

    /// The id of glyph `name`, else of the first of `codes` the font maps.
    pub fn get(&self, name: &str, codes: &[u32]) -> Option<u16> {
        self.by_name.get(name).copied().or_else(|| codes.iter().find_map(|c| self.by_code.get(c).copied()))
    }
}

/// One glyph's image as it goes into `EBDT`.
struct Placed<'a> {
    id: u16,
    glyph: &'a StrikeGlyph,
    /// Small glyph metrics: height, width, bearing x, bearing y, advance.
    metrics: [u8; 5],
}

/// One index subtable: its glyph range and the data-table range of each glyph it lists.
struct Subtable {
    first: u16,
    last: u16,
    locations: Vec<(u16, usize, usize)>,
}

/// `EBDT` and `EBLC` for `strikes`, each glyph placed at the id `glyph_id` gives it; glyphs
/// without one are left out and listed. Refused: a strike above [`MAX_EMBEDDED_PPEM`], a bitmap
/// whose size or offsets do not fit small glyph metrics, and strikes none of which has a glyph
/// of the font.
pub fn build(strikes: &[&BitmapStrike], glyph_id: &dyn Fn(&StrikeGlyph) -> Option<u16>) -> Result<BitmapTables> {
    let mut ebdt = VERSION.to_vec();
    let mut done: Vec<(&BitmapStrike, [u8; 12], Vec<Subtable>)> = Vec::with_capacity(strikes.len());
    let mut written = Vec::new();
    let mut empty = Vec::new();
    let mut left_out = Vec::new();
    let mut left_seen: HashSet<&str> = HashSet::new();
    let mut subtable_count = 0usize;
    for strike in strikes {
        let label = strike.label();
        strike.check().map_err(|m| TablesError::invalid(format!("strike {label}: {m}")))?;
        if strike.ppem > MAX_EMBEDDED_PPEM {
            return Err(TablesError::Unsupported(format!("strike {label}: embedded bitmaps go up to {MAX_EMBEDDED_PPEM} pixels per em")));
        }
        let mut placed: Vec<Placed<'_>> = Vec::with_capacity(strike.glyphs.len());
        let mut ids = HashSet::with_capacity(strike.glyphs.len());
        for g in &strike.glyphs {
            let Some(id) = glyph_id(g) else {
                if left_seen.insert(g.glyph.as_str()) {
                    left_out.push(g.glyph.as_str());
                }
                continue;
            };
            if ids.insert(id) {
                placed.push(Placed { id, glyph: g, metrics: small_metrics(g).map_err(|m| TablesError::invalid(format!("strike {label}: {m}")))? });
            }
        }
        if placed.is_empty() {
            empty.push(label);
            continue;
        }
        placed.sort_by_key(|p| p.id);
        let line = line_metrics(strike, &placed);
        let mut subtables: Vec<Subtable> = Vec::new();
        for p in &placed {
            let start = ebdt.len();
            ebdt.extend_from_slice(&p.metrics);
            ebdt.extend_from_slice(&p.glyph.data);
            if ebdt.len() > MAX_SFNT {
                return Err(TablesError::TooLarge { what: "a bitmap data table", limit: MAX_SFNT });
            }
            let location = (p.id, start, ebdt.len());
            match subtables.last_mut() {
                Some(sub) if u32::from(p.id).saturating_sub(u32::from(sub.last)) <= MAX_GAP + 1 => {
                    sub.last = p.id;
                    sub.locations.push(location);
                }
                _ => {
                    subtable_count += 1;
                    if subtable_count > MAX_BITMAP_SUBTABLES {
                        return Err(TablesError::TooLarge { what: "the index subtables of a bitmap location table", limit: MAX_BITMAP_SUBTABLES });
                    }
                    subtables.push(Subtable { first: p.id, last: p.id, locations: vec![location] });
                }
            }
        }
        written.push(WrittenStrike { label, glyphs: placed.len() });
        done.push((strike, line, subtables));
    }
    if done.is_empty() {
        return Err(TablesError::invalid("no strike has a bitmap for a glyph of the font"));
    }
    let eblc = location_table(&done)?;
    let left_out_count = left_out.len();
    Ok(BitmapTables {
        ebdt,
        eblc,
        strikes: written,
        empty,
        left_out: left_out.into_iter().take(MAX_LISTED).map(str::to_string).collect(),
        left_out_count,
    })
}

/// Height, width, bearing x, bearing y (pixels from the baseline up to the top row) and advance,
/// each in the byte small glyph metrics give it.
fn small_metrics(g: &StrikeGlyph) -> std::result::Result<[u8; 5], String> {
    let top = g.bottom.saturating_add(i32::from(g.height));
    let fits = |v: i32| i8::try_from(v).ok().map(|b| b.to_be_bytes()[0]);
    match (u8::try_from(g.height), u8::try_from(g.width), fits(g.left), fits(top), u8::try_from(g.advance)) {
        (Ok(h), Ok(w), Some(x), Some(y), Ok(a)) => Ok([h, w, x, y, a]),
        _ => Err(format!("glyph `{}` does not fit an embedded bitmap: sizes and the advance go up to 255 pixels, offsets from -128 to 127", g.glyph)),
    }
}

fn clamp_i8(v: i32) -> u8 {
    (v.clamp(-128, 127) as i8).to_be_bytes()[0]
}

/// The horizontal `sbitLineMetrics`: the strike's ascent and descent (from the bitmaps when
/// both are 0), the widest advance, an upright caret, and the extremes of the bitmaps that have
/// pixels.
fn line_metrics(strike: &BitmapStrike, placed: &[Placed<'_>]) -> [u8; 12] {
    let inked: Vec<&StrikeGlyph> = placed.iter().map(|p| p.glyph).filter(|g| g.width > 0 && g.height > 0).collect();
    let fold = |f: fn(&StrikeGlyph) -> i32, min: bool| {
        let values = inked.iter().map(|g| f(g));
        if min { values.min() } else { values.max() }.unwrap_or(0)
    };
    let min_origin = fold(|g| g.left, true);
    let min_advance = fold(|g| g.advance.saturating_sub(g.left).saturating_sub(i32::from(g.width)), true);
    let max_before = fold(|g| g.bottom.saturating_add(i32::from(g.height)), false);
    let min_after = fold(|g| g.bottom, true);
    let (ascender, descender) =
        if strike.ascent == 0 && strike.descent == 0 { (max_before.max(0), min_after.min(0)) } else { (strike.ascent, -strike.descent) };
    let width_max = placed.iter().map(|p| p.glyph.advance).max().unwrap_or(0).clamp(0, 255) as u8;
    [
        clamp_i8(ascender),
        clamp_i8(descender),
        width_max,
        1,
        0,
        0,
        clamp_i8(min_origin),
        clamp_i8(min_advance),
        clamp_i8(max_before),
        clamp_i8(min_after),
        0,
        0,
    ]
}

fn be32(out: &mut Vec<u8>, v: usize, what: &'static str) -> Result<()> {
    let v = u32::try_from(v).map_err(|_| TablesError::TooLarge { what, limit: u32::MAX as usize })?;
    out.extend_from_slice(&v.to_be_bytes());
    Ok(())
}

/// `EBLC` for the strikes and subtables [`build`] laid out.
fn location_table(strikes: &[(&BitmapStrike, [u8; 12], Vec<Subtable>)]) -> Result<Vec<u8>> {
    let mut sizes = Vec::with_capacity(strikes.len() * SIZE_TABLE);
    let mut tail = Vec::new();
    let mut at = HEADER + SIZE_TABLE * strikes.len();
    for (strike, line, subtables) in strikes {
        let array = at;
        let mut pairs = Vec::with_capacity(8 * subtables.len());
        let mut bodies = Vec::new();
        let mut body_at = 8 * subtables.len();
        for sub in subtables {
            let body = index_subtable(sub)?;
            pairs.extend_from_slice(&sub.first.to_be_bytes());
            pairs.extend_from_slice(&sub.last.to_be_bytes());
            be32(&mut pairs, body_at, "an index subtable offset")?;
            body_at += body.len();
            bodies.extend_from_slice(&body);
        }
        at += body_at;
        if at > MAX_SFNT {
            return Err(TablesError::TooLarge { what: "a bitmap location table", limit: MAX_SFNT });
        }
        let (Some(first), Some(last)) = (subtables.first(), subtables.last()) else {
            return Err(TablesError::invalid("a strike without index subtables"));
        };
        be32(&mut sizes, array, "an index subtable array offset")?;
        be32(&mut sizes, body_at, "the size of the index tables")?;
        be32(&mut sizes, subtables.len(), "the number of index subtables")?;
        sizes.extend_from_slice(&[0; 4]);
        sizes.extend_from_slice(line);
        sizes.extend_from_slice(&[0; 12]);
        sizes.extend_from_slice(&first.first.to_be_bytes());
        sizes.extend_from_slice(&last.last.to_be_bytes());
        let ppem = u8::try_from(strike.ppem).map_err(|_| TablesError::invalid(format!("strike {}: the size does not fit a byte", strike.label())))?;
        sizes.extend_from_slice(&[ppem, ppem, strike.depth, HORIZONTAL_METRICS]);
        tail.extend_from_slice(&pairs);
        tail.extend_from_slice(&bodies);
    }
    let mut out = Vec::with_capacity(at);
    out.extend_from_slice(&VERSION);
    be32(&mut out, strikes.len(), "the number of strikes")?;
    out.extend_from_slice(&sizes);
    out.extend_from_slice(&tail);
    Ok(out)
}

/// An index subtable of format 3, or 1 when its images span more than 65,535 bytes: one offset
/// per glyph of its range and one past the last image, a glyph without a bitmap at the offset
/// where the next image starts.
fn index_subtable(sub: &Subtable) -> Result<Vec<u8>> {
    let (Some(&(_, base, _)), Some(&(_, _, end))) = (sub.locations.first(), sub.locations.last()) else {
        return Err(TablesError::invalid("an index subtable without images"));
    };
    let span = end.saturating_sub(base);
    let short = span <= usize::from(u16::MAX);
    let mut offsets = Vec::with_capacity(usize::from(sub.last.saturating_sub(sub.first)) + 2);
    let mut queue = sub.locations.iter().peekable();
    let mut cursor = base;
    for id in sub.first..=sub.last {
        match queue.peek() {
            Some(&&(next, start, stop)) if next == id => {
                offsets.push(start.saturating_sub(base));
                cursor = stop;
                queue.next();
            }
            _ => offsets.push(cursor.saturating_sub(base)),
        }
    }
    offsets.push(span);
    let mut out = Vec::with_capacity(8 + offsets.len() * 4 + 2);
    out.extend_from_slice(&(if short { 3u16 } else { 1u16 }).to_be_bytes());
    out.extend_from_slice(&1u16.to_be_bytes());
    be32(&mut out, base, "an image data offset")?;
    for o in &offsets {
        if short {
            // `short` bounds every offset by the span.
            out.extend_from_slice(&u16::try_from(*o).unwrap_or(u16::MAX).to_be_bytes());
        } else {
            be32(&mut out, *o, "an image offset")?;
        }
    }
    if short && !offsets.len().is_multiple_of(2) {
        out.extend_from_slice(&[0, 0]);
    }
    Ok(out)
}

/// `font` with `tables` as its `EBDT` and `EBLC`, replacing any it had; true when it had them.
pub fn embed(font: &mut SfntFont, tables: &BitmapTables) -> bool {
    let had = font.get(*b"EBDT").is_some() || font.get(*b"EBLC").is_some();
    font.insert(*b"EBDT", tables.ebdt.clone());
    font.insert(*b"EBLC", tables.eblc.clone());
    had
}

/// A bitmap-only sfnt from a compiled TrueType font: its `head hhea hmtx maxp cmap name OS/2
/// post`, an empty `glyf` with a `loca` of zeros, and `tables`. `maxp` keeps the glyph count and
/// clears the outline maxima; `head` asks for short `loca` offsets.
pub fn bitmap_only(font: &SfntFont, tables: &BitmapTables) -> Result<SfntFont> {
    let mut out = SfntFont::new(Flavour::TrueType);
    for tag in BITMAP_ONLY_TABLES {
        if let Some(data) = font.get(tag) {
            out.insert(tag, data.to_vec());
        }
    }
    let count = crate::ttx::num_glyphs(font)?;
    let mut head = font.get(*b"head").ok_or_else(|| TablesError::MissingTable("head".into()))?.to_vec();
    match head.get_mut(50..54) {
        Some(formats) => formats.copy_from_slice(&[0, 0, 0, 0]),
        None => return Err(TablesError::invalid("the 'head' table is cut short")),
    }
    out.insert(*b"head", head);
    let mut maxp = vec![0u8; 32];
    if let Some(b) = maxp.get_mut(0..6) {
        b.copy_from_slice(&[0, 1, 0, 0, 0, 0]);
    }
    if let (Some(b), Ok(n)) = (maxp.get_mut(4..6), u16::try_from(count)) {
        b.copy_from_slice(&n.to_be_bytes());
    }
    // maxZones: 1, no twilight zone.
    if let Some(b) = maxp.get_mut(14..16) {
        b.copy_from_slice(&[0, 1]);
    }
    out.insert(*b"maxp", maxp);
    out.insert(*b"glyf", Vec::new());
    out.insert(*b"loca", vec![0; count.saturating_add(1).saturating_mul(2)]);
    embed(&mut out, tables);
    Ok(out)
}
