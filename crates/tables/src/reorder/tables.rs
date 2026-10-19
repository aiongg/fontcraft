//! The glyph-indexed tables other than `cmap`, `post`, `CFF ` and the layout tables, put in a
//! new glyph order. Each function takes the table's bytes and the order as `new_to_old`
//! (`new_to_old[new] = old`) or `old_to_new`, and returns the new bytes.

use super::be::{Data, put_u16, put_u32, set_u16, to_u16, to_u32};
use crate::{Result, TablesError};

/// The glyph `new_to_old[i]` for every new glyph id `i`.
pub(crate) struct Order {
    pub new_to_old: Vec<u16>,
    pub old_to_new: Vec<u16>,
}

impl Order {
    /// The new id of a glyph a subset drops: no table may refer to it.
    pub const DROPPED: u16 = u16::MAX;

    pub fn new(new_to_old: Vec<u16>) -> Self {
        let mut old_to_new = vec![0u16; new_to_old.len()];
        for (new, old) in new_to_old.iter().enumerate() {
            if let Some(slot) = old_to_new.get_mut(usize::from(*old)) {
                *slot = new as u16;
            }
        }
        Order { new_to_old, old_to_new }
    }

    pub fn len(&self) -> usize {
        self.new_to_old.len()
    }

    /// The new id of old glyph `old`; ids past the glyph count, and glyphs a subset drops, are
    /// an error.
    pub fn map(&self, old: u16) -> Result<u16> {
        match self.old_to_new.get(usize::from(old)).copied() {
            None => Err(TablesError::invalid(format!("glyph id {old} is past the glyph count"))),
            Some(Self::DROPPED) => Err(TablesError::invalid(format!("glyph id {old} is not in the subset but a table still refers to it"))),
            Some(new) => Ok(new),
        }
    }

    /// `items` (indexed by old glyph id) in the new order.
    pub fn permute<T: Clone>(&self, items: &[T]) -> Result<Vec<T>> {
        self.new_to_old
            .iter()
            .map(|o| items.get(usize::from(*o)).cloned().ok_or_else(|| TablesError::invalid("a glyph array is shorter than the glyph count")))
            .collect()
    }
}

/// `glyf` and `loca` in the new order; composite glyphs point at their components' new ids.
pub(crate) fn glyf_loca(glyf: &[u8], loca: &[u8], long: bool, order: &Order) -> Result<(Vec<u8>, Vec<u8>)> {
    let l = Data::new(*b"loca", loca);
    let g = Data::new(*b"glyf", glyf);
    let n = order.len();
    let mut offsets = Vec::with_capacity(n + 1);
    for i in 0..=n {
        offsets.push(if long { l.u32_usize(4 * i)? } else { 2 * usize::from(l.u16(2 * i)?) });
    }
    let mut glyphs: Vec<&[u8]> = Vec::with_capacity(n);
    for w in offsets.windows(2) {
        let (start, end) = (w.first().copied().unwrap_or(0), w.get(1).copied().unwrap_or(0));
        if end < start {
            return Err(l.bad("has decreasing offsets"));
        }
        glyphs.push(g.slice(start, end - start)?);
    }
    let mut out_glyf = Vec::with_capacity(glyf.len());
    let mut out_loca = Vec::with_capacity(loca.len());
    let put_offset = |out: &mut Vec<u8>, at: usize| -> Result<()> {
        if long {
            put_u32(out, to_u32(at, "the glyf table")?);
        } else {
            put_u16(out, to_u16(at / 2, "the glyf table for short loca offsets")?);
        }
        Ok(())
    };
    for old in &order.new_to_old {
        put_offset(&mut out_loca, out_glyf.len())?;
        let data = glyphs.get(usize::from(*old)).copied().unwrap_or_default();
        let start = out_glyf.len();
        out_glyf.extend_from_slice(data);
        if let Some(glyph) = out_glyf.get_mut(start..) {
            remap_composite(glyph, order)?;
        }
    }
    put_offset(&mut out_loca, out_glyf.len())?;
    Ok((out_glyf, out_loca))
}

