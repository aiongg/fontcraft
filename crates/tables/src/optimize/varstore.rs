//! The `varstore` step of `font.optimize`: the item variation store of `GDEF`, which `GPOS`
//! shares, merged and packed again. A port of fontTools' `VarStore_optimize`,
//! `VarData_calculateNumShorts(optimize=True)`, `VarStore_prune_regions` and
//! `remap_device_varidxes` (`varLib.varStore` and `varLib.builder`, MIT, Copyright 2017 Google,
//! Inc.), run as `fonttools varLib.varStore FONT OUT [--quantization Q]` runs them.
//!
//! Every row is widened to one column per region (deltas for the same region added), quantised
//! when asked, and rows that are all zero become "no variation" (`0xFFFF/0xFFFF`). Identical rows
//! are shared. Rows are grouped by their characteristic (how many bytes each column needs), and
//! groups are merged greedily, the merge that saves most bytes first, as long as one saves any.
//! Each group becomes one or more subtables of at most 65,535 rows, sorted; columns are ordered
//! widest first and empty columns dropped; regions no subtable uses are removed.
//!
//! `GPOS` keeps its bytes but for the variation indices of its device tables, patched in place.
//! `GDEF` is written again with the new store and its caret device tables remapped.
//!
//! Limits: [`MAX_STORE_CELLS`] deltas after widening, [`MAX_MERGE_WORK`] for the merge search,
//! [`MAX_DEVICE_WALK`] records visited in `GPOS`.

use std::cmp::{Ordering, Reverse};
use std::collections::{BTreeSet, BinaryHeap, HashMap};

use read_fonts::tables::gdef::Gdef as ReadGdef;
use read_fonts::tables::variations::ItemVariationStore as ReadStore;
use read_fonts::{FontData, FontRead};
use write_fonts::NullableOffsetMarker;
use write_fonts::from_obj::ToOwnedTable;
use write_fonts::tables::gdef::{CaretValue, Gdef};
use write_fonts::tables::layout::DeviceOrVariationIndex;

use crate::sfnt::SfntFont;
use crate::{Result, TablesError, Tag};

/// A table's tag and its new data.
type Table = (Tag, Vec<u8>);

/// Most deltas the store may hold once every row has a column per region.
pub const MAX_STORE_CELLS: usize = 1 << 24;
/// Most work (characteristic words compared) the merge search may do.
pub const MAX_MERGE_WORK: usize = 1 << 28;
/// Most lookups, subtables and records visited in `GPOS` looking for device tables.
pub const MAX_DEVICE_WALK: usize = crate::limits::MAX_LAYOUT_ITEMS;
/// Largest quantum.
pub const MAX_QUANTUM: u32 = 32_767;
/// "No variation".
const NO_VARIATION: u32 = 0xFFFF_FFFF;
/// Most rows in one subtable.
const MAX_ROWS: usize = 0xFFFF;

fn bad(what: impl Into<String>) -> TablesError {
    TablesError::invalid(what)
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes([*b.get(at)?, *b.get(at + 1)?, *b.get(at + 2)?, *b.get(at + 3)?]))
}

/// A store as rows of deltas, each with its region indices.
struct Store {
    axis_count: u16,
    /// Each region's raw records.
    regions: Vec<Vec<u8>>,
    /// Per subtable: region indices and rows.
    data: Vec<(Vec<u16>, Vec<Vec<i32>>)>,
}

