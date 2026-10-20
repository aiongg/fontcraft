//! TTX reader: the tables of the miscellaneous writer ([`super::misc`]) that have compilers of
//! their own: the Apple `trak` and `ltag`, and `DSIG`, `FFTM`, `LTSH` and `VDMX` (M12.7c), and the
//! glyph metamorphosis tables `mort` and `morx` (M12.7d, in `morph`).
//!
//! `ankr`, `bsln`, `cidg`, `feat`, `gcid`, `lcar`, `opbd` and `prop` go through the generic
//! compile of [`super::read_otdata`], whose layouts include them, as in fontTools; `kern` is read
//! there too. Ported from fontTools' `ttLib.tables` `_t_r_a_k`, `_l_t_a_g`, `D_S_I_G_`,
//! `F_F_T_M_`, `L_T_S_H_` and `V_D_M_X_` (MIT).

mod morph;
#[cfg(test)]
mod morph_tests;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use crate::limits::MAX_LAYOUT_ITEMS;
use crate::sfnt::Tag;
use crate::{Result, TablesError};

use super::Element;
use super::read::{Out, ReadContext, float_lit, int_lit, ot_round, timestamp_from_string};

fn bad(what: impl Into<String>) -> TablesError {
    TablesError::invalid(what.into())
}

/// The table a TTX table element stands for, as bytes; `None` for an element this module does
/// not handle.
pub(crate) fn read_table_with(element: &Element, ctx: &ReadContext) -> Result<Option<(Tag, Vec<u8>)>> {
    if element.name == "LTSH" {
        return Ok(Some((*b"LTSH", ltsh(element, ctx)?)));
    }
    Ok(Some(match element.name.as_str() {
        "trak" => (*b"trak", trak(element)?),
        "ltag" => (*b"ltag", ltag(element)?),
        "DSIG" => (*b"DSIG", dsig(element)?),
        "FFTM" => (*b"FFTM", fftm(element)?),
        "VDMX" => (*b"VDMX", vdmx(element)?),
        "mort" => (*b"mort", morph::compile(element, ctx, true)?),
        "morx" => (*b"morx", morph::compile(element, ctx, false)?),
        _ => return Ok(None),
    }))
}

/// `str2fl(text, 16)` as the 16.16 number it packs to.
fn fixed(text: &str) -> Result<i64> {
    ot_round(float_lit(text)? * 65536.0)
}

/// One direction of `trak`: track value to (name index, size to value), all 16.16 numbers.
type TrackData = BTreeMap<i64, (i64, BTreeMap<i64, i64>)>;

fn track_data(e: &Element) -> Result<TrackData> {
    let mut data = TrackData::new();
    for entry in e.children.iter().filter(|c| c.name == "trackEntry") {
        if data.len() >= MAX_LAYOUT_ITEMS {
            return Err(TablesError::TooLarge { what: "the tracks of 'trak'", limit: MAX_LAYOUT_ITEMS });
        }
        let track = fixed(entry.attribute("value").ok_or_else(|| bad("a track entry without a value"))?)?;
        let name = int_lit(entry.attribute("nameIndex").ok_or_else(|| bad("a track entry without a name index"))?)?;
        let mut sizes = BTreeMap::new();
        for t in entry.children.iter().filter(|c| c.name == "track") {
            if sizes.len() >= MAX_LAYOUT_ITEMS {
                return Err(TablesError::TooLarge { what: "the sizes of a 'trak' entry", limit: MAX_LAYOUT_ITEMS });
            }
            let size = fixed(t.attribute("size").ok_or_else(|| bad("a track value without a size"))?)?;
            sizes.insert(size, int_lit(t.attribute("value").ok_or_else(|| bad("a track value without a value"))?)?);
        }
        data.insert(track, (name, sizes));
    }
    Ok(data)
}

