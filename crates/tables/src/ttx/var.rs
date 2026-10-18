//! TTX writer: gvar, cvar, HVAR, VVAR and MVAR (M12.7).
//!
//! Each table is written element for element as fontTools' `ttx` writes it, and read with the
//! same checks: a table fontTools would refuse to read is an error here, and [`super::dump`]
//! then writes it as hex. Ported from fontTools' `ttLib.tables` (MIT): `_g_v_a_r`, `_c_v_a_r`,
//! `TupleVariation`, and the `VarStore`, `VarIdxMap`, `HVAR`, `VVAR` and `MVAR` parts of
//! `otData`, `otConverters` and `otTables`.
//!
//! The byte readers at the end are shared with [`super::color`] and [`super::misc`]. They read
//! at absolute positions and fail past the end, as fontTools' `struct.unpack` does; [`slice`]
//! clamps as a Python slice does.

#[cfg(test)]
mod fonts;
#[cfg(test)]
pub(super) mod tests;

use std::collections::BTreeMap;

use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

use super::{XmlWriter, fixed_to_str};

/// Writes table `tag` (bytes `data`, from `font`) into `w`. Returns false, writing nothing,
/// for a tag this module does not handle.
pub fn write_table(tag: Tag, data: &[u8], font: &SfntFont, w: &mut XmlWriter) -> Result<bool> {
    match &tag {
        b"gvar" => gvar(data, font, w)?,
        b"cvar" => cvar(data, font, w)?,
        b"HVAR" => metrics_variations(data, font, w, &["AdvWidthMap", "LsbMap", "RsbMap"])?,
        b"VVAR" => metrics_variations(data, font, w, &["AdvHeightMap", "TsbMap", "BsbMap", "VOrgMap"])?,
        b"MVAR" => mvar(data, w)?,
        _ => return Ok(false),
    }
    Ok(true)
}

// ── gvar, cvar ─────────────────────────────────────────────────────────────────────────────────

const EMBEDDED_PEAK_TUPLE: u16 = 0x8000;
const INTERMEDIATE_REGION: u16 = 0x4000;
const PRIVATE_POINT_NUMBERS: u16 = 0x2000;
const TUPLE_INDEX_MASK: u16 = 0x0FFF;
const TUPLES_SHARE_POINT_NUMBERS: u16 = 0x8000;
const TUPLE_COUNT_MASK: u16 = 0x0FFF;
/// Points `getNumPoints_` adds to every glyph: the four phantom points.
const PHANTOM_POINTS: usize = 4;

/// One decoded tuple variation: the region on each axis (start, peak, end, in F2Dot14 units;
/// `None` where all three are zero) and the deltas by point number.
struct TupleVariation {
    axes: Vec<Option<(i16, i16, i16)>>,
    deltas: BTreeMap<usize, (i32, i32)>,
}

#[derive(Clone, Copy, PartialEq)]
enum Store {
    Gvar,
    Cvar,
}

/// The axis tags of `fvar`, in order; `None` when the font has no `fvar`.
fn fvar_axes(font: &SfntFont) -> Result<Option<Vec<String>>> {
    let Some(fvar) = font.get(*b"fvar") else { return Ok(None) };
    let offset = usize::from(u16_at(fvar, 4)?);
    let count = usize::from(u16_at(fvar, 8)?);
    let size = usize::from(u16_at(fvar, 10)?);
    let mut tags = Vec::with_capacity(count);
    for i in 0..count {
        let pos = offset.saturating_add(i.saturating_mul(size));
        // fontTools reads each axis record from a slice of `axisSize` bytes and needs 20 of them.
        if size < 20 {
            return Err(TablesError::invalid("the 'fvar' axis records are too short"));
        }
        bytes_at(fvar, pos, 20)?;
        tags.push(latin1(bytes_at(fvar, pos, 4)?));
    }
    Ok(Some(tags))
}

fn f2dot14(raw: i16) -> String {
    fixed_to_str(f64::from(raw) / 16384.0, 14)
}

