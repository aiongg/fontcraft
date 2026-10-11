//! `cmap`: the subtables as fontTools reads them, and their TTX form. Mappings to glyph 0 are
//! left out, a later mapping of a code point replaces an earlier one, format 4's last segment is
//! never read, and formats fontTools does not read (8, 10 and unknown ones) are written as hex.

use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

use super::super::{XmlWriter, py_hex};
use super::{Rd, glyph_name};

/// Most mappings read from one subtable; more is refused rather than allocated.
const MAX_ENTRIES: usize = 1 << 22;

pub(crate) struct Subtable {
    pub platform: u16,
    pub encoding: u16,
    pub format: u16,
    pub kind: Kind,
}

#[derive(Clone)]
pub(crate) enum Kind {
    /// Formats 0, 2, 4 and 6: code point and glyph id, sorted by code point.
    Short { language: u32, map: Vec<(u32, u32)> },
    /// Formats 12 and 13, with their header fields.
    Long { reserved: u16, length: u32, language: u32, groups: u32, map: Vec<(u32, u32)> },
    /// Format 14: variation selector, base character and the glyph for non-default ones.
    Uvs { entries: Vec<(u32, u32, Option<u32>)> },
    /// A format fontTools keeps as bytes.
    Unknown { data: Vec<u8> },
}

impl Subtable {
    /// fontTools' `isUnicode`: platform 0, or Windows with encoding 0, 1 or 10.
    pub fn is_unicode(&self) -> bool {
        self.platform == 0 || (self.platform == 3 && matches!(self.encoding, 0 | 1 | 10))
    }

    /// The code point to glyph id mappings.
    pub fn map(&self) -> &[(u32, u32)] {
        match &self.kind {
            Kind::Short { map, .. } | Kind::Long { map, .. } => map,
            _ => &[],
        }
    }
}

/// The table version and the subtables, in directory order.
pub(crate) fn parse(data: &[u8]) -> Result<(u16, Vec<Subtable>)> {
    if data.len() < 4 {
        return Err(TablesError::invalid("the 'cmap' table is too short"));
    }
    let mut r = Rd::new(data);
    let version = r.u16()?;
    let count = usize::from(r.u16()?);
    let mut out: Vec<Subtable> = Vec::with_capacity(count);
    let mut seen: Vec<(usize, usize)> = Vec::new();
    for _ in 0..count {
        let platform = r.u16()?;
        let encoding = r.u16()?;
        let offset = r.u32()? as usize;
        let mut h = Rd::at(data, offset);
        if offset.saturating_add(4) > data.len() {
            return Err(TablesError::invalid("a 'cmap' subtable offset is out of bounds"));
        }
        let format = h.u16()?;
        let length = match format {
            8 | 10 | 12 | 13 => Rd::at(data, offset.saturating_add(4)).u32()? as usize,
            14 => Rd::at(data, offset.saturating_add(2)).u32()? as usize,
            _ => usize::from(h.u16()?),
        };
        if length == 0 {
            continue;
        }
        let header = match format {
            0 | 2 | 4 | 6 => 6,
            12 | 13 => 16,
            14 => 10,
            _ => 0,
        };
        if length < header || offset.saturating_add(length) > data.len() {
            return Err(TablesError::invalid("a 'cmap' subtable is cut short"));
        }
        let body = data.get(offset..offset.saturating_add(length)).unwrap_or_default();
        let kind = match seen.iter().find(|(o, _)| *o == offset).and_then(|(_, at)| out.get(*at)) {
            Some(earlier) => earlier.kind.clone(),
            None => {
                seen.push((offset, out.len()));
                read_subtable(format, body)?
            }
        };
        out.push(Subtable { platform, encoding, format, kind });
    }
    Ok((version, out))
}

fn too_many() -> TablesError {
    TablesError::TooLarge { what: "the mappings of a 'cmap' subtable", limit: MAX_ENTRIES }
}

/// Last mapping of each code point wins; glyph 0 is dropped; sorted by code point.
fn finish(mut pairs: Vec<(u32, u32)>) -> Vec<(u32, u32)> {
    pairs.reverse();
    pairs.sort_by_key(|(c, _)| *c);
    pairs.dedup_by_key(|(c, _)| *c);
    pairs.retain(|(_, g)| *g != 0);
    pairs
}