/// `TrackData.compile` at `offset` from the start of the table.
fn compile_track_data(data: &TrackData, offset: usize) -> Result<Vec<u8>> {
    let mut sizes: Option<Vec<i64>> = None;
    for (_, entry_sizes) in data.values() {
        let these: Vec<i64> = entry_sizes.keys().copied().collect();
        match &sizes {
            None => sizes = Some(these),
            Some(s) if *s == these => {}
            Some(_) => return Err(bad("'trak' table entries must give the same sizes")),
        }
    }
    let sizes = sizes.unwrap_or_default();
    let mut at = offset + 8 + 8 * data.len();
    let mut o = Out::default();
    let count = |n: usize| i64::try_from(n).unwrap_or(i64::MAX);
    o.u16(count(data.len()), "the number of tracks")?;
    o.u16(count(sizes.len()), "the number of track sizes")?;
    o.u32(count(at), "the size table offset")?;
    at += 4 * sizes.len();
    let mut per_size = Out::default();
    for (track, (name, values)) in data {
        o.i32(*track, "a track value")?;
        o.u16(*name, "a track name index")?;
        o.u16(count(at), "a track value offset")?;
        for v in values.values() {
            per_size.i16(*v, "a per-size track value")?;
        }
        at += 2 * sizes.len();
    }
    for s in &sizes {
        o.i32(*s, "a track size")?;
    }
    o.bytes(&per_size.0);
    Ok(o.0)
}

fn trak(e: &Element) -> Result<Vec<u8>> {
    let (mut version, mut format) = (None, None);
    let mut directions: [TrackData; 2] = [TrackData::new(), TrackData::new()];
    for child in &e.children {
        match child.name.as_str() {
            "version" => version = Some(fixed(child.attribute("value").ok_or_else(|| bad("the 'trak' version has no value"))?)?),
            "format" => format = Some(int_lit(child.attribute("value").ok_or_else(|| bad("the 'trak' format has no value"))?)?),
            "horizData" | "vertData" => {
                if let Some(slot) = directions.get_mut(usize::from(child.name == "vertData")) {
                    *slot = track_data(child)?;
                }
            }
            _ => {}
        }
    }
    let mut offset = 12usize;
    let mut offsets = [0usize; 2];
    let mut body = Vec::new();
    for (slot, data) in offsets.iter_mut().zip(&directions) {
        if data.is_empty() {
            continue;
        }
        let aligned = (offset + 3) & !3;
        body.resize(body.len() + (aligned - offset), 0);
        offset = aligned;
        *slot = offset;
        let bytes = compile_track_data(data, offset)?;
        offset += bytes.len();
        body.extend(bytes);
    }
    let mut o = Out::default();
    o.i32(version.ok_or_else(|| bad("'trak' without a version"))?, "the 'trak' version")?;
    o.u16(format.ok_or_else(|| bad("'trak' without a format"))?, "the 'trak' format")?;
    for v in offsets {
        o.u16(i64::try_from(v).unwrap_or(i64::MAX), "a 'trak' offset")?;
    }
    o.u16(0, "reserved")?;
    o.bytes(&body);
    Ok(o.0)
}

