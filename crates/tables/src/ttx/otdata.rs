//! TTX writer: GSUB, GPOS, GDEF, BASE, JSTF, MATH, STAT, fvar, avar and meta (M12.6).
//!
//! The OpenType layout tables and their relatives go through one generic writer
//! ([`engine::Walker`]) driven by their layouts ([`schema`], generated from fontTools'
//! `otData`): every field is read in order and written as `ttx` writes its kind, under its
//! fontTools name. fvar, avar and meta have writers of their own, as in fontTools. Ported from
//! fontTools' `ttLib.tables` (MIT): `otBase`, `otConverters`, `otTables`, `otData`,
//! `_f_v_a_r`, `_a_v_a_r` and `_m_e_t_a`.
//!
//! Other packages reach the generic writer through [`write_layout_table`]: a table whose
//! layout is in [`schema`] (HVAR, VVAR and MVAR are) is written by naming its tag.
//!
//! Known differences:
//! - `ttx` puts two spaces between a name ID or STAT flags element and the comment after it;
//!   here the comment follows the element directly. Comments carry no data.
//! - Name comments decode UTF-16 records and ASCII Macintosh records only; a name found only
//!   in another legacy encoding is reported as missing from the name table.
//! - A coverage or class definition table whose ranges list more than
//!   [`crate::limits::MAX_LAYOUT_ITEMS`] glyphs, and tables nested so deep that their elements
//!   would pass [`crate::limits::MAX_TTX_DEPTH`] (only possible through extension lookups
//!   pointing at extension lookups), are refused, so the table is written as hex. `ttx` writes
//!   the first in full and fails on the second.

mod engine;
mod schema;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::rc::Rc;

use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

use super::{XmlWriter, fixed_to_str};
use engine::{Reader, Walker};

/// Writes table `tag` (bytes `data`, from `font`) into `w`. Returns false, writing nothing,
/// for a tag this module does not handle.
pub fn write_table(tag: Tag, data: &[u8], font: &SfntFont, w: &mut XmlWriter) -> Result<bool> {
    match &tag {
        b"GSUB" | b"GPOS" | b"GDEF" | b"BASE" | b"JSTF" | b"MATH" | b"STAT" => write_layout_table(tag, data, font, w)?,
        b"fvar" => fvar(data, font, w)?,
        b"avar" => avar(data, font, w)?,
        b"meta" => meta(data, w)?,
        _ => return Ok(false),
    }
    Ok(true)
}

/// Writes a table whose layout is in the generated schema under the table's tag (`GSUB`,
/// `HVAR`…), as `ttx` writes it. An error for a tag without a layout.
pub(crate) fn write_layout_table(tag: Tag, data: &[u8], font: &SfntFont, w: &mut XmlWriter) -> Result<()> {
    let order = glyph_order(font, w);
    let walker = Walker::new(data, tag, order, font.get(*b"name"));
    walker.write_top(crate::sfnt::tag_str(tag).trim_end(), w)
}

/// The glyph names. A font whose glyph order cannot be read still has its tables written, with
/// every glyph named `glyphNNNNN`.
fn glyph_order(font: &SfntFont, w: &mut XmlWriter) -> Rc<Vec<String>> {
    w.glyph_order(font).unwrap_or_else(|_| Rc::new(Vec::new()))
}

// ── fvar ───────────────────────────────────────────────────────────────────────────────────────

struct Axis {
    tag: String,
    min: f64,
    default: f64,
    max: f64,
    flags: u16,
    name_id: u16,
}

struct Instance {
    subfamily_name_id: u16,
    flags: u16,
    /// Axis tag to value; a tag that repeats keeps the last value, as fontTools' dictionary does.
    coordinates: BTreeMap<String, f64>,
    postscript_name_id: u16,
}

struct Fvar {
    axes: Vec<Axis>,
    instances: Vec<Instance>,
}

fn be_u16(data: &[u8], pos: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*data.get(pos)?, *data.get(pos.checked_add(1)?)?]))
}

fn be_u32(data: &[u8], pos: usize) -> Option<u32> {
    let b = data.get(pos..pos.checked_add(4)?)?;
    Some(b.iter().fold(0u32, |v, x| (v << 8) | u32::from(*x)))
}