/// Rewrites the component glyph ids of one composite glyph in place.
fn remap_composite(glyph: &mut [u8], order: &Order) -> Result<()> {
    if glyph.len() < 10 {
        return Ok(());
    }
    let contours = Data::new(*b"glyf", glyph).i16(0)?;
    if contours >= 0 {
        return Ok(());
    }
    let mut at = 10usize;
    loop {
        let d = Data::new(*b"glyf", glyph);
        let flags = d.u16(at)?;
        let old = d.u16(at + 2)?;
        let new = order.map(old)?;
        set_u16(glyph, at + 2, new)?;
        at += 4 + if flags & 0x0001 != 0 { 4 } else { 2 };
        if flags & 0x0008 != 0 {
            at += 2;
        } else if flags & 0x0040 != 0 {
            at += 4;
        } else if flags & 0x0080 != 0 {
            at += 8;
        }
        if flags & 0x0020 == 0 {
            return Ok(());
        }
        if at >= glyph.len() {
            return Err(TablesError::invalid("a composite glyph is cut short"));
        }
    }
}

/// `hmtx` or `vmtx` in the new order, with the new count of long metrics for `hhea` or `vhea`.
/// Trailing glyphs with the advance of the last one share it, as fontTools writes them.
pub(crate) fn metrics(table: &[u8], tag: [u8; 4], long_count: usize, order: &Order) -> Result<(Vec<u8>, u16)> {
    let d = Data::new(tag, table);
    let n = order.len();
    if long_count == 0 || long_count > n {
        return Err(d.bad(format!("has {long_count} long metrics for {n} glyphs")));
    }
    let mut all = Vec::with_capacity(n);
    let mut last_advance = 0u16;
    for g in 0..n {
        if g < long_count {
            last_advance = d.u16(4 * g)?;
            all.push((last_advance, d.u16(4 * g + 2)?));
        } else {
            all.push((last_advance, d.u16(4 * long_count + 2 * (g - long_count))?));
        }
    }
    let all = order.permute(&all)?;
    let final_advance = all.last().map_or(0, |m| m.0);
    let mut count = n;
    while count >= 2 && all.get(count - 2).is_some_and(|m| m.0 == final_advance) {
        count -= 1;
    }
    let mut out = Vec::with_capacity(4 * count + 2 * (n - count));
    for (i, (advance, side)) in all.iter().enumerate() {
        if i < count {
            put_u16(&mut out, *advance);
        }
        put_u16(&mut out, *side);
    }
    Ok((out, to_u16(count, "the long metrics count")?))
}

/// `hdmx`: the widths of every device record in the new order.
pub(crate) fn hdmx(table: &[u8], order: &Order) -> Result<Vec<u8>> {
    let d = Data::new(*b"hdmx", table);
    let records = usize::from(d.u16(2)?);
    let size = d.u32_usize(4)?;
    let n = order.len();
    if size < 2 + n {
        return Err(d.bad("has device records shorter than the glyph count"));
    }
    let mut out = d.slice(0, 8)?.to_vec();
    for r in 0..records {
        let record = d.slice(8 + r * size, size)?;
        out.extend_from_slice(record.get(..2).unwrap_or_default());
        let widths = record.get(2..2 + n).unwrap_or_default();
        out.extend(order.permute(widths)?);
        out.extend_from_slice(record.get(2 + n..).unwrap_or_default());
    }
    Ok(out)
}

/// `LTSH`: one byte per glyph.
pub(crate) fn ltsh(table: &[u8], order: &Order) -> Result<Vec<u8>> {
    let d = Data::new(*b"LTSH", table);
    let mut out = d.slice(0, 4)?.to_vec();
    out.extend(order.permute(d.slice(4, order.len())?)?);
    Ok(out)
}

/// `VORG`: records sorted by the new glyph ids.
pub(crate) fn vorg(table: &[u8], order: &Order) -> Result<Vec<u8>> {
    let d = Data::new(*b"VORG", table);
    let count = usize::from(d.u16(6)?);
    let mut records = Vec::with_capacity(count);
    for i in 0..count {
        records.push((order.map(d.u16(8 + 4 * i)?)?, d.u16(10 + 4 * i)?));
    }
    records.sort_unstable();
    let mut out = d.slice(0, 8)?.to_vec();
    for (g, y) in records {
        put_u16(&mut out, g);
        put_u16(&mut out, y);
    }
    Ok(out)
}

