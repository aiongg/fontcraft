//! The united `cmap`, after fontTools' `merge/cmap.py` (MIT License): one Unicode subtable per
//! font (format 12 when it has one, else format 4) and its variation sequences; a code point
//! keeps the first font's glyph, and each later glyph for it is recorded for the `locl` lookup.

use std::collections::BTreeMap;

use read_fonts::tables::cmap::{Cmap, CmapSubtable, MapVariant};
use read_fonts::{FontData, FontRead};

use super::plan::Plan;
use crate::limits::MAX_CODE_POINTS;
use crate::reorder::be::{put_u16, put_u32, to_u32};
use crate::reorder::cmap::{write_format4, write_format12};
use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

#[derive(Clone, Debug, Default)]
pub(super) struct United {
    /// Code point → new glyph id.
    pub map: BTreeMap<u32, u16>,
    /// Per font: the glyph an earlier font maps a code point to → this font's glyph for it.
    pub duplicates: Vec<BTreeMap<u16, u16>>,
    /// (font, how many of its code points an earlier font keeps).
    pub kept_by_earlier: Vec<(usize, usize)>,
    /// Variation selector → code point → glyph (`None`: the default glyph).
    pub uvs: BTreeMap<u32, BTreeMap<u32, Option<u16>>>,
}

/// fontTools' list of Unicode subtables merge reads: (format, platform, encoding).
const BMP: [(u16, u16, u16); 4] = [(4, 3, 1), (4, 0, 3), (4, 0, 4), (4, 0, 6)];
const FULL: [(u16, u16, u16); 3] = [(12, 3, 10), (12, 0, 4), (12, 0, 6)];
const UVS: (u16, u16, u16) = (14, 0, 5);

fn format_of(sub: &CmapSubtable<'_>) -> u16 {
    match sub {
        CmapSubtable::Format0(_) => 0,
        CmapSubtable::Format2(_) => 2,
        CmapSubtable::Format4(_) => 4,
        CmapSubtable::Format6(_) => 6,
        CmapSubtable::Format8(_) => 8,
        CmapSubtable::Format10(_) => 10,
        CmapSubtable::Format12(_) => 12,
        CmapSubtable::Format13(_) => 13,
        CmapSubtable::Format14(_) => 14,
    }
}

/// Unicode's Default_Ignorable_Code_Point ranges (Unicode 14.0), as fontTools lists them.
fn default_ignorable(u: u32) -> bool {
    matches!(
        u,
        0x00AD
            | 0x034F
            | 0x061C
            | 0x115F..=0x1160
            | 0x17B4..=0x17B5
            | 0x180B..=0x180F
            | 0x200B..=0x200F
            | 0x202A..=0x202E
            | 0x2060..=0x206F
            | 0x3164
            | 0xFE00..=0xFE0F
            | 0xFEFF
            | 0xFFA0
            | 0xFFF0..=0xFFF8
            | 0x1BCA0..=0x1BCA3
            | 0x1D173..=0x1D17A
            | 0xE0000..=0xE0FFF
    )
}

/// The united mapping of the fonts `which` (indices into `fonts`, in order).
pub(super) fn unite(fonts: &[SfntFont], plan: &Plan, which: &[usize], warnings: &mut Vec<String>) -> Result<United> {
    let mut united = United { duplicates: vec![BTreeMap::new(); fonts.len()], ..United::default() };
    let mut uvs_lists: Vec<(usize, Vec<(u32, u32, Option<u32>)>)> = Vec::new();
    for &font_index in which {
        let Some(bytes) = fonts.get(font_index).and_then(|f| f.get(*b"cmap")) else { continue };
        let cmap = Cmap::read(FontData::new(bytes)).map_err(|e| TablesError::invalid(format!("the 'cmap' table cannot be read: {e}")))?;
        let mut bmp = None;
        let mut full = None;
        let mut dropped: Vec<String> = Vec::new();
        for record in cmap.encoding_records() {
            let Ok(sub) = record.subtable(cmap.offset_data()) else {
                return Err(TablesError::invalid(format!("a 'cmap' subtable of font {} cannot be read", font_index + 1)));
            };
            let key = (format_of(&sub), record.platform_id() as u16, record.encoding_id());
            if BMP.contains(&key) {
                bmp = Some(sub);
            } else if FULL.contains(&key) {
                full = Some(sub);
            } else if key == UVS {
                if let CmapSubtable::Format14(t) = &sub {
                    let list = t
                        .iter()
                        .take(MAX_CODE_POINTS)
                        .map(|(cp, selector, v)| match v {
                            MapVariant::UseDefault => (cp, selector, None),
                            MapVariant::Variant(g) => (cp, selector, Some(g.to_u32())),
                        })
                        .collect();
                    uvs_lists.push((font_index, list));
                }
            } else {
                dropped.push(format!("format {} ({}, {})", key.0, key.1, key.2));
            }
        }
        if !dropped.is_empty() {
            warnings.push(format!("'cmap' subtables of font {} are left out: {}", font_index + 1, dropped.join(", ")));
        }
        let Some(sub) = full.or(bmp) else { continue };
        let mut kept = 0usize;
        let dups = united.duplicates.get_mut(font_index).ok_or_else(|| TablesError::invalid("font index out of range"))?;
        for (cp, gid) in sub.iter().take(MAX_CODE_POINTS) {
            let Some(new) = u16::try_from(gid.to_u32()).ok().and_then(|g| plan.new_id(font_index, g)) else { continue };
            match united.map.get(&cp).copied() {
                None => {
                    united.map.insert(cp, new);
                }
                Some(_) if default_ignorable(cp) || cp == 0x25CC => {}
                Some(old) if old != new => match dups.get(&old) {
                    None => {
                        dups.insert(old, new);
                        kept += 1;
                    }
                    Some(other) if *other != new => {
                        warnings.push(format!("U+{cp:04X} of font {} is left out: its earlier glyph already has another substitute", font_index + 1));
                    }
                    Some(_) => kept += 1,
                },
                Some(_) => {}
            }
        }
        if kept > 0 {
            united.kept_by_earlier.push((font_index, kept));
        }
    }
    for (font_index, list) in uvs_lists {
        for (cp, selector, glyph) in list {
            let glyph = match glyph {
                None => None,
                Some(g) => match u16::try_from(g).ok().and_then(|g| plan.new_id(font_index, g)) {
                    Some(new) if united.map.get(&cp) == Some(&new) => None,
                    Some(new) => Some(new),
                    None => continue,
                },
            };
            united.uvs.entry(selector).or_default().entry(cp).or_insert(glyph);
        }
    }
    Ok(united)
}