fn read_store(b: &[u8]) -> std::result::Result<Store, String> {
    if u16_at(b, 0) != Some(1) {
        return Err("the item variation store is not format 1".into());
    }
    let regions_at = u32_at(b, 2).ok_or("truncated store")? as usize;
    let count = usize::from(u16_at(b, 6).ok_or("truncated store")?);
    let mut regions = Vec::new();
    let mut axis_count = 0u16;
    if regions_at != 0 {
        axis_count = u16_at(b, regions_at).ok_or("truncated region list")?;
        let n = usize::from(u16_at(b, regions_at + 2).ok_or("truncated region list")?);
        let size = 6 * usize::from(axis_count);
        for r in 0..n {
            let at = regions_at + 4 + size * r;
            regions.push(b.get(at..at + size).ok_or("truncated region list")?.to_vec());
        }
    }
    let mut data = Vec::with_capacity(count);
    let mut cells = 0usize;
    for i in 0..count {
        let at = u32_at(b, 8 + 4 * i).ok_or("truncated store")? as usize;
        if at == 0 {
            data.push((Vec::new(), Vec::new()));
            continue;
        }
        let items = usize::from(u16_at(b, at).ok_or("truncated subtable")?);
        let words = u16_at(b, at + 2).ok_or("truncated subtable")?;
        let rc = usize::from(u16_at(b, at + 4).ok_or("truncated subtable")?);
        let long = words & 0x8000 != 0;
        let wc = usize::from(words & 0x7FFF);
        if wc > rc {
            return Err("a subtable has more word columns than regions".into());
        }
        let mut idx = Vec::with_capacity(rc);
        for r in 0..rc {
            let v = u16_at(b, at + 6 + 2 * r).ok_or("truncated subtable")?;
            if usize::from(v) >= regions.len() {
                return Err(format!("a subtable uses region {v}, which the store lacks"));
            }
            idx.push(v);
        }
        cells = cells.saturating_add(items.saturating_mul(regions.len().max(1)));
        if cells > MAX_STORE_CELLS {
            return Err(format!("the store would hold more than {MAX_STORE_CELLS} deltas"));
        }
        let (wide, narrow) = if long { (4, 2) } else { (2, 1) };
        let mut p = at + 6 + 2 * rc;
        let mut rows = Vec::with_capacity(items);
        for _ in 0..items {
            let mut row = Vec::with_capacity(rc);
            for c in 0..rc {
                let size = if c < wc { wide } else { narrow };
                let v = match size {
                    4 => u32_at(b, p).map(|v| v as i32),
                    2 => u16_at(b, p).map(|v| i32::from(v as i16)),
                    _ => b.get(p).map(|v| i32::from(*v as i8)),
                };
                row.push(v.ok_or("truncated deltas")?);
                p += size;
            }
            rows.push(row);
        }
        data.push((idx, rows));
    }
    Ok(Store { axis_count, regions, data })
}

/// A characteristic: four bits per column (bit 0 a nonzero delta, bit 1 one that needs more
/// than a byte, bits 2 and 3 more than two), sixteen columns to a word, as fontTools' integer.
#[derive(Clone, PartialEq, Eq, Hash)]
struct Chars(Vec<u64>);

impl Chars {
    fn of(row: &[i32]) -> Self {
        let mut words = vec![0u64; row.len().div_ceil(16)];
        let long = row.iter().any(|v| !(-32768..=32767).contains(v));
        for (c, v) in row.iter().enumerate() {
            let nibble: u64 = if long {
                if *v == 0 {
                    0
                } else if (-32768..=32767).contains(v) {
                    0b0011
                } else {
                    0b1111
                }
            } else if *v == 0 {
                0
            } else if (-128..=127).contains(v) {
                0b0001
            } else {
                0b0011
            };
            if let Some(w) = words.get_mut(c / 16) {
                *w |= nibble << (4 * (c % 16));
            }
        }
        Chars(words)
    }

    fn or(&self, other: &Chars) -> Chars {
        Chars(self.0.iter().zip(&other.0).map(|(a, b)| a | b).collect())
    }

    fn width(&self) -> i64 {
        self.0.iter().map(|w| i64::from(w.count_ones())).sum()
    }

    /// 10 bytes of header and offset, and 2 per column in use.
    fn overhead(&self) -> i64 {
        let columns: i64 = self
            .0
            .iter()
            .map(|w| {
                let any = (w | (w >> 1) | (w >> 2) | (w >> 3)) & 0x1111_1111_1111_1111;
                i64::from(any.count_ones())
            })
            .sum();
        10 + 2 * columns
    }

    /// As fontTools compares the integers: the highest column first.
    fn cmp_value(&self, other: &Chars) -> Ordering {
        self.0.iter().rev().cmp(other.0.iter().rev())
    }
}

struct Encoding {
    chars: Chars,
    width: i64,
    overhead: i64,
    items: Vec<usize>,
}