fn fixed(data: &[u8], pos: usize) -> Option<f64> {
    Some(f64::from(be_u32(data, pos)? as i32) / 65536.0)
}

fn read_fvar(data: &[u8]) -> Result<Fvar> {
    let short = || TablesError::invalid("the 'fvar' table is cut short");
    let version = be_u32(data, 0).ok_or_else(short)?;
    if version != 0x0001_0000 {
        return Err(TablesError::invalid(format!("unsupported 'fvar' version {version:04x}")));
    }
    let header = |pos| be_u16(data, pos).ok_or_else(short);
    let (offset, axis_count, axis_size, instance_count, instance_size) = (header(4)?, header(8)?, header(10)?, header(12)?, header(14)?);
    let mut pos = usize::from(offset);
    let mut axes = Vec::with_capacity(usize::from(axis_count));
    for _ in 0..axis_count {
        // fontTools unpacks the first 20 bytes of each record of `axisSize` bytes.
        let record = data.get(pos..data.len().min(pos.saturating_add(usize::from(axis_size)))).unwrap_or_default();
        let record = record.get(..20).ok_or_else(short)?;
        axes.push(Axis {
            tag: record.get(..4).unwrap_or_default().iter().map(|b| char::from(*b)).collect(),
            min: fixed(record, 4).ok_or_else(short)?,
            default: fixed(record, 8).ok_or_else(short)?,
            max: fixed(record, 12).ok_or_else(short)?,
            flags: be_u16(record, 16).ok_or_else(short)?,
            name_id: be_u16(record, 18).ok_or_else(short)?,
        });
        pos = pos.saturating_add(usize::from(axis_size));
    }
    let mut instances = Vec::with_capacity(usize::from(instance_count));
    for _ in 0..instance_count {
        let record = data.get(pos.min(data.len())..data.len().min(pos.saturating_add(usize::from(instance_size)))).unwrap_or_default();
        let subfamily_name_id = be_u16(record, 0).ok_or_else(short)?;
        let flags = be_u16(record, 2).ok_or_else(short)?;
        let mut coordinates = BTreeMap::new();
        let mut at = 4usize;
        for axis in &axes {
            coordinates.insert(axis.tag.clone(), fixed(record, at).ok_or_else(short)?);
            at = at.saturating_add(4);
        }
        let postscript_name_id = be_u16(record, at).unwrap_or(0xFFFF);
        instances.push(Instance { subfamily_name_id, flags, coordinates, postscript_name_id });
        pos = pos.saturating_add(usize::from(instance_size));
    }
    Ok(Fvar { axes, instances })
}

/// `<Tag>text</Tag>` on one line, as fontTools writes the fields of an fvar axis.
fn inline(w: &mut XmlWriter, name: &str, text: &str) {
    let mut line = String::with_capacity(name.len() * 2 + text.len() + 5);
    line.push('<');
    line.push_str(name);
    line.push('>');
    for c in text.chars() {
        match c {
            '&' => line.push_str("&amp;"),
            '<' => line.push_str("&lt;"),
            '>' => line.push_str("&gt;"),
            _ => line.push(c),
        }
    }
    line.push_str("</");
    line.push_str(name);
    line.push('>');
    w.text_raw(&line, text);
}