fn read_subtable(format: u16, body: &[u8]) -> Result<Kind> {
    match format {
        0 | 2 | 4 | 6 => {
            let mut h = Rd::new(body);
            let _ = h.u16()?;
            let _ = h.u16()?;
            let language = u32::from(h.u16()?);
            let rest = h.rest();
            let map = match format {
                0 => format0(body.len(), rest)?,
                2 => format2(rest)?,
                4 => format4(rest)?,
                _ => format6(rest)?,
            };
            Ok(Kind::Short { language, map: finish(map) })
        }
        12 | 13 => {
            let mut h = Rd::new(body);
            let _ = h.u16()?;
            let reserved = h.u16()?;
            let length = h.u32()?;
            let language = h.u32()?;
            let groups = h.u32()?;
            if (groups as usize).checked_mul(12).and_then(|n| n.checked_add(16)) != Some(length as usize) {
                return Err(TablesError::invalid("a 'cmap' format 12 or 13 subtable has an inconsistent group count"));
            }
            let map = format12(h.rest(), groups as usize, format == 12)?;
            Ok(Kind::Long { reserved, length, language, groups, map: finish(map) })
        }
        14 => Ok(Kind::Uvs { entries: format14(body)? }),
        _ => Ok(Kind::Unknown { data: body.to_vec() }),
    }
}

fn u16s(data: &[u8]) -> Result<Vec<u32>> {
    if data.len() % 2 != 0 {
        return Err(TablesError::invalid("a 'cmap' array has an odd length"));
    }
    Ok(data.chunks(2).map(|p| u32::from(p.first().copied().unwrap_or(0)) << 8 | u32::from(p.get(1).copied().unwrap_or(0))).collect())
}

fn format0(length: usize, rest: &[u8]) -> Result<Vec<(u32, u32)>> {
    if length != 262 {
        return Err(TablesError::invalid("a format 0 'cmap' subtable is not 262 bytes"));
    }
    Ok(rest.iter().enumerate().map(|(c, g)| (c as u32, u32::from(*g))).collect())
}

fn format2(rest: &[u8]) -> Result<Vec<(u32, u32)>> {
    let keys = u16s(rest.get(..512).ok_or_else(|| TablesError::invalid("a format 2 'cmap' subtable is cut short"))?)?;
    let keys: Vec<usize> = keys.iter().map(|k| (*k / 8) as usize).collect();
    let data = rest.get(512..).unwrap_or_default();
    let max = keys.iter().copied().max().unwrap_or(0);
    let mut headers = Vec::with_capacity(max.saturating_add(1));
    let mut pos = 0usize;
    for _ in 0..=max {
        let mut r = Rd::at(data, pos);
        let first = r.u16()?;
        let count = r.u16()?;
        let delta = r.i16()?;
        let range_offset = r.u16()?;
        pos = pos.saturating_add(8);
        let at = pos.saturating_add(usize::from(range_offset)).saturating_sub(2);
        let end = at.saturating_add(usize::from(count).saturating_mul(2)).min(data.len());
        let gids = u16s(data.get(at.min(end)..end).unwrap_or_default())?;
        headers.push((u32::from(first), u32::from(count), i64::from(delta), gids));
    }
    let mut map = Vec::new();
    let bad = || TablesError::invalid("a format 2 'cmap' subtable points past its glyph array");
    for (first_byte, key) in keys.iter().enumerate() {
        let first_byte = first_byte as u32;
        let (first, count, delta, gids) = headers.get(*key).ok_or_else(bad)?;
        if *key == 0 {
            if first_byte < *first || first_byte >= first.saturating_add(*count) {
                continue;
            }
            let gi = *gids.get((first_byte - first) as usize).ok_or_else(bad)?;
            if gi != 0 {
                map.push((first_byte, (i64::from(gi) + delta).rem_euclid(0x10000) as u32));
            }
        } else {
            for i in 0..*count {
                let gi = *gids.get(i as usize).ok_or_else(bad)?;
                if gi != 0 {
                    map.push((first_byte * 256 + first + i, (i64::from(gi) + delta).rem_euclid(0x10000) as u32));
                }
            }
        }
        if map.len() > MAX_ENTRIES {
            return Err(too_many());
        }
    }
    Ok(map)
}

