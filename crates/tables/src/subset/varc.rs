//! Subsetting the `VARC` table (M12.19c), ported from fontTools' subsetter (`VARC.closure_glyphs`,
//! `VARC.subset_glyphs` and `VARC.prune_post_subset` in `subset/__init__.py`, Copyright 2013
//! Google, Inc.) and from `varLib.multiVarStore` and `varLib.varStore` (the multi-item store's
//! `subset_varidxes` and `prune_regions`, Copyright 2017 Google, Inc.), MIT License.
//!
//! - Closure: [`Varc::component_glyphs`] lists the glyphs the variable components of a glyph
//!   name; [`super::closure`] walks them with the `glyf` components in one work list.
//! - The glyph records of the kept glyphs stay, in new glyph id order. Each component keeps its
//!   axis values, transform and trailing reserved values byte for byte; its glyph id, condition
//!   index, axis indices index and variation indices are renumbered.
//! - `ConditionList` and `AxisIndicesList` keep the entries the kept components use, in their
//!   order; a list nothing uses is dropped.
//! - The `MultiItemVariationStore` keeps the delta sets the kept components and conditions use,
//!   each delta set list in its order, and the regions those lists use; a store left without
//!   delta sets is dropped.
//!
//! The layout read and written is the current one: a 32-bit region count, offsets to each
//! region's axis records, and the delta sets of a list behind an offset (read-fonts and fontTools
//! read the same layout).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::Plan;
use crate::reorder::be::{Data, put_u16, put_u32, to_u16, to_u32};
use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

const TAG: Tag = *b"VARC";

/// The variation index that means "no variation".
const NO_VARIATION: u32 = 0xFFFF_FFFF;

/// Deepest nesting of `AND`, `OR` and `NOT` conditions read.
pub const MAX_CONDITION_DEPTH: usize = 32;
/// Most condition tables one subset reads (shared subtables count each time they are reached).
pub const MAX_CONDITION_NODES: usize = 1 << 20;
/// Most axes one entry of the axis indices list may name.
pub const MAX_COMPONENT_AXES: usize = 1 << 16;
/// Most variable components one subset reads, over the closure and the kept records.
pub const MAX_COMPONENTS: usize = 1 << 22;

const HAVE_AXES: u32 = 1 << 1;
const AXIS_VALUES_HAVE_VARIATION: u32 = 1 << 2;
const TRANSFORM_HAS_VARIATION: u32 = 1 << 3;
const HAVE_CONDITION: u32 = 1 << 7;
const GID_IS_24BIT: u32 = 1 << 12;
/// The nine transform fields, each an `int16` when its bit is set.
const TRANSFORM_FLAGS: u32 = (1 << 4) | (1 << 5) | (1 << 6) | (1 << 8) | (1 << 9) | (1 << 10) | (1 << 11) | (1 << 13) | (1 << 14);
/// Bits with no meaning yet: each set bit is followed by one `uint32var` at the end of the
/// component.
const RESERVED_MASK: u32 = !((1 << 15) - 1);

/// Writes the subset of `VARC` from `input` into `output`; a table left without glyph records
/// is not written.
pub fn apply(plan: &Plan, input: &SfntFont, output: &mut SfntFont) -> Result<()> {
    let Some(bytes) = input.get(TAG) else { return Ok(()) };
    if let Some(table) = subset_varc(plan, bytes)? {
        output.insert(TAG, table);
    }
    Ok(())
}

// ── Reading ──

/// A `CFF2`-style INDEX: a 32-bit count, an offset size, `count + 1` offsets, then the data.
#[derive(Clone, Copy)]
struct Index2<'a> {
    d: Data<'a>,
    count: usize,
    off_size: usize,
    offsets_at: usize,
    /// The byte before the first item: offsets count from it.
    base: usize,
}