fn fvar(data: &[u8], font: &SfntFont, w: &mut XmlWriter) -> Result<()> {
    let table = read_fvar(data)?;
    let names = font.get(*b"name");
    let debug = |id: u16| names.and_then(|n| debug_name(n, i64::from(id)));
    for axis in &table.axes {
        if let Some(name) = debug(axis.name_id) {
            w.blank();
            w.comment(&name);
        }
        w.open("Axis", &[]);
        inline(w, "AxisTag", &axis.tag);
        inline(w, "Flags", &format!("0x{:X}", axis.flags));
        inline(w, "MinValue", &fixed_to_str(axis.min, 16));
        inline(w, "DefaultValue", &fixed_to_str(axis.default, 16));
        inline(w, "MaxValue", &fixed_to_str(axis.max, 16));
        inline(w, "AxisNameID", &axis.name_id.to_string());
        w.close("Axis");
        w.check()?;
    }
    for instance in &table.instances {
        if let Some(name) = debug(instance.subfamily_name_id) {
            w.blank();
            w.comment(&name);
        }
        if let Some(name) = debug(instance.postscript_name_id) {
            w.comment(&format!("PostScript: {name}"));
        }
        let flags = format!("0x{:X}", instance.flags);
        let subfamily = instance.subfamily_name_id.to_string();
        let postscript = instance.postscript_name_id.to_string();
        let mut attrs = vec![("flags", flags.as_str())];
        if instance.postscript_name_id != 0xFFFF {
            attrs.push(("postscriptNameID", postscript.as_str()));
        }
        attrs.push(("subfamilyNameID", subfamily.as_str()));
        w.open("NamedInstance", &attrs);
        for axis in &table.axes {
            let value = instance.coordinates.get(&axis.tag).copied().unwrap_or(0.0);
            w.simple("coord", &[("axis", &axis.tag), ("value", &fixed_to_str(value, 16))]);
        }
        w.close("NamedInstance");
        w.check()?;
    }
    Ok(())
}

// ── avar ───────────────────────────────────────────────────────────────────────────────────────

fn avar(data: &[u8], font: &SfntFont, w: &mut XmlWriter) -> Result<()> {
    let order = glyph_order(font, w);
    let walker = Walker::new(data, *b"avar", order, font.get(*b"name"));
    let mut r = Reader::default();
    let version = walker.u32(&mut r)? as i32;
    walker.u16(&mut r)?;
    let axis_count = walker.u16(&mut r)?;
    let mut maps: Vec<Vec<(i16, i16)>> = Vec::with_capacity(usize::from(axis_count));
    for _ in 0..axis_count {
        let count = walker.u16(&mut r)?;
        let mut map = Vec::with_capacity(usize::from(count));
        for _ in 0..count {
            let from = walker.u16(&mut r)? as i16;
            let to = walker.u16(&mut r)? as i16;
            map.push((from, to));
        }
        maps.push(map);
        w.check()?;
    }
    let mut v2 = None;
    if version >= 0x0002_0000 {
        let index_map = walker.u32(&mut r)?;
        let store = walker.u32(&mut r)?;
        v2 = Some((index_map, store));
    }
    let major = version >> 16;
    if major != 1 && major != 2 {
        return Err(TablesError::invalid("unknown 'avar' table version"));
    }
    let fvar_data = font.get(*b"fvar").ok_or_else(|| TablesError::invalid("the 'avar' table needs an 'fvar' table"))?;
    let axes = read_fvar(fvar_data)?.axes;
    // Segments by axis tag: axes past the maps get none, and a tag that repeats keeps the last
    // map, as fontTools' dictionary does. Each map is keyed by its from-value, the last kept.
    let mut segments: BTreeMap<&str, BTreeMap<i16, i16>> = axes.iter().map(|a| (a.tag.as_str(), BTreeMap::new())).collect();
    for (axis, map) in axes.iter().zip(&maps) {
        segments.insert(axis.tag.as_str(), map.iter().copied().collect());
    }
    w.simple("version", &[("major", &major.to_string()), ("minor", &(version & 0xFFFF).to_string())]);
    for axis in &axes {
        w.open("segment", &[("axis", &axis.tag)]);
        for (from, to) in segments.get(axis.tag.as_str()).into_iter().flatten() {
            let from = fixed_to_str(f64::from(*from) / 16384.0, 14);
            let to = fixed_to_str(f64::from(*to) / 16384.0, 14);
            w.simple("mapping", &[("from", &from), ("to", &to)]);
        }
        w.close("segment");
        w.check()?;
    }
    if let Some((index_map, store)) = v2.filter(|_| major >= 2) {
        if index_map != 0 {
            walker.write_struct("DeltaSetIndexMap", &mut r.sub(index_map), w, "VarIdxMap", None, 1)?;
        }
        if store != 0 {
            walker.write_struct("VarStore", &mut r.sub(store), w, "VarStore", None, 1)?;
        }
    }
    Ok(())
}

// ── meta ───────────────────────────────────────────────────────────────────────────────────────

