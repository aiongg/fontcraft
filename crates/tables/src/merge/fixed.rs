//! The tables of fixed fields, merged field by field with the rules of fontTools'
//! `merge/tables.py` (MIT License): `head`, `hhea`, `vhea`, `maxp`, `OS/2` and the `post` header.
//! Glyph-derived fields (bounding box, `hhea` extents, `xAvgCharWidth`) are recomputed from
//! the merged glyphs, as fontTools recomputes them when it saves.

use super::glyphs::{Built, average_advance, long_metric_count};
use crate::reorder::be::{Data, set_u16};
use crate::reorder::names::write_post_v2;
use crate::{Result, TablesError};

fn first<'a>(tables: &[&'a [u8]], tag: [u8; 4]) -> Result<&'a [u8]> {
    tables.first().copied().ok_or_else(|| TablesError::MissingTable(crate::sfnt::tag_str(tag)))
}

fn put_u32_at(out: &mut [u8], at: usize, v: u32) -> Result<()> {
    set_u16(out, at, (v >> 16) as u16)?;
    set_u16(out, at + 2, v as u16)
}

/// The values of one field in the tables long enough to hold it.
fn values_u16(tables: &[&[u8]], tag: [u8; 4], at: usize) -> Vec<u16> {
    tables.iter().filter_map(|t| Data::new(tag, t).u16(at).ok()).collect()
}

fn values_u32(tables: &[&[u8]], tag: [u8; 4], at: usize) -> Vec<u32> {
    tables.iter().filter_map(|t| Data::new(tag, t).u32(at).ok()).collect()
}

#[derive(Clone, Copy)]
enum Rule {
    MaxU16,
    MinU16,
    MaxI16,
    MinI16,
    MaxU32,
    MaxI32,
    MinU32,
    OrU32,
    /// The first table that has the field.
    First,
}

/// Applies `rule` to the field at `at` over the tables that have it; fields no table has are
/// left alone.
fn field(out: &mut [u8], tables: &[&[u8]], tag: [u8; 4], at: usize, rule: Rule) -> Result<()> {
    let w16 = values_u16(tables, tag, at);
    let w32 = values_u32(tables, tag, at);
    match rule {
        Rule::MaxU16 => w16.iter().max().map_or(Ok(()), |v| set_u16(out, at, *v)),
        Rule::MinU16 => w16.iter().min().map_or(Ok(()), |v| set_u16(out, at, *v)),
        Rule::MaxI16 => w16.iter().map(|v| *v as i16).max().map_or(Ok(()), |v| set_u16(out, at, v as u16)),
        Rule::MinI16 => w16.iter().map(|v| *v as i16).min().map_or(Ok(()), |v| set_u16(out, at, v as u16)),
        Rule::MaxU32 => w32.iter().max().map_or(Ok(()), |v| put_u32_at(out, at, *v)),
        Rule::MaxI32 => w32.iter().map(|v| *v as i32).max().map_or(Ok(()), |v| put_u32_at(out, at, v as u32)),
        Rule::MinU32 => w32.iter().min().map_or(Ok(()), |v| put_u32_at(out, at, *v)),
        Rule::OrU32 => {
            if w32.is_empty() {
                return Ok(());
            }
            put_u32_at(out, at, w32.iter().fold(0, |a, b| a | b))
        }
        Rule::First => w16.first().map_or(Ok(()), |v| set_u16(out, at, *v)),
    }
}

/// `head` flag bits merged with AND (the rest with OR); bits 6 and 15 are always clear.
const HEAD_FLAGS_AND: u16 = (1 << 1) | (1 << 2) | (1 << 3) | (1 << 5) | (1 << 11) | (1 << 13) | (1 << 14);
const HEAD_FLAGS_ZERO: u16 = (1 << 6) | (1 << 15);

pub(super) fn head(tables: &[&[u8]]) -> Result<Vec<u8>> {
    let tag = *b"head";
    let mut out = first(tables, tag)?.to_vec();
    Data::new(tag, &out).slice(0, 54)?;
    field(&mut out, tables, tag, 0, Rule::MaxU32)?;
    field(&mut out, tables, tag, 4, Rule::MaxI32)?;
    put_u32_at(&mut out, 8, 0)?;
    let flags = values_u16(tables, tag, 16);
    let and = flags.iter().fold(u16::MAX, |a, b| a & b);
    let or = flags.iter().fold(0, |a, b| a | b);
    set_u16(&mut out, 16, ((or & !HEAD_FLAGS_AND) | (and & HEAD_FLAGS_AND)) & !HEAD_FLAGS_ZERO)?;
    field(&mut out, tables, tag, 46, Rule::MaxU16)?;
    set_u16(&mut out, 48, 2)?;
    Ok(out)
}