/// The number of points fontTools gives glyph `gid` in `gvar`: its outline points (or its
/// components) and the four phantom points.
fn glyph_points(glyf: &[u8], offsets: &[usize], gid: usize) -> Result<usize> {
    let (Some(&start), Some(&end)) = (offsets.get(gid), offsets.get(gid.saturating_add(1))) else {
        return Err(TablesError::invalid("the 'loca' table is shorter than the glyph count"));
    };
    let glyph = slice(glyf, start, end);
    if glyph.is_empty() {
        return Ok(PHANTOM_POINTS);
    }
    let contours = i16_at(glyph, 0)?;
    if contours > 0 {
        let last = usize::from(u16_at(glyph, 10usize.saturating_add(2 * (contours as usize - 1)))?);
        return Ok(last.saturating_add(1).saturating_add(PHANTOM_POINTS));
    }
    if contours == 0 {
        return Ok(PHANTOM_POINTS);
    }
    let mut pos = 10usize;
    let mut components = 0usize;
    loop {
        let flags = u16_at(glyph, pos)?;
        u16_at(glyph, pos.saturating_add(2))?;
        let args = if flags & 0x0001 != 0 { 4 } else { 2 };
        let scale = if flags & 0x0008 != 0 {
            2
        } else if flags & 0x0040 != 0 {
            4
        } else if flags & 0x0080 != 0 {
            8
        } else {
            0
        };
        pos = pos.saturating_add(4 + args + scale);
        bytes_at(glyph, pos.saturating_sub(args + scale), args + scale)?;
        components = components.saturating_add(1);
        if flags & 0x0020 == 0 {
            break;
        }
    }
    Ok(components.saturating_add(PHANTOM_POINTS))
}

fn gvar(data: &[u8], font: &SfntFont, w: &mut XmlWriter) -> Result<()> {
    let version = u16_at(data, 0)?;
    let reserved = u16_at(data, 2)?;
    let axis_count = usize::from(u16_at(data, 4)?);
    let shared_count = usize::from(u16_at(data, 6)?);
    let shared_offset = u32_at(data, 8)? as usize;
    let glyph_count = usize::from(u16_at(data, 12)?);
    let flags = u16_at(data, 14)?;
    let data_offset = u32_at(data, 16)? as usize;
    const HEADER: usize = 20;

    let order = w.glyph_order(font)?;
    if order.len() != glyph_count {
        return Err(TablesError::invalid(format!("'gvar' has {glyph_count} glyphs, the font {}", order.len())));
    }
    let fvar = fvar_axes(font)?;
    let axes: Vec<String> = match &fvar {
        Some(tags) => tags.clone(),
        None => (0..axis_count).map(|i| i.to_string()).collect(),
    };
    if axes.len() != axis_count {
        return Err(TablesError::invalid(format!("'gvar' has {axis_count} axes, 'fvar' {}", axes.len())));
    }
    let mut shared = Vec::with_capacity(shared_count.min(data.len()));
    for i in 0..shared_count {
        let start = shared_offset.saturating_add(i.saturating_mul(axis_count).saturating_mul(2));
        shared.push(coord(data, start, axis_count)?);
    }
    let glyf = font.get(*b"glyf").ok_or_else(|| TablesError::MissingTable("glyf".into()))?;
    let loca = font.get(*b"loca").ok_or_else(|| TablesError::MissingTable("loca".into()))?;
    let offsets = super::core::loca_offsets(loca, font)?;

    w.simple("version", &[("value", &version.to_string())]);
    w.simple("reserved", &[("value", &reserved.to_string())]);
    if fvar.is_none() {
        w.simple("axisCount", &[("value", &axis_count.to_string())]);
    }
    let mut by_name: Vec<(&String, usize)> = order.iter().enumerate().map(|(gid, name)| (name, gid)).collect();
    by_name.sort();
    let long = flags & 1 != 0;
    for (name, gid) in by_name {
        let (start, end) = if long {
            (u32_at(data, HEADER + 4 * gid)? as usize, u32_at(data, HEADER + 4 * gid + 4)? as usize)
        } else {
            (usize::from(u16_at(data, HEADER + 2 * gid)?) * 2, usize::from(u16_at(data, HEADER + 2 * gid + 2)?) * 2)
        };
        let glyph = slice(data, data_offset.saturating_add(start), data_offset.saturating_add(end));
        if glyph.is_empty() {
            continue;
        }
        let points = glyph_points(glyf, &offsets, gid)?;
        if glyph.len() < 4 {
            continue;
        }
        let count = u16_at(glyph, 0)?;
        let first_data = usize::from(u16_at(glyph, 2)?);
        let tuples = tuple_store(Store::Gvar, axis_count, count, points, Some(&shared), glyph, 4, first_data)?;
        if tuples.is_empty() {
            continue;
        }
        w.open("glyphVariations", &[("glyph", name)]);
        for t in &tuples {
            write_tuple(Store::Gvar, t, &axes, w);
        }
        w.close("glyphVariations");
        w.check()?;
    }
    Ok(())
}

