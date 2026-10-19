//! The item variation store and the delta-set index map, read from bytes, subset and written.
//! A port of `VarStore_subset_varidxes`, `VarStore_prune_regions` and
//! `VarData_calculateNumShorts` (fontTools `varLib.varStore` and `varLib.builder`, MIT,
//! Copyright 2017 Google, Inc.) and of the index map writer of `otTables.DeltaSetIndexMap`.
//!
//! The store is read lazily: a row of deltas is decoded only when the subset keeps it, and the
//! rows decoded in one subset are capped by [`MAX_STORE_DELTAS`].

use std::collections::{BTreeMap, BTreeSet};

use crate::reorder::be::{Data, put_u16, put_u32, to_u16, to_u32};
use crate::sfnt::Tag;
use crate::{Result, TablesError};

/// "No variation": the variation index of a value without deltas.
pub(crate) const NO_VARIATION: u32 = 0xFFFF_FFFF;

/// Most deltas the kept rows of one store may hold.
pub(crate) const MAX_STORE_DELTAS: usize = 1 << 24;

/// One item variation data subtable, its rows still encoded.
struct VarData<'a> {
    regions: Vec<u16>,
    item_count: usize,
    word_count: usize,
    long: bool,
    rows: &'a [u8],
}

impl VarData<'_> {
    fn row_size(&self) -> usize {
        let (wide, narrow) = if self.long { (4, 2) } else { (2, 1) };
        self.word_count * wide + (self.regions.len() - self.word_count) * narrow
    }

    /// The deltas of row `minor`, or `None` past the last row.
    fn row(&self, minor: usize) -> Option<Vec<i32>> {
        if minor >= self.item_count {
            return None;
        }
        let size = self.row_size();
        let bytes = self.rows.get(minor * size..(minor + 1) * size)?;
        let mut out = Vec::with_capacity(self.regions.len());
        let mut at = 0usize;
        for column in 0..self.regions.len() {
            let wide = column < self.word_count;
            let v = match (self.long, wide) {
                (true, true) => i32::from_be_bytes([*bytes.get(at)?, *bytes.get(at + 1)?, *bytes.get(at + 2)?, *bytes.get(at + 3)?]),
                (true, false) | (false, true) => i32::from(i16::from_be_bytes([*bytes.get(at)?, *bytes.get(at + 1)?])),
                (false, false) => i32::from(*bytes.get(at)? as i8),
            };
            at += match (self.long, wide) {
                (true, true) => 4,
                (true, false) | (false, true) => 2,
                (false, false) => 1,
            };
            out.push(v);
        }
        Some(out)
    }
}

/// An item variation store read from the bytes that start at it.
pub(crate) struct Store<'a> {
    tag: Tag,
    format: u16,
    axis_count: u16,
    /// Each region's records, `axis_count` × (start, peak, end), as they are in the input.
    regions: Vec<&'a [u8]>,
    data: Vec<VarData<'a>>,
}

/// One subset item variation data subtable.
struct NewData {
    regions: Vec<u16>,
    items: Vec<Vec<i32>>,
}

/// A store after [`Store::subset`], ready to write.
pub(crate) struct Subset {
    format: u16,
    axis_count: u16,
    regions: Vec<Vec<u8>>,
    data: Vec<NewData>,
}

