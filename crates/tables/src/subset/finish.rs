//! What the subsetter does to the subset font after the tables are subset, as fontTools 4.66.1
//! does it (MIT, Copyright 2013 Google, Inc.): the `OS/2` recalculations of
//! `Subsetter._prune_post_subset` (`--recalc-average-width`, `--recalc-max-context`), the name
//! obfuscation of the `name` table's `prune_post_subset` (`--obfuscate-names`), the emptied
//! `DSIG` of its `prune_pre_subset`, and what `TTFont.save` recomputes with `recalcBBoxes`
//! (`--recalc-bounds`: `maxp.recalc`, `head.compile`, `hhea.recalc`, `vhea.recalc` and the CFF
//! `recalcFontBBox`) and `recalcTimestamp` (`--recalc-timestamp`).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::Plan;
use super::core::Glyf;
use crate::reorder::be::{Data, put_u16, set_u16, to_u16};
use crate::reorder::cff;
use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

/// Seconds from 1904-01-01 (the `head` epoch) to 1970-01-01.
const HEAD_EPOCH_OFFSET: i64 = 2_082_844_800;

/// The name IDs `--obfuscate-names` replaces or drops.
const OBFUSCATED: [u16; 8] = [1, 2, 3, 4, 6, 16, 17, 18];

/// The Top DICT operator of `FontBBox`.
const FONT_BBOX: u16 = 5;

/// Runs the recalculations the options ask for, in pyftsubset's order: the `OS/2` fields after
/// subsetting, then what saving recomputes.
pub(super) fn apply(plan: &Plan, font: &mut SfntFont) -> Result<()> {
    let o = &plan.options;
    if o.recalc_average_width {
        average_width(font)?;
    }
    if o.recalc_max_context {
        max_context(font)?;
    }
    if o.recalc_bounds {
        bounds(font)?;
    }
    if let Some(unix) = o.recalc_timestamp
        && let Some(head) = font.tables.get_mut(b"head")
    {
        let stamp = unix.saturating_add(HEAD_EPOCH_OFFSET).max(0);
        let slot = head.get_mut(28..36).ok_or_else(|| TablesError::invalid("the 'head' table is cut short"))?;
        slot.copy_from_slice(&stamp.to_be_bytes());
    }
    Ok(())
}

/// `DSIG` without its signatures, which subsetting makes invalid: version and flags kept.
pub(super) fn empty_dsig(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut out = Data::new(*b"DSIG", bytes).slice(0, 8)?.to_vec();
    set_u16(&mut out, 4, 0)?;
    Ok(out)
}

/// The glyph count `maxp` gives.
fn glyph_count(font: &SfntFont) -> Result<usize> {
    let maxp = font.get(*b"maxp").ok_or_else(|| TablesError::MissingTable("maxp".into()))?;
    Ok(usize::from(Data::new(*b"maxp", maxp).u16(4)?))
}

/// Advance and side bearing of every glyph from `hmtx` or `vmtx`; `None` without the table or
/// its header. Glyphs past the long metrics take the last advance.
fn metrics(font: &SfntFont, vertical: bool, count: usize) -> Result<Option<Vec<(i64, i64)>>> {
    let (table, header) = if vertical { (*b"vmtx", *b"vhea") } else { (*b"hmtx", *b"hhea") };
    let (Some(data), Some(head)) = (font.get(table), font.get(header)) else { return Ok(None) };
    let d = Data::new(table, data);
    let long = usize::from(Data::new(header, head).u16(34)?).min(count);
    let mut out = Vec::with_capacity(count);
    let mut last = 0i64;
    for g in 0..count {
        if g < long {
            last = i64::from(d.u16(4 * g)?);
            out.push((last, i64::from(d.i16(4 * g + 2)?)));
        } else {
            out.push((last, i64::from(d.i16(4 * long + 2 * (g - long))?)));
        }
    }
    Ok(Some(out))
}

/// `OS/2.xAvgCharWidth`: the rounded mean of the advances that are not zero, 0 without any.
fn average_width(font: &mut SfntFont) -> Result<()> {
    if font.get(*b"OS/2").is_none() {
        return Ok(());
    }
    let count = glyph_count(font)?;
    let widths: Vec<i64> = metrics(font, false, count)?.unwrap_or_default().into_iter().map(|(a, _)| a).filter(|a| *a > 0).collect();
    let average = if widths.is_empty() { 0 } else { (widths.iter().sum::<i64>() as f64 / widths.len() as f64 + 0.5).floor() as i64 };
    let value = i16::try_from(average).map_err(|_| TablesError::invalid("the average advance does not fit OS/2.xAvgCharWidth"))?;
    if let Some(os2) = font.tables.get_mut(b"OS/2") {
        set_u16(os2, 2, value as u16)?;
    }
    Ok(())
}

