//! `cmap` from its subtable elements, compiled as fontTools' `_c_m_a_p` compiles them: the
//! subtables sorted by platform, encoding and language, identical subtable data stored once,
//! format 4 segments split where a run of consecutive glyphs saves space.

use std::collections::{BTreeMap, HashMap};

use crate::{Result, TablesError};

use super::super::Element;
use super::super::read::{Out, ReadContext, attr, int_lit, read_hex};

enum Body {
    /// Formats 0, 4, 6, 12 and 13: code point to glyph name.
    Map(BTreeMap<i64, String>),
    /// Format 14: variation selector to (base character, glyph name or `None` for the default).
    Uvs(BTreeMap<i64, Vec<(i64, Option<String>)>>),
    /// A format fontTools keeps as bytes.
    Raw(Vec<u8>),
}

struct Subtable {
    platform: i64,
    encoding: i64,
    format: i64,
    language: Option<i64>,
    reserved: i64,
    body: Body,
}

fn bad(what: impl Into<String>) -> TablesError {
    TablesError::invalid(what.into())
}

fn read_subtable(e: &Element, format: i64, ctx: &ReadContext) -> Result<Subtable> {
    let platform = int_lit(attr(e, "platformID")?)?;
    let encoding = int_lit(attr(e, "platEncID")?)?;
    let maps = || -> Result<BTreeMap<i64, String>> {
        let mut m = BTreeMap::new();
        for map in e.children.iter().filter(|c| c.name == "map") {
            m.insert(int_lit(attr(map, "code")?)?, attr(map, "name")?.to_string());
        }
        if m.len() > crate::limits::MAX_TTX_CMAP_ENTRIES {
            return Err(TablesError::TooLarge { what: "the cmap mappings", limit: crate::limits::MAX_TTX_CMAP_ENTRIES });
        }
        Ok(m)
    };
    let (language, reserved, body) = match format {
        0 | 4 | 6 => (Some(int_lit(attr(e, "language")?)?), 0, Body::Map(maps()?)),
        12 | 13 => (Some(int_lit(attr(e, "language")?)?), int_lit(attr(e, "reserved")?)?, Body::Map(maps()?)),
        14 => {
            let mut uvs: BTreeMap<i64, Vec<(i64, Option<String>)>> = BTreeMap::new();
            for map in e.children.iter().filter(|c| c.name == "map") {
                // A glyph named `None` only when the glyph order has one: otherwise the default.
                let name = map.attribute("name").filter(|n| *n != "None" || ctx.glyph_id("None").is_ok()).map(str::to_string);
                uvs.entry(int_lit(attr(map, "uvs")?)?).or_default().push((int_lit(attr(map, "uv")?)?, name));
            }
            (Some(0xFF), 0, Body::Uvs(uvs))
        }
        2 => return Err(TablesError::Unsupported("reading cmap format 2 from TTX".into())),
        _ => (None, 0, Body::Raw(read_hex(&e.text)?)),
    };
    Ok(Subtable { platform, encoding, format, language, reserved, body })
}

/// The glyph id of a name as formats 4, 12 and 13 look it up (`gidN` names a glyph by index).
fn gid_of(name: &str, ctx: &ReadContext) -> Result<i64> {
    match ctx.glyph_id(name) {
        Ok(id) => Ok(i64::from(id)),
        Err(e) => match name.strip_prefix("gid").and_then(|d| d.parse::<i64>().ok()) {
            Some(id) => Ok(id),
            None => Err(e),
        },
    }
}

/// fontTools' `getSearchRange(n, itemSize)`.
fn search_range(n: i64, item_size: i64) -> (i64, i64, i64) {
    let mut exponent = 0i64;
    let mut x = n;
    while x > 0 {
        x >>= 1;
        exponent += 1;
    }
    let exponent = (exponent - 1).max(0);
    let range = (1i64 << exponent.min(30)) * item_size;
    (range, exponent, (n * item_size - range).max(0))
}

