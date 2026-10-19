//! TTX reader: layout and the other structured tables (M12.9).
//!
//! GSUB, GPOS, GDEF, BASE, JSTF, MATH and STAT go through one generic compile ([`build`])
//! driven by the layouts the writer walks (`super::otdata`'s schema, from fontTools'
//! `otData`), and are assembled as fontTools assembles them ([`pack`]): equal subtables shared,
//! tables laid out in the same order, so the bytes match `ttx`'s. fvar, avar, meta and kern have
//! compilers of their own, as in fontTools. Ported from fontTools' `ttLib.tables.otBase`,
//! `otConverters`, `otTables`, `_f_v_a_r`, `_a_v_a_r`, `_m_e_t_a` and `_k_e_r_n` (MIT).
//!
//! Known differences: an offset that overflows its 16 bits is an error here, where fontTools
//! splits subtables or moves lookups into extension lookups and tries again.

pub(crate) mod build;
pub(crate) mod pack;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

use super::Element;
use super::read::{Out, ReadContext, float_lit, int_lit, ot_round, py_strip, read_hex};
use build::{el, val};

/// The table a TTX table element stands for, as bytes; `None` for an element this module does
/// not handle. `font` gives the context some tables need (glyph order, `maxp`, `head`).
pub fn read_table(element: &Element, font: &SfntFont) -> Result<Option<(Tag, Vec<u8>)>> {
    read_table_with(element, &ReadContext::from_font(font), font)
}

/// [`read_table`] with the glyph order of the text being read; `font` holds the tables read so
/// far (`fvar` among the first).
pub(crate) fn read_table_with(element: &Element, ctx: &ReadContext, font: &SfntFont) -> Result<Option<(Tag, Vec<u8>)>> {
    let tag = super::read::xml_to_tag(&element.name)?;
    let data = match &tag {
        b"GSUB" | b"GPOS" | b"GDEF" | b"BASE" | b"JSTF" | b"MATH" | b"STAT" => build::compile_layout(&element.name, element, ctx, font)?,
        b"fvar" => fvar(element)?,
        b"avar" => avar(element, ctx, font)?,
        b"meta" => meta(element)?,
        b"kern" => kern(element, ctx)?,
        _ => return Ok(None),
    };
    Ok(Some((tag, data)))
}

fn bad(what: impl Into<String>) -> TablesError {
    TablesError::invalid(what.into())
}

/// A tag packed as `struct` packs `4s`: Latin-1, cut or padded with zeros to four bytes.
pub(crate) fn tag_bytes(text: &str) -> Result<[u8; 4]> {
    let mut out = [0u8; 4];
    for (slot, c) in out.iter_mut().zip(text.chars()) {
        *slot = u8::try_from(u32::from(c)).map_err(|_| bad(format!("the tag '{text}' is not Latin-1")))?;
    }
    Ok(out)
}

/// The axis tags of an `fvar` table, in order.
pub(crate) fn fvar_axis_tags(data: &[u8]) -> Result<Vec<String>> {
    let short = || bad("the 'fvar' table is cut short");
    let u16_at = |at: usize| data.get(at..at + 2).map(|b| u16::from_be_bytes([b.first().copied().unwrap_or(0), b.get(1).copied().unwrap_or(0)]));
    let offset = usize::from(u16_at(4).ok_or_else(short)?);
    let count = usize::from(u16_at(8).ok_or_else(short)?);
    let size = usize::from(u16_at(10).ok_or_else(short)?);
    let mut tags = Vec::with_capacity(count.min(crate::limits::MAX_AXES));
    for i in 0..count {
        let at = offset.saturating_add(i.saturating_mul(size));
        let tag = data.get(at..at.saturating_add(4)).ok_or_else(short)?;
        tags.push(tag.iter().map(|b| char::from(*b)).collect());
    }
    Ok(tags)
}

// ── fvar ───────────────────────────────────────────────────────────────────────────────────────

/// A 16.16 value from text, as `str2fl` and `fl2fi` give it.
fn fixed16(text: &str) -> Result<i64> {
    ot_round(float_lit(text)? * 65536.0)
}

