//! `cmap` in a new glyph order. Every subtable keeps its format and encoding record: formats
//! 0, 6, 13 and 14 have their glyph ids rewritten in place, formats 4 and 12 are encoded again
//! from their mappings. Formats 2, 8 and 10 are refused.

use std::collections::HashMap;

use read_fonts::tables::cmap::{Cmap4, Cmap12};
use read_fonts::{FontData, FontRead};

use super::be::{Data, put_u16, put_u32, search_params, set_u16, to_u16, to_u32};
use super::tables::Order;
use crate::limits::MAX_CODE_POINTS;
use crate::{Result, TablesError};

const TAG: [u8; 4] = *b"cmap";

pub(crate) fn cmap(table: &[u8], order: &Order) -> Result<Vec<u8>> {
    let d = Data::new(TAG, table);
    let count = usize::from(d.u16(2)?);
    let mut records = Vec::with_capacity(count);
    for i in 0..count {
        records.push((d.slice(4 + 8 * i, 4)?, d.u32_usize(8 + 8 * i)?));
    }
    let mut out = d.slice(0, 4)?.to_vec();
    let mut body: Vec<u8> = Vec::new();
    let mut placed: HashMap<usize, usize> = HashMap::new();
    let header_len = 4 + 8 * count;
    for (ids, at) in &records {
        let new_at = match placed.get(at) {
            Some(new_at) => *new_at,
            None => {
                let sub = subtable(d, *at, order)?;
                let new_at = header_len + body.len();
                body.extend(sub);
                while !body.len().is_multiple_of(4) {
                    body.push(0);
                }
                placed.insert(*at, new_at);
                new_at
            }
        };
        out.extend_from_slice(ids);
        put_u32(&mut out, to_u32(new_at, "the cmap table")?);
    }
    out.extend(body);
    Ok(out)
}

fn subtable(d: Data<'_>, at: usize, order: &Order) -> Result<Vec<u8>> {
    let format = d.u16(at)?;
    match format {
        0 => {
            let mut sub = d.slice(at, 262)?.to_vec();
            for slot in sub.iter_mut().skip(6) {
                let new = order.map(u16::from(*slot))?;
                *slot = u8::try_from(new).map_err(|_| TablesError::Unsupported(format!("glyph id {new} in a format 0 cmap subtable")))?;
            }
            Ok(sub)
        }
        6 => {
            let count = usize::from(d.u16(at + 8)?);
            let mut sub = d.slice(at, 10 + 2 * count)?.to_vec();
            for i in 0..count {
                let old = d.u16(at + 10 + 2 * i)?;
                set_u16(&mut sub, 10 + 2 * i, order.map(old)?)?;
            }
            Ok(sub)
        }
        4 => {
            let len = usize::from(d.u16(at + 2)?);
            let language = d.u16(at + 4)?;
            let table =
                Cmap4::read(FontData::new(d.slice(at, len)?)).map_err(|e| d.bad(format!("has a format 4 subtable that cannot be read: {e}")))?;
            let mut map = Vec::new();
            for (cp, gid) in table.iter() {
                let old = u16::try_from(gid.to_u32()).map_err(|_| d.bad("maps to a glyph id past 65535"))?;
                if old != 0 && cp < 0xFFFF {
                    map.push((cp, order.map(old)?));
                }
            }
            write_format4(language, &map)
        }
        12 => {
            let len = d.u32_usize(at + 4)?;
            let language = d.u32(at + 8)?;
            let table =
                Cmap12::read(FontData::new(d.slice(at, len)?)).map_err(|e| d.bad(format!("has a format 12 subtable that cannot be read: {e}")))?;
            let mut map = Vec::new();
            for (cp, gid) in table.iter() {
                if map.len() >= MAX_CODE_POINTS {
                    return Err(TablesError::TooLarge { what: "the mappings of a cmap subtable", limit: MAX_CODE_POINTS });
                }
                let old = u16::try_from(gid.to_u32()).map_err(|_| d.bad("maps to a glyph id past 65535"))?;
                if old != 0 {
                    map.push((cp, order.map(old)?));
                }
            }
            write_format12(language, &map)
        }
        13 => {
            let len = d.u32_usize(at + 4)?;
            let groups = capped(d.u32_usize(at + 12)?)?;
            if len < 16 + 12 * groups {
                return Err(d.bad("has a format 13 subtable shorter than its groups"));
            }
            let mut sub = d.slice(at, len)?.to_vec();
            for g in 0..groups {
                let slot = 16 + 12 * g + 8;
                let old = d.u32(at + slot)?;
                let old = u16::try_from(old).map_err(|_| d.bad("maps to a glyph id past 65535"))?;
                let bytes = u32::from(order.map(old)?).to_be_bytes();
                if let Some(s) = sub.get_mut(slot..slot + 4) {
                    s.copy_from_slice(&bytes);
                }
            }
            Ok(sub)
        }
        14 => {
            let len = d.u32_usize(at + 2)?;
            let mut sub = d.slice(at, len)?.to_vec();
            let records = capped(d.u32_usize(at + 6)?)?;
            for r in 0..records {
                let rec = at + 10 + 11 * r;
                let non_default = d.u32_usize(rec + 7)?;
                if non_default == 0 {
                    continue;
                }
                let count = capped(d.u32_usize(at + non_default)?)?;
                for m in 0..count {
                    let slot = non_default + 4 + 5 * m + 3;
                    let old = d.u16(at + slot)?;
                    set_u16(&mut sub, slot, order.map(old)?)?;
                }
            }
            Ok(sub)
        }
        f => Err(TablesError::Unsupported(format!("a cmap subtable of format {f}"))),
    }
}