fn meta(data: &[u8], w: &mut XmlWriter) -> Result<()> {
    let short = || TablesError::invalid("the 'meta' table is cut short");
    let version = be_u32(data, 0).ok_or_else(short)?;
    if version != 1 {
        return Err(TablesError::invalid(format!("unsupported 'meta' version {version}")));
    }
    let count = be_u32(data, 12).ok_or_else(short)?;
    // Each map needs 12 bytes of the table, so the count is bounded by the table's length.
    let mut maps: BTreeMap<String, &[u8]> = BTreeMap::new();
    for i in 0..count {
        let at = usize::try_from(i).ok().and_then(|i| i.checked_mul(12)).and_then(|o| o.checked_add(16)).ok_or_else(short)?;
        let record = data.get(at..at.saturating_add(12)).ok_or_else(short)?;
        let tag: String = record.get(..4).unwrap_or_default().iter().map(|b| char::from(*b)).collect();
        let offset = usize::try_from(be_u32(record, 4).unwrap_or(0)).unwrap_or(usize::MAX);
        let length = usize::try_from(be_u32(record, 8).unwrap_or(0)).unwrap_or(usize::MAX);
        let bytes = data.get(offset.min(data.len())..data.len().min(offset.saturating_add(length))).unwrap_or_default();
        if (tag == "dlng" || tag == "slng") && std::str::from_utf8(bytes).is_err() {
            return Err(TablesError::invalid(format!("the '{tag}' entry of the 'meta' table is not UTF-8")));
        }
        maps.insert(tag, bytes);
    }
    for (tag, bytes) in maps {
        if tag == "dlng" || tag == "slng" {
            w.open("text", &[("tag", &tag)]);
            w.text(std::str::from_utf8(bytes).unwrap_or_default());
            w.close("text");
        } else {
            w.open("hexdata", &[("tag", &tag)]);
            if !bytes.is_empty() && bytes.iter().all(|b| (0x20..=0x7E).contains(b)) {
                w.comment(&format!("ascii: {}", bytes.iter().map(|b| char::from(*b)).collect::<String>()));
            }
            w.hex(bytes);
            w.close("hexdata");
        }
        w.check()?;
    }
    Ok(())
}

// ── Names for comments ─────────────────────────────────────────────────────────────────────────

/// The text of name `id` as fontTools' `getDebugName` gives it: the first record for an
/// English platform (Macintosh language 0, Windows 0x409), else the last that decodes. Only
/// UTF-16 and ASCII Macintosh records are decoded here.
pub(crate) fn debug_name(data: &[u8], id: i64) -> Option<String> {
    let count = be_u16(data, 2)?;
    let storage = usize::from(be_u16(data, 4)?);
    let mut some = None;
    for i in 0..usize::from(count) {
        let at = 6usize.checked_add(i.checked_mul(12)?)?;
        let (platform, encoding, language, name_id) = (be_u16(data, at)?, be_u16(data, at + 2)?, be_u16(data, at + 4)?, be_u16(data, at + 6)?);
        if i64::from(name_id) != id {
            continue;
        }
        let (length, offset) = (usize::from(be_u16(data, at + 8)?), usize::from(be_u16(data, at + 10)?));
        let start = storage.checked_add(offset)?;
        let Some(bytes) = data.get(start..start.checked_add(length)?) else { continue };
        let text = match (platform, encoding) {
            (0, _) | (3, 0 | 1 | 10) => {
                if !bytes.len().is_multiple_of(2) {
                    continue;
                }
                let units = bytes.chunks(2).map(|p| u16::from_be_bytes([p.first().copied().unwrap_or(0), p.get(1).copied().unwrap_or(0)]));
                match char::decode_utf16(units).collect::<std::result::Result<String, _>>() {
                    Ok(t) => t,
                    Err(_) => continue,
                }
            }
            (1, 0) if bytes.is_ascii() => bytes.iter().map(|b| char::from(*b)).collect(),
            _ => continue,
        };
        let english = matches!((platform, language), (1, 0) | (3, 0x409));
        some = Some(text);
        if english {
            break;
        }
    }
    some.filter(|t| !t.is_empty())
}