/// Python's `a[:n]` and `a[n:]`.
fn split(a: &[u32], n: usize) -> (&[u32], &[u32]) {
    a.split_at(n.min(a.len()))
}

fn format4(rest: &[u8]) -> Result<Vec<(u32, u32)>> {
    let mut h = Rd::new(rest);
    let seg = usize::from(h.u16()?) / 2;
    let _ = (h.u16()?, h.u16()?, h.u16()?);
    let all = u16s(h.rest())?;
    let (end_codes, rest) = split(&all, seg);
    let rest = rest.get(1..).unwrap_or_default();
    let (start_codes, rest) = split(rest, seg);
    let (deltas, rest) = split(rest, seg);
    let (range_offsets, glyph_array) = split(rest, seg);
    let bad = || TablesError::invalid("a format 4 'cmap' subtable is cut short");
    let mut map = Vec::new();
    let mut work = 0usize;
    for i in 0..start_codes.len().saturating_sub(1) {
        let start = i64::from(*start_codes.get(i).ok_or_else(bad)?);
        let end = i64::from(*end_codes.get(i).ok_or_else(bad)?);
        let delta = i64::from(*deltas.get(i).ok_or_else(bad)?);
        let range_offset = i64::from(*range_offsets.get(i).ok_or_else(bad)?);
        let partial = range_offset / 2 - start + i as i64 - range_offsets.len() as i64;
        work = work.saturating_add(usize::try_from(end - start + 1).unwrap_or(0));
        if work > MAX_ENTRIES {
            return Err(too_many());
        }
        for code in start..=end {
            let gid = if range_offset == 0 {
                (code + delta) & 0xFFFF
            } else {
                let index = code + partial;
                let g = usize::try_from(index)
                    .ok()
                    .and_then(|ix| glyph_array.get(ix))
                    .ok_or_else(|| TablesError::invalid(format!("a format 4 'cmap' subtable points outside its glyph array in segment {i}")))?;
                if *g != 0 { (i64::from(*g) + delta) & 0xFFFF } else { 0 }
            };
            if gid != 0 {
                map.push((code as u32, gid as u32));
            }
        }
    }
    Ok(map)
}

fn format6(rest: &[u8]) -> Result<Vec<(u32, u32)>> {
    let mut h = Rd::new(rest);
    let first = u32::from(h.u16()?);
    let count = usize::from(h.u16()?);
    let data = h.rest();
    let gids = u16s(data.get(..count.saturating_mul(2).min(data.len())).unwrap_or_default())?;
    Ok(gids.iter().enumerate().map(|(i, g)| (first.saturating_add(i as u32), *g)).collect())
}

fn format12(rest: &[u8], groups: usize, sequential: bool) -> Result<Vec<(u32, u32)>> {
    let mut r = Rd::new(rest);
    let mut map = Vec::new();
    let mut last_end = 0u32;
    for _ in 0..groups {
        let mut start = r.u32()?;
        let end = r.u32()?.min(0x10_FFFF);
        let mut gid = r.u32()?;
        if start > end || start < last_end {
            continue;
        }
        last_end = end;
        if gid == 0 {
            if !sequential {
                continue;
            }
            start = start.saturating_add(1);
            gid = 1;
        }
        let mut code = start;
        while code <= end {
            map.push((code, gid));
            if sequential {
                gid = gid.saturating_add(1);
            }
            if map.len() > MAX_ENTRIES {
                return Err(too_many());
            }
            match code.checked_add(1) {
                Some(c) => code = c,
                None => break,
            }
        }
    }
    Ok(map)
}