/// `OS/2.usMaxContext` from the layout lookups (0 without `GSUB` and `GPOS`); the field exists
/// from `OS/2` version 2 on.
fn max_context(font: &mut SfntFont) -> Result<()> {
    let Some(os2) = font.get(*b"OS/2") else { return Ok(()) };
    if Data::new(*b"OS/2", os2).u16(0)? < 2 || os2.len() < 96 {
        return Ok(());
    }
    let value = if font.get(*b"GSUB").is_none() && font.get(*b"GPOS").is_none() { 0 } else { crate::layout_install::max_context(font)? };
    if let Some(os2) = font.tables.get_mut(b"OS/2") {
        set_u16(os2, 94, value)?;
    }
    Ok(())
}

/// What saving with `recalcBBoxes` recomputes. TrueType: the `maxp` maxima, the `head` box and
/// its `flags` bit 1 from the glyph boxes; CFF: the Top DICT `FontBBox` from the charstrings and
/// the `head` box from it; CFF2: the `head` box from the charstrings. Then the `hhea` and `vhea`
/// extremes. Glyph boxes are the ones the glyphs store.
fn bounds(font: &mut SfntFont) -> Result<()> {
    let count = glyph_count(font)?;
    let union =
        |boxes: &[Option<[f64; 4]>]| boxes.iter().flatten().copied().reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]);
    let int_rect = |b: [f64; 4]| [b[0].floor() as i64, b[1].floor() as i64, b[2].ceil() as i64, b[3].ceil() as i64];
    let (boxes, from_cff): (Vec<Option<[f64; 4]>>, bool) = if font.get(*b"glyf").is_some() {
        crate::ttx::read_core::recalc_compiled_maxp(font)?;
        let Some(glyf) = Glyf::read(font, count)? else { return Ok(()) };
        let mut boxes = Vec::with_capacity(count);
        for g in 0..glyf.len() {
            let d = Data::new(*b"glyf", glyf.glyph(g));
            if d.bytes.is_empty() || d.i16(0)? == 0 {
                boxes.push(None);
            } else {
                boxes.push(Some([f64::from(d.i16(2)?), f64::from(d.i16(4)?), f64::from(d.i16(6)?), f64::from(d.i16(8)?)]));
            }
        }
        (boxes, false)
    } else if let Some(data) = font.get(*b"CFF ") {
        let boxes = crate::ttx::read_cff::glyph_bounds(data)?;
        let bbox = union(&boxes).map_or([0; 4], int_rect);
        let mut table = cff::parse(data, count)?;
        table.set_top_ints(FONT_BBOX, &bbox)?;
        let identity: Vec<u16> = (0..count).map(|g| to_u16(g, "a glyph id")).collect::<Result<_>>()?;
        let rebuilt = table.rebuild(&identity, None)?;
        font.insert(*b"CFF ", rebuilt);
        set_head_box(font, bbox)?;
        (boxes, true)
    } else if let Some(data) = font.get(*b"CFF2") {
        let boxes = crate::ttx::read_cff::glyph_bounds(data)?;
        if let Some(b) = union(&boxes) {
            set_head_box(font, int_rect(b))?;
        }
        (boxes, true)
    } else {
        return Ok(());
    };
    for vertical in [false, true] {
        let header = if vertical { *b"vhea" } else { *b"hhea" };
        if font.get(header).is_none() {
            continue;
        }
        let Some(m) = metrics(font, vertical, count)? else { continue };
        let advance_max = m.iter().map(|(a, _)| *a).max().unwrap_or(0);
        let (mut min_first, mut min_second, mut max_extent) = (i64::MAX, i64::MAX, i64::MIN);
        for (b, (advance, bearing)) in boxes.iter().zip(&m) {
            let Some(b) = b else { continue };
            let (lo, hi) = if vertical { (b[1], b[3]) } else { (b[0], b[2]) };
            let size = if from_cff { (hi.ceil() - lo.floor()) as i64 } else { (hi - lo) as i64 };
            min_first = min_first.min(*bearing);
            min_second = min_second.min(advance - bearing - size);
            max_extent = max_extent.max(bearing + size);
        }
        if max_extent == i64::MIN {
            (min_first, min_second, max_extent) = (0, 0, 0);
        }
        let Some(table) = font.tables.get_mut(&header) else { continue };
        let fits = |v: i64| {
            i16::try_from(v).map(|v| v as u16).map_err(|_| TablesError::invalid("a recalculated side bearing or extent does not fit 16 bits"))
        };
        set_u16(table, 10, u16::try_from(advance_max).map_err(|_| TablesError::invalid("the largest advance does not fit 16 bits"))?)?;
        set_u16(table, 12, fits(min_first)?)?;
        set_u16(table, 14, fits(min_second)?)?;
        set_u16(table, 16, fits(max_extent)?)?;
    }
    Ok(())
}