fn cvar(data: &[u8], font: &SfntFont, w: &mut XmlWriter) -> Result<()> {
    let axes = fvar_axes(font)?.ok_or_else(|| TablesError::MissingTable("fvar".into()))?;
    let cvt = font.get(*b"cvt ").ok_or_else(|| TablesError::MissingTable("cvt ".into()))?;
    if cvt.len() % 2 != 0 {
        return Err(TablesError::invalid("the 'cvt ' table has an odd length"));
    }
    let major = u16_at(data, 0)?;
    let minor = u16_at(data, 2)?;
    let count = u16_at(data, 4)?;
    let first_data = usize::from(u16_at(data, 6)?);
    if major != 1 {
        return Err(TablesError::invalid(format!("unknown 'cvar' major version {major}")));
    }
    let tuples = tuple_store(Store::Cvar, axes.len(), count, cvt.len() / 2, None, data, 8, first_data)?;
    w.simple("version", &[("major", &major.to_string()), ("minor", &minor.to_string())]);
    for t in &tuples {
        write_tuple(Store::Cvar, t, &axes, w);
        w.check()?;
    }
    Ok(())
}

fn write_tuple(store: Store, t: &TupleVariation, axes: &[String], w: &mut XmlWriter) {
    w.open("tuple", &[]);
    for (axis, region) in axes.iter().zip(&t.axes) {
        let Some((start, peak, end)) = *region else { continue };
        if start == peak.min(0) && end == peak.max(0) {
            w.simple("coord", &[("axis", axis), ("value", &f2dot14(peak))]);
        } else {
            w.simple("coord", &[("axis", axis), ("min", &f2dot14(start)), ("value", &f2dot14(peak)), ("max", &f2dot14(end))]);
        }
    }
    for (point, (x, y)) in &t.deltas {
        match store {
            Store::Gvar => w.simple("delta", &[("pt", &point.to_string()), ("x", &x.to_string()), ("y", &y.to_string())]),
            Store::Cvar => w.simple("delta", &[("cvt", &point.to_string()), ("value", &x.to_string())]),
        }
    }
    if t.deltas.is_empty() {
        w.comment("no deltas");
    }
    w.close("tuple");
}

/// A tuple of `axis_count` F2Dot14 values at `pos`.
fn coord(data: &[u8], pos: usize, axis_count: usize) -> Result<Vec<i16>> {
    (0..axis_count).map(|a| i16_at(data, pos.saturating_add(2 * a))).collect()
}