/// `head` with the times, the bounding box of every glyph and the `loca` format.
pub(super) fn finish_head(mut head: Vec<u8>, built: &Built, (created, modified): (i64, i64)) -> Result<Vec<u8>> {
    for (at, t) in [(20, created), (28, modified)] {
        put_u32_at(&mut head, at, (t >> 32) as u32)?;
        put_u32_at(&mut head, at + 4, t as u32)?;
    }
    let mut bbox: Option<[i16; 4]> = None;
    for b in built.bounds.iter().flatten() {
        bbox = Some(match bbox {
            None => *b,
            Some(a) => [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])],
        });
    }
    for (i, v) in bbox.unwrap_or_default().iter().enumerate() {
        set_u16(&mut head, 36 + 2 * i, *v as u16)?;
    }
    set_u16(&mut head, 50, u16::from(built.long_loca))?;
    Ok(head)
}

/// `hhea` or `vhea` (the two share a layout): ascent and line gap the largest, descent the
/// smallest, the rest the first font's. The extents are set by [`finish_hea`].
pub(super) fn hhea(tables: &[&[u8]]) -> Result<Vec<u8>> {
    let tag = *b"hhea";
    let mut out = first(tables, tag)?.to_vec();
    Data::new(tag, &out).slice(0, 36)?;
    field(&mut out, tables, tag, 0, Rule::MaxU32)?;
    field(&mut out, tables, tag, 4, Rule::MaxI16)?;
    field(&mut out, tables, tag, 6, Rule::MinI16)?;
    field(&mut out, tables, tag, 8, Rule::MaxI16)?;
    Ok(out)
}

/// The largest advance, the side-bearing extremes and the extent from the merged glyphs, and
/// the long-metric count. `vertical` reads the bounds' y range.
pub(super) fn finish_hea(mut hea: Vec<u8>, metrics: &[(u16, i16)], bounds: &[Option<[i16; 4]>], vertical: bool) -> Result<Vec<u8>> {
    let advance_max = metrics.iter().map(|m| m.0).max().unwrap_or(0);
    let mut min_start = i32::MAX;
    let mut min_end = i32::MAX;
    let mut max_extent = i32::MIN;
    for (m, b) in metrics.iter().zip(bounds) {
        let Some(b) = b else { continue };
        let size = if vertical { i32::from(b[3]) - i32::from(b[1]) } else { i32::from(b[2]) - i32::from(b[0]) };
        let side = i32::from(m.1);
        min_start = min_start.min(side);
        min_end = min_end.min(i32::from(m.0) - side - size);
        max_extent = max_extent.max(side + size);
    }
    let clamp = |v: i32| if v == i32::MAX || v == i32::MIN { 0 } else { v.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16 };
    set_u16(&mut hea, 10, advance_max)?;
    set_u16(&mut hea, 12, clamp(min_start) as u16)?;
    set_u16(&mut hea, 14, clamp(min_end) as u16)?;
    set_u16(&mut hea, 16, clamp(max_extent) as u16)?;
    set_u16(&mut hea, 34, long_metric_count(metrics)?)?;
    Ok(hea)
}

/// `maxp`: every limit the largest, except the storage and definition counts, which are the
/// first font's (its font program is the one kept).
pub(super) fn maxp(tables: &[&[u8]]) -> Result<Vec<u8>> {
    let tag = *b"maxp";
    let mut out = first(tables, tag)?.to_vec();
    if out.len() >= 32 {
        for at in (6..32).step_by(2) {
            if !matches!(at, 18 | 20 | 22) {
                field(&mut out, tables, tag, at, Rule::MaxU16)?;
            }
        }
    }
    Ok(out)
}

pub(super) fn finish_maxp(mut maxp: Vec<u8>, count: usize) -> Result<Vec<u8>> {
    set_u16(&mut maxp, 4, crate::reorder::be::to_u16(count, "the glyph count")?)?;
    Ok(maxp)
}

/// fontTools' `mergeOs2FsType`: the least restrictive embedding the fonts allow together.
fn fs_type(values: &[u16]) -> u16 {
    if values.iter().all(|v| *v == 0) {
        return 0;
    }
    let adjusted: Vec<u16> = values
        .iter()
        .map(|v| {
            if v & 0x000C != 0 {
                v & !0x0002
            } else if *v == 0 {
                0x000C
            } else {
                *v
            }
        })
        .collect();
    let or = adjusted.iter().fold(0u16, |a, b| a | b);
    let and = adjusted.iter().fold(u16::MAX, |a, b| a & b);
    let or_bits: u16 = (1 << 1) | (1 << 8) | (1 << 9);
    let and_bits: u16 = (1 << 2) | (1 << 3);
    let mut out = (or & or_bits) | (and & and_bits);
    if out & 0x0002 != 0 {
        out &= !0x000C;
    }
    out
}