fn format14(body: &[u8]) -> Result<Vec<(u32, u32, Option<u32>)>> {
    let mut h = Rd::new(body);
    let _ = (h.u16()?, h.u32()?);
    let records = h.u32()?;
    let data = h.rest();
    let mut by_selector: Vec<(u32, Vec<(u32, Option<u32>)>)> = Vec::new();
    let mut r = Rd::new(data);
    let bad = || TablesError::invalid("a format 14 'cmap' subtable points before its records");
    let mut total = 0usize;
    for _ in 0..records {
        let selector = r.u24()?;
        let default_offset = r.u32()? as usize;
        let non_default_offset = r.u32()? as usize;
        let mut list: Vec<(u32, Option<u32>)> = Vec::new();
        if default_offset != 0 {
            let mut d = Rd::at(data, default_offset.checked_sub(10).ok_or_else(bad)?);
            for _ in 0..d.u32()? {
                let first = d.u24()?;
                let more = u32::from(d.u8()?);
                for uv in first..=first.saturating_add(more) {
                    list.push((uv, None));
                }
                total = total.saturating_add(usize::try_from(more).unwrap_or(0).saturating_add(1));
                if total > MAX_ENTRIES {
                    return Err(too_many());
                }
            }
        }
        if non_default_offset != 0 {
            let mut d = Rd::at(data, non_default_offset.checked_sub(10).ok_or_else(bad)?);
            for _ in 0..d.u32()? {
                let uv = d.u24()?;
                let gid = u32::from(d.u16()?);
                list.push((uv, Some(gid)));
                total = total.saturating_add(1);
                if total > MAX_ENTRIES {
                    return Err(too_many());
                }
            }
        }
        match by_selector.iter_mut().find(|(s, _)| *s == selector) {
            Some((_, existing)) => existing.extend(list),
            None => by_selector.push((selector, list)),
        }
    }
    by_selector.sort_by_key(|(s, _)| *s);
    let mut out = Vec::new();
    for (selector, mut list) in by_selector {
        list.sort_by_key(|(uv, gid)| (gid.is_some(), *uv, *gid));
        out.extend(list.into_iter().map(|(uv, gid)| (selector, uv, gid)));
    }
    Ok(out)
}

pub(super) fn write(data: &[u8], font: &SfntFont, w: &mut XmlWriter) -> Result<()> {
    let (version, subtables) = parse(data)?;
    let needs_names = subtables.iter().any(|s| !matches!(s.kind, Kind::Unknown { .. }));
    let order = if needs_names { w.glyph_order(font)? } else { std::rc::Rc::new(Vec::new()) };
    w.simple("tableVersion", &[("version", &version.to_string())]);
    for st in &subtables {
        let platform = st.platform.to_string();
        let encoding = st.encoding.to_string();
        let element = format!("cmap_format_{}", st.format);
        match &st.kind {
            Kind::Short { language, map } => {
                w.open(&element, &[("platformID", &platform), ("platEncID", &encoding), ("language", &language.to_string())]);
                write_map(w, map, &order);
                w.close(&element);
            }
            Kind::Long { reserved, length, language, groups, map } => {
                w.open(
                    &element,
                    &[
                        ("platformID", &platform),
                        ("platEncID", &encoding),
                        ("format", &st.format.to_string()),
                        ("reserved", &reserved.to_string()),
                        ("length", &length.to_string()),
                        ("language", &language.to_string()),
                        ("nGroups", &groups.to_string()),
                    ],
                );
                write_map(w, map, &order);
                w.close(&element);
            }
            Kind::Uvs { entries } => {
                w.open(&element, &[("platformID", &platform), ("platEncID", &encoding)]);
                for (selector, uv, gid) in entries {
                    let uv = py_hex(i64::from(*uv));
                    let uvs = py_hex(i64::from(*selector));
                    match gid {
                        Some(g) => w.simple("map", &[("uv", &uv), ("uvs", &uvs), ("name", &glyph_name(&order, *g))]),
                        None => w.simple("map", &[("uv", &uv), ("uvs", &uvs)]),
                    }
                }
                w.close(&element);
            }
            Kind::Unknown { data } => {
                w.open(&element, &[("platformID", &platform), ("platEncID", &encoding)]);
                w.hex(data);
                w.close(&element);
            }
        }
    }
    Ok(())
}

fn write_map(w: &mut XmlWriter, map: &[(u32, u32)], order: &[String]) {
    for (code, gid) in map {
        w.simple("map", &[("code", &py_hex(i64::from(*code))), ("name", &glyph_name(order, *gid))]);
    }
}