fn ltag(e: &Element) -> Result<Vec<u8>> {
    let (mut version, mut flags) = (None, None);
    let mut tags: Vec<&str> = Vec::new();
    for child in &e.children {
        match (child.name.as_str(), child.attribute("value")) {
            ("LanguageTag", _) => {
                if tags.len() >= MAX_LAYOUT_ITEMS {
                    return Err(TablesError::TooLarge { what: "the tags of 'ltag'", limit: MAX_LAYOUT_ITEMS });
                }
                let tag = child.attribute("tag").ok_or_else(|| bad("a language tag without a tag"))?;
                if !tag.is_ascii() {
                    return Err(bad(format!("the language tag '{tag}' is not ASCII")));
                }
                tags.push(tag);
            }
            ("version", Some(v)) => version = Some(int_lit(v)?),
            ("flags", Some(v)) => flags = Some(int_lit(v)?),
            _ => {}
        }
    }
    let mut o = Out::default();
    o.u32(version.ok_or_else(|| bad("'ltag' without a version"))?, "the 'ltag' version")?;
    o.u32(flags.ok_or_else(|| bad("'ltag' without flags"))?, "the 'ltag' flags")?;
    o.u32(i64::try_from(tags.len()).unwrap_or(i64::MAX), "the number of language tags")?;
    // Each tag points at its first occurrence in the pool, as a substring.
    let mut pool = String::new();
    let base = 12 + 4 * tags.len();
    for tag in &tags {
        let at = match pool.find(tag) {
            Some(i) => i,
            None => {
                let i = pool.len();
                pool.push_str(tag);
                i
            }
        };
        o.u16(i64::try_from(at + base).unwrap_or(i64::MAX), "a language tag offset")?;
        o.u16(i64::try_from(tag.len()).unwrap_or(i64::MAX), "a language tag length")?;
    }
    o.bytes(pool.as_bytes());
    Ok(o.0)
}

// ── DSIG ───────────────────────────────────────────────────────────────────────────────────────

/// Python's `base64.b64decode` without validation: characters outside the alphabet are dropped,
/// and the padding must be right.
fn base64(text: &str) -> Result<Vec<u8>> {
    let value = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let kept: Vec<u8> = text.bytes().filter(|c| value(*c).is_some() || *c == b'=').collect();
    if !kept.len().is_multiple_of(4) {
        return Err(bad("a signature whose base64 text has incorrect padding"));
    }
    let mut out = Vec::with_capacity(kept.len() / 4 * 3);
    for (i, quad) in kept.chunks(4).enumerate() {
        let pad = quad.iter().rev().take_while(|c| **c == b'=').count();
        let last = i + 1 == kept.len() / 4;
        if pad > 2 || (pad > 0 && !last) || quad.iter().take(4 - pad).any(|c| *c == b'=') {
            return Err(bad("a signature whose base64 text has incorrect padding"));
        }
        let n = quad.iter().take(4 - pad).fold(0u32, |acc, c| (acc << 6) | u32::from(value(*c).unwrap_or(0))) << (6 * pad as u32);
        let bytes = n.to_be_bytes();
        out.extend(bytes.iter().skip(1).take(3 - pad));
    }
    Ok(out)
}

fn dsig(e: &Element) -> Result<Vec<u8>> {
    let mut header: Option<(i64, i64, i64)> = None;
    let mut records: Vec<(i64, i64, i64, Vec<u8>)> = Vec::new();
    for child in &e.children {
        match child.name.as_str() {
            "tableHeader" => {
                let get = |n: &str| int_lit(child.attribute(n).ok_or_else(|| bad(format!("the 'DSIG' header has no '{n}'")))?);
                header = Some((get("version")?, get("numSigs")?, get("flag")?));
                records.clear();
            }
            "SignatureRecord" => {
                if records.len() >= MAX_LAYOUT_ITEMS {
                    return Err(TablesError::TooLarge { what: "the signatures of 'DSIG'", limit: MAX_LAYOUT_ITEMS });
                }
                let format = int_lit(child.attribute("format").ok_or_else(|| bad("a signature without a format"))?)?;
                let r1 = int_lit(child.attribute("reserved1").unwrap_or("0"))?;
                let r2 = int_lit(child.attribute("reserved2").unwrap_or("0"))?;
                let text: String = child.text.lines().filter(|l| !matches!(l.trim(), "-----BEGIN PKCS7-----" | "-----END PKCS7-----" | "")).collect();
                records.push((format, r1, r2, base64(&text)?));
            }
            _ => {}
        }
    }
    let (version, count, flag) = header.ok_or_else(|| bad("'DSIG' without its header"))?;
    let mut o = Out::default();
    o.u32(version, "the 'DSIG' version")?;
    o.u16(count, "the number of signatures")?;
    o.u16(flag, "the 'DSIG' flags")?;
    // The header's count, not the records read, places the first signature block.
    let mut offset = 8i64.saturating_add(count.saturating_mul(12));
    let mut data = Out::default();
    for (format, r1, r2, pkcs7) in &records {
        let len = i64::try_from(pkcs7.len()).unwrap_or(i64::MAX);
        let mut block = Out::default();
        block.u16(*r1, "a reserved field")?;
        block.u16(*r2, "a reserved field")?;
        block.i32(len, "a signature length")?;
        block.bytes(pkcs7);
        let length = i64::try_from(block.0.len()).unwrap_or(i64::MAX);
        o.u32(*format, "a signature format")?;
        o.u32(length, "a signature block length")?;
        o.u32(offset, "a signature block offset")?;
        offset = offset.saturating_add(length);
        data.bytes(&block.0);
    }
    if offset % 2 != 0 {
        data.u8(0, "a pad byte")?;
    }
    o.bytes(&data.0);
    Ok(o.0)
}