impl<'a> Index2<'a> {
    fn read(d: Data<'a>, at: usize) -> Result<Self> {
        let count = d.u32_usize(at)?;
        if count == 0 {
            return Ok(Index2 { d, count: 0, off_size: 1, offsets_at: at + 4, base: at + 4 });
        }
        let off_size = usize::from(d.u8(at + 4)?);
        if !(1..=4).contains(&off_size) {
            return Err(d.bad(format!("has an INDEX with offset size {off_size}")));
        }
        let offsets_at = at + 5;
        let table = count.checked_add(1).and_then(|c| c.checked_mul(off_size)).ok_or_else(|| d.bad("has an INDEX past its end"))?;
        d.slice(offsets_at, table)?;
        Ok(Index2 { d, count, off_size, offsets_at, base: offsets_at + table - 1 })
    }

    fn offset(&self, i: usize) -> Result<usize> {
        let bytes = self.d.slice(self.offsets_at + i * self.off_size, self.off_size)?;
        Ok(bytes.iter().fold(0usize, |v, b| (v << 8) | usize::from(*b)))
    }

    fn get(&self, i: usize) -> Result<&'a [u8]> {
        if i >= self.count {
            return Err(self.d.bad(format!("refers to item {i} of an INDEX of {} items", self.count)));
        }
        let (start, end) = (self.offset(i)?, self.offset(i + 1)?);
        if start == 0 || end < start {
            return Err(self.d.bad("has an INDEX with decreasing offsets"));
        }
        self.d.slice(self.base.saturating_add(start), end - start)
    }
}

fn u24(d: &Data<'_>, at: usize) -> Result<usize> {
    let s = d.slice(at, 3)?;
    Ok(s.iter().fold(0usize, |v, b| (v << 8) | usize::from(*b)))
}

/// A `uint32var`: one to five bytes, the length given by the leading one bits of the first.
fn read_u32var(d: &Data<'_>, at: &mut usize) -> Result<u32> {
    let b0 = d.u8(*at)?;
    let (len, first) = match b0 {
        0..=0x7F => (1, b0),
        0x80..=0xBF => (2, b0 & 0x3F),
        0xC0..=0xDF => (3, b0 & 0x1F),
        0xE0..=0xEF => (4, b0 & 0x0F),
        0xF0 => (5, 0),
        _ => return Err(d.bad(format!("has a variable-length number past 32 bits at byte {at}"))),
    };
    let mut v = u32::from(first);
    for i in 1..len {
        v = (v << 8) | u32::from(d.u8(*at + i)?);
    }
    *at += len;
    Ok(v)
}

fn put_u32var(out: &mut Vec<u8>, v: u32) {
    if v < 0x80 {
        out.push(v as u8);
    } else if v < 0x4000 {
        put_u16(out, (v | 0x8000) as u16);
    } else if v < 0x20_0000 {
        out.extend_from_slice((v | 0xC0_0000).to_be_bytes().get(1..).unwrap_or_default());
    } else if v < 0x1000_0000 {
        put_u32(out, v | 0xE000_0000);
    } else {
        out.push(0xF0);
        put_u32(out, v);
    }
}

/// The size of one run of packed values from its header byte: the count and the bytes per value.
fn run(header: u8) -> (usize, usize) {
    let size = match header & 0xC0 {
        0x80 => 0,
        0x40 => 2,
        0xC0 => 4,
        _ => 1,
    };
    (usize::from(header & 0x3F) + 1, size)
}

/// The byte length of the `count` packed values at `at`.
fn packed_len(d: &Data<'_>, at: usize, count: usize) -> Result<usize> {
    let (mut n, mut p) = (0usize, at);
    while n < count {
        let (values, size) = run(d.u8(p)?);
        d.slice(p + 1, values * size)?;
        p += 1 + values * size;
        n += values;
    }
    if n != count {
        return Err(d.bad(format!("packs {n} axis values where its axis indices name {count}")));
    }
    Ok(p - at)
}

/// How many values `bytes` packs, read to its end.
fn packed_count(d: &Data<'_>, bytes: &[u8]) -> Result<usize> {
    let item = Data::new(d.tag, bytes);
    let (mut n, mut p) = (0usize, 0usize);
    while p < bytes.len() {
        let (values, size) = run(item.u8(p)?);
        item.slice(p + 1, values * size)?;
        p += 1 + values * size;
        n += values;
        if n > MAX_COMPONENT_AXES {
            return Err(TablesError::TooLarge { what: "the axes of a variable component", limit: MAX_COMPONENT_AXES });
        }
    }
    Ok(n)
}

