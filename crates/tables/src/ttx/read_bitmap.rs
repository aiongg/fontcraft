//! TTX reader: the embedded bitmap tables (`EBLC` with `EBDT`, `CBLC` with `CBDT`, the Apple
//! `bloc` with `bdat`), `EBSC` and `sbix` (M12.7c).
//!
//! A location table and its data table compile together, as in fontTools, where the data table
//! is compiled first and tells the location table where each image went: [`bitmap_pair`] takes
//! both elements. Images are read from `rawimagedata`, the form the writer and `ttx` write by
//! default, and from the row and bitwise forms of `ttx -z` (module `forms`); the external-file
//! form is refused. Ported from fontTools' `ttLib.tables` (MIT): `E_B_L_C_`, `E_B_D_T_`,
//! `C_B_L_C_`, `C_B_D_T_`, `E_B_S_C_`, `BitmapGlyphMetrics`, `_s_b_i_x`, `sbixStrike` and
//! `sbixGlyph`.

mod forms;
#[cfg(test)]
pub(crate) mod tests;

use std::collections::{BTreeMap, HashMap};

use crate::limits::{MAX_BITMAP_SUBTABLES, MAX_LAYOUT_ITEMS};
use crate::sfnt::Tag;
use crate::{Result, TablesError};

use super::Element;
use super::read::{Out, ReadContext, binary2num, float_lit, int_lit, read_hex};
use super::read_otdata::tag_bytes;

fn bad(what: impl Into<String>) -> TablesError {
    TablesError::invalid(what.into())
}

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

/// The big glyph metrics, and whether each is signed.
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

/// The location table that goes with data table `tag`, and the other way round.
pub(crate) fn partner(tag: Tag) -> Option<Tag> {
    match &tag {
        b"EBDT" => Some(*b"EBLC"),
        b"CBDT" => Some(*b"CBLC"),
        b"bdat" => Some(*b"bloc"),
        b"EBLC" => Some(*b"EBDT"),
        b"CBLC" => Some(*b"CBDT"),
        b"bloc" => Some(*b"bdat"),
        _ => None,
    }
}

/// Whether `tag` is a location table.
pub(crate) fn is_location(tag: Tag) -> bool {
    matches!(&tag, b"EBLC" | b"CBLC" | b"bloc")
}

/// A version as `16.16F` packs a `safeEval`ed value.
fn fixed_version(text: &str) -> Result<i64> {
    super::read::ot_round(float_lit(text)? * 65536.0)
}

fn value(e: &Element) -> Result<i64> {
    int_lit(e.attribute("value").ok_or_else(|| bad(format!("the element '{}' has no value", e.name)))?)
}

/// Named number fields from the `value` of child elements, the last of a name winning; every
/// field must be there, as `sstruct.pack` needs it.
fn fields(e: &Element, names: &[&str], what: &str) -> Result<Vec<i64>> {
    let mut out = Vec::with_capacity(names.len());
    for name in names {
        let child = e.children.iter().rev().find(|c| c.name == *name).ok_or_else(|| bad(format!("{what} has no '{name}'")))?;
        out.push(value(child)?);
    }
    Ok(out)
}

fn pack_signed(o: &mut Out, values: &[i64], signed: &[bool], what: &str) -> Result<()> {
    for (v, s) in values.iter().zip(signed) {
        if *s { o.i8(*v, what)? } else { o.u8(*v, what)? }
    }
    Ok(())
}

/// `sbitLineMetrics`: twelve bytes, `widthMax` unsigned.
fn line_metrics(e: &Element, o: &mut Out) -> Result<()> {
    let v = fields(e, &SBIT_LINE_METRICS, "a line metrics record")?;
    let signed: Vec<bool> = SBIT_LINE_METRICS.iter().map(|n| *n != "widthMax").collect();
    pack_signed(o, &v, &signed, "a line metric")
}

/// The `hori` and `vert` line metrics of a strike or scale table, packed.
fn both_line_metrics(e: &Element, what: &str) -> Result<[Vec<u8>; 2]> {
    let mut out: [Option<Vec<u8>>; 2] = [None, None];
    for m in e.children.iter().filter(|c| c.name == "sbitLineMetrics") {
        let slot = match m.attribute("direction") {
            Some("hori") => 0,
            Some("vert") => 1,
            other => return Err(bad(format!("{what} has line metrics of the direction {other:?}"))),
        };
        let mut o = Out::default();
        line_metrics(m, &mut o)?;
        if let Some(s) = out.get_mut(slot) {
            *s = Some(o.0);
        }
    }
    let [Some(hori), Some(vert)] = out else {
        return Err(bad(format!("{what} lacks its horizontal or vertical line metrics")));
    };
    Ok([hori, vert])
}