impl Encoding {
    fn new(chars: Chars) -> Self {
        let (width, overhead) = (chars.width(), chars.overhead());
        Encoding { chars, width, overhead, items: Vec::new() }
    }

    fn sort_key(&self, other: &Encoding) -> Ordering {
        self.width.cmp(&other.width).then_with(|| self.chars.cmp_value(&other.chars))
    }

    fn gain(&self, other: &Encoding) -> i64 {
        let combined = self.chars.or(&other.chars);
        let w = combined.width();
        self.overhead + other.overhead
            - combined.overhead()
            - (w - self.width) * self.items.len() as i64
            - (w - other.width) * other.items.len() as i64
    }
}

/// The optimised store's bytes and the old → new variation indices.
fn optimize_store(store: &Store, quantum: u32) -> std::result::Result<(Vec<u8>, HashMap<u32, u32>), String> {
    let n = store.regions.len();
    let mut unique: HashMap<Vec<i32>, usize> = HashMap::new();
    let mut rows: Vec<Vec<i32>> = Vec::new();
    let mut front: Vec<(u32, Option<usize>)> = Vec::new();
    for (major, (idx, items)) in store.data.iter().enumerate() {
        for (minor, item) in items.iter().enumerate() {
            let mut row = vec![0i64; n];
            for (r, v) in idx.iter().zip(item) {
                let v = if quantum == 1 {
                    i64::from(*v)
                } else {
                    let q = f64::from(quantum);
                    (f64::from(*v) / q).round_ties_even() as i64 * i64::from(quantum)
                };
                if let Some(cell) = row.get_mut(usize::from(*r)) {
                    *cell += v;
                }
            }
            let row: Vec<i32> =
                row.into_iter().map(i32::try_from).collect::<std::result::Result<_, _>>().map_err(|_| "a delta beyond the 32-bit range")?;
            let old = ((major as u32) << 16) | minor as u32;
            if row.iter().all(|v| *v == 0) {
                front.push((old, None));
                continue;
            }
            let id = *unique.entry(row.clone()).or_insert_with(|| {
                rows.push(row);
                rows.len() - 1
            });
            front.push((old, Some(id)));
        }
    }
    drop(unique);

    // Group the rows by characteristic.
    let mut by_chars: HashMap<Chars, usize> = HashMap::new();
    let mut todo: Vec<Option<Encoding>> = Vec::new();
    for (id, row) in rows.iter().enumerate() {
        let chars = Chars::of(row);
        let k = *by_chars.entry(chars.clone()).or_insert_with(|| {
            todo.push(Some(Encoding::new(chars)));
            todo.len() - 1
        });
        if let Some(Some(e)) = todo.get_mut(k) {
            e.items.push(id);
        }
    }
    drop(by_chars);
    todo.sort_by(|a, b| match (a, b) {
        (Some(a), Some(b)) => a.sort_key(b),
        _ => Ordering::Equal,
    });

    let words = n.div_ceil(16).max(1);
    let mut work = MAX_MERGE_WORK;
    let mut spend = |units: usize| -> std::result::Result<(), String> {
        work = work
            .checked_sub(units.saturating_mul(words))
            .ok_or_else(|| format!("merging the store's subtables needs more than {MAX_MERGE_WORK} steps"))?;
        Ok(())
    };
    let mut heap: BinaryHeap<Reverse<(i64, usize, usize)>> = BinaryHeap::new();
    let len = todo.len();
    spend(len.saturating_mul(len) / 2)?;
    for i in 0..len {
        for j in i + 1..len {
            if let (Some(Some(a)), Some(Some(b))) = (todo.get(i), todo.get(j)) {
                let g = a.gain(b);
                if g > 0 {
                    heap.push(Reverse((-g, i, j)));
                }
            }
        }
    }
    while let Some(Reverse((_, i, j))) = heap.pop() {
        if !matches!((todo.get(i), todo.get(j)), (Some(Some(_)), Some(Some(_)))) {
            continue;
        }
        let (Some(a), Some(b)) = (todo.get_mut(i).and_then(Option::take), todo.get_mut(j).and_then(Option::take)) else { continue };
        let mut combined = Encoding::new(b.chars.or(&a.chars));
        combined.items.extend(a.items);
        combined.items.extend(b.items);
        spend(todo.len())?;
        let next = todo.len();
        for (k, slot) in todo.iter_mut().enumerate() {
            let Some(e) = slot else { continue };
            if e.chars == combined.chars {
                combined.items.append(&mut e.items);
                *slot = None;
                continue;
            }
            let g = combined.gain(e);
            if g > 0 {
                heap.push(Reverse((-g, k, next)));
            }
        }
        todo.push(Some(combined));
    }
    let mut encodings: Vec<Encoding> = todo.into_iter().flatten().collect();
    encodings.sort_by(|a, b| a.sort_key(b));

    // Assemble: subtables of sorted rows, every region a column, then packed.
    let mut back: Vec<u32> = vec![NO_VARIATION; rows.len()];
    let mut subtables: Vec<Vec<usize>> = Vec::new();
    for mut e in encodings {
        e.items.sort_by(|a, b| rows.get(*a).cmp(&rows.get(*b)));
        for chunk in e.items.chunks(MAX_ROWS) {
            let major = u32::try_from(subtables.len()).ok().filter(|m| *m < 0xFFFF).ok_or("too many subtables")?;
            for (minor, id) in chunk.iter().enumerate() {
                if let Some(slot) = back.get_mut(*id) {
                    *slot = (major << 16) | minor as u32;
                }
            }
            subtables.push(chunk.to_vec());
        }
    }
    let mut map = HashMap::with_capacity(front.len() + 1);
    map.insert(NO_VARIATION, NO_VARIATION);
    for (old, id) in front {
        map.insert(old, id.and_then(|i| back.get(i).copied()).unwrap_or(NO_VARIATION));
    }

    // calculateNumShorts(optimize=True): columns widest first, empty ones dropped.
    let byte_len = |v: i32| match v {
        0 => 0u8,
        -128..=127 => 1,
        -32_768..=32_767 => 2,
        _ => 4,
    };
    let mut packed: Vec<(Vec<usize>, Vec<Vec<i32>>)> = Vec::with_capacity(subtables.len());
    for ids in &subtables {
        let mut widths = vec![0u8; n];
        for id in ids {
            for (w, v) in widths.iter_mut().zip(rows.get(*id).into_iter().flatten()) {
                *w = (*w).max(byte_len(*v));
            }
        }
        let mut columns = Vec::new();
        for want in [4u8, 2, 1] {
            columns.extend(widths.iter().enumerate().filter(|(_, w)| **w == want).map(|(c, _)| c));
        }
        let items = ids.iter().map(|id| columns.iter().map(|c| rows.get(*id).and_then(|r| r.get(*c)).copied().unwrap_or(0)).collect()).collect();
        packed.push((columns, items));
    }
    // prune_regions: the regions in use, in their order.
    let used: BTreeSet<usize> = packed.iter().flat_map(|(c, _)| c.iter().copied()).collect();
    let region_map: HashMap<usize, u16> = used.iter().enumerate().map(|(new, old)| (*old, new as u16)).collect();
    let regions: Vec<&Vec<u8>> = used.iter().filter_map(|r| store.regions.get(*r)).collect();
    let data: Vec<(Vec<u16>, Vec<Vec<i32>>)> =
        packed.into_iter().map(|(cols, items)| (cols.iter().map(|c| region_map.get(c).copied().unwrap_or(0)).collect(), items)).collect();
    Ok((write_store(store.axis_count, &regions, &data)?, map))
}