/// One variable component, with the parts copied as they are.
struct Component<'a> {
    flags: u32,
    gid: u32,
    condition: Option<u32>,
    axis_indices: Option<u32>,
    axis_count: usize,
    axis_values: &'a [u8],
    axis_values_var: u32,
    transform_var: u32,
    transform: &'a [u8],
    reserved: &'a [u8],
}

/// A `VARC` table read lazily: a glyph record is parsed when the closure or the subset asks for it.
pub(crate) struct Varc<'a> {
    d: Data<'a>,
    version: u32,
    /// Glyph id → index of its glyph record (the first coverage entry wins).
    records: BTreeMap<u32, usize>,
    glyphs: Index2<'a>,
    axis_indices: Option<Index2<'a>>,
    store_at: usize,
    conditions_at: usize,
    /// Axis indices index → number of axes, as parsed so far.
    axis_counts: HashMap<u32, usize>,
    /// Components left to read under [`MAX_COMPONENTS`].
    budget: usize,
}

impl<'a> Varc<'a> {
    /// The `VARC` table of `font`, or `None` without one.
    pub(crate) fn read(font: &'a SfntFont) -> Result<Option<Self>> {
        font.get(TAG).map(Self::parse).transpose()
    }

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let d = Data::new(TAG, bytes);
        let version = d.u32(0)?;
        if version >> 16 != 1 {
            return Err(d.bad(format!("has version {}.{}", version >> 16, version & 0xFFFF)));
        }
        let coverage_at = d.u32_usize(4)?;
        let store_at = d.u32_usize(8)?;
        let conditions_at = d.u32_usize(12)?;
        let axis_at = d.u32_usize(16)?;
        let glyphs_at = d.u32_usize(20)?;
        if coverage_at == 0 || glyphs_at == 0 {
            return Err(d.bad("has no coverage table or no glyph records"));
        }
        let mut records = BTreeMap::new();
        for (g, i) in read_coverage(&d, coverage_at)? {
            records.entry(g).or_insert(i);
        }
        let glyphs = Index2::read(d, glyphs_at)?;
        let axis_indices = if axis_at == 0 { None } else { Some(Index2::read(d, axis_at)?) };
        Ok(Varc { d, version, records, glyphs, axis_indices, store_at, conditions_at, axis_counts: HashMap::new(), budget: MAX_COMPONENTS })
    }

    /// The glyphs the variable components of glyph `g` name (none when `g` has no record).
    pub(crate) fn component_glyphs(&mut self, g: u32) -> Result<Vec<u32>> {
        let Some(i) = self.records.get(&g).copied() else { return Ok(Vec::new()) };
        let record = self.glyphs.get(i)?;
        Ok(self.components(record)?.iter().map(|c| c.gid).collect())
    }

    fn axis_count(&mut self, index: u32) -> Result<usize> {
        if let Some(n) = self.axis_counts.get(&index) {
            return Ok(*n);
        }
        let list = self.axis_indices.ok_or_else(|| self.d.bad("has a component with axes but no axis indices list"))?;
        let n = packed_count(&self.d, list.get(index as usize)?)?;
        self.axis_counts.insert(index, n);
        Ok(n)
    }

    fn components(&mut self, record: &'a [u8]) -> Result<Vec<Component<'a>>> {
        let d = Data::new(TAG, record);
        let mut at = 0usize;
        let mut out = Vec::new();
        while at < record.len() {
            self.budget = self.budget.checked_sub(1).ok_or(TablesError::TooLarge { what: "the variable components", limit: MAX_COMPONENTS })?;
            let flags = read_u32var(&d, &mut at)?;
            let gid = if flags & GID_IS_24BIT != 0 {
                let g = u24(&d, at)?;
                at += 3;
                g as u32
            } else {
                let g = d.u16(at)?;
                at += 2;
                u32::from(g)
            };
            let condition = if flags & HAVE_CONDITION != 0 { Some(read_u32var(&d, &mut at)?) } else { None };
            let (axis_indices, axis_count, axis_values) = if flags & HAVE_AXES != 0 {
                let index = read_u32var(&d, &mut at)?;
                let count = self.axis_count(index)?;
                let len = packed_len(&d, at, count)?;
                let values = d.slice(at, len)?;
                at += len;
                (Some(index), count, values)
            } else {
                (None, 0, <&[u8]>::default())
            };
            let axis_values_var = if flags & AXIS_VALUES_HAVE_VARIATION != 0 { read_u32var(&d, &mut at)? } else { NO_VARIATION };
            let transform_var = if flags & TRANSFORM_HAS_VARIATION != 0 { read_u32var(&d, &mut at)? } else { NO_VARIATION };
            let len = 2 * (flags & TRANSFORM_FLAGS).count_ones() as usize;
            let transform = d.slice(at, len)?;
            at += len;
            let start = at;
            for _ in 0..(flags & RESERVED_MASK).count_ones() {
                read_u32var(&d, &mut at)?;
            }
            let reserved = d.slice(start, at - start)?;
            out.push(Component { flags, gid, condition, axis_indices, axis_count, axis_values, axis_values_var, transform_var, transform, reserved });
        }
        Ok(out)
    }
}