/// A metrics element (`BigGlyphMetrics` or `SmallGlyphMetrics`) of `e`, packed.
fn glyph_metrics(e: &Element, element: &str, layout: &[(&str, bool)]) -> Result<Option<Vec<u8>>> {
    let Some(m) = e.children.iter().rev().find(|c| c.name == element) else { return Ok(None) };
    let names: Vec<&str> = layout.iter().map(|(n, _)| *n).collect();
    let signed: Vec<bool> = layout.iter().map(|(_, s)| *s).collect();
    let v = fields(m, &names, "a glyph metrics record")?;
    let mut o = Out::default();
    pack_signed(&mut o, &v, &signed, "a glyph metric")?;
    Ok(Some(o.0))
}

// ── EBLC, CBLC, bloc ───────────────────────────────────────────────────────────────────────────

struct IndexSubTable {
    index_format: i64,
    image_format: i64,
    names: Vec<String>,
    /// Formats 2 and 5: the size every image is padded to, and the metrics they share.
    image_size: Option<i64>,
    metrics: Option<Vec<u8>>,
}

struct Strike {
    hori: Vec<u8>,
    vert: Vec<u8>,
    /// `colorRef`, `ppemX`, `ppemY`, `bitDepth`, `flags`.
    fields: Vec<i64>,
    subtables: Vec<IndexSubTable>,
}

/// Items at their `index` attribute; a gap or a repeated index is an error, as in fontTools.
fn by_index<T>(items: Vec<(i64, T)>, what: &str) -> Result<Vec<T>> {
    let mut slots: Vec<Option<T>> = Vec::new();
    for (index, item) in items {
        let i = usize::try_from(index).map_err(|_| bad(format!("{what} has the index {index}")))?;
        if i >= MAX_BITMAP_SUBTABLES {
            return Err(TablesError::TooLarge { what: "the strikes of a bitmap table", limit: MAX_BITMAP_SUBTABLES });
        }
        if i >= slots.len() {
            slots.resize_with(i + 1, || None);
        }
        match slots.get_mut(i) {
            Some(slot @ None) => *slot = Some(item),
            _ => return Err(bad(format!("two {what}s have the index {i}"))),
        }
    }
    slots.into_iter().enumerate().map(|(i, s)| s.ok_or_else(|| bad(format!("no {what} has the index {i}")))).collect()
}

fn read_location(e: &Element) -> Result<(i64, Vec<Strike>)> {
    let mut version = None;
    let mut strikes = Vec::new();
    let mut subtables = 0usize;
    let mut glyphs = 0usize;
    for child in &e.children {
        match child.name.as_str() {
            "header" => version = Some(fixed_version(child.attribute("version").ok_or_else(|| bad("a bitmap header without a version"))?)?),
            "strike" => {
                let index = int_lit(child.attribute("index").ok_or_else(|| bad("a strike without an index"))?)?;
                let mut size: Option<(Vec<u8>, Vec<u8>, Vec<i64>)> = None;
                let mut subs = Vec::new();
                for c in &child.children {
                    if c.name == "bitmapSizeTable" {
                        let [hori, vert] = both_line_metrics(c, "a bitmap size table")?;
                        let f = fields(c, &["colorRef", "ppemX", "ppemY", "bitDepth", "flags"], "a bitmap size table")?;
                        size = Some((hori, vert, f));
                    } else if let Some(format) = c.name.strip_prefix("eblc_index_sub_table_") {
                        subtables += 1;
                        if subtables > MAX_BITMAP_SUBTABLES {
                            return Err(TablesError::TooLarge {
                                what: "the index subtables of a bitmap location table",
                                limit: MAX_BITMAP_SUBTABLES,
                            });
                        }
                        let index_format = int_lit(format)?;
                        let image_format = int_lit(c.attribute("imageFormat").ok_or_else(|| bad("an index subtable without an image format"))?)?;
                        let names: Vec<String> = c
                            .children
                            .iter()
                            .filter(|g| g.name == "glyphLoc")
                            .map(|g| g.attribute("name").map(str::to_string).ok_or_else(|| bad("a glyph location without a name")))
                            .collect::<Result<_>>()?;
                        glyphs = glyphs.saturating_add(names.len());
                        if glyphs > MAX_LAYOUT_ITEMS {
                            return Err(TablesError::TooLarge { what: "the glyphs of a bitmap location table", limit: MAX_LAYOUT_ITEMS });
                        }
                        let (image_size, metrics) = if matches!(index_format, 2 | 5) {
                            let size = c.children.iter().rev().find(|m| m.name == "imageSize").map(value).transpose()?;
                            (size, glyph_metrics(c, "BigGlyphMetrics", &BIG_METRICS)?)
                        } else {
                            (None, None)
                        };
                        subs.push(IndexSubTable { index_format, image_format, names, image_size, metrics });
                    }
                }
                let (hori, vert, fields) = size.ok_or_else(|| bad("a strike without its bitmap size table"))?;
                strikes.push((index, Strike { hori, vert, fields, subtables: subs }));
            }
            _ => {}
        }
    }
    Ok((version.ok_or_else(|| bad("a bitmap location table without a header"))?, by_index(strikes, "strike")?))
}