pub(super) fn compile(element: &Element, ctx: &ReadContext) -> Result<Vec<u8>> {
    let mut version = 0i64;
    let mut tables: Vec<Subtable> = Vec::new();
    for child in &element.children {
        if child.name == "tableVersion" {
            version = int_lit(attr(child, "version")?)?;
        } else if let Some(format) = child.name.strip_prefix("cmap_format_") {
            tables.push(read_subtable(child, int_lit(format)?, ctx)?);
        }
    }
    tables.sort_by_key(|t| (t.platform, t.encoding, t.language.unwrap_or(-1)));
    for pair in tables.windows(2) {
        if let [a, b] = pair
            && (a.platform, a.encoding, a.language) == (b.platform, b.encoding, b.language)
        {
            return Err(bad(format!(
                "cmap subtables have duplicate (platformID, platEncID, language) entries: ({}, {}, {:?})",
                a.platform, a.encoding, a.language
            )));
        }
    }
    let mut out = Out::default();
    out.u16(version, "cmap.tableVersion")?;
    out.u16(tables.len() as i64, "the cmap subtable count")?;
    let base = 4 + 8 * tables.len();
    let mut data: Vec<u8> = Vec::new();
    let mut done: HashMap<Vec<u8>, usize> = HashMap::new();
    for t in &tables {
        let chunk = compile_subtable(t, ctx)?;
        let offset = match done.get(&chunk) {
            Some(o) => *o,
            None => {
                let o = base + data.len();
                data.extend_from_slice(&chunk);
                done.insert(chunk, o);
                o
            }
        };
        out.u16(t.platform, "cmap.platformID")?;
        out.u16(t.encoding, "cmap.platEncID")?;
        out.u32(offset as i64, "a cmap subtable offset")?;
    }
    out.bytes(&data);
    Ok(out.0)
}

fn compile_subtable(t: &Subtable, ctx: &ReadContext) -> Result<Vec<u8>> {
    let language = t.language.unwrap_or(0);
    let mut out = Out::default();
    match (&t.body, t.format) {
        (Body::Raw(data), _) => {
            if data.is_empty() {
                return Err(bad(format!("the cmap format {} subtable has no data", t.format)));
            }
            out.bytes(data);
        }
        (Body::Map(map), 0) => {
            out.u16(0, "format")?;
            out.u16(262, "length")?;
            out.u16(language, "language")?;
            let mut gids = [0u8; 256];
            for (code, name) in map {
                let slot = usize::try_from(*code).ok().and_then(|c| gids.get_mut(c)).ok_or_else(|| bad("a cmap format 0 code is past 255"))?;
                *slot = u8::try_from(ctx.glyph_id(name)?).map_err(|_| bad("a cmap format 0 glyph is past 255"))?;
            }
            out.bytes(&gids);
        }
        (Body::Map(map), 4) => format4(&mut out, map, language, ctx)?,
        (Body::Map(map), 6) => {
            let (first, last) = match (map.keys().next(), map.keys().next_back()) {
                (Some(f), Some(l)) => (*f, *l),
                _ => (0, -1),
            };
            let count = last - first + 1;
            if count > 0x10000 {
                return Err(bad("a cmap format 6 subtable spans more than 65,536 codes"));
            }
            out.u16(6, "format")?;
            out.u16(count * 2 + 10, "length")?;
            out.u16(language, "language")?;
            out.u16(first, "firstCode")?;
            out.u16(count, "entryCount")?;
            for code in first..=last {
                let gid = match map.get(&code) {
                    Some(name) => i64::from(ctx.glyph_id(name)?),
                    None => 0,
                };
                out.u16(gid, "a glyph id")?;
            }
        }
        (Body::Map(map), 12 | 13) => {
            let step = i64::from(t.format == 12);
            let mut groups: Vec<(i64, i64, i64)> = Vec::new();
            let mut iter = map.iter();
            let (first_code, first_name) = iter.next().ok_or_else(|| bad(format!("the cmap format {} subtable is empty", t.format)))?;
            let mut start = (*first_code, gid_of(first_name, ctx)?);
            let (mut last_code, mut last_gid) = start;
            for (code, name) in iter {
                let gid = gid_of(name, ctx)?;
                if !(gid == last_gid + step && *code == last_code + 1) {
                    groups.push((start.0, last_code, start.1));
                    start = (*code, gid);
                }
                (last_code, last_gid) = (*code, gid);
            }
            groups.push((start.0, last_code, start.1));
            out.u16(t.format, "format")?;
            out.u16(t.reserved, "reserved")?;
            out.u32(groups.len() as i64 * 12 + 16, "length")?;
            out.u32(language, "language")?;
            out.u32(groups.len() as i64, "nGroups")?;
            for (s, e, g) in groups {
                out.u32(s, "startCharCode")?;
                out.u32(e, "endCharCode")?;
                out.u32(g, "startGlyphID")?;
            }
        }
        (Body::Uvs(uvs), _) => format14(&mut out, uvs, ctx)?,
        (Body::Map(_), f) => return Err(TablesError::Unsupported(format!("cmap format {f}"))),
    }
    Ok(out.0)
}