/// The glyphs of a coverage table with their coverage indices.
fn read_coverage(d: &Data<'_>, at: usize) -> Result<Vec<(u32, usize)>> {
    let format = d.u16(at)?;
    let count = usize::from(d.u16(at + 2)?);
    let mut out = Vec::new();
    match format {
        1 => {
            for i in 0..count {
                out.push((u32::from(d.u16(at + 4 + 2 * i)?), i));
            }
        }
        2 => {
            for i in 0..count {
                let r = at + 4 + 6 * i;
                let (start, end, first) = (d.u16(r)?, d.u16(r + 2)?, usize::from(d.u16(r + 4)?));
                if end < start {
                    return Err(d.bad("has a coverage range that ends before it starts"));
                }
                out.extend((start..=end).map(|g| (u32::from(g), first + usize::from(g - start))));
                if out.len() > 1 << 16 {
                    return Err(d.bad("has a coverage table with overlapping ranges"));
                }
            }
        }
        _ => return Err(d.bad(format!("has a coverage table of format {format}"))),
    }
    Ok(out)
}

/// A condition table, its offsets resolved.
enum Condition<'a> {
    /// Format 1: axis index and the filter range, copied.
    Axis(&'a [u8]),
    /// Format 2: default value and variation index.
    Value(u16, u32),
    /// Formats 3 (and) and 4 (or).
    List(u16, Vec<Condition<'a>>),
    /// Format 5.
    Not(Box<Condition<'a>>),
}

fn read_condition<'a>(d: &Data<'a>, at: usize, depth: usize, nodes: &mut usize) -> Result<Condition<'a>> {
    if depth > MAX_CONDITION_DEPTH {
        return Err(TablesError::TooLarge { what: "the condition nesting", limit: MAX_CONDITION_DEPTH });
    }
    *nodes = nodes.checked_sub(1).ok_or(TablesError::TooLarge { what: "the conditions", limit: MAX_CONDITION_NODES })?;
    let child = |off: usize| if off == 0 { Err(d.bad("has a condition with a null offset")) } else { Ok(at.saturating_add(off)) };
    match d.u16(at)? {
        1 => Ok(Condition::Axis(d.slice(at + 2, 6)?)),
        2 => Ok(Condition::Value(d.u16(at + 2)?, d.u32(at + 4)?)),
        format @ (3 | 4) => {
            let count = usize::from(d.u8(at + 2)?);
            let mut list = Vec::with_capacity(count);
            for i in 0..count {
                list.push(read_condition(d, child(u24(d, at + 3 + 3 * i)?)?, depth + 1, nodes)?);
            }
            Ok(Condition::List(format, list))
        }
        5 => Ok(Condition::Not(Box::new(read_condition(d, child(u24(d, at + 2)?)?, depth + 1, nodes)?))),
        format => Err(d.bad(format!("has a condition of format {format}"))),
    }
}