// ── EBDT, CBDT, bdat ───────────────────────────────────────────────────────────────────────────

/// One bitmap glyph compiled (`BitmapGlyph.compile`).
fn compile_glyph(e: &Element, format: i64, ctx: &ReadContext) -> Result<Vec<u8>> {
    let small = || glyph_metrics(e, "SmallGlyphMetrics", &SMALL_METRICS)?.ok_or_else(|| bad(format!("a format {format} bitmap without its metrics")));
    let big = || glyph_metrics(e, "BigGlyphMetrics", &BIG_METRICS)?.ok_or_else(|| bad(format!("a format {format} bitmap without its metrics")));
    let image = || -> Result<Vec<u8>> {
        let data = e
            .children
            .iter()
            .rev()
            .find(|c| c.name.ends_with("imagedata"))
            .ok_or_else(|| bad(format!("a format {format} bitmap without image data")))?;
        forms::image_data(data, format)
    };
    let mut o = Out::default();
    match format {
        1 | 2 => {
            o.bytes(&small()?);
            o.bytes(&image()?);
        }
        5 => o.bytes(&image()?),
        6 | 7 => {
            o.bytes(&big()?);
            o.bytes(&image()?);
        }
        8 | 9 => {
            if format == 8 {
                o.bytes(&small()?);
                o.u8(0, "a pad byte")?;
            } else {
                o.bytes(&big()?);
            }
            let components: Vec<&Element> =
                e.children.iter().filter(|c| c.name == "components").flat_map(|c| c.children.iter().filter(|x| x.name == "ebdtComponent")).collect();
            o.u16(i64::try_from(components.len()).unwrap_or(i64::MAX), "the number of components")?;
            for c in components {
                let name = c.attribute("name").ok_or_else(|| bad("a bitmap component without a glyph"))?;
                let v = fields(c, &["xOffset", "yOffset"], "a bitmap component")?;
                o.u16(i64::from(ctx.glyph_id(name)?), "a component glyph")?;
                pack_signed(&mut o, &v, &[true, true], "a component offset")?;
            }
        }
        17..=19 => {
            match format {
                17 => o.bytes(&small()?),
                18 => o.bytes(&big()?),
                _ => {}
            }
            let data = image()?;
            o.u32(i64::try_from(data.len()).unwrap_or(i64::MAX), "an image length")?;
            o.bytes(&data);
        }
        _ => return Err(bad(format!("unknown bitmap image format {format}"))),
    }
    Ok(o.0)
}

/// One strike of a data table: glyph name to its image format and element.
type StrikeData<'a> = HashMap<String, (i64, &'a Element)>;