fn fvar(element: &Element) -> Result<Vec<u8>> {
    struct Axis {
        tag: [u8; 4],
        min: i64,
        default: i64,
        max: i64,
        flags: i64,
        name_id: i64,
    }
    struct Instance {
        subfamily: i64,
        flags: i64,
        postscript: i64,
        coords: Vec<(String, i64)>,
    }
    let mut axes: Vec<Axis> = Vec::new();
    let mut instances: Vec<Instance> = Vec::new();
    for child in &element.children {
        match child.name.as_str() {
            "Axis" => {
                if axes.len() >= crate::limits::MAX_AXES {
                    return Err(TablesError::TooLarge { what: "the axes of an 'fvar' table", limit: crate::limits::MAX_AXES });
                }
                let mut axis = Axis { tag: [0; 4], min: -65536, default: 0, max: 65536, flags: 0, name_id: 0 };
                let mut tagged = false;
                for f in &child.children {
                    let text = f.text.as_str();
                    match f.name.as_str() {
                        "AxisTag" => {
                            axis.tag = tag_bytes(text)?;
                            tagged = true;
                        }
                        "Flags" => axis.flags = int_lit(text)?,
                        "MinValue" => axis.min = fixed16(text)?,
                        "DefaultValue" => axis.default = fixed16(text)?,
                        "MaxValue" => axis.max = fixed16(text)?,
                        "AxisNameID" => axis.name_id = int_lit(text)?,
                        _ => {}
                    }
                }
                if !tagged {
                    return Err(bad("an 'fvar' axis without a tag"));
                }
                axes.push(axis);
            }
            "NamedInstance" => {
                if instances.len() >= crate::limits::MAX_NAMED_INSTANCES {
                    return Err(TablesError::TooLarge { what: "the named instances of an 'fvar' table", limit: crate::limits::MAX_NAMED_INSTANCES });
                }
                let subfamily = int_lit(child.attribute("subfamilyNameID").ok_or_else(|| bad("a named instance without 'subfamilyNameID'"))?)?;
                let flags = int_lit(child.attribute("flags").unwrap_or("0"))?;
                let postscript = match child.attribute("postscriptNameID") {
                    Some(v) => int_lit(v)?,
                    None => 0xFFFF,
                };
                let mut coords: Vec<(String, i64)> = Vec::new();
                for c in child.children.iter().filter(|c| c.name == "coord") {
                    let axis = c.attribute("axis").ok_or_else(|| bad("a coordinate without an axis"))?.to_string();
                    let value = fixed16(c.attribute("value").ok_or_else(|| bad("a coordinate without a value"))?)?;
                    coords.retain(|(a, _)| *a != axis);
                    coords.push((axis, value));
                }
                instances.push(Instance { subfamily, flags, postscript, coords });
            }
            _ => {}
        }
    }
    let with_postscript = instances.iter().any(|i| i.postscript != 0xFFFF);
    let count = |n: usize| i64::try_from(n).unwrap_or(i64::MAX);
    let instance_size = 4 + 4 * count(axes.len()) + if with_postscript { 2 } else { 0 };
    let mut o = Out::default();
    o.u32(0x0001_0000, "fvar.version")?;
    for (v, what) in [(16, "offsetToData"), (2, "countSizePairs"), (count(axes.len()), "axisCount"), (20, "axisSize")] {
        o.u16(v, what)?;
    }
    o.u16(count(instances.len()), "instanceCount")?;
    o.u16(instance_size, "instanceSize")?;
    for a in &axes {
        o.bytes(&a.tag);
        o.i32(a.min, "minValue")?;
        o.i32(a.default, "defaultValue")?;
        o.i32(a.max, "maxValue")?;
        o.u16(a.flags, "flags")?;
        o.u16(a.name_id, "axisNameID")?;
    }
    for i in &instances {
        o.u16(i.subfamily, "subfamilyNameID")?;
        o.u16(i.flags, "flags")?;
        for a in &axes {
            let tag: String = a.tag.iter().map(|b| char::from(*b)).collect();
            let v = i
                .coords
                .iter()
                .find(|(t, _)| *t == tag)
                .map(|(_, v)| *v)
                .ok_or_else(|| bad(format!("a named instance without a coordinate on '{tag}'")))?;
            o.i32(v, "coordinate")?;
        }
        if with_postscript {
            o.u16(i.postscript, "postscriptNameID")?;
        }
    }
    Ok(o.0)
}

