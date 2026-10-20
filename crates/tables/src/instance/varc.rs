//! `VARC` in instances: a port of `_getVARCAxisIndexRecords`, `_checkVARCAxisLimits` and
//! `instantiateVARC` of fontTools' `varLib.instancer` (MIT, Copyright (c) 2019 Google LLC and
//! the fontTools authors).
//!
//! The axes the table uses (in the regions of its multi-item variation store, in its axis-range
//! conditions and in its axis index lists) cannot be pinned or limited. Other axes can; when
//! they leave `fvar`, the table's axis indexes are renumbered. The variation data itself does
//! not change.

use std::collections::BTreeSet;

use crate::sfnt::{SfntFont, Tag, tag_str};
use crate::{Result, TablesError};

/// Most condition tables visited in one `VARC` table.
const MAX_CONDITIONS: usize = 1 << 16;
/// Most values in one `VARC` table's axis index lists; an empty list counts as one.
const MAX_AXIS_INDEXES: usize = 1 << 20;
/// Most axis records read from the regions of one `VARC` table's store. Regions at different
/// offsets can overlap, so the bytes of the table do not bound this on their own.
const MAX_REGION_AXIS_RECORDS: usize = 1 << 20;

fn bad(what: impl Into<String>) -> TablesError {
    TablesError::invalid(what)
}

/// `base + rel` for an offset read from the table; a hostile offset can reach past `usize` on
/// 32-bit targets.
fn add(base: usize, rel: usize) -> Result<usize> {
    base.checked_add(rel).ok_or_else(|| bad("an offset in the VARC table points past its end"))
}

fn u8_at(d: &[u8], at: usize) -> Result<usize> {
    d.get(at).map(|b| usize::from(*b)).ok_or_else(|| bad("the VARC table is truncated"))
}

fn u16_at(d: &[u8], at: usize) -> Result<usize> {
    d.get(at..at.saturating_add(2))
        .and_then(|b| <[u8; 2]>::try_from(b).ok())
        .map(|b| usize::from(u16::from_be_bytes(b)))
        .ok_or_else(|| bad("the VARC table is truncated"))
}

fn u24_at(d: &[u8], at: usize) -> Result<usize> {
    Ok((u8_at(d, at)? << 16) | u16_at(d, at.saturating_add(1))?)
}

fn u32_at(d: &[u8], at: usize) -> Result<usize> {
    d.get(at..at.saturating_add(4))
        .and_then(|b| <[u8; 4]>::try_from(b).ok())
        .map(|b| u32::from_be_bytes(b) as usize)
        .ok_or_else(|| bad("the VARC table is truncated"))
}

/// Where the table names axes: the byte positions of its 16-bit axis indexes (regions and
/// axis-range conditions, each position once), and its axis index lists.
struct AxisRecords {
    positions: Vec<usize>,
    lists: Vec<Vec<i64>>,
}

fn axis_records(data: &[u8]) -> Result<AxisRecords> {
    let mut positions: BTreeSet<usize> = BTreeSet::new();
    let store = u32_at(data, 8)?;
    if store != 0 {
        let regions = add(store, u32_at(data, add(store, 2)?)?)?;
        let count = u16_at(data, regions)?;
        let mut seen: BTreeSet<usize> = BTreeSet::new();
        let mut records = 0usize;
        for r in 0..count {
            let region = add(regions, u32_at(data, regions + 2 + 4 * r)?)?;
            if !seen.insert(region) {
                continue;
            }
            let axes = u16_at(data, region)?;
            records = records.saturating_add(axes);
            if records > MAX_REGION_AXIS_RECORDS {
                return Err(TablesError::TooLarge { what: "the region axis records of a VARC table", limit: MAX_REGION_AXIS_RECORDS });
            }
            for k in 0..axes {
                let at = region + 2 + 8 * k;
                u16_at(data, at)?;
                positions.insert(at);
            }
        }
    }
    let list = u32_at(data, 12)?;
    if list != 0 {
        let count = u32_at(data, list)?;
        let mut stack: Vec<usize> = Vec::new();
        for c in 0..count.min(MAX_CONDITIONS) {
            stack.push(add(list, u32_at(data, list + 4 + 4 * c)?)?);
        }
        let mut seen: BTreeSet<usize> = BTreeSet::new();
        while let Some(at) = stack.pop() {
            if !seen.insert(at) {
                continue;
            }
            if seen.len() > MAX_CONDITIONS {
                return Err(TablesError::TooLarge { what: "the conditions of a VARC table", limit: MAX_CONDITIONS });
            }
            match u16_at(data, at)? {
                1 => {
                    u16_at(data, at + 2)?;
                    positions.insert(at + 2);
                }
                3 | 4 => {
                    let n = u8_at(data, at + 2)?;
                    for k in 0..n {
                        stack.push(add(at, u24_at(data, at + 3 + 3 * k)?)?);
                    }
                }
                5 => stack.push(add(at, u24_at(data, at + 2)?)?),
                _ => {}
            }
        }
    }
    let indices = u32_at(data, 16)?;
    let lists = if indices == 0 { Vec::new() } else { read_index(data, indices)? };
    Ok(AxisRecords { positions: positions.into_iter().collect(), lists })
}