/// fontTools' `decompileTupleVariationStore`: `count` tuple headers from `pos`, their point
/// and delta data from `data_pos`.
#[allow(clippy::too_many_arguments)]
fn tuple_store(
    store: Store,
    axis_count: usize,
    count: u16,
    point_count: usize,
    shared: Option<&[Vec<i16>]>,
    data: &[u8],
    pos: usize,
    data_pos: usize,
) -> Result<Vec<TupleVariation>> {
    let mut pos = pos;
    let mut data_pos = data_pos;
    let shared_points = if count & TUPLES_SHARE_POINT_NUMBERS != 0 {
        let (points, next) = decode_points(data, data_pos)?;
        data_pos = next;
        points
    } else {
        Some(Vec::new())
    };
    let mut out = Vec::new();
    for _ in 0..count & TUPLE_COUNT_MASK {
        let size = usize::from(u16_at(data, pos)?);
        let flags = u16_at(data, pos.saturating_add(2))?;
        let mut header_size = 4;
        if flags & EMBEDDED_PEAK_TUPLE != 0 {
            header_size += axis_count * 2;
        }
        if flags & INTERMEDIATE_REGION != 0 {
            header_size += axis_count * 4;
        }
        let header = slice(data, pos, pos.saturating_add(header_size));
        let deltas = slice(data, data_pos, data_pos.saturating_add(size));
        out.push(tuple_variation(store, axis_count, point_count, shared, &shared_points, header, deltas)?);
        pos = pos.saturating_add(header_size);
        data_pos = data_pos.saturating_add(size);
    }
    Ok(out)
}

fn tuple_variation(
    store: Store,
    axis_count: usize,
    point_count: usize,
    shared: Option<&[Vec<i16>]>,
    shared_points: &Option<Vec<usize>>,
    header: &[u8],
    data: &[u8],
) -> Result<TupleVariation> {
    let flags = u16_at(header, 2)?;
    let mut pos = 4;
    let peak = if flags & EMBEDDED_PEAK_TUPLE == 0 {
        let index = usize::from(flags & TUPLE_INDEX_MASK);
        shared.and_then(|s| s.get(index)).cloned().ok_or_else(|| TablesError::invalid(format!("shared tuple {index} does not exist")))?
    } else {
        let c = coord(header, pos, axis_count)?;
        pos += 2 * axis_count;
        c
    };
    let (start, end) = if flags & INTERMEDIATE_REGION != 0 {
        let start = coord(header, pos, axis_count)?;
        let end = coord(header, pos + 2 * axis_count, axis_count)?;
        (start, end)
    } else {
        (peak.iter().map(|p| (*p).min(0)).collect(), peak.iter().map(|p| (*p).max(0)).collect())
    };
    let axes = (0..axis_count)
        .map(|a| {
            let region = (start.get(a).copied().unwrap_or(0), peak.get(a).copied().unwrap_or(0), end.get(a).copied().unwrap_or(0));
            (region != (0, 0, 0)).then_some(region)
        })
        .collect();
    let mut pos = 0;
    let private;
    let points = if flags & PRIVATE_POINT_NUMBERS != 0 {
        let (p, next) = decode_points(data, 0)?;
        pos = next;
        private = p;
        &private
    } else {
        shared_points
    };
    let all: Vec<usize>;
    let points: &[usize] = match points {
        Some(p) => p,
        None => {
            all = (0..point_count).collect();
            &all
        }
    };
    let mut deltas = BTreeMap::new();
    let (xs, next) = decode_deltas(data, pos, points.len())?;
    let ys = if store == Store::Gvar { decode_deltas(data, next, points.len())?.0 } else { Vec::new() };
    for (i, (p, x)) in points.iter().zip(&xs).enumerate() {
        if *p < point_count {
            deltas.insert(*p, (*x, ys.get(i).copied().unwrap_or(0)));
        }
    }
    Ok(TupleVariation { axes, deltas })
}