fn put16(out: &mut Vec<u8>, v: usize) -> std::result::Result<(), String> {
    out.extend_from_slice(&u16::try_from(v).map_err(|_| "a count beyond 65,535 in the store")?.to_be_bytes());
    Ok(())
}

fn put32(out: &mut Vec<u8>, v: usize) -> std::result::Result<(), String> {
    out.extend_from_slice(&u32::try_from(v).map_err(|_| "the store is too large")?.to_be_bytes());
    Ok(())
}

/// A format 1 store: header, region list, then the subtables.
fn write_store(axis_count: u16, regions: &[&Vec<u8>], data: &[(Vec<u16>, Vec<Vec<i32>>)]) -> std::result::Result<Vec<u8>, String> {
    let mut region_list = Vec::new();
    region_list.extend_from_slice(&axis_count.to_be_bytes());
    put16(&mut region_list, regions.len())?;
    for r in regions {
        region_list.extend_from_slice(r);
    }
    let mut subtables = Vec::with_capacity(data.len());
    for (idx, items) in data {
        let byte_len = |v: i32| -> u8 {
            if v == 0 {
                0
            } else if (-128..=127).contains(&v) {
                1
            } else if (-32768..=32767).contains(&v) {
                2
            } else {
                4
            }
        };
        let mut widths = vec![0u8; idx.len()];
        for item in items {
            for (w, v) in widths.iter_mut().zip(item) {
                *w = (*w).max(byte_len(*v));
            }
        }
        let long = widths.iter().any(|w| *w > 2);
        let words = widths.iter().rposition(|w| if long { *w > 2 } else { *w > 1 }).map_or(0, |i| i + 1);
        let mut t = Vec::new();
        put16(&mut t, items.len())?;
        put16(&mut t, words)?;
        if long && let Some(hi) = t.get_mut(2) {
            *hi |= 0x80;
        }
        put16(&mut t, idx.len())?;
        for r in idx {
            t.extend_from_slice(&r.to_be_bytes());
        }
        for item in items {
            for (c, v) in item.iter().enumerate() {
                match (long, c < words) {
                    (true, true) => t.extend_from_slice(&v.to_be_bytes()),
                    (true, false) | (false, true) => t.extend_from_slice(&(*v as i16).to_be_bytes()),
                    (false, false) => t.push(*v as i8 as u8),
                }
            }
        }
        subtables.push(t);
    }
    let mut out = Vec::new();
    out.extend_from_slice(&1u16.to_be_bytes());
    let header = 8 + 4 * subtables.len();
    put32(&mut out, header)?;
    put16(&mut out, subtables.len())?;
    let mut at = header + region_list.len();
    for t in &subtables {
        put32(&mut out, at)?;
        at += t.len();
    }
    out.extend(region_list);
    for t in subtables {
        out.extend(t);
    }
    Ok(out)
}