/// The packed value lists of the CFF2-style index at `at`.
fn read_index(data: &[u8], at: usize) -> Result<Vec<Vec<i64>>> {
    // Every list counts at least one against the limit, so a larger count fails before its
    // offsets are read.
    if u32_at(data, at)? > MAX_AXIS_INDEXES {
        return Err(TablesError::TooLarge { what: "the axis indexes of a VARC table", limit: MAX_AXIS_INDEXES });
    }
    let index = fontcraft_cff::raw::read_index(data, at, true).map_err(|e| bad(format!("the VARC axis index list cannot be read: {e}")))?;
    let mut lists = Vec::new();
    let mut total = 0usize;
    for item in index.items {
        let values = unpack(item)?;
        total = total.saturating_add(values.len().max(1));
        if total > MAX_AXIS_INDEXES {
            return Err(TablesError::TooLarge { what: "the axis indexes of a VARC table", limit: MAX_AXIS_INDEXES });
        }
        lists.push(values);
    }
    Ok(lists)
}

/// Packed values: runs of zeros, bytes, words or longs, each led by a control byte.
fn unpack(mut d: &[u8]) -> Result<Vec<i64>> {
    let mut out = Vec::new();
    while let Some((&control, rest)) = d.split_first() {
        let n = usize::from(control & 0x3F) + 1;
        let width = match control & 0xC0 {
            0x80 => 0,
            0x00 => 1,
            0x40 => 2,
            _ => 4,
        };
        let body = rest.get(..n * width).ok_or_else(|| bad("a VARC axis index list is cut short"))?;
        if width == 0 {
            out.extend(std::iter::repeat_n(0, n));
        } else {
            for chunk in body.chunks(width) {
                let v = chunk.iter().fold(0i64, |acc, b| (acc << 8) | i64::from(*b));
                let bits = 8 * width as u32;
                out.push(if v >= 1 << (bits - 1) { v - (1 << bits) } else { v });
            }
        }
        d = rest.get(n * width..).unwrap_or_default();
    }
    Ok(out)
}

fn pack(values: &[i64]) -> Vec<u8> {
    let mut out = Vec::new();
    for run in values.chunks(64) {
        let n = run.len() as u8 - 1;
        let fits = |lo: i64, hi: i64| run.iter().all(|v| (lo..=hi).contains(v));
        if run.iter().all(|v| *v == 0) {
            out.push(0x80 | n);
        } else if fits(-128, 127) {
            out.push(n);
            out.extend(run.iter().map(|v| *v as i8 as u8));
        } else if fits(-32768, 32767) {
            out.push(0x40 | n);
            for v in run {
                out.extend_from_slice(&(*v as i16).to_be_bytes());
            }
        } else {
            out.push(0xC0 | n);
            for v in run {
                out.extend_from_slice(&(*v as i32).to_be_bytes());
            }
        }
    }
    out
}

fn write_index(lists: &[Vec<i64>]) -> Result<Vec<u8>> {
    let items: Vec<Vec<u8>> = lists.iter().map(|l| pack(l)).collect();
    let total = items.iter().map(Vec::len).sum::<usize>() + 1;
    let size = match total {
        0..=0xFF => 1,
        0x100..=0xFFFF => 2,
        0x1_0000..=0xFF_FFFF => 3,
        _ => 4,
    };
    let count = u32::try_from(items.len()).map_err(|_| bad("too many VARC axis index lists"))?;
    let mut out = count.to_be_bytes().to_vec();
    out.push(size as u8);
    let mut at = 1usize;
    for item in items.iter().map(Vec::len).chain(std::iter::once(0)) {
        out.extend_from_slice(
            u32::try_from(at).map_err(|_| bad("the VARC axis index list is too large"))?.to_be_bytes().get(4 - size..).unwrap_or_default(),
        );
        at += item;
    }
    for item in &items {
        out.extend_from_slice(item);
    }
    Ok(out)
}