impl Condition<'_> {
    fn var_indices(&self, out: &mut BTreeSet<u32>) {
        match self {
            Condition::Axis(_) => {}
            Condition::Value(_, v) => {
                out.insert(*v);
            }
            Condition::List(_, list) => list.iter().for_each(|c| c.var_indices(out)),
            Condition::Not(c) => c.var_indices(out),
        }
    }

    /// The condition with its offsets from its own start and its variation indices renumbered.
    fn write(&self, var_map: &BTreeMap<u32, u32>) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        match self {
            Condition::Axis(bytes) => {
                put_u16(&mut out, 1);
                out.extend_from_slice(bytes);
            }
            Condition::Value(default, v) => {
                put_u16(&mut out, 2);
                put_u16(&mut out, *default);
                put_u32(&mut out, remap(var_map, *v)?);
            }
            Condition::List(format, list) => {
                put_u16(&mut out, *format);
                out.push(u8::try_from(list.len()).map_err(|_| TablesError::TooLarge { what: "the conditions of a condition", limit: 255 })?);
                let bodies = list.iter().map(|c| c.write(var_map)).collect::<Result<Vec<_>>>()?;
                let mut at = 3 + 3 * bodies.len();
                for b in &bodies {
                    put_u24(&mut out, at)?;
                    at += b.len();
                }
                bodies.iter().for_each(|b| out.extend_from_slice(b));
            }
            Condition::Not(c) => {
                put_u16(&mut out, 5);
                put_u24(&mut out, 5)?;
                out.extend(c.write(var_map)?);
            }
        }
        Ok(out)
    }
}

fn put_u24(out: &mut Vec<u8>, v: usize) -> Result<()> {
    if v >= 1 << 24 {
        return Err(TablesError::TooLarge { what: "a condition table", limit: (1 << 24) - 1 });
    }
    out.extend_from_slice((v as u32).to_be_bytes().get(1..).unwrap_or_default());
    Ok(())
}

fn remap(map: &BTreeMap<u32, u32>, v: u32) -> Result<u32> {
    if v == NO_VARIATION {
        return Ok(v);
    }
    map.get(&v).copied().ok_or_else(|| TablesError::invalid(format!("the 'VARC' table refers to delta set {v:#x}, which its variation store lacks")))
}

/// The renumbering of the used entries of a list: old index → new index, in the old order.
fn renumber(used: &BTreeSet<u32>) -> BTreeMap<u32, u32> {
    used.iter().enumerate().map(|(new, old)| (*old, new as u32)).collect()
}

// ── The multi-item variation store ──

/// One delta set list of the subset store: its region indices and the kept delta sets.
struct VarData<'a> {
    regions: Vec<u16>,
    items: Vec<&'a [u8]>,
}