// ── avar ───────────────────────────────────────────────────────────────────────────────────────

fn avar(element: &Element, ctx: &ReadContext, font: &SfntFont) -> Result<Vec<u8>> {
    let (mut major, mut minor) = (1i64, 0i64);
    let mut segments: BTreeMap<String, BTreeMap<i64, i64>> = BTreeMap::new();
    let mut rest: Vec<Element> = Vec::new();
    for child in &element.children {
        match child.name.as_str() {
            "version" => {
                major = int_lit(child.attribute("major").ok_or_else(|| bad("the 'avar' version has no major number"))?)?;
                minor = int_lit(child.attribute("minor").ok_or_else(|| bad("the 'avar' version has no minor number"))?)?;
            }
            "segment" => {
                let axis = child.attribute("axis").ok_or_else(|| bad("an 'avar' segment without an axis"))?;
                let mut map = BTreeMap::new();
                for m in child.children.iter().filter(|m| m.name == "mapping") {
                    let from = ot_round(float_lit(m.attribute("from").ok_or_else(|| bad("a mapping without 'from'"))?)? * 16384.0)?;
                    let to = ot_round(float_lit(m.attribute("to").ok_or_else(|| bad("a mapping without 'to'"))?)? * 16384.0)?;
                    map.insert(from, to);
                }
                segments.insert(axis.to_string(), map);
            }
            _ => rest.push(child.clone()),
        }
    }
    let fvar = font.get(*b"fvar").ok_or_else(|| bad("the 'avar' table needs an 'fvar' table"))?;
    let version = (major << 16) | minor;
    let mut children = vec![val("Version", format!("0x{version:08x}")), val("Reserved", "0")];
    for tag in fvar_axis_tags(fvar)? {
        let map = segments.get(&tag).ok_or_else(|| bad(format!("the 'avar' table has no segment for the axis '{tag}'")))?;
        let maps = map
            .iter()
            .map(|(from, to)| {
                let f2 = |v: i64| format!("{:?}", v as f64 / 16384.0);
                el("AxisValueMap", Vec::new(), vec![val("FromCoordinate", f2(*from)), val("ToCoordinate", f2(*to))])
            })
            .collect();
        children.push(el("AxisSegmentMap", Vec::new(), maps));
    }
    children.extend(rest);
    let raw = el("avar", Vec::new(), children);
    build::compile_layout("avar", &raw, ctx, font)
}

// ── meta ───────────────────────────────────────────────────────────────────────────────────────

fn meta(element: &Element) -> Result<Vec<u8>> {
    let mut data: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for child in &element.children {
        let tag = child.attribute("tag").ok_or_else(|| bad(format!("the 'meta' element '{}' has no tag", child.name)))?;
        match child.name.as_str() {
            "hexdata" => {
                data.insert(tag.to_string(), read_hex(&child.text)?);
            }
            "text" if tag == "dlng" || tag == "slng" => {
                data.insert(tag.to_string(), py_strip(&child.text).as_bytes().to_vec());
            }
            _ => return Err(bad(format!("the 'meta' table cannot hold the element '{}'", child.name))),
        }
    }
    let n = i64::try_from(data.len()).unwrap_or(i64::MAX);
    let mut offset = 16 + 12 * n;
    let mut o = Out::default();
    o.u32(1, "meta.version")?;
    o.u32(0, "meta.flags")?;
    o.u32(offset, "meta.dataOffset")?;
    o.u32(n, "meta.numDataMaps")?;
    for (tag, bytes) in &data {
        o.bytes(&tag_bytes(tag)?);
        o.u32(offset, "a data offset")?;
        let len = i64::try_from(bytes.len()).unwrap_or(i64::MAX);
        o.u32(len, "a data length")?;
        offset = offset.saturating_add(len);
    }
    for bytes in data.values() {
        o.bytes(bytes);
    }
    Ok(o.0)
}