/// The version of a data table and its strikes.
fn read_data(e: &Element, color: bool) -> Result<(i64, Vec<StrikeData<'_>>)> {
    let mut version = None;
    let mut strikes = Vec::new();
    for child in &e.children {
        match child.name.as_str() {
            "header" => version = Some(fixed_version(child.attribute("version").ok_or_else(|| bad("a bitmap header without a version"))?)?),
            "strikedata" => {
                let index = int_lit(child.attribute("index").ok_or_else(|| bad("strike data without an index"))?)?;
                let mut glyphs = HashMap::new();
                for g in &child.children {
                    let Some(format) = g.name.get(4..).and_then(|n| n.strip_prefix("_bitmap_format_")) else { continue };
                    let format = int_lit(format)?;
                    if !color && format >= 17 {
                        return Err(bad(format!("unknown bitmap image format {format}")));
                    }
                    let name = g.attribute("name").ok_or_else(|| bad("a bitmap glyph without a name"))?;
                    if glyphs.insert(name.to_string(), (format, g)).is_some() {
                        return Err(bad(format!("the glyph '{name}' has two bitmaps in one strike")));
                    }
                }
                strikes.push((index, glyphs));
            }
            _ => {}
        }
    }
    Ok((version.ok_or_else(|| bad("a bitmap data table without a header"))?, by_index(strikes, "strike data")?))
}

/// A location table (`location`) and its data table (`data`, of tag `data_tag`), compiled
/// together: the data table's bytes, then the location table's.
pub(crate) fn bitmap_pair(data_tag: Tag, data: &Element, location: &Element, ctx: &ReadContext) -> Result<(Vec<u8>, Vec<u8>)> {
    let (data_version, glyphs) = read_data(data, &data_tag == b"CBDT")?;
    let (loc_version, strikes) = read_location(location)?;

    // `EBDT.compile`: each glyph of a strike stored once, in subtable order.
    let mut out = Out::default();
    out.i32(data_version, "the bitmap data version")?;
    let mut locations: Vec<Vec<Vec<(u64, u64)>>> = Vec::with_capacity(strikes.len());
    for (strike, strike_glyphs) in strikes.iter().zip(&glyphs) {
        let mut done: HashMap<&str, (u64, u64)> = HashMap::new();
        let mut per_sub = Vec::with_capacity(strike.subtables.len());
        for sub in &strike.subtables {
            let mut locs = Vec::with_capacity(sub.names.len());
            for name in &sub.names {
                let loc = match done.get(name.as_str()) {
                    Some(l) => *l,
                    None => {
                        let (format, e) = strike_glyphs.get(name).ok_or_else(|| bad(format!("the glyph '{name}' has no bitmap in its strike")))?;
                        let mut bytes = compile_glyph(e, *format, ctx)?;
                        if let Some(size) = sub.image_size {
                            let size = u32::try_from(size).map_err(|_| bad("an image size outside 0..=4294967295"))?;
                            let size = usize::try_from(size).unwrap_or(usize::MAX);
                            // Checked before the resize: the size comes from the file and the
                            // padding would otherwise be allocated first.
                            if size > crate::limits::MAX_SFNT.saturating_sub(out.0.len()) {
                                return Err(TablesError::TooLarge { what: "a bitmap data table", limit: crate::limits::MAX_SFNT });
                            }
                            if bytes.len() > size {
                                return Err(bad(format!(
                                    "a bitmap is larger than the fixed image size of its format {} index subtable",
                                    sub.index_format
                                )));
                            }
                            bytes.resize(size, 0);
                        }
                        let start = out.0.len() as u64;
                        out.bytes(&bytes);
                        if out.0.len() > crate::limits::MAX_SFNT {
                            return Err(TablesError::TooLarge { what: "a bitmap data table", limit: crate::limits::MAX_SFNT });
                        }
                        let l = (start, out.0.len() as u64);
                        done.insert(name, l);
                        l
                    }
                };
                locs.push(loc);
            }
            per_sub.push(locs);
        }
        locations.push(per_sub);
    }
    let data_bytes = out.0;

    // `EBLC.compile`.
    let mut head = Out::default();
    head.i32(loc_version, "the bitmap location version")?;
    head.u32(i64::try_from(strikes.len()).unwrap_or(i64::MAX), "the number of strikes")?;
    let mut size = 8u64 + 48 * strikes.len() as u64;
    let mut sizes = Out::default();
    let mut tail = Out::default();
    for (strike, locs) in strikes.iter().zip(locations.iter().chain(std::iter::repeat(&Vec::new()))) {
        let array = size;
        size += 8 * strike.subtables.len() as u64;
        let mut pairs = Out::default();
        let mut bodies = Out::default();
        let (mut first_all, mut last_all) = (u32::MAX, 0u32);
        for (i, sub) in strike.subtables.iter().enumerate() {
            let ids: Vec<u32> = sub.names.iter().map(|n| ctx.glyph_id(n)).collect::<Result<_>>()?;
            let (Some(first), Some(last)) = (ids.iter().min().copied(), ids.iter().max().copied()) else {
                return Err(bad("a bitmap index subtable lists no glyph"));
            };
            let offset = size - array;
            let body = index_subtable(sub, &ids, first, last, locs.get(i).map(Vec::as_slice).unwrap_or_default())?;
            size += body.len() as u64;
            if size > crate::limits::MAX_SFNT as u64 {
                return Err(TablesError::TooLarge { what: "a bitmap location table", limit: crate::limits::MAX_SFNT });
            }
            pairs.u16(i64::from(first), "a first glyph index")?;
            pairs.u16(i64::from(last), "a last glyph index")?;
            pairs.u32(i64::try_from(offset).unwrap_or(i64::MAX), "an index subtable offset")?;
            bodies.bytes(&body);
            first_all = first_all.min(first);
            last_all = last_all.max(last);
        }
        if strike.subtables.is_empty() {
            return Err(bad("a bitmap strike without index subtables"));
        }
        let [color_ref, ppem_x, ppem_y, bit_depth, flags] = strike.fields.as_slice() else {
            return Err(bad("a bitmap size table is incomplete"));
        };
        sizes.u32(i64::try_from(array).unwrap_or(i64::MAX), "an index subtable array offset")?;
        sizes.u32(i64::try_from(size - array).unwrap_or(i64::MAX), "the size of the index tables")?;
        sizes.u32(i64::try_from(strike.subtables.len()).unwrap_or(i64::MAX), "the number of index subtables")?;
        sizes.u32(*color_ref, "colorRef")?;
        sizes.bytes(&strike.hori);
        sizes.bytes(&strike.vert);
        sizes.u16(i64::from(first_all), "startGlyphIndex")?;
        sizes.u16(i64::from(last_all), "endGlyphIndex")?;
        sizes.u8(*ppem_x, "ppemX")?;
        sizes.u8(*ppem_y, "ppemY")?;
        sizes.u8(*bit_depth, "bitDepth")?;
        sizes.i8(*flags, "flags")?;
        tail.bytes(&pairs.0);
        tail.bytes(&bodies.0);
    }
    head.bytes(&sizes.0);
    head.bytes(&tail.0);
    Ok((data_bytes, head.0))
}