/// The OpenType `kern` table (version 0) with format 0 subtables: pairs remapped and sorted.
pub(crate) fn kern(table: &[u8], order: &Order) -> Result<Vec<u8>> {
    let d = Data::new(*b"kern", table);
    if d.u16(0)? != 0 {
        return Err(TablesError::Unsupported("a kern table of the Apple version".into()));
    }
    let count = usize::from(d.u16(2)?);
    let mut out = d.slice(0, 4)?.to_vec();
    let mut at = 4usize;
    for _ in 0..count {
        let coverage = d.u16(at + 4)?;
        if coverage >> 8 != 0 {
            return Err(TablesError::Unsupported(format!("a kern subtable of format {}", coverage >> 8)));
        }
        let pairs = usize::from(d.u16(at + 6)?);
        // The length field wraps for large subtables, so the pair count decides the size.
        let mut list = Vec::with_capacity(pairs);
        for p in 0..pairs {
            let base = at + 14 + 6 * p;
            list.push(((order.map(d.u16(base)?)?, order.map(d.u16(base + 2)?)?), d.u16(base + 4)?));
        }
        list.sort_unstable_by_key(|(pair, _)| *pair);
        out.extend_from_slice(d.slice(at, 14)?);
        for ((left, right), value) in list {
            put_u16(&mut out, left);
            put_u16(&mut out, right);
            put_u16(&mut out, value);
        }
        at += 14 + 6 * pairs;
    }
    Ok(out)
}

/// `gvar`: the variation data of each glyph in the new order; shared tuples are kept.
pub(crate) fn gvar(table: &[u8], order: &Order) -> Result<Vec<u8>> {
    let d = Data::new(*b"gvar", table);
    let axes = usize::from(d.u16(4)?);
    let shared = usize::from(d.u16(6)?);
    let shared_at = d.u32_usize(8)?;
    let count = usize::from(d.u16(12)?);
    let flags = d.u16(14)?;
    let data_at = d.u32_usize(16)?;
    let long = flags & 1 != 0;
    if count != order.len() {
        return Err(d.bad(format!("has {count} glyphs, the font {}", order.len())));
    }
    let mut offsets = Vec::with_capacity(count + 1);
    for i in 0..=count {
        offsets.push(if long { d.u32_usize(20 + 4 * i)? } else { 2 * usize::from(d.u16(20 + 2 * i)?) });
    }
    let mut glyphs = Vec::with_capacity(count);
    for w in offsets.windows(2) {
        let (start, end) = (w.first().copied().unwrap_or(0), w.get(1).copied().unwrap_or(0));
        if end < start {
            return Err(d.bad("has decreasing offsets"));
        }
        glyphs.push(d.slice(data_at + start, end - start)?);
    }
    let tuples = d.slice(shared_at, shared * axes * 2)?;
    let offsets_len = (count + 1) * if long { 4 } else { 2 };
    let new_shared_at = 20 + offsets_len;
    let new_data_at = new_shared_at + tuples.len();
    let mut out = d.slice(0, 20)?.to_vec();
    out.truncate(8);
    put_u32(&mut out, to_u32(new_shared_at, "the gvar table")?);
    out.extend_from_slice(d.slice(12, 4)?);
    put_u32(&mut out, to_u32(new_data_at, "the gvar table")?);
    let mut data = Vec::new();
    let put = |out: &mut Vec<u8>, at: usize| -> Result<()> {
        if long {
            put_u32(out, to_u32(at, "the gvar table")?);
        } else {
            put_u16(out, to_u16(at / 2, "the gvar table for short offsets")?);
        }
        Ok(())
    };
    for old in &order.new_to_old {
        put(&mut out, data.len())?;
        data.extend_from_slice(glyphs.get(usize::from(*old)).copied().unwrap_or_default());
    }
    put(&mut out, data.len())?;
    out.extend_from_slice(tuples);
    out.extend(data);
    Ok(out)
}

/// `HVAR` (`header` 20 bytes, mappings at 8, 12, 16) or `VVAR` (24 bytes, at 8 to 20). The
/// advance mapping, implicit or explicit, becomes an explicit map in the new order, and each
/// other mapping present is reordered. The new maps are appended; the variation store stays
/// where it is.
pub(crate) fn metrics_variations(table: &[u8], tag: [u8; 4], order: &Order) -> Result<Vec<u8>> {
    let d = Data::new(tag, table);
    let header = if &tag == b"HVAR" { 20 } else { 24 };
    let mut out = table.to_vec();
    for (i, slot) in (8..header).step_by(4).enumerate() {
        let at = d.u32_usize(slot)?;
        let entries: Vec<(u16, u16)> = if at == 0 {
            if i != 0 {
                continue;
            }
            (0..order.len()).map(|g| (0, g as u16)).collect()
        } else {
            read_delta_set_map(d, at, order.len())?
        };
        let map = write_delta_set_map(&order.permute(&entries)?)?;
        let new_at = to_u32(out.len(), "the variations table")?;
        out.extend(map);
        if let Some(s) = out.get_mut(slot..slot + 4) {
            s.copy_from_slice(&new_at.to_be_bytes());
        }
    }
    Ok(out)
}