impl<'a> Store<'a> {
    /// Reads the store at the start of `bytes` (which may run past its end) of table `tag`.
    pub fn read(tag: Tag, bytes: &'a [u8]) -> Result<Self> {
        let d = Data::new(tag, bytes);
        let format = d.u16(0)?;
        if format != 1 {
            return Err(d.bad(format!("has an item variation store of format {format}")));
        }
        let regions_at = d.u32_usize(2)?;
        let count = usize::from(d.u16(6)?);
        let mut regions = Vec::new();
        let mut axis_count = 0u16;
        if regions_at != 0 {
            axis_count = d.u16(regions_at)?;
            let region_count = usize::from(d.u16(regions_at + 2)?);
            let size = 6 * usize::from(axis_count);
            d.slice(regions_at + 4, size * region_count)?;
            regions.reserve(region_count);
            for r in 0..region_count {
                regions.push(d.slice(regions_at + 4 + size * r, size)?);
            }
        }
        let mut data = Vec::with_capacity(count);
        for i in 0..count {
            let at = d.u32_usize(8 + 4 * i)?;
            if at == 0 {
                data.push(VarData { regions: Vec::new(), item_count: 0, word_count: 0, long: false, rows: &[] });
                continue;
            }
            let item_count = usize::from(d.u16(at)?);
            let words = d.u16(at + 2)?;
            let region_count = usize::from(d.u16(at + 4)?);
            let word_count = usize::from(words & 0x7FFF);
            if word_count > region_count {
                return Err(d.bad("has an item variation data subtable with more word deltas than regions"));
            }
            let mut indices = Vec::with_capacity(region_count);
            for r in 0..region_count {
                let index = d.u16(at + 6 + 2 * r)?;
                if usize::from(index) >= regions.len() {
                    return Err(d.bad(format!("uses variation region {index}, which the store lacks")));
                }
                indices.push(index);
            }
            let mut vd = VarData { regions: indices, item_count, word_count, long: words & 0x8000 != 0, rows: &[] };
            let rows_at = at + 6 + 2 * region_count;
            vd.rows = d.slice(rows_at, vd.row_size().saturating_mul(item_count))?;
            data.push(vd);
        }
        Ok(Store { tag, format, axis_count, regions, data })
    }

    /// The store keeping only the delta sets of `used` (variation indices, outer << 16 | inner),
    /// with all-zero region columns and unused regions removed, and the old → new map of the
    /// kept indices ([`NO_VARIATION`] maps to itself).
    ///
    /// With `retain_first`, subtable 0 keeps every row in place (unused rows zeroed). Otherwise
    /// subtable 0 lists the rows of `first` before its other used rows, each group in index
    /// order; rows of `first` must exist. A used index naming no delta set is left out of the
    /// map.
    pub fn subset(&self, used: &BTreeSet<u32>, retain_first: bool, first: &BTreeSet<u16>) -> Result<(Subset, BTreeMap<u32, u32>)> {
        let mut by_major: BTreeMap<u16, BTreeSet<u16>> = BTreeMap::new();
        for v in used {
            if *v != NO_VARIATION {
                by_major.entry((v >> 16) as u16).or_default().insert(*v as u16);
            }
        }
        let mut map = BTreeMap::from([(NO_VARIATION, NO_VARIATION)]);
        let mut out = Vec::new();
        let mut budget = MAX_STORE_DELTAS;
        let mut take = |n: usize| -> Result<()> {
            budget = budget.checked_sub(n).ok_or(TablesError::TooLarge { what: "the kept variation deltas", limit: MAX_STORE_DELTAS })?;
            Ok(())
        };
        for (major, data) in self.data.iter().enumerate() {
            let Some(minors) = by_major.get(&(major as u16)) else { continue };
            let new_major = out.len() as u32;
            let mut items = Vec::new();
            if major == 0 && retain_first {
                for minor in 0..data.item_count {
                    take(data.regions.len())?;
                    let row = if minors.contains(&(minor as u16)) { data.row(minor) } else { None };
                    items.push(row.unwrap_or_else(|| vec![0; data.regions.len()]));
                    map.insert(minor as u32, minor as u32);
                }
            } else {
                let order: Vec<(u16, bool)> = if major == 0 {
                    first.iter().map(|m| (*m, true)).chain(minors.difference(first).map(|m| (*m, false))).collect()
                } else {
                    minors.iter().map(|m| (*m, false)).collect()
                };
                for (minor, required) in order {
                    let Some(row) = data.row(usize::from(minor)) else {
                        if required {
                            return Err(TablesError::invalid(format!(
                                "the '{}' table has no delta set for glyph {minor}",
                                crate::sfnt::tag_str(self.tag)
                            )));
                        }
                        continue;
                    };
                    take(row.len())?;
                    map.insert((major as u32) << 16 | u32::from(minor), new_major << 16 | items.len() as u32);
                    items.push(row);
                }
            }
            out.push(NewData { regions: data.regions.clone(), items });
        }
        for data in &mut out {
            drop_zero_columns(data);
        }
        // Keep the regions still in use, in their order.
        let used_regions: BTreeSet<u16> = out.iter().flat_map(|d| d.regions.iter().copied()).collect();
        let region_map: BTreeMap<u16, u16> = used_regions.iter().enumerate().map(|(new, old)| (*old, new as u16)).collect();
        let regions = used_regions.iter().filter_map(|r| self.regions.get(usize::from(*r)).map(|b| b.to_vec())).collect();
        for data in &mut out {
            for r in &mut data.regions {
                *r = region_map.get(r).copied().unwrap_or(0);
            }
        }
        Ok((Subset { format: self.format, axis_count: self.axis_count, regions, data: out }, map))
    }
}