/// One index subtable compiled, with the image locations the data table gave its glyphs.
fn index_subtable(sub: &IndexSubTable, ids: &[u32], first: u32, last: u32, locs: &[(u64, u64)]) -> Result<Vec<u8>> {
    let consecutive = || locs.windows(2).all(|w| matches!(w, [a, b] if a.1 == b.0));
    let min_start = locs.iter().map(|l| l.0).min().ok_or_else(|| bad("a bitmap index subtable without images"))?;
    let mut o = Out::default();
    let header = |o: &mut Out, offset: u64| -> Result<()> {
        o.u16(sub.index_format, "an index format")?;
        o.u16(sub.image_format, "an image format")?;
        o.u32(i64::try_from(offset).unwrap_or(i64::MAX), "an image data offset")
    };
    let metrics = || sub.metrics.clone().ok_or_else(|| bad(format!("a format {} index subtable without metrics", sub.index_format)));
    let image_size = || sub.image_size.ok_or_else(|| bad(format!("a format {} index subtable without an image size", sub.index_format)));
    match sub.index_format {
        1 | 3 => {
            if !consecutive() {
                return Err(bad("the images of an offset-array index subtable are not consecutive"));
            }
            if ids.windows(2).any(|w| matches!(w, [a, b] if a >= b)) {
                return Err(bad("the glyphs of an offset-array index subtable are not in increasing order"));
            }
            // Glyphs of the range without an image get an empty one where the next image starts.
            let mut all: Vec<(u64, u64)> = Vec::new();
            let mut queue = ids.iter().zip(locs).peekable();
            for id in first..=last {
                let Some(&(&next_id, &loc)) = queue.peek() else { break };
                if id == next_id {
                    all.push(loc);
                    queue.next();
                } else {
                    all.push((loc.0, loc.0));
                }
            }
            let mut offsets: Vec<u64> = all.first().map(|l| vec![l.0, l.1]).unwrap_or_default();
            offsets.extend(all.iter().skip(1).map(|l| l.1));
            let base = offsets.iter().copied().min().unwrap_or(0);
            header(&mut o, base)?;
            for v in &offsets {
                let v = i64::try_from(v - base).unwrap_or(i64::MAX);
                if sub.index_format == 1 { o.u32(v, "an image offset")? } else { o.u16(v, "an image offset")? }
            }
            let size = if sub.index_format == 1 { 4 } else { 2 };
            if !(size * offsets.len()).is_multiple_of(4) {
                o.bytes(&vec![0; size]);
            }
        }
        2 => {
            if ids.iter().copied().ne(first..=last) {
                return Err(bad("the glyphs of a format 2 index subtable are not consecutive"));
            }
            header(&mut o, min_start)?;
            o.u32(image_size()?, "an image size")?;
            o.bytes(&metrics()?);
        }
        4 => {
            if !consecutive() {
                return Err(bad("the images of a format 4 index subtable are not consecutive"));
            }
            let mut offsets: Vec<u64> = locs.first().map(|l| vec![l.0, l.1]).unwrap_or_default();
            offsets.extend(locs.iter().skip(1).map(|l| l.1));
            let base = offsets.iter().copied().min().unwrap_or(0);
            header(&mut o, base)?;
            o.u32(i64::try_from(ids.len()).unwrap_or(i64::MAX), "the number of glyphs")?;
            for (id, off) in ids.iter().copied().chain(std::iter::once(0)).zip(&offsets) {
                o.u16(i64::from(id), "a glyph index")?;
                o.u16(i64::try_from(off - base).unwrap_or(i64::MAX), "an image offset")?;
            }
        }
        5 => {
            header(&mut o, min_start)?;
            o.u32(image_size()?, "an image size")?;
            o.bytes(&metrics()?);
            o.u32(i64::try_from(ids.len()).unwrap_or(i64::MAX), "the number of glyphs")?;
            for id in ids {
                o.u16(i64::from(*id), "a glyph index")?;
            }
            if ids.len() % 2 == 1 {
                o.u16(0, "a pad")?;
            }
        }
        f => return Err(bad(format!("unknown bitmap index subtable format {f}"))),
    }
    Ok(o.0)
}