/// `OS/2` of the highest version among the fonts. Fields a lower version lacks come from the
/// first font that has them.
pub(super) fn os2(tables: &[&[u8]], h_metrics: &[(u16, i16)]) -> Result<Vec<u8>> {
    let tag = *b"OS/2";
    let base = first(tables, tag)?;
    let version = values_u16(tables, tag, 0).into_iter().max().unwrap_or(0);
    let len = match version {
        0 => 78,
        1 => 86,
        2..=4 => 96,
        _ => 100,
    };
    let mut out = vec![0u8; len];
    for at in (0..len).step_by(2) {
        if let Some(t) = tables.iter().find(|t| t.len() >= at + 2)
            && let Ok(bytes) = Data::new(tag, t).slice(at, 2)
            && let Some(slot) = out.get_mut(at..at + 2)
        {
            slot.copy_from_slice(bytes);
        }
    }
    if base.len() < 68 {
        return Err(TablesError::invalid("the 'OS/2' table is cut short"));
    }
    set_u16(&mut out, 0, version)?;
    set_u16(&mut out, 2, average_advance(h_metrics))?;
    let mut fs = fs_type(&values_u16(tables, tag, 8));
    if version < 2 {
        fs &= !0x0300;
    }
    if version >= 3 && fs & 0x0008 != 0 {
        fs &= !0x0004;
    }
    set_u16(&mut out, 8, fs)?;
    let rules: [(usize, Rule); 20] = [
        (42, Rule::OrU32),
        (46, Rule::OrU32),
        (50, Rule::OrU32),
        (54, Rule::OrU32),
        (64, Rule::MinU16),
        (66, Rule::MaxU16),
        (68, Rule::MaxI16),
        (70, Rule::MinI16),
        (72, Rule::MaxI16),
        (74, Rule::MaxU16),
        (76, Rule::MaxU16),
        (78, Rule::OrU32),
        (82, Rule::OrU32),
        (86, Rule::MaxI16),
        (88, Rule::MaxI16),
        (90, Rule::First),
        (92, Rule::First),
        (94, Rule::MaxU16),
        (96, Rule::MinU16),
        (98, Rule::MaxU16),
    ];
    for (at, rule) in rules {
        if at < len {
            field(&mut out, tables, tag, at, rule)?;
        }
    }
    Ok(out)
}

/// `post`: version 3 (no names) when that is the highest version among the fonts, else version
/// 2 with the merged glyph names. Fixed pitch only when every font is; the memory hints the
/// largest (the maxima zero).
pub(super) fn post(tables: &[&[u8]], names: &[String]) -> Result<Vec<u8>> {
    let tag = *b"post";
    let mut header = first(tables, tag)?.get(..32).ok_or_else(|| TablesError::invalid("the 'post' table is cut short"))?.to_vec();
    let version = values_u32(tables, tag, 0).into_iter().max().unwrap_or(0x0003_0000);
    field(&mut header, tables, tag, 12, Rule::MinU32)?;
    field(&mut header, tables, tag, 16, Rule::MaxU32)?;
    field(&mut header, tables, tag, 24, Rule::MaxU32)?;
    put_u32_at(&mut header, 20, 0)?;
    put_u32_at(&mut header, 28, 0)?;
    if version == 0x0003_0000 {
        put_u32_at(&mut header, 0, version)?;
        return Ok(header);
    }
    write_post_v2(&header, names)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    #[test]
    fn fs_type_takes_the_least_restrictive_embedding() {
        assert_eq!(fs_type(&[0, 0]), 0);
        assert_eq!(fs_type(&[0, 0x0008]), 0x0008, "installable and editable give editable");
        assert_eq!(fs_type(&[0x0002, 0]), 0x0002, "restricted wins over everything");
        assert_eq!(fs_type(&[0x0004, 0x0008]), 0, "bits 2 and 3 are ANDed, so preview and editable share none");
    }

    #[test]
    fn os2_takes_the_extremes_and_ors_the_ranges() {
        let mut a = vec![0u8; 96];
        let mut b = vec![0u8; 78];
        set_u16(&mut a, 0, 4).unwrap();
        set_u16(&mut a, 68, 800).unwrap();
        set_u16(&mut b, 68, 900).unwrap();
        set_u16(&mut a, 70, (-200i16) as u16).unwrap();
        set_u16(&mut b, 70, (-250i16) as u16).unwrap();
        put_u32_at(&mut a, 42, 1).unwrap();
        put_u32_at(&mut b, 42, 4).unwrap();
        set_u16(&mut a, 88, 700).unwrap();
        let out = os2(&[&a, &b], &[(500, 0), (0, 0), (600, 0)]).unwrap();
        let d = Data::new(*b"OS/2", &out);
        assert_eq!(out.len(), 96);
        assert_eq!(d.u16(2).unwrap(), 550, "the mean of the advances that are not zero");
        assert_eq!(d.i16(68).unwrap(), 900);
        assert_eq!(d.i16(70).unwrap(), -250);
        assert_eq!(d.u32(42).unwrap(), 5);
        assert_eq!(d.i16(88).unwrap(), 700, "only the version 4 table has a cap height");
    }
}