// ── kern ───────────────────────────────────────────────────────────────────────────────────────

/// fontTools' `getSearchRange` for items of `size` bytes.
fn search_range(n: i64, size: i64) -> (i64, i64, i64) {
    let mut exponent = 0i64;
    let mut x = n;
    while x > 0 {
        x >>= 1;
        exponent += 1;
    }
    let exponent = (exponent - 1).max(0);
    let range = (1i64 << exponent) * size;
    (range, exponent, (n * size - range).max(0))
}

/// The OpenType (version 0) and Apple (version 1.0) forms; format 0 subtables from their pairs,
/// others from their hex data.
fn kern(element: &Element, ctx: &ReadContext) -> Result<Vec<u8>> {
    let mut apple = false;
    let mut subtables: Vec<Vec<u8>> = Vec::new();
    for child in &element.children {
        match child.name.as_str() {
            "version" => {
                let v = float_lit(child.attribute("value").ok_or_else(|| bad("the 'kern' version has no value"))?)?;
                apple = v == 1.0;
            }
            "kernsubtable" => {
                let format = int_lit(child.attribute("format").ok_or_else(|| bad("a 'kern' subtable without a format"))?)?;
                if format != 0 {
                    subtables.push(read_hex(&child.text)?);
                    continue;
                }
                let coverage = int_lit(child.attribute("coverage").ok_or_else(|| bad("a 'kern' subtable without a coverage"))?)?;
                let tuple = match child.attribute("tupleIndex") {
                    Some(t) if apple => int_lit(t)?,
                    _ => 0,
                };
                let mut pairs: BTreeMap<(String, String), i64> = BTreeMap::new();
                for p in &child.children {
                    let side = |n: &str| p.attribute(n).map(str::to_string).ok_or_else(|| bad(format!("a kerning pair without '{n}'")));
                    let v = int_lit(p.attribute("v").ok_or_else(|| bad("a kerning pair without a value"))?)?;
                    pairs.insert((side("l")?, side("r")?), v);
                }
                let mut ids: Vec<(u32, u32, i64)> = Vec::with_capacity(pairs.len());
                for ((l, r), v) in pairs {
                    ids.push((ctx.glyph_id(&l)?, ctx.glyph_id(&r)?, v));
                }
                ids.sort_unstable();
                let n = i64::try_from(ids.len()).unwrap_or(i64::MAX);
                let (range, selector, shift) = search_range(n.min(0xFFFF), 6);
                let mut o = Out::default();
                o.u16(n.min(0xFFFF), "nPairs")?;
                o.u16(range & 0xFFFF, "searchRange")?;
                o.u16(selector.min(0xFFFF), "entrySelector")?;
                o.u16(shift.min(0xFFFF), "rangeShift")?;
                for (l, r, v) in ids {
                    o.u16(i64::from(l), "a left glyph")?;
                    o.u16(i64::from(r), "a right glyph")?;
                    o.i16(v, "a kerning value")?;
                }
                let len = i64::try_from(o.0.len()).unwrap_or(i64::MAX);
                let mut header = Out::default();
                if apple {
                    header.u32(len + 8, "a subtable length")?;
                    header.u8(coverage, "coverage")?;
                    header.u8(0, "format")?;
                    header.u16(tuple, "tupleIndex")?;
                } else {
                    header.u16(0, "a subtable version")?;
                    // fontTools keeps the low 16 bits of a length that overflows.
                    header.u16((len + 6) & 0xFFFF, "a subtable length")?;
                    header.u8(0, "format")?;
                    header.u8(coverage, "coverage")?;
                }
                header.bytes(&o.0);
                subtables.push(header.0);
            }
            _ => {}
        }
    }
    let n = i64::try_from(subtables.len()).unwrap_or(i64::MAX);
    let mut o = Out::default();
    if apple {
        o.u32(0x0001_0000, "kern.version")?;
        o.u32(n, "nTables")?;
    } else {
        o.u16(0, "kern.version")?;
        o.u16(n, "nTables")?;
    }
    for s in subtables {
        o.bytes(&s);
    }
    Ok(o.0)
}