// ── FFTM ───────────────────────────────────────────────────────────────────────────────────────

fn fftm(e: &Element) -> Result<Vec<u8>> {
    let mut values: [Option<u64>; 4] = [None; 4];
    for child in &e.children {
        let Some(value) = child.attribute("value") else { continue };
        let (slot, v) = match child.name.as_str() {
            "version" => (0, u64::try_from(int_lit(value)?).map_err(|_| bad("a negative 'FFTM' version"))?),
            "FFTimeStamp" => (1, timestamp_from_string(value)?),
            "sourceCreated" => (2, timestamp_from_string(value)?),
            "sourceModified" => (3, timestamp_from_string(value)?),
            _ => continue,
        };
        if let Some(s) = values.get_mut(slot) {
            *s = Some(v);
        }
    }
    let [Some(version), Some(stamp), Some(created), Some(modified)] = values else {
        return Err(bad("'FFTM' lacks a field"));
    };
    let mut o = Out::default();
    o.u32(i64::try_from(version).unwrap_or(i64::MAX), "the 'FFTM' version")?;
    for v in [stamp, created, modified] {
        o.u64(v);
    }
    Ok(o.0)
}

// ── LTSH ───────────────────────────────────────────────────────────────────────────────────────

/// `LTSH`: one byte per name given, at its glyph's index; a glyph past the names given is an
/// error, as in fontTools.
fn ltsh(e: &Element, ctx: &ReadContext) -> Result<Vec<u8>> {
    let mut by_name: Vec<(&str, i64)> = Vec::new();
    let mut index: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for y in e.children.iter().filter(|c| c.name == "yPel") {
        if by_name.len() >= crate::limits::MAX_GLYPHS {
            return Err(TablesError::TooLarge { what: "the glyphs of 'LTSH'", limit: crate::limits::MAX_GLYPHS });
        }
        let name = y.attribute("name").ok_or_else(|| bad("an 'LTSH' entry without a glyph"))?;
        let v = int_lit(y.attribute("value").ok_or_else(|| bad("an 'LTSH' entry without a value"))?)?;
        match index.get(name).and_then(|&i| by_name.get_mut(i)) {
            Some(slot) => slot.1 = v,
            None => {
                index.insert(name, by_name.len());
                by_name.push((name, v));
            }
        }
    }
    let mut pels = vec![0i64; by_name.len()];
    for (name, v) in &by_name {
        let gid = usize::try_from(ctx.glyph_id(name)?).unwrap_or(usize::MAX);
        *pels.get_mut(gid).ok_or_else(|| bad(format!("the 'LTSH' glyph '{name}' is past the glyphs it lists")))? = *v;
    }
    let mut o = Out::default();
    o.u16(0, "the 'LTSH' version")?;
    o.u16(i64::try_from(pels.len()).unwrap_or(i64::MAX), "the number of glyphs")?;
    for v in pels {
        o.u8(v, "a yPel")?;
    }
    Ok(o.0)
}