/// Refuses limits on an axis the font's `VARC` table uses; `limited` is by `fvar` index.
pub(crate) fn check_limits(font: &SfntFont, axes: &[Tag], limited: &[bool]) -> Result<()> {
    let Some(data) = font.get(*b"VARC") else { return Ok(()) };
    let records = axis_records(data)?;
    let mut used: BTreeSet<usize> = BTreeSet::new();
    for at in &records.positions {
        used.insert(u16_at(data, *at)?);
    }
    for list in &records.lists {
        used.extend(list.iter().filter_map(|v| usize::try_from(*v).ok()));
    }
    for i in used {
        let Some(tag) = axes.get(i) else {
            return Err(bad(format!("the VARC table names axis {i}, which the font does not have")));
        };
        if limited.get(i).copied().unwrap_or(false) {
            return Err(TablesError::Unsupported(format!(
                "instancing the `{}` axis, which the VARC table's components use",
                tag_str(*tag).trim_end()
            )));
        }
    }
    Ok(())
}

/// Renumbers the axis indexes of the `VARC` table of `out` once the `removed` axes leave
/// `fvar`. [`check_limits`] has made sure no removed axis is used.
pub(crate) fn renumber(out: &mut SfntFont, removed: &[bool]) -> Result<()> {
    let Some(data) = out.get(*b"VARC") else { return Ok(()) };
    if !removed.iter().any(|r| *r) {
        return Ok(());
    }
    let new_index = |i: usize| -> usize { removed.iter().take(i).filter(|r| !**r).count() };
    let records = axis_records(data)?;
    let mut table = data.to_vec();
    for at in &records.positions {
        let old = u16_at(&table, *at)?;
        let new = u16::try_from(new_index(old)).map_err(|_| bad("a VARC axis index is out of range"))?;
        if let Some(slot) = table.get_mut(*at..at.saturating_add(2)) {
            slot.copy_from_slice(&new.to_be_bytes());
        }
    }
    let changed = records.lists.iter().flatten().any(|v| usize::try_from(*v).is_ok_and(|i| new_index(i) != i));
    if changed {
        let lists: Vec<Vec<i64>> =
            records.lists.iter().map(|l| l.iter().map(|v| usize::try_from(*v).map_or(*v, |i| new_index(i) as i64)).collect()).collect();
        // The new lists go at the end; the old ones stay behind, unused.
        let at = u32::try_from(table.len()).map_err(|_| bad("the VARC table is too large"))?;
        table.extend_from_slice(&write_index(&lists)?);
        if let Some(slot) = table.get_mut(16..20) {
            slot.copy_from_slice(&at.to_be_bytes());
        }
        if table.len() > crate::limits::MAX_SFNT {
            return Err(TablesError::TooLarge { what: "the VARC table", limit: crate::limits::MAX_SFNT });
        }
    }
    out.insert(*b"VARC", table);
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    /// A `VARC` table with a store whose one region uses axis 2, a condition list with an
    /// `and` of an axis-range condition on axis 3, and one axis index list [2, 3, 200].
    fn varc() -> Vec<u8> {
        let mut t = Vec::new();
        t.extend_from_slice(&[0, 1, 0, 0]);
        t.extend_from_slice(&0u32.to_be_bytes()); // coverage (not read here)
        t.extend_from_slice(&24u32.to_be_bytes()); // store
        t.extend_from_slice(&50u32.to_be_bytes()); // conditions
        t.extend_from_slice(&72u32.to_be_bytes()); // axis indices
        t.extend_from_slice(&0u32.to_be_bytes());
        // store at 24: format, region list at +8 (32), no data
        t.extend_from_slice(&[0, 1, 0, 0, 0, 8, 0, 0]);
        // region list at 32: one region at +6 (38)
        t.extend_from_slice(&[0, 1, 0, 0, 0, 6]);
        // region at 38: one axis record, axis 2, (0, 1, 1)
        t.extend_from_slice(&[0, 1, 0, 2, 0, 0, 0x40, 0, 0x40, 0]);
        // 48: padding
        t.extend_from_slice(&[0, 0]);
        // condition list at 50: one condition at +8 (58)
        t.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 8]);
        // 58: format 3 with one condition at +6 (64)
        t.extend_from_slice(&[0, 3, 1, 0, 0, 6]);
        // 64: format 1 on axis 3
        t.extend_from_slice(&[0, 1, 0, 3, 0, 0, 0x40, 0]);
        // 72: index of one item: [2, 3, 200] as words
        let item = pack(&[2, 3, 200]);
        t.extend_from_slice(&1u32.to_be_bytes());
        t.push(1);
        t.extend_from_slice(&[1, 1 + item.len() as u8]);
        t.extend_from_slice(&item);
        t
    }

    /// A `VARC` table with only a store whose region list holds 65,535 offsets from `offset(k)`,
    /// followed by `regions`.
    fn varc_with_regions(offset: impl Fn(u32) -> u32, regions: &[u8]) -> Vec<u8> {
        let mut t = vec![0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 24];
        t.resize(24, 0);
        t.extend_from_slice(&[0, 1, 0, 0, 0, 8, 0, 0]);
        t.extend_from_slice(&[0xFF, 0xFF]);
        for k in 0..0xFFFF {
            t.extend_from_slice(&offset(k).to_be_bytes());
        }
        t.extend_from_slice(regions);
        t
    }

    #[test]
    fn repeated_or_overlapping_regions_finish_quickly() {
        let after_list = 2 + 4 * 0xFFFF;
        // Every offset names one region with 60,000 axis records on axis 0.
        let mut one = vec![0xEA, 0x60];
        one.resize(2 + 8 * 60_000, 0);
        let mut font = SfntFont::new(crate::sfnt::Flavour::TrueType);
        font.insert(*b"VARC", varc_with_regions(|_| after_list, &one));
        check_limits(&font, &[*b"wght"], &[false]).unwrap();
        // Offsets two bytes apart into 0x7F bytes: 65,535 regions of 32,639 records each.
        font.insert(*b"VARC", varc_with_regions(|k| after_list + 2 * k, &std::iter::repeat_n(0x7F, 400_000).collect::<Vec<u8>>()));
        assert!(matches!(check_limits(&font, &[*b"wght"], &[false]), Err(TablesError::TooLarge { .. })));
    }

    #[test]
    fn empty_axis_index_lists_count_against_the_limit() {
        let count = MAX_AXIS_INDEXES as u32 + 1;
        let mut t = vec![0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 24, 0, 0, 0, 0];
        t.extend_from_slice(&count.to_be_bytes());
        t.push(1);
        t.resize(t.len() + count as usize + 1, 1);
        let mut font = SfntFont::new(crate::sfnt::Flavour::TrueType);
        font.insert(*b"VARC", t);
        assert!(matches!(check_limits(&font, &[*b"wght"], &[false]), Err(TablesError::TooLarge { .. })));
    }

    #[test]
    fn varc_axes_cannot_be_limited_and_the_others_renumber() {
        let tags: Vec<Tag> = (0..201u16).map(|i| [b'a', b'x', (i / 16) as u8 + b'a', (i % 16) as u8 + b'a']).collect();
        let mut font = SfntFont::new(crate::sfnt::Flavour::TrueType);
        font.insert(*b"VARC", varc());
        let mut limited = vec![false; tags.len()];
        limited[3] = true;
        assert!(matches!(check_limits(&font, &tags, &limited), Err(TablesError::Unsupported(_))));
        limited[3] = false;
        limited[0] = true;
        limited[1] = true;
        check_limits(&font, &tags, &limited).unwrap();
        renumber(&mut font, &limited).unwrap();
        let data = font.get(*b"VARC").unwrap();
        let records = axis_records(data).unwrap();
        let named: Vec<usize> = records.positions.iter().map(|at| u16_at(data, *at).unwrap()).collect();
        assert_eq!(named, vec![0, 1], "the region's axis 2 and the condition's axis 3");
        assert_eq!(records.lists, vec![vec![0, 1, 198]]);
    }
}