/// The store at `at` keeping the delta sets in `used`, written, and the old → new variation
/// index map. `None` when no delta set is used.
fn subset_store(d: &Data<'_>, at: usize, used: &BTreeSet<u32>) -> Result<(Option<Vec<u8>>, BTreeMap<u32, u32>)> {
    let mut by_major: BTreeMap<u16, BTreeSet<u16>> = BTreeMap::new();
    for v in used.iter().filter(|v| **v != NO_VARIATION) {
        by_major.entry((v >> 16) as u16).or_default().insert(*v as u16);
    }
    let mut map = BTreeMap::new();
    if by_major.is_empty() {
        return Ok((None, map));
    }
    if at == 0 {
        return Err(d.bad("refers to delta sets but has no variation store"));
    }
    if d.u16(at)? != 1 {
        return Err(d.bad("has a variation store of an unknown format"));
    }
    let region_list_at = at.saturating_add(d.u32_usize(at + 2)?);
    let data_count = d.u16(at + 6)?;
    let mut datas: Vec<VarData<'_>> = Vec::new();
    for (major, minors) in &by_major {
        if *major >= data_count {
            return Err(d.bad(format!("refers to delta set list {major} of a store with {data_count}")));
        }
        let data_at = at.saturating_add(d.u32_usize(at + 8 + 4 * usize::from(*major))?);
        if d.u8(data_at)? != 1 {
            return Err(d.bad("has a delta set list of an unknown format"));
        }
        let region_count = usize::from(d.u16(data_at + 1)?);
        let regions = (0..region_count).map(|i| d.u16(data_at + 3 + 2 * i)).collect::<Result<Vec<_>>>()?;
        let sets_at = d.u32_usize(data_at + 3 + 2 * region_count)?;
        if sets_at == 0 {
            return Err(d.bad("has a delta set list without delta sets"));
        }
        let sets = Index2::read(*d, data_at.saturating_add(sets_at))?;
        let new_major = u32::from(to_u16(datas.len(), "the delta set lists")?);
        let mut items = Vec::with_capacity(minors.len());
        for minor in minors {
            map.insert((u32::from(*major) << 16) | u32::from(*minor), (new_major << 16) | items.len() as u32);
            items.push(sets.get(usize::from(*minor))?);
        }
        datas.push(VarData { regions, items });
    }

    let used_regions: BTreeSet<u16> = datas.iter().flat_map(|v| v.regions.iter().copied()).collect();
    let region_map: BTreeMap<u16, u16> = used_regions.iter().enumerate().map(|(new, old)| (*old, new as u16)).collect();
    let region_count = d.u32_usize(region_list_at)?;
    let mut regions: Vec<Vec<&[u8]>> = Vec::with_capacity(used_regions.len());
    for r in &used_regions {
        let r = usize::from(*r);
        if r >= region_count {
            return Err(d.bad(format!("uses region {r} of a store with {region_count}")));
        }
        let region_at = region_list_at.saturating_add(d.u32_usize(region_list_at + 4 + 4 * r)?);
        let axis_count = usize::from(d.u16(region_at)?);
        let mut axes = Vec::with_capacity(axis_count);
        for i in 0..axis_count {
            let axis_at = region_at.saturating_add(d.u32_usize(region_at + 2 + 4 * i)?);
            axes.push(d.slice(axis_at, 8)?);
        }
        regions.push(axes);
    }

    // Header, region list, then the delta set lists.
    let mut list = Vec::new();
    put_u32(&mut list, to_u32(regions.len(), "the variation regions")?);
    let mut region_bytes = Vec::new();
    let mut offset = 4 + 4 * regions.len();
    for axes in &regions {
        put_u32(&mut list, to_u32(offset, "the variation region list")?);
        let mut region = Vec::new();
        put_u16(&mut region, to_u16(axes.len(), "the axes of a region")?);
        let mut axis_at = 2 + 4 * axes.len();
        for _ in axes {
            put_u32(&mut region, to_u32(axis_at, "a variation region")?);
            axis_at += 8;
        }
        axes.iter().for_each(|a| region.extend_from_slice(a));
        offset += region.len();
        region_bytes.extend(region);
    }
    list.extend(region_bytes);

    let mut data_bytes: Vec<Vec<u8>> = Vec::with_capacity(datas.len());
    for v in &datas {
        let mut out = vec![1u8];
        put_u16(&mut out, to_u16(v.regions.len(), "the regions of a delta set list")?);
        for r in &v.regions {
            put_u16(&mut out, region_map.get(r).copied().unwrap_or_default());
        }
        let sets_at = out.len() + 4;
        put_u32(&mut out, to_u32(sets_at, "a delta set list")?);
        write_index2(&mut out, &v.items)?;
        data_bytes.push(out);
    }

    let header = 8 + 4 * datas.len();
    let mut out = Vec::new();
    put_u16(&mut out, 1);
    put_u32(&mut out, to_u32(header, "the variation store")?);
    put_u16(&mut out, to_u16(datas.len(), "the delta set lists")?);
    let mut offset = header + list.len();
    for b in &data_bytes {
        put_u32(&mut out, to_u32(offset, "the variation store")?);
        offset += b.len();
    }
    out.extend(list);
    data_bytes.iter().for_each(|b| out.extend_from_slice(b));
    Ok((Some(out), map))
}

// ── Writing ──

fn write_index2(out: &mut Vec<u8>, items: &[&[u8]]) -> Result<()> {
    put_u32(out, to_u32(items.len(), "the items of an INDEX")?);
    if items.is_empty() {
        return Ok(());
    }
    let last = items.iter().map(|i| i.len()).sum::<usize>() + 1;
    to_u32(last, "an INDEX")?;
    let off_size: usize = match last {
        0..=0xFF => 1,
        0x100..=0xFFFF => 2,
        0x1_0000..=0xFF_FFFF => 3,
        _ => 4,
    };
    out.push(off_size as u8);
    let put = |out: &mut Vec<u8>, v: usize| out.extend_from_slice((v as u32).to_be_bytes().get(4 - off_size..).unwrap_or_default());
    let mut offset = 1usize;
    put(out, offset);
    for item in items {
        offset += item.len();
        put(out, offset);
    }
    items.iter().for_each(|i| out.extend_from_slice(i));
    Ok(())
}