// ── VDMX ───────────────────────────────────────────────────────────────────────────────────────

fn vdmx(e: &Element) -> Result<Vec<u8>> {
    let mut version = None;
    let mut ratios: Vec<[i64; 5]> = Vec::new();
    let mut groups: Vec<BTreeMap<i64, (i64, i64)>> = Vec::new();
    for child in &e.children {
        match child.name.as_str() {
            "version" => version = Some(int_lit(child.attribute("value").ok_or_else(|| bad("the 'VDMX' version has no value"))?)?),
            "ratRanges" => {
                for r in child.children.iter().filter(|r| r.name == "ratRange") {
                    if ratios.len() >= MAX_LAYOUT_ITEMS {
                        return Err(TablesError::TooLarge { what: "the ratios of 'VDMX'", limit: MAX_LAYOUT_ITEMS });
                    }
                    let get = |n: &str| int_lit(r.attribute(n).ok_or_else(|| bad(format!("a 'VDMX' ratio without '{n}'")))?);
                    ratios.push([get("bCharSet")?, get("xRatio")?, get("yStartRatio")?, get("yEndRatio")?, get("groupIndex")?]);
                }
            }
            "groups" => {
                for g in child.children.iter().filter(|g| g.name == "group") {
                    let mut group = BTreeMap::new();
                    for rec in g.children.iter().filter(|r| r.name == "record") {
                        if group.len() >= MAX_LAYOUT_ITEMS || groups.len() >= MAX_LAYOUT_ITEMS {
                            return Err(TablesError::TooLarge { what: "the records of 'VDMX'", limit: MAX_LAYOUT_ITEMS });
                        }
                        let get = |n: &str| int_lit(rec.attribute(n).ok_or_else(|| bad(format!("a 'VDMX' record without '{n}'")))?);
                        group.insert(get("yPelHeight")?, (get("yMax")?, get("yMin")?));
                    }
                    groups.push(group);
                }
            }
            _ => {}
        }
    }
    let version = version.ok_or_else(|| bad("'VDMX' without a version"))?;
    if !(0..=1).contains(&version) {
        return Err(bad(format!("unknown format for VDMX table: version {version}")));
    }
    if ratios.is_empty() || groups.is_empty() {
        // fontTools has no count to pack without them.
        return Err(bad("'VDMX' without ratios or groups"));
    }
    let count = |n: usize| i64::try_from(n).unwrap_or(i64::MAX);
    let mut o = Out::default();
    o.u16(version, "the 'VDMX' version")?;
    o.u16(count(groups.len()), "the number of groups")?;
    o.u16(count(ratios.len()), "the number of ratios")?;
    for r in &ratios {
        for (i, v) in r.iter().take(4).enumerate() {
            o.u8(*v, ["bCharSet", "xRatio", "yStartRatio", "yEndRatio"].get(i).copied().unwrap_or("a ratio"))?;
        }
    }
    let mut at = 6 + 4 * ratios.len() + 2 * ratios.len();
    let mut starts = Vec::with_capacity(groups.len());
    for g in &groups {
        starts.push(at);
        at += 4 + 6 * g.len();
    }
    for r in &ratios {
        let [_, _, _, _, group] = *r;
        let start = usize::try_from(group).ok().and_then(|i| starts.get(i)).ok_or_else(|| bad("a 'VDMX' ratio names a group that does not exist"))?;
        o.u16(count(*start), "a group offset")?;
    }
    for g in &groups {
        let (Some(first), Some(last)) = (g.keys().next(), g.keys().next_back()) else {
            return Err(bad("an empty 'VDMX' group"));
        };
        o.u16(count(g.len()), "the number of records")?;
        o.u8(*first, "startsz")?;
        o.u8(*last, "endsz")?;
        for (height, (max, min)) in g {
            o.u16(*height, "yPelHeight")?;
            o.i16(*max, "yMax")?;
            o.i16(*min, "yMin")?;
        }
    }
    Ok(o.0)
}