/// The positions of the `VariationIndex` device tables `GPOS` reaches.
struct DeviceWalk<'a> {
    b: &'a [u8],
    found: BTreeSet<usize>,
    left: usize,
}

impl DeviceWalk<'_> {
    fn u16(&self, at: usize) -> std::result::Result<u16, String> {
        u16_at(self.b, at).ok_or_else(|| format!("GPOS is truncated at byte {at}"))
    }

    fn step(&mut self, n: usize) -> std::result::Result<(), String> {
        self.left = self.left.checked_sub(n).ok_or_else(|| format!("GPOS has more than {MAX_DEVICE_WALK} records"))?;
        Ok(())
    }

    /// The table at offset field `field` from `base`; `None` for a null offset.
    fn offset(&self, base: usize, field: usize) -> std::result::Result<Option<usize>, String> {
        let o = usize::from(self.u16(field)?);
        Ok((o != 0).then_some(base + o))
    }

    fn device(&mut self, at: Option<usize>) -> std::result::Result<(), String> {
        if let Some(d) = at
            && self.u16(d + 4)? == 0x8000
        {
            self.found.insert(d);
        }
        Ok(())
    }

    fn anchor(&mut self, at: Option<usize>) -> std::result::Result<(), String> {
        if let Some(a) = at
            && self.u16(a)? == 3
        {
            let x = self.offset(a, a + 6)?;
            let y = self.offset(a, a + 8)?;
            self.device(x)?;
            self.device(y)?;
        }
        Ok(())
    }

    /// A value record at `at` whose device offsets count from `base`; its size.
    fn value(&mut self, format: u16, base: usize, at: usize) -> std::result::Result<usize, String> {
        let mut p = at + 2 * (format & 0x0F).count_ones() as usize;
        for bit in [0x10u16, 0x20, 0x40, 0x80] {
            if format & bit != 0 {
                let d = self.offset(base, p)?;
                self.device(d)?;
                p += 2;
            }
        }
        Ok(2 * (format & 0xFF).count_ones() as usize)
    }

    fn lookups(&mut self) -> std::result::Result<(), String> {
        let Some(list) = self.offset(0, 8)? else { return Ok(()) };
        let count = usize::from(self.u16(list)?);
        for l in 0..count {
            self.step(1)?;
            let Some(lookup) = self.offset(list, list + 2 + 2 * l)? else { continue };
            let kind = self.u16(lookup)?;
            let subs = usize::from(self.u16(lookup + 4)?);
            for s in 0..subs {
                self.step(1)?;
                let Some(sub) = self.offset(lookup, lookup + 6 + 2 * s)? else { continue };
                self.subtable(kind, sub)?;
            }
        }
        Ok(())
    }

    fn subtable(&mut self, kind: u16, s: usize) -> std::result::Result<(), String> {
        let format = self.u16(s)?;
        match (kind, format) {
            (9, 1) => {
                let inner = self.u16(s + 2)?;
                let o = u32_at(self.b, s + 4).ok_or("GPOS is truncated")? as usize;
                if inner != 9 && o != 0 {
                    self.subtable(inner, s + o)?;
                }
            }
            (1, 1) => {
                let vf = self.u16(s + 4)?;
                self.value(vf, s, s + 6)?;
            }
            (1, 2) => {
                let vf = self.u16(s + 4)?;
                let n = usize::from(self.u16(s + 6)?);
                self.step(n)?;
                let size = 2 * (vf & 0xFF).count_ones() as usize;
                for i in 0..n {
                    self.value(vf, s, s + 8 + size * i)?;
                }
            }
            (2, 1) => {
                let (vf1, vf2) = (self.u16(s + 4)?, self.u16(s + 6)?);
                let sets = usize::from(self.u16(s + 8)?);
                let size = 2 + 2 * (vf1 & 0xFF).count_ones() as usize + 2 * (vf2 & 0xFF).count_ones() as usize;
                for i in 0..sets {
                    let Some(set) = self.offset(s, s + 10 + 2 * i)? else { continue };
                    let n = usize::from(self.u16(set)?);
                    self.step(n.max(1))?;
                    for k in 0..n {
                        let r = set + 2 + size * k;
                        let first = self.value(vf1, set, r + 2)?;
                        self.value(vf2, set, r + 2 + first)?;
                    }
                }
            }
            (2, 2) => {
                let (vf1, vf2) = (self.u16(s + 4)?, self.u16(s + 6)?);
                if (vf1 | vf2) & 0xF0 == 0 {
                    return Ok(());
                }
                let (c1, c2) = (usize::from(self.u16(s + 12)?), usize::from(self.u16(s + 14)?));
                let n = c1.saturating_mul(c2);
                self.step(n)?;
                let s1 = 2 * (vf1 & 0xFF).count_ones() as usize;
                let size = s1 + 2 * (vf2 & 0xFF).count_ones() as usize;
                for i in 0..n {
                    let r = s + 16 + size * i;
                    self.value(vf1, s, r)?;
                    self.value(vf2, s, r + s1)?;
                }
            }
            (3, 1) => {
                let n = usize::from(self.u16(s + 4)?);
                self.step(n)?;
                for i in 0..n {
                    let entry = self.offset(s, s + 6 + 4 * i)?;
                    let exit = self.offset(s, s + 8 + 4 * i)?;
                    self.anchor(entry)?;
                    self.anchor(exit)?;
                }
            }
            (4..=6, 1) => {
                let classes = usize::from(self.u16(s + 6)?);
                if let Some(marks) = self.offset(s, s + 8)? {
                    let n = usize::from(self.u16(marks)?);
                    self.step(n)?;
                    for i in 0..n {
                        let a = self.offset(marks, marks + 4 + 4 * i)?;
                        self.anchor(a)?;
                    }
                }
                let Some(bases) = self.offset(s, s + 10)? else { return Ok(()) };
                let n = usize::from(self.u16(bases)?);
                if kind == 5 {
                    for i in 0..n {
                        let Some(lig) = self.offset(bases, bases + 2 + 2 * i)? else { continue };
                        let comps = usize::from(self.u16(lig)?);
                        self.anchors(lig, lig + 2, comps, classes)?;
                    }
                } else {
                    self.anchors(bases, bases + 2, n, classes)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// `rows` × `classes` anchor offsets from `base`, starting at `at`.
    fn anchors(&mut self, base: usize, at: usize, rows: usize, classes: usize) -> std::result::Result<(), String> {
        let n = rows.saturating_mul(classes);
        self.step(n)?;
        for i in 0..n {
            let a = self.offset(base, at + 2 * i)?;
            self.anchor(a)?;
        }
        Ok(())
    }
}

fn remap(map: &HashMap<u32, u32>, outer: u16, inner: u16) -> std::result::Result<(u16, u16), String> {
    let old = (u32::from(outer) << 16) | u32::from(inner);
    let new = map.get(&old).ok_or_else(|| format!("a device table names variation index {outer}/{inner}, which the store lacks"))?;
    Ok(((new >> 16) as u16, *new as u16))
}

/// The new `GDEF` and `GPOS` (when the font has it); `None` when `GDEF` has no store.
pub fn run(font: &SfntFont, quantum: u32) -> Result<Option<Vec<Table>>> {
    if !(1..=MAX_QUANTUM).contains(&quantum) {
        return Err(TablesError::BadOptions(format!("the quantum must be 1 to {MAX_QUANTUM}, not {quantum}")));
    }
    let Some(gdef) = font.get(*b"GDEF") else { return Ok(None) };
    let version = u32_at(gdef, 0).unwrap_or(0);
    let at = if version >= 0x0001_0003 { u32_at(gdef, 14).unwrap_or(0) as usize } else { 0 };
    if at == 0 {
        return Ok(None);
    }
    let store_bytes = gdef.get(at..).ok_or_else(|| bad("GDEF points at a variation store past its end"))?;
    let store = read_store(store_bytes).map_err(|e| bad(format!("GDEF: {e}")))?;
    let (new_store, map) = optimize_store(&store, quantum).map_err(|e| bad(format!("GDEF: {e}")))?;

    crate::layout_budget::check_gdef(gdef)?;
    let read = ReadGdef::read(FontData::new(gdef)).map_err(|e| bad(format!("the GDEF table cannot be read: {e}")))?;
    let mut owned: Gdef = read.to_owned_table();
    if let Some(list) = owned.lig_caret_list.as_mut() {
        for lig in list.lig_glyphs.iter_mut() {
            for caret in lig.caret_values.iter_mut() {
                if let CaretValue::Format3(t) = &mut **caret
                    && let DeviceOrVariationIndex::VariationIndex(v) = &mut *t.device
                {
                    (v.delta_set_outer_index, v.delta_set_inner_index) =
                        remap(&map, v.delta_set_outer_index, v.delta_set_inner_index).map_err(bad)?;
                }
            }
        }
    }
    let store = ReadStore::read(FontData::new(&new_store)).map_err(|e| bad(format!("the optimised store cannot be read back: {e}")))?;
    owned.item_var_store = NullableOffsetMarker::new(Some(store.to_owned_table()));
    let gdef_bytes = write_fonts::dump_table(&owned).map_err(|e| bad(format!("the GDEF table could not be written again: {e}")))?;
    let mut out = vec![(*b"GDEF", gdef_bytes)];

    if let Some(gpos) = font.get(*b"GPOS") {
        let mut walk = DeviceWalk { b: gpos, found: BTreeSet::new(), left: MAX_DEVICE_WALK };
        walk.lookups().map_err(bad)?;
        let mut bytes = gpos.to_vec();
        for d in walk.found {
            let (Some(outer), Some(inner)) = (u16_at(gpos, d), u16_at(gpos, d + 2)) else { continue };
            let (o, i) = remap(&map, outer, inner).map_err(bad)?;
            if let Some(slot) = bytes.get_mut(d..d + 4) {
                slot.copy_from_slice(&[(o >> 8) as u8, o as u8, (i >> 8) as u8, i as u8]);
            }
        }
        out.push((*b"GPOS", bytes));
    }
    Ok(Some(out))
}

#[cfg(test)]
#[path = "varstore_tests.rs"]
mod tests;