/// Packed point numbers at `pos`: `None` for "every point", else the absolute point numbers.
fn decode_points(data: &[u8], pos: usize) -> Result<(Option<Vec<usize>>, usize)> {
    let mut pos = pos;
    let first = u8_at(data, pos)?;
    pos += 1;
    let mut count = usize::from(first);
    if first & 0x80 != 0 {
        count = (usize::from(first & 0x7F) << 8) | usize::from(u8_at(data, pos)?);
        pos += 1;
    }
    if count == 0 {
        return Ok((None, pos));
    }
    let mut points = Vec::new();
    while points.len() < count {
        let run_header = u8_at(data, pos)?;
        pos += 1;
        let run = usize::from(run_header & 0x7F) + 1;
        for _ in 0..run {
            if run_header & 0x80 != 0 {
                points.push(usize::from(u16_at(data, pos)?));
                pos += 2;
            } else {
                points.push(usize::from(u8_at(data, pos)?));
                pos += 1;
            }
        }
    }
    let mut current = 0usize;
    for p in &mut points {
        current = current.saturating_add(*p);
        *p = current;
    }
    Ok((Some(points), pos))
}

/// `count` packed deltas at `pos`; a run that overshoots `count` is an error, as in fontTools.
fn decode_deltas(data: &[u8], pos: usize, count: usize) -> Result<(Vec<i32>, usize)> {
    let mut pos = pos;
    let mut out = Vec::new();
    while out.len() < count {
        let run_header = u8_at(data, pos)?;
        pos += 1;
        let run = usize::from(run_header & 0x3F) + 1;
        match run_header & 0xC0 {
            0x80 => out.extend(std::iter::repeat_n(0, run)),
            0xC0 => {
                for _ in 0..run {
                    out.push(i32_at(data, pos)?);
                    pos += 4;
                }
            }
            0x40 => {
                for _ in 0..run {
                    out.push(i32::from(i16_at(data, pos)?));
                    pos += 2;
                }
            }
            _ => {
                for _ in 0..run {
                    out.push(i32::from(u8_at(data, pos)? as i8));
                    pos += 1;
                }
            }
        }
    }
    if out.len() != count {
        return Err(TablesError::invalid("a delta run runs past the point count"));
    }
    Ok((out, pos))
}

// ── HVAR, VVAR, MVAR ───────────────────────────────────────────────────────────────────────────

/// fontTools' `Version` converter: `0x%08x` of a signed 32-bit value.
pub(super) fn version_str(raw: i32) -> String {
    if raw < 0 { format!("0x-{:07x}", raw.unsigned_abs()) } else { format!("0x{raw:08x}") }
}

fn metrics_variations(data: &[u8], font: &SfntFont, w: &mut XmlWriter, maps: &[&str]) -> Result<()> {
    w.simple("Version", &[("value", &version_str(i32_at(data, 0)?))]);
    let store = u32_at(data, 4)? as usize;
    if store != 0 {
        var_store(data, store, w)?;
    }
    for (i, name) in maps.iter().enumerate() {
        let offset = u32_at(data, 8 + 4 * i)? as usize;
        if offset != 0 {
            index_map(data, offset, name, font, w)?;
        }
    }
    Ok(())
}

fn mvar(data: &[u8], w: &mut XmlWriter) -> Result<()> {
    w.simple("Version", &[("value", &version_str(i32_at(data, 0)?))]);
    w.simple("Reserved", &[("value", &u16_at(data, 4)?.to_string())]);
    let record_size = usize::from(u16_at(data, 6)?);
    w.simple("ValueRecordSize", &[("value", &record_size.to_string())]);
    let count = usize::from(u16_at(data, 8)?);
    let store = usize::from(u16_at(data, 10)?);
    if store != 0 {
        var_store(data, store, w)?;
    }
    let more = record_size.saturating_sub(8);
    let mut pos = 12usize;
    for i in 0..count {
        let tag = latin1(bytes_at(data, pos, 4)?);
        let index = u32_at(data, pos.saturating_add(4))?;
        w.open("ValueRecord", &[("index", &i.to_string())]);
        w.simple("ValueTag", &[("value", &tag)]);
        w.simple("VarIdx", &[("value", &index.to_string())]);
        for j in 0..more {
            let b = u8_at(data, pos.saturating_add(8 + j))?;
            w.simple("MoreBytes", &[("index", &j.to_string()), ("value", &b.to_string())]);
        }
        w.close("ValueRecord");
        pos = pos.saturating_add(8 + more);
        w.check()?;
    }
    Ok(())
}