/// fontTools' `splitRange`: the extra segment starts and the ends for codes `start..=end`.
fn split_range(start: i64, end: i64, cmap: &BTreeMap<i64, i64>) -> (Vec<i64>, Vec<i64>) {
    if start == end {
        return (Vec::new(), vec![end]);
    }
    let gid = |c: i64| cmap.get(&c).copied().unwrap_or(0);
    let mut last_id = gid(start);
    let mut last_code = start;
    let mut in_order: Option<bool> = None;
    let mut ordered_begin: Option<i64> = None;
    let mut sub: Vec<(i64, i64)> = Vec::new();
    for code in start + 1..=end {
        let id = gid(code);
        if id - 1 == last_id {
            if in_order != Some(true) {
                in_order = Some(true);
                ordered_begin = Some(last_code);
            }
        } else if in_order == Some(true) {
            in_order = Some(false);
            sub.push((ordered_begin.unwrap_or(last_code), last_code));
            ordered_begin = None;
        }
        last_id = id;
        last_code = code;
    }
    if in_order == Some(true) {
        sub.push((ordered_begin.unwrap_or(last_code), last_code));
    }
    let mut kept: Vec<(i64, i64)> = Vec::new();
    for (b, e) in sub {
        if b == start && e == end {
            break;
        }
        let threshold = if b == start || e == end { 4 } else { 8 };
        if e - b + 1 > threshold {
            kept.push((b, e));
        }
    }
    let (Some(first), Some(last)) = (kept.first().copied(), kept.last().copied()) else { return (Vec::new(), vec![end]) };
    if first.0 != start {
        kept.insert(0, (start, first.0 - 1));
    }
    if last.1 != end {
        kept.push((last.1 + 1, end));
    }
    let mut i = 1;
    while i < kept.len() {
        if let (Some(prev), Some(cur)) = (kept.get(i - 1).copied(), kept.get(i).copied())
            && prev.1 + 1 != cur.0
        {
            kept.insert(i, (prev.1 + 1, cur.0 - 1));
            i += 1;
        }
        i += 1;
    }
    let starts = kept.iter().skip(1).map(|r| r.0).collect();
    let ends = kept.iter().map(|r| r.1).collect();
    (starts, ends)
}