// ── EBSC ───────────────────────────────────────────────────────────────────────────────────────

/// `EBSC`: the header, then each scale table in the order given.
pub(crate) fn ebsc(e: &Element) -> Result<Vec<u8>> {
    let mut version = None;
    let mut tables = Vec::new();
    for child in &e.children {
        match child.name.as_str() {
            "header" => version = Some(fixed_version(child.attribute("version").ok_or_else(|| bad("an 'EBSC' header without a version"))?)?),
            "bitmapScaleTable" => {
                if tables.len() >= MAX_BITMAP_SUBTABLES {
                    return Err(TablesError::TooLarge { what: "the scale tables of 'EBSC'", limit: MAX_BITMAP_SUBTABLES });
                }
                let [hori, vert] = both_line_metrics(child, "a bitmap scale table")?;
                let mut o = Out::default();
                o.bytes(&hori);
                o.bytes(&vert);
                for name in ["ppemX", "ppemY", "substitutePpemX", "substitutePpemY"] {
                    let v = child.children.iter().rev().find(|c| c.name == name).map(value).transpose()?.unwrap_or(0);
                    o.u8(v, name)?;
                }
                tables.push(o.0);
            }
            _ => {}
        }
    }
    let mut o = Out::default();
    o.i32(version.ok_or_else(|| bad("an 'EBSC' table without a header"))?, "the 'EBSC' version")?;
    o.u32(i64::try_from(tables.len()).unwrap_or(i64::MAX), "the number of scale tables")?;
    for t in tables {
        o.bytes(&t);
    }
    Ok(o.0)
}

// ── sbix ───────────────────────────────────────────────────────────────────────────────────────

/// One `sbix` glyph: no image, or its header and data.
fn sbix_glyph(g: &Element, ctx: &ReadContext) -> Result<Vec<u8>> {
    let Some(graphic_type) = g.attribute("graphicType") else { return Ok(Vec::new()) };
    let mut o = Out::default();
    o.i16(g.attribute("originOffsetX").map(int_lit).transpose()?.unwrap_or(0), "originOffsetX")?;
    o.i16(g.attribute("originOffsetY").map(int_lit).transpose()?.unwrap_or(0), "originOffsetY")?;
    o.bytes(&tag_bytes(graphic_type)?);
    let (mut reference, mut image) = (None, None);
    for c in &g.children {
        match c.name.as_str() {
            "ref" => reference = Some(ctx.glyph_id(c.attribute("glyphname").ok_or_else(|| bad("an 'sbix' reference without a glyph"))?)?),
            "hexdata" => image = Some(read_hex(&c.text)?),
            other => return Err(bad(format!("an 'sbix' glyph cannot hold '{other}'"))),
        }
    }
    if graphic_type == "dupe" {
        o.u16(i64::from(reference.ok_or_else(|| bad("an 'sbix' dupe glyph without a reference"))?), "a reference glyph")?;
    } else {
        o.bytes(&image.ok_or_else(|| bad("an 'sbix' glyph without image data"))?);
    }
    Ok(o.0)
}