/// A count read from a subtable, refused past [`MAX_CODE_POINTS`].
fn capped(count: usize) -> Result<usize> {
    if count > MAX_CODE_POINTS {
        return Err(TablesError::TooLarge { what: "the records of a cmap subtable", limit: MAX_CODE_POINTS });
    }
    Ok(count)
}

/// A format 4 subtable: one segment per run of consecutive code points, with `idDelta` when
/// the glyph ids rise with the code points and a glyph id array otherwise.
fn write_format4(language: u16, map: &[(u32, u16)]) -> Result<Vec<u8>> {
    let mut segments: Vec<Vec<(u32, u16)>> = Vec::new();
    for (cp, gid) in map {
        match segments.last_mut() {
            Some(seg) if seg.last().is_some_and(|(last, _)| last + 1 == *cp) => seg.push((*cp, *gid)),
            _ => segments.push(vec![(*cp, *gid)]),
        }
    }
    let mut ends = Vec::new();
    let mut starts = Vec::new();
    let mut deltas = Vec::new();
    let mut ranges: Vec<Option<Vec<u16>>> = Vec::new();
    for seg in &segments {
        let (first_cp, first_gid) = seg.first().copied().unwrap_or((0, 0));
        let delta = i32::from(first_gid) - first_cp as i32;
        let rising = seg.iter().all(|(cp, gid)| i32::from(*gid) - *cp as i32 == delta);
        starts.push(first_cp as u16);
        ends.push(seg.last().map_or(0, |s| s.0) as u16);
        if rising {
            deltas.push(delta as u16);
            ranges.push(None);
        } else {
            deltas.push(0);
            ranges.push(Some(seg.iter().map(|s| s.1).collect()));
        }
    }
    starts.push(0xFFFF);
    ends.push(0xFFFF);
    deltas.push(1);
    ranges.push(None);
    let seg_count = starts.len();
    let mut glyph_array: Vec<u16> = Vec::new();
    let mut range_offsets = Vec::with_capacity(seg_count);
    for (i, r) in ranges.iter().enumerate() {
        match r {
            Some(ids) => {
                // Bytes from this idRangeOffset slot to the segment's first glyph id.
                let offset = 2 * (seg_count - i) + 2 * glyph_array.len();
                range_offsets.push(to_u16(offset, "a format 4 cmap subtable")?);
                glyph_array.extend(ids);
            }
            None => range_offsets.push(0),
        }
    }
    let len = 16 + 8 * seg_count + 2 * glyph_array.len();
    let mut out = Vec::with_capacity(len);
    put_u16(&mut out, 4);
    put_u16(&mut out, to_u16(len, "a format 4 cmap subtable")?);
    put_u16(&mut out, language);
    let (range, selector, shift) = search_params(seg_count, 2);
    put_u16(&mut out, to_u16(2 * seg_count, "a format 4 cmap subtable")?);
    put_u16(&mut out, range);
    put_u16(&mut out, selector);
    put_u16(&mut out, shift);
    ends.iter().for_each(|v| put_u16(&mut out, *v));
    put_u16(&mut out, 0);
    starts.iter().for_each(|v| put_u16(&mut out, *v));
    deltas.iter().for_each(|v| put_u16(&mut out, *v));
    range_offsets.iter().for_each(|v| put_u16(&mut out, *v));
    glyph_array.iter().for_each(|v| put_u16(&mut out, *v));
    Ok(out)
}

/// A format 12 subtable: one group per run of consecutive code points and glyph ids.
fn write_format12(language: u32, map: &[(u32, u16)]) -> Result<Vec<u8>> {
    let mut groups: Vec<(u32, u32, u32)> = Vec::new();
    for (cp, gid) in map {
        let gid = u32::from(*gid);
        match groups.last_mut() {
            Some((start, end, first)) if *end + 1 == *cp && *first + (*cp - *start) == gid => *end = *cp,
            _ => groups.push((*cp, *cp, gid)),
        }
    }
    let len = 16 + 12 * groups.len();
    let mut out = Vec::with_capacity(len);
    put_u16(&mut out, 12);
    put_u16(&mut out, 0);
    put_u32(&mut out, to_u32(len, "a format 12 cmap subtable")?);
    put_u32(&mut out, language);
    put_u32(&mut out, to_u32(groups.len(), "a format 12 cmap subtable")?);
    for (start, end, gid) in groups {
        put_u32(&mut out, start);
        put_u32(&mut out, end);
        put_u32(&mut out, gid);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    #[test]
    fn format4_mappings_survive_encoding() {
        let map = vec![(0x20, 3), (0x41, 1), (0x42, 2), (0x43, 9), (0x44, 4), (0xE9, 7)];
        let bytes = write_format4(0, &map).unwrap();
        let table = Cmap4::read(FontData::new(&bytes)).unwrap();
        let back: Vec<(u32, u16)> = table.iter().filter(|(cp, _)| *cp != 0xFFFF).map(|(cp, g)| (cp, g.to_u32() as u16)).collect();
        assert_eq!(back, map);
        let bytes = write_format12(0, &map).unwrap();
        let table = Cmap12::read(FontData::new(&bytes)).unwrap();
        let back: Vec<(u32, u16)> = table.iter().map(|(cp, g)| (cp, g.to_u32() as u16)).collect();
        assert_eq!(back, map);
    }
}