/// An ItemVariationStore at `base`, as `<VarStore Format="1">`.
pub(super) fn var_store(data: &[u8], base: usize, w: &mut XmlWriter) -> Result<()> {
    let format = u16_at(data, base)?;
    let regions = u32_at(data, base.saturating_add(2))? as usize;
    let count = usize::from(u16_at(data, base.saturating_add(6))?);
    w.open("VarStore", &[("Format", &format.to_string())]);
    w.simple("Format", &[("value", &format.to_string())]);
    if regions != 0 {
        let at = base.saturating_add(regions);
        let axis_count = usize::from(u16_at(data, at)?);
        let region_count = usize::from(u16_at(data, at.saturating_add(2))?);
        w.open("VarRegionList", &[]);
        let mut pos = at.saturating_add(4);
        for r in 0..region_count {
            w.open("Region", &[("index", &r.to_string())]);
            for a in 0..axis_count {
                w.open("VarRegionAxis", &[("index", &a.to_string())]);
                w.simple("StartCoord", &[("value", &f2dot14(i16_at(data, pos)?))]);
                w.simple("PeakCoord", &[("value", &f2dot14(i16_at(data, pos.saturating_add(2))?))]);
                w.simple("EndCoord", &[("value", &f2dot14(i16_at(data, pos.saturating_add(4))?))]);
                w.close("VarRegionAxis");
                pos = pos.saturating_add(6);
            }
            w.close("Region");
            w.check()?;
        }
        w.close("VarRegionList");
    }
    for i in 0..count {
        let offset = u32_at(data, base.saturating_add(8 + 4 * i))? as usize;
        if offset == 0 {
            w.simple("VarData", &[("index", &i.to_string()), ("empty", "1")]);
        } else {
            var_data(data, base.saturating_add(offset), i, w)?;
        }
    }
    w.close("VarStore");
    Ok(())
}

fn var_data(data: &[u8], at: usize, index: usize, w: &mut XmlWriter) -> Result<()> {
    let item_count = usize::from(u16_at(data, at)?);
    let num_shorts = u16_at(data, at.saturating_add(2))?;
    let region_count = usize::from(u16_at(data, at.saturating_add(4))?);
    w.open("VarData", &[("index", &index.to_string())]);
    w.simple("NumShorts", &[("value", &num_shorts.to_string())]);
    let mut pos = at.saturating_add(6);
    for k in 0..region_count {
        w.simple("VarRegionIndex", &[("index", &k.to_string()), ("value", &u16_at(data, pos)?.to_string())]);
        pos = pos.saturating_add(2);
    }
    let long = num_shorts & 0x8000 != 0;
    let words = usize::from(num_shorts & 0x7FFF);
    let (n1, n2) = (region_count.min(words), region_count.max(words));
    for item in 0..item_count {
        let mut values = Vec::with_capacity(n2);
        for v in 0..n2 {
            let big = v < n1;
            let value = match (long, big) {
                (true, true) => {
                    let v = i64::from(i32_at(data, pos)?);
                    pos += 4;
                    v
                }
                (true, false) | (false, true) => {
                    let v = i64::from(i16_at(data, pos)?);
                    pos += 2;
                    v
                }
                (false, false) => {
                    let v = i64::from(u8_at(data, pos)? as i8);
                    pos += 1;
                    v
                }
            };
            values.push(value);
        }
        values.truncate(region_count);
        w.simple("Item", &[("index", &item.to_string()), ("value", &py_list(&values))]);
        w.check()?;
    }
    w.close("VarData");
    Ok(())
}