fn format4(out: &mut Out, map: &BTreeMap<i64, String>, language: i64, ctx: &ReadContext) -> Result<()> {
    let mut cmap: BTreeMap<i64, i64> = BTreeMap::new();
    for (code, name) in map {
        cmap.insert(*code, gid_of(name, ctx)?);
    }
    let codes: Vec<i64> = cmap.keys().copied().collect();
    let (mut starts, mut ends) = (Vec::new(), Vec::new());
    if let Some((&first, rest)) = codes.split_first() {
        let mut last = first;
        starts.push(first);
        for &code in rest {
            if code == last + 1 {
                last = code;
                continue;
            }
            let (s, e) = split_range(starts.last().copied().unwrap_or(first), last, &cmap);
            starts.extend(s);
            ends.extend(e);
            starts.push(code);
            last = code;
        }
        let (s, e) = split_range(starts.last().copied().unwrap_or(first), last, &cmap);
        starts.extend(s);
        ends.extend(e);
    }
    starts.push(0xFFFF);
    ends.push(0xFFFF);
    let seg_count = ends.len();
    let mut deltas: Vec<i64> = Vec::new();
    let mut range_offsets: Vec<i64> = Vec::new();
    let mut glyph_ids: Vec<i64> = Vec::new();
    for i in 0..seg_count - 1 {
        let (s, e) = (starts.get(i).copied().unwrap_or(0), ends.get(i).copied().unwrap_or(0));
        let indices: Vec<i64> = (s..=e).map(|c| cmap.get(&c).copied().unwrap_or(0)).collect();
        let first = indices.first().copied().unwrap_or(0);
        if indices.iter().enumerate().all(|(k, g)| *g == first + k as i64) {
            deltas.push((first - s).rem_euclid(0x10000));
            range_offsets.push(0);
        } else {
            deltas.push(0);
            range_offsets.push(2 * (seg_count as i64 + glyph_ids.len() as i64 - i as i64));
            glyph_ids.extend(indices);
        }
        if glyph_ids.len() > 0x10000 {
            return Err(bad("a cmap format 4 subtable is too large"));
        }
    }
    deltas.push(1);
    range_offsets.push(0);
    let (range, selector, shift) = search_range(seg_count as i64, 2);
    let mut body = Out::default();
    for e in &ends {
        body.u16(*e, "endCode")?;
    }
    body.u16(0, "reservedPad")?;
    for s in &starts {
        body.u16(*s, "startCode")?;
    }
    for d in &deltas {
        body.u16(*d, "idDelta")?;
    }
    for r in range_offsets.iter().chain(&glyph_ids) {
        body.u16(*r, "idRangeOffset")?;
    }
    out.u16(4, "format")?;
    out.u16(14 + body.0.len() as i64, "length")?;
    out.u16(language, "language")?;
    out.u16(seg_count as i64 * 2, "segCountX2")?;
    out.u16(range, "searchRange")?;
    out.u16(selector, "entrySelector")?;
    out.u16(shift, "rangeShift")?;
    out.bytes(&body.0);
    Ok(())
}

fn format14(out: &mut Out, uvs: &BTreeMap<i64, Vec<(i64, Option<String>)>>, ctx: &ReadContext) -> Result<()> {
    let n = uvs.len() as i64;
    let mut offset = 10 + n * 11;
    let mut records = Out::default();
    let mut data = Out::default();
    for (selector, entries) in uvs {
        let mut defaults: Vec<i64> = entries.iter().filter(|e| e.1.is_none()).map(|e| e.0).collect();
        let default_offset = if defaults.is_empty() {
            0
        } else {
            defaults.sort_unstable();
            let mut ranges: Vec<(i64, i64)> = Vec::new();
            let mut last = defaults.first().copied().unwrap_or(0);
            let mut cnt = -1i64;
            for d in &defaults {
                cnt += 1;
                if last + cnt != *d {
                    ranges.push((last, cnt - 1));
                    last = *d;
                    cnt = 0;
                }
            }
            ranges.push((last, cnt));
            let at = offset;
            data.u32(ranges.len() as i64, "numUnicodeValueRanges")?;
            for (start, more) in &ranges {
                data.u24(*start, "startUnicodeValue")?;
                data.u8(*more, "additionalCount")?;
            }
            offset += 4 + ranges.len() as i64 * 4;
            at
        };
        let mut mapped: Vec<(i64, &String)> = entries.iter().filter_map(|e| e.1.as_ref().map(|n| (e.0, n))).collect();
        let mapped_offset = if mapped.is_empty() {
            0
        } else {
            mapped.sort();
            let at = offset;
            data.u32(mapped.len() as i64, "numUVSMappings")?;
            offset += 4 + mapped.len() as i64 * 5;
            for (uv, name) in mapped {
                data.u24(uv, "unicodeValue")?;
                data.u16(i64::from(ctx.glyph_id(name)?), "glyphID")?;
            }
            at
        };
        records.u24(*selector, "varSelector")?;
        records.u32(default_offset, "defaultUVSOffset")?;
        records.u32(mapped_offset, "nonDefaultUVSOffset")?;
    }
    out.u16(14, "format")?;
    out.u32(10 + records.0.len() as i64 + data.0.len() as i64, "length")?;
    out.u32(n, "numVarSelectorRecords")?;
    out.bytes(&records.0);
    out.bytes(&data.0);
    Ok(())
}