fn set_head_box(font: &mut SfntFont, b: [i64; 4]) -> Result<()> {
    let Some(head) = font.tables.get_mut(b"head") else { return Ok(()) };
    for (at, v) in [(36, b[0]), (38, b[1]), (40, b[2]), (42, b[3])] {
        let v = i16::try_from(v).map_err(|_| TablesError::invalid("the font bounding box does not fit 16 bits"))?;
        set_u16(head, at, v as u16)?;
    }
    Ok(())
}

/// One record of a `name` table: platform, encoding, language, name ID, and the string's bytes.
type NameRecord = ([u16; 4], Vec<u8>);

/// pyftsubset's `--obfuscate-names` on the subset `name` table. When `fvar` or `STAT` is in the
/// font and some table refers to a name about to be obfuscated (`referenced`), those names are
/// first copied to new IDs above 255 and `fvar` and `STAT` are pointed at the copies.
pub(super) fn obfuscate_names(font: &mut SfntFont, referenced: &BTreeSet<u16>) -> Result<()> {
    let Some(bytes) = font.get(*b"name") else { return Ok(()) };
    let d = Data::new(*b"name", bytes);
    let format = d.u16(0)?;
    let count = usize::from(d.u16(2)?);
    let storage = usize::from(d.u16(4)?);
    let mut records: Vec<NameRecord> = Vec::with_capacity(count);
    for i in 0..count {
        let at = 6 + 12 * i;
        let r = [d.u16(at)?, d.u16(at + 2)?, d.u16(at + 4)?, d.u16(at + 6)?];
        let text = d.slice(storage + usize::from(d.u16(at + 10)?), usize::from(d.u16(at + 8)?))?;
        records.push((r, text.to_vec()));
    }
    let mut lang_tags: Vec<Vec<u8>> = Vec::new();
    if format == 1 {
        let tags_at = 6 + 12 * count;
        for i in 0..usize::from(d.u16(tags_at)?) {
            let at = tags_at + 2 + 4 * i;
            lang_tags.push(d.slice(storage + usize::from(d.u16(at + 2)?), usize::from(d.u16(at)?))?.to_vec());
        }
    }

    let need: Vec<u16> = OBFUSCATED.iter().copied().filter(|id| referenced.contains(id)).collect();
    if !need.is_empty() && (font.get(*b"fvar").is_some() || font.get(*b"STAT").is_some()) {
        let mut next = records.iter().map(|(r, _)| r[3]).chain([255]).max().unwrap_or(255);
        let mut remap: BTreeMap<u16, u16> = BTreeMap::new();
        for id in need {
            next = next
                .checked_add(1)
                .filter(|n| *n <= 32_767)
                .ok_or_else(|| TablesError::invalid("no name ID is left to keep the obfuscated names"))?;
            remap.insert(id, next);
        }
        let copies: Vec<NameRecord> =
            records.iter().filter_map(|(r, text)| remap.get(&r[3]).map(|new| ([r[0], r[1], r[2], *new], text.clone()))).collect();
        records.extend(copies);
        for tag in [*b"fvar", *b"STAT"] {
            if let Some(mut table) = font.get(tag).map(<[u8]>::to_vec) {
                let slots = if tag == *b"fvar" { fvar_name_slots(&table)? } else { stat_name_slots(&table)? };
                for at in slots {
                    let id = Data::new(tag, &table).u16(at)?;
                    if let Some(new) = remap.get(&id) {
                        set_u16(&mut table, at, *new)?;
                    }
                }
                font.insert(tag, table);
            }
        }
    }

    let unicode = |r: &[u16; 4]| r[0] == 0 || (r[0] == 3 && matches!(r[1], 0 | 1 | 10));
    records.retain(|(r, _)| !matches!(r[3], 16..=18));
    for (r, text) in &mut records {
        let r = *r;
        let replacement: &[u8] = match (r[3], unicode(&r)) {
            (1 | 4, true) => &[0, b'.', 0, 0x7F],
            (1 | 4, false) => b".\x7F",
            (2 | 6, true) => &[0, 0x7F],
            (2 | 6, false) => b"\x7F",
            (3, _) => b"",
            _ => continue,
        };
        *text = replacement.to_vec();
    }
    records.sort_by_key(|(r, _)| *r);
    font.insert(*b"name", write_name(format, &records, &lang_tags)?);
    Ok(())
}