impl Store<'_> {
    /// The whole store, every region, subtable and row kept, for a table that copies its store.
    pub fn whole(&self) -> Result<Subset> {
        let mut budget = MAX_STORE_DELTAS;
        let mut data = Vec::with_capacity(self.data.len());
        for d in &self.data {
            budget = budget
                .checked_sub(d.item_count.saturating_mul(d.regions.len()))
                .ok_or(TablesError::TooLarge { what: "the variation deltas", limit: MAX_STORE_DELTAS })?;
            let items = (0..d.item_count).map(|minor| d.row(minor).unwrap_or_else(|| vec![0; d.regions.len()])).collect();
            data.push(NewData { regions: d.regions.clone(), items });
        }
        let regions = self.regions.iter().map(|r| r.to_vec()).collect();
        Ok(Subset { format: self.format, axis_count: self.axis_count, regions, data })
    }
}

/// Byte length of a delta as `calculateNumShorts` counts it: 0 for 0, else 1, 2 or 4.
fn byte_len(v: i32) -> u8 {
    match v {
        0 => 0,
        -128..=127 => 1,
        -32_768..=32_767 => 2,
        _ => 4,
    }
}

/// Orders the columns of `data` widest first (keeping their order within a width) and drops
/// the columns that are zero in every row, as `calculateNumShorts(optimize=True)` does.
fn drop_zero_columns(data: &mut NewData) {
    let mut widths = vec![0u8; data.regions.len()];
    for item in &data.items {
        for (w, v) in widths.iter_mut().zip(item) {
            *w = (*w).max(byte_len(*v));
        }
    }
    let mut columns: Vec<usize> = Vec::with_capacity(widths.len());
    for want in [4u8, 2, 1] {
        columns.extend(widths.iter().enumerate().filter(|(_, w)| **w == want).map(|(i, _)| i));
    }
    data.regions = columns.iter().filter_map(|c| data.regions.get(*c).copied()).collect();
    for item in &mut data.items {
        *item = columns.iter().map(|c| item.get(*c).copied().unwrap_or(0)).collect();
    }
}