/// `sbix`: strikes sorted by ppem (a later strike of the same ppem replaces an earlier one),
/// each with an offset for every glyph of the glyph order.
pub(crate) fn sbix(e: &Element, ctx: &ReadContext) -> Result<Vec<u8>> {
    let order = ctx.order()?;
    let (mut version, mut flags) = (None, 0u64);
    let mut strikes: BTreeMap<i64, (i64, HashMap<String, Vec<u8>>)> = BTreeMap::new();
    for child in &e.children {
        match child.name.as_str() {
            "version" => version = Some(value(child)?),
            "flags" => flags = binary2num(child.attribute("value").ok_or_else(|| bad("'sbix' flags without a value"))?),
            "strike" => {
                if strikes.len() >= MAX_BITMAP_SUBTABLES {
                    return Err(TablesError::TooLarge { what: "the strikes of 'sbix'", limit: MAX_BITMAP_SUBTABLES });
                }
                let (mut ppem, mut resolution) = (None, None);
                let mut glyphs = HashMap::new();
                for c in &child.children {
                    match c.name.as_str() {
                        "ppem" => ppem = Some(value(c)?),
                        "resolution" => resolution = Some(value(c)?),
                        "glyph" => {
                            let name =
                                c.attribute("glyphname").or_else(|| c.attribute("name")).ok_or_else(|| bad("an 'sbix' glyph without a name"))?;
                            glyphs.insert(name.to_string(), sbix_glyph(c, ctx)?);
                        }
                        other => return Err(bad(format!("an 'sbix' strike cannot hold '{other}'"))),
                    }
                }
                let ppem = ppem.ok_or_else(|| bad("an 'sbix' strike without a ppem"))?;
                strikes.insert(ppem, (resolution.ok_or_else(|| bad("an 'sbix' strike without a resolution"))?, glyphs));
            }
            other => return Err(bad(format!("'sbix' cannot hold '{other}'"))),
        }
    }
    let mut o = Out::default();
    o.u16(version.ok_or_else(|| bad("'sbix' without a version"))?, "the 'sbix' version")?;
    o.u16(i64::try_from(flags).unwrap_or(i64::MAX), "the 'sbix' flags")?;
    o.u32(i64::try_from(strikes.len()).unwrap_or(i64::MAX), "the number of strikes")?;
    let mut offset = 8u64 + 4 * strikes.len() as u64;
    let mut body = Out::default();
    let empty = Vec::new();
    for (ppem, (resolution, glyphs)) in &strikes {
        o.u32(i64::try_from(offset).unwrap_or(i64::MAX), "a strike offset")?;
        let mut s = Out::default();
        s.u16(*ppem, "a strike ppem")?;
        s.u16(*resolution, "a strike resolution")?;
        let mut at = 4u64 + 4 * (order.len() as u64 + 1);
        let mut data = Vec::new();
        for name in order {
            s.u32(i64::try_from(at).unwrap_or(i64::MAX), "a glyph offset")?;
            let raw = glyphs.get(name).unwrap_or(&empty);
            at += raw.len() as u64;
            data.extend_from_slice(raw);
            if data.len() > crate::limits::MAX_SFNT {
                return Err(TablesError::TooLarge { what: "an 'sbix' strike", limit: crate::limits::MAX_SFNT });
            }
        }
        s.u32(i64::try_from(at).unwrap_or(i64::MAX), "a glyph offset")?;
        s.bytes(&data);
        offset += s.0.len() as u64;
        body.bytes(&s.0);
        if body.0.len() > crate::limits::MAX_SFNT {
            return Err(TablesError::TooLarge { what: "an 'sbix' table", limit: crate::limits::MAX_SFNT });
        }
    }
    o.bytes(&body.0);
    Ok(o.0)
}