fn write_name(format: u16, records: &[NameRecord], lang_tags: &[Vec<u8>]) -> Result<Vec<u8>> {
    let header = 6 + 12 * records.len() + if format == 1 { 2 + 4 * lang_tags.len() } else { 0 };
    let mut out = Vec::new();
    let mut strings: Vec<u8> = Vec::new();
    let mut placed: HashMap<&[u8], usize> = HashMap::new();
    put_u16(&mut out, format);
    put_u16(&mut out, to_u16(records.len(), "the name record count")?);
    put_u16(&mut out, to_u16(header, "the name table")?);
    for (r, text) in records {
        for v in r {
            put_u16(&mut out, *v);
        }
        let at = *placed.entry(text.as_slice()).or_insert_with(|| {
            strings.extend_from_slice(text);
            strings.len() - text.len()
        });
        put_u16(&mut out, to_u16(text.len(), "a name string")?);
        put_u16(&mut out, to_u16(at, "the name strings")?);
    }
    if format == 1 {
        put_u16(&mut out, to_u16(lang_tags.len(), "the language tag count")?);
        for tag in lang_tags {
            put_u16(&mut out, to_u16(tag.len(), "a language tag")?);
            put_u16(&mut out, to_u16(strings.len(), "the name strings")?);
            strings.extend_from_slice(tag);
        }
    }
    out.extend(strings);
    Ok(out)
}

/// Where `fvar` stores name IDs: each axis name, each instance's subfamily name and, when the
/// records have room for it, its PostScript name.
fn fvar_name_slots(table: &[u8]) -> Result<Vec<usize>> {
    let d = Data::new(*b"fvar", table);
    let axes_at = usize::from(d.u16(4)?);
    let axis_count = usize::from(d.u16(8)?);
    let axis_size = usize::from(d.u16(10)?);
    let instance_count = usize::from(d.u16(12)?);
    let instance_size = usize::from(d.u16(14)?);
    let mut slots = Vec::new();
    let at =
        |base: usize, i: usize, size: usize, plus: usize| i.checked_mul(size).and_then(|o| o.checked_add(base)).and_then(|o| o.checked_add(plus));
    let overflow = || TablesError::invalid("the 'fvar' table is too large");
    for a in 0..axis_count {
        slots.push(at(axes_at, a, axis_size, 18).ok_or_else(overflow)?);
    }
    let instances_at = at(axes_at, axis_count, axis_size, 0).ok_or_else(overflow)?;
    let ps_at = 4 + 4 * axis_count;
    for i in 0..instance_count {
        slots.push(at(instances_at, i, instance_size, 0).ok_or_else(overflow)?);
        if instance_size >= ps_at + 2 {
            slots.push(at(instances_at, i, instance_size, ps_at).ok_or_else(overflow)?);
        }
    }
    Ok(slots)
}

/// Where `STAT` stores name IDs: the elided fallback name (version 1.1 on), each design axis
/// name and each axis value name.
fn stat_name_slots(table: &[u8]) -> Result<Vec<usize>> {
    let d = Data::new(*b"STAT", table);
    let minor = d.u16(2)?;
    let axis_size = usize::from(d.u16(4)?);
    let axis_count = usize::from(d.u16(6)?);
    let axes_at = d.u32_usize(8)?;
    let value_count = usize::from(d.u16(12)?);
    let values_at = d.u32_usize(14)?;
    let mut slots = Vec::new();
    if minor >= 1 {
        slots.push(18);
    }
    let overflow = || TablesError::invalid("the 'STAT' table is too large");
    for a in 0..axis_count {
        slots.push(a.checked_mul(axis_size).and_then(|o| o.checked_add(axes_at)).and_then(|o| o.checked_add(4)).ok_or_else(overflow)?);
    }
    if values_at != 0 {
        for v in 0..value_count {
            let offset = usize::from(d.u16(values_at.checked_add(2 * v).ok_or_else(overflow)?)?);
            slots.push(values_at.checked_add(offset).and_then(|o| o.checked_add(6)).ok_or_else(overflow)?);
        }
    }
    Ok(slots)
}

#[cfg(test)]
#[path = "finish_tests.rs"]
mod tests;