/// The `cmap` table: format 14 (0, 5) for variation sequences, format 4 (3, 1) for the Basic
/// Multilingual Plane, and format 12 (3, 10) when a code point lies beyond it.
pub(super) fn write(united: &United) -> Result<Vec<u8>> {
    let all: Vec<(u32, u16)> = united.map.iter().map(|(c, g)| (*c, *g)).collect();
    let bmp: Vec<(u32, u16)> = all.iter().copied().filter(|(c, _)| *c <= 0xFFFF).collect();
    let mut subtables: Vec<(u16, u16, Vec<u8>)> = Vec::new();
    if !united.uvs.is_empty() {
        subtables.push((0, 5, write_format14(&united.uvs)?));
    }
    subtables.push((3, 1, write_format4(0, &bmp)?));
    if bmp.len() != all.len() {
        subtables.push((3, 10, write_format12(0, &all)?));
    }
    let mut out = Vec::new();
    put_u16(&mut out, 0);
    put_u16(&mut out, subtables.len() as u16);
    let mut offset = 4 + 8 * subtables.len();
    for (platform, encoding, data) in &subtables {
        put_u16(&mut out, *platform);
        put_u16(&mut out, *encoding);
        put_u32(&mut out, to_u32(offset, "the cmap table")?);
        offset += data.len();
    }
    for (_, _, data) in subtables {
        out.extend_from_slice(&data);
    }
    Ok(out)
}

fn put_u24(out: &mut Vec<u8>, v: u32) {
    let [_, a, b, c] = v.to_be_bytes();
    out.extend_from_slice(&[a, b, c]);
}

fn write_format14(uvs: &BTreeMap<u32, BTreeMap<u32, Option<u16>>>) -> Result<Vec<u8>> {
    let mut records: Vec<(u32, Vec<u8>, Vec<u8>)> = Vec::new();
    for (selector, map) in uvs {
        let mut ranges: Vec<(u32, u8)> = Vec::new();
        let mut mappings: Vec<(u32, u16)> = Vec::new();
        for (cp, glyph) in map {
            match glyph {
                None => match ranges.last_mut() {
                    Some((start, extra)) if *extra < u8::MAX && *start + u32::from(*extra) + 1 == *cp => *extra += 1,
                    _ => ranges.push((*cp, 0)),
                },
                Some(g) => mappings.push((*cp, *g)),
            }
        }
        let mut default = Vec::new();
        if !ranges.is_empty() {
            put_u32(&mut default, to_u32(ranges.len(), "a default UVS table")?);
            for (start, extra) in ranges {
                put_u24(&mut default, start);
                default.push(extra);
            }
        }
        let mut non_default = Vec::new();
        if !mappings.is_empty() {
            put_u32(&mut non_default, to_u32(mappings.len(), "a non-default UVS table")?);
            for (cp, g) in mappings {
                put_u24(&mut non_default, cp);
                put_u16(&mut non_default, g);
            }
        }
        records.push((*selector, default, non_default));
    }
    let header = 10 + 11 * records.len();
    let mut body: Vec<u8> = Vec::new();
    let mut table = Vec::new();
    for (selector, default, non_default) in &records {
        put_u24(&mut table, *selector);
        for part in [default, non_default] {
            if part.is_empty() {
                put_u32(&mut table, 0);
            } else {
                put_u32(&mut table, to_u32(header + body.len(), "the cmap format 14 subtable")?);
                body.extend_from_slice(part);
            }
        }
    }
    let mut out = Vec::with_capacity(header + body.len());
    put_u16(&mut out, 14);
    put_u32(&mut out, to_u32(header + body.len(), "the cmap format 14 subtable")?);
    put_u32(&mut out, to_u32(records.len(), "the cmap format 14 subtable")?);
    out.extend_from_slice(&table);
    out.extend_from_slice(&body);
    Ok(out)
}