/// The (outer, inner) pair of every glyph from a DeltaSetIndexMap; glyphs past its end use
/// the last entry.
fn read_delta_set_map(d: Data<'_>, at: usize, n: usize) -> Result<Vec<(u16, u16)>> {
    let format = d.u8(at)?;
    let entry_format = d.u8(at + 1)?;
    let (count, data_at) = match format {
        0 => (usize::from(d.u16(at + 2)?), at + 4),
        1 => (d.u32_usize(at + 2)?, at + 6),
        f => return Err(d.bad(format!("has a delta-set index map of format {f}"))),
    };
    if count == 0 {
        return Err(d.bad("has an empty delta-set index map"));
    }
    let size = usize::from((entry_format >> 4) & 3) + 1;
    let inner_bits = u32::from(entry_format & 0x0F) + 1;
    let mut out = Vec::with_capacity(n);
    for g in 0..n {
        let i = g.min(count - 1);
        let raw = d.slice(data_at + i * size, size)?.iter().fold(0u32, |acc, b| acc << 8 | u32::from(*b));
        out.push(((raw >> inner_bits) as u16, (raw & ((1u32 << inner_bits) - 1)) as u16));
    }
    Ok(out)
}

fn write_delta_set_map(entries: &[(u16, u16)]) -> Result<Vec<u8>> {
    let bits = |v: u16| (16 - v.leading_zeros()).max(1);
    let inner_bits = entries.iter().map(|e| bits(e.1)).max().unwrap_or(1);
    let outer_bits = entries.iter().map(|e| if e.0 == 0 { 0 } else { bits(e.0) }).max().unwrap_or(0);
    let total = inner_bits + outer_bits;
    let size = total.div_ceil(8).max(1);
    let mut out = Vec::new();
    if entries.len() <= usize::from(u16::MAX) {
        out.push(0);
        out.push((((size - 1) << 4) | (inner_bits - 1)) as u8);
        put_u16(&mut out, entries.len() as u16);
    } else {
        out.push(1);
        out.push((((size - 1) << 4) | (inner_bits - 1)) as u8);
        put_u32(&mut out, to_u32(entries.len(), "a delta-set index map")?);
    }
    for (outer, inner) in entries {
        let raw = (u32::from(*outer) << inner_bits) | u32::from(*inner);
        let bytes = raw.to_be_bytes();
        out.extend_from_slice(bytes.get(4 - size as usize..).unwrap_or_default());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    #[test]
    fn trailing_equal_advances_share_one_long_metric() {
        // Three glyphs: advances 500, 600, 600 with two long metrics; the new order 0, 2, 1
        // gives 500, 600, 600 again (one fewer long metric is not possible: 500 differs).
        let mut hmtx = Vec::new();
        for v in [500u16, 10, 600, 20, 30] {
            put_u16(&mut hmtx, v);
        }
        let order = Order::new(vec![0, 2, 1]);
        let (out, count) = metrics(&hmtx, *b"hmtx", 2, &order).unwrap();
        assert_eq!(count, 2);
        let words: Vec<u16> = out.chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        assert_eq!(words, [500, 10, 600, 30, 20]);
        let order = Order::new(vec![1, 2, 0]);
        let (_, count) = metrics(&hmtx, *b"hmtx", 2, &order).unwrap();
        assert_eq!(count, 3, "600, 600, 500: every advance is written");
    }

    #[test]
    fn delta_set_maps_round_trip() {
        let entries = vec![(0, 3), (1, 0), (0, 300)];
        let bytes = write_delta_set_map(&entries).unwrap();
        let back = read_delta_set_map(Data::new(*b"HVAR", &bytes), 0, 4).unwrap();
        assert_eq!(back, [(0, 3), (1, 0), (0, 300), (0, 300)], "a glyph past the end uses the last entry");
    }
}