impl Subset {
    /// No item variation data is left.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// The store in the binary format.
    pub fn write(&self) -> Result<Vec<u8>> {
        let mut regions = Vec::new();
        put_u16(&mut regions, self.axis_count);
        put_u16(&mut regions, to_u16(self.regions.len(), "the variation region count")?);
        for r in &self.regions {
            regions.extend_from_slice(r);
        }
        let mut subtables = Vec::with_capacity(self.data.len());
        for d in &self.data {
            let mut widths = vec![0u8; d.regions.len()];
            for item in &d.items {
                for (w, v) in widths.iter_mut().zip(item) {
                    *w = (*w).max(byte_len(*v));
                }
            }
            let long = widths.iter().any(|w| *w > 2);
            let words = if long {
                widths.iter().rposition(|w| *w > 2).map_or(0, |i| i + 1)
            } else {
                widths.iter().rposition(|w| *w > 1).map_or(0, |i| i + 1)
            };
            let mut t = Vec::new();
            put_u16(&mut t, to_u16(d.items.len(), "the delta set count")?);
            put_u16(&mut t, to_u16(words, "the word delta count")? | if long { 0x8000 } else { 0 });
            put_u16(&mut t, to_u16(d.regions.len(), "the region index count")?);
            for r in &d.regions {
                put_u16(&mut t, *r);
            }
            for item in &d.items {
                for (column, v) in item.iter().enumerate() {
                    match (long, column < words) {
                        (true, true) => put_u32(&mut t, *v as u32),
                        (true, false) | (false, true) => put_u16(&mut t, *v as i16 as u16),
                        (false, false) => t.push(*v as i8 as u8),
                    }
                }
            }
            subtables.push(t);
        }
        let mut out = Vec::new();
        put_u16(&mut out, self.format);
        let header = 8 + 4 * subtables.len();
        put_u32(&mut out, to_u32(header, "the item variation store")?);
        put_u16(&mut out, to_u16(subtables.len(), "the item variation data count")?);
        let mut at = header + regions.len();
        for t in &subtables {
            put_u32(&mut out, to_u32(at, "the item variation store")?);
            at += t.len();
        }
        out.extend(regions);
        for t in subtables {
            out.extend(t);
        }
        Ok(out)
    }
}

/// Reads a delta-set index map at `at` in `d` as variation indices (outer << 16 | inner):
/// at most `limit` entries, then extended to `limit` with its last entry (with
/// [`NO_VARIATION`] when the map is empty), as fontTools reads the glyph maps of HVAR and VVAR.
pub(crate) fn read_index_map(d: Data<'_>, at: usize, limit: usize) -> Result<Vec<u32>> {
    let format = d.u8(at)?;
    let entry_format = d.u8(at + 1)?;
    let (count, start) = match format {
        0 => (usize::from(d.u16(at + 2)?), at + 4),
        1 => (d.u32_usize(at + 2)?, at + 6),
        _ => return Err(d.bad(format!("has a delta-set index map of format {format}"))),
    };
    let inner_bits = u32::from(entry_format & 0x0F) + 1;
    let size = usize::from((entry_format & 0x30) >> 4) + 1;
    let n = count.min(limit);
    let bytes = d.slice(start, n * size)?;
    let mut out = Vec::with_capacity(limit);
    for chunk in bytes.chunks_exact(size) {
        let entry = chunk.iter().fold(0u32, |acc, b| acc << 8 | u32::from(*b));
        let outer = entry >> inner_bits;
        let inner = entry & ((1u32 << inner_bits) - 1);
        out.push(outer << 16 | inner);
    }
    let last = out.last().copied().unwrap_or(NO_VARIATION);
    out.resize(limit, last);
    Ok(out)
}

/// `mapping` as a delta-set index map in format 0, with its repeated tail trimmed, the entry
/// size and inner bit count `DeltaSetIndexMap.getEntryFormat` picks.
pub(crate) fn write_glyph_map(mapping: &[u32]) -> Result<Vec<u8>> {
    let mut mapping = mapping.to_vec();
    while mapping.len() > 1 && mapping.get(mapping.len() - 2) == mapping.last() {
        mapping.pop();
    }
    let ored = mapping.iter().fold(0u32, |a, v| a | v);
    let inner_bits = (32 - (ored & 0xFFFF).leading_zeros()).max(1);
    let packed = (ored >> (16 - inner_bits)) | (ored & ((1u32 << inner_bits) - 1));
    let size: usize = match packed {
        0..=0xFF => 1,
        0x100..=0xFFFF => 2,
        0x1_0000..=0xFF_FFFF => 3,
        _ => 4,
    };
    let mut out = vec![0u8, (((size - 1) << 4) as u8) | (inner_bits - 1) as u8];
    put_u16(&mut out, to_u16(mapping.len(), "the delta-set index map")?);
    for v in &mapping {
        let entry = (v >> 16) << inner_bits | (v & 0xFFFF);
        out.extend_from_slice(entry.to_be_bytes().get(4 - size..).unwrap_or_default());
    }
    Ok(out)
}