/// A DeltaSetIndexMap read as fontTools' `VarIdxMap` (16-bit entry format and count), one
/// `<Map>` per glyph in name order, the last entry repeated for the glyphs past the end.
fn index_map(data: &[u8], at: usize, name: &str, font: &SfntFont, w: &mut XmlWriter) -> Result<()> {
    let format = u16_at(data, at)?;
    let count = usize::from(u16_at(data, at.saturating_add(2))?);
    let inner_bits = 1 + u32::from(format & 0x000F);
    let inner_mask = (1u64 << inner_bits) - 1;
    let outer_mask = 0xFFFF_FFFFu64 - inner_mask;
    let outer_shift = 16 - inner_bits;
    let size = 1 + usize::from((format & 0x0030) >> 4);
    let mut values = Vec::with_capacity(count.min(data.len()));
    for i in 0..count {
        let raw = bytes_at(data, at.saturating_add(4 + i * size), size)?.iter().fold(0u64, |acc, b| acc << 8 | u64::from(*b));
        values.push(((raw & outer_mask) << outer_shift) | (raw & inner_mask));
    }
    let order = w.glyph_order(font)?;
    let Some(&last) = values.last() else {
        if order.is_empty() {
            w.open(name, &[]);
            w.close(name);
            return Ok(());
        }
        return Err(TablesError::invalid(format!("the '{name}' mapping is empty")));
    };
    let mut by_name: BTreeMap<&str, u64> = BTreeMap::new();
    for (i, glyph) in order.iter().enumerate() {
        by_name.insert(glyph, values.get(i).copied().unwrap_or(last));
    }
    w.open(name, &[]);
    for (glyph, value) in by_name {
        w.simple("Map", &[("glyph", glyph), ("outer", &(value >> 16).to_string()), ("inner", &(value & 0xFFFF).to_string())]);
    }
    w.close(name);
    w.check()
}

/// A list as Python's `repr` writes it: `[1, -2]`.
pub(super) fn py_list(values: &[i64]) -> String {
    let parts: Vec<String> = values.iter().map(i64::to_string).collect();
    format!("[{}]", parts.join(", "))
}

// ── Byte readers ───────────────────────────────────────────────────────────────────────────────

pub(super) fn cut() -> TablesError {
    TablesError::invalid("the table is cut short")
}

pub(super) fn bytes_at(data: &[u8], pos: usize, n: usize) -> Result<&[u8]> {
    data.get(pos..pos.checked_add(n).ok_or_else(cut)?).ok_or_else(cut)
}

pub(super) fn u8_at(data: &[u8], pos: usize) -> Result<u8> {
    data.get(pos).copied().ok_or_else(cut)
}

pub(super) fn u16_at(data: &[u8], pos: usize) -> Result<u16> {
    Ok(bytes_at(data, pos, 2)?.iter().fold(0u16, |acc, b| acc << 8 | u16::from(*b)))
}

pub(super) fn i16_at(data: &[u8], pos: usize) -> Result<i16> {
    Ok(u16_at(data, pos)? as i16)
}

pub(super) fn u32_at(data: &[u8], pos: usize) -> Result<u32> {
    Ok(bytes_at(data, pos, 4)?.iter().fold(0u32, |acc, b| acc << 8 | u32::from(*b)))
}

pub(super) fn i32_at(data: &[u8], pos: usize) -> Result<i32> {
    Ok(u32_at(data, pos)? as i32)
}

pub(super) fn u64_at(data: &[u8], pos: usize) -> Result<u64> {
    Ok(bytes_at(data, pos, 8)?.iter().fold(0u64, |acc, b| acc << 8 | u64::from(*b)))
}

/// `data[start:end]` as Python slices: clamped to the data, empty when `end <= start`.
pub(super) fn slice(data: &[u8], start: usize, end: usize) -> &[u8] {
    let start = start.min(data.len());
    let end = end.clamp(start, data.len());
    data.get(start..end).unwrap_or_default()
}

/// Bytes as Latin-1 text, as fontTools' `Tag` reads them.
pub(super) fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|b| char::from(*b)).collect()
}