/// A coverage table of the sorted `glyphs`, in the smaller format.
fn write_coverage(glyphs: &[u16]) -> Result<Vec<u8>> {
    let mut ranges: Vec<(u16, u16, u16)> = Vec::new();
    for (i, g) in glyphs.iter().enumerate() {
        match ranges.last_mut() {
            Some(r) if u32::from(r.1) + 1 == u32::from(*g) => r.1 = *g,
            _ => ranges.push((*g, *g, to_u16(i, "the coverage table")?)),
        }
    }
    let mut out = Vec::new();
    if 3 * ranges.len() < glyphs.len() {
        put_u16(&mut out, 2);
        put_u16(&mut out, to_u16(ranges.len(), "the coverage table")?);
        for (start, end, first) in ranges {
            put_u16(&mut out, start);
            put_u16(&mut out, end);
            put_u16(&mut out, first);
        }
    } else {
        put_u16(&mut out, 1);
        put_u16(&mut out, to_u16(glyphs.len(), "the coverage table")?);
        glyphs.iter().for_each(|g| put_u16(&mut out, *g));
    }
    Ok(out)
}

/// The renumbering a kept component goes through.
struct Maps<'m> {
    glyphs: &'m BTreeMap<u32, u32>,
    conditions: &'m BTreeMap<u32, u32>,
    axes: &'m BTreeMap<u32, u32>,
    var: &'m BTreeMap<u32, u32>,
}

/// Writes `c` renumbered. The presence bits follow what is written, as fontTools' compiler sets
/// them; the other bits pass through.
fn write_component(out: &mut Vec<u8>, c: &Component<'_>, maps: &Maps<'_>) -> Result<()> {
    let gid = maps
        .glyphs
        .get(&c.gid)
        .copied()
        .ok_or_else(|| TablesError::invalid(format!("a variable component names glyph {}, which the subset does not keep", c.gid)))?;
    let mut flags = c.flags;
    let mut body = Vec::new();
    if gid > 0xFFFF {
        flags |= GID_IS_24BIT;
        body.extend_from_slice(gid.to_be_bytes().get(1..).unwrap_or_default());
    } else {
        flags &= !GID_IS_24BIT;
        put_u16(&mut body, gid as u16);
    }
    match c.condition {
        Some(i) => {
            flags |= HAVE_CONDITION;
            put_u32var(&mut body, maps.conditions.get(&i).copied().unwrap_or_default());
        }
        None => flags &= !HAVE_CONDITION,
    }
    match c.axis_indices {
        Some(i) if c.axis_count > 0 => {
            flags |= HAVE_AXES;
            put_u32var(&mut body, maps.axes.get(&i).copied().unwrap_or_default());
            body.extend_from_slice(c.axis_values);
        }
        _ => flags &= !HAVE_AXES,
    }
    for (bit, v) in [(AXIS_VALUES_HAVE_VARIATION, c.axis_values_var), (TRANSFORM_HAS_VARIATION, c.transform_var)] {
        let v = remap(maps.var, v)?;
        if v == NO_VARIATION {
            flags &= !bit;
        } else {
            flags |= bit;
            put_u32var(&mut body, v);
        }
    }
    body.extend_from_slice(c.transform);
    body.extend_from_slice(c.reserved);
    put_u32var(out, flags);
    out.extend(body);
    Ok(())
}

fn subset_varc(plan: &Plan, bytes: &[u8]) -> Result<Option<Vec<u8>>> {
    let mut varc = Varc::parse(bytes)?;
    let d = varc.d;
    // pyftsubset's `glyphs_retained`: emptied glyphs lose their records.
    let wanted: Vec<(u32, usize)> = varc.records.iter().filter(|(g, _)| plan.glyphs.contains(g)).map(|(g, i)| (*g, *i)).collect();
    let mut kept: Vec<(u16, Vec<Component<'_>>)> = Vec::with_capacity(wanted.len());
    for (old, i) in wanted {
        let Some(new) = plan.glyph_map.get(&old).copied() else { continue };
        let record = varc.glyphs.get(i)?;
        kept.push((to_u16(new as usize, "a glyph id in the coverage table")?, varc.components(record)?));
    }
    if kept.is_empty() {
        return Ok(None);
    }
    kept.sort_by_key(|(g, _)| *g);

    let components = || kept.iter().flat_map(|(_, cs)| cs.iter());
    let used_axes: BTreeSet<u32> = components().filter_map(|c| c.axis_indices).collect();
    let used_conditions: BTreeSet<u32> = components().filter_map(|c| c.condition).collect();
    let (axis_map, condition_map) = (renumber(&used_axes), renumber(&used_conditions));

    let mut conditions = Vec::with_capacity(used_conditions.len());
    if !used_conditions.is_empty() {
        let at = varc.conditions_at;
        if at == 0 {
            return Err(d.bad("has a component with a condition but no condition list"));
        }
        let count = d.u32_usize(at)?;
        let mut nodes = MAX_CONDITION_NODES;
        for i in &used_conditions {
            let i = *i as usize;
            if i >= count {
                return Err(d.bad(format!("refers to condition {i} of a list of {count}")));
            }
            let off = d.u32_usize(at + 4 + 4 * i)?;
            if off == 0 {
                return Err(d.bad("has a null condition offset"));
            }
            conditions.push(read_condition(&d, at.saturating_add(off), 0, &mut nodes)?);
        }
    }

    // Conditions can use the store too, so their indices count.
    let mut used_var: BTreeSet<u32> = components().flat_map(|c| [c.axis_values_var, c.transform_var]).collect();
    conditions.iter().for_each(|c| c.var_indices(&mut used_var));
    let (store, var_map) = subset_store(&d, varc.store_at, &used_var)?;

    let coverage = write_coverage(&kept.iter().map(|(g, _)| *g).collect::<Vec<_>>())?;
    let maps = Maps { glyphs: &plan.glyph_map, conditions: &condition_map, axes: &axis_map, var: &var_map };
    let mut records: Vec<Vec<u8>> = Vec::with_capacity(kept.len());
    for (_, cs) in &kept {
        let mut record = Vec::new();
        for c in cs {
            write_component(&mut record, c, &maps)?;
        }
        records.push(record);
    }
    let mut glyphs = Vec::new();
    write_index2(&mut glyphs, &records.iter().map(Vec::as_slice).collect::<Vec<_>>())?;

    let condition_list = if conditions.is_empty() {
        None
    } else {
        let bodies = conditions.iter().map(|c| c.write(&var_map)).collect::<Result<Vec<_>>>()?;
        let mut out = Vec::new();
        put_u32(&mut out, to_u32(bodies.len(), "the condition list")?);
        let mut at = 4 + 4 * bodies.len();
        for b in &bodies {
            put_u32(&mut out, to_u32(at, "the condition list")?);
            at += b.len();
        }
        bodies.iter().for_each(|b| out.extend_from_slice(b));
        Some(out)
    };

    let axis_list = match (varc.axis_indices, used_axes.is_empty()) {
        (Some(list), false) => {
            let items = used_axes.iter().map(|i| list.get(*i as usize)).collect::<Result<Vec<_>>>()?;
            let mut out = Vec::new();
            write_index2(&mut out, &items)?;
            Some(out)
        }
        _ => None,
    };

    let mut out = Vec::new();
    put_u32(&mut out, varc.version);
    let parts = [Some(coverage), store, condition_list, axis_list, Some(glyphs)];
    let mut at = 4 + 4 * parts.len();
    for part in &parts {
        match part {
            Some(p) => {
                put_u32(&mut out, to_u32(at, "the VARC table")?);
                at += p.len();
            }
            None => put_u32(&mut out, 0),
        }
    }
    parts.iter().flatten().for_each(|p| out.extend_from_slice(p));
    Ok(Some(out))
}

#[cfg(test)]
#[path = "varc_tests.rs"]
mod tests;
