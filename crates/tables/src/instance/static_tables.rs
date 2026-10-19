//! The smaller tables of a full instance, following fontTools' `varLib.instancer` and the table
//! classes it saves through (MIT, Copyright (c) 2019 Google LLC and the fontTools authors):
//! `cvar` into `cvt `, `MVAR` into the metrics it names, `OS/2` and `post` from the location,
//! the font-wide bounds of `head`, `hhea` and `vhea`, `STAT` reduced to the location, and the
//! `name` records only the variation tables used.

use read_fonts::tables::cvar::Cvar;
use read_fonts::{FontData, FontRead};

use super::glyf::{Glyph, Metrics, has_outline};
use super::ivs::StoreDeltas;
use super::{ot_round, tuple_scalar};
use crate::limits::MAX_TUPLE_VARIATIONS;
use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

fn bad(what: impl Into<String>) -> TablesError {
    TablesError::invalid(what)
}

pub(super) fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    let s = b.get(at..at.checked_add(2)?)?;
    Some(u16::from_be_bytes([*s.first()?, *s.get(1)?]))
}

fn i16_at(b: &[u8], at: usize) -> Option<i16> {
    u16_at(b, at).map(|v| v as i16)
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some((u32::from(u16_at(b, at)?) << 16) | u32::from(u16_at(b, at.checked_add(2)?)?))
}

/// Writes a 16-bit value into `table` at `at` when the table is long enough.
pub(super) fn put_u16(font: &mut SfntFont, tag: Tag, at: usize, v: u16) {
    if let Some(t) = font.tables.get_mut(&tag)
        && let Some(s) = t.get_mut(at..at + 2)
    {
        s.copy_from_slice(&v.to_be_bytes());
    }
}

fn put_i16(font: &mut SfntFont, tag: Tag, at: usize, v: i16) {
    put_u16(font, tag, at, v as u16);
}

fn get_i16(font: &SfntFont, tag: Tag, at: usize) -> Option<i16> {
    font.get(tag).and_then(|t| i16_at(t, at))
}

fn clamp_i16(v: f64) -> i16 {
    v.clamp(-32768.0, 32767.0) as i16
}

/// Adds the `cvar` deltas at `coords` to `cvt `, each sum rounded once.
pub(super) fn apply_cvar(font: &mut SfntFont, axis_count: usize, coords: &[f64]) -> Result<()> {
    let Some(data) = font.get(*b"cvar") else { return Ok(()) };
    let Some(cvt) = font.get(*b"cvt ") else { return Ok(()) };
    let count = cvt.len() / 2;
    let cvar = Cvar::read(FontData::new(data)).map_err(|e| bad(format!("the cvar table cannot be read: {e}")))?;
    let axes = u16::try_from(axis_count).map_err(|_| bad("too many axes"))?;
    let Some(variations) = cvar.variation_data(axes) else { return Err(bad("the cvar variation data cannot be read")) };
    let mut sums = vec![0.0f64; count];
    for (n, tuple) in variations.tuples().enumerate() {
        if n >= MAX_TUPLE_VARIATIONS {
            return Err(TablesError::TooLarge { what: "the tuple variations of cvar", limit: MAX_TUPLE_VARIATIONS });
        }
        let peak = tuple.peak();
        let start = tuple.intermediate_start();
        let end = tuple.intermediate_end();
        let region = (0..axis_count).map(|i| {
            let p = peak.get(i).map_or(0.0, |v| f64::from(v.to_bits()) / 16384.0);
            let s = start.as_ref().and_then(|t| t.get(i)).map_or(p.min(0.0), |v| f64::from(v.to_bits()) / 16384.0);
            let e = end.as_ref().and_then(|t| t.get(i)).map_or(p.max(0.0), |v| f64::from(v.to_bits()) / 16384.0);
            (s, p, e)
        });
        let scalar = tuple_scalar(coords, region);
        if scalar == 0.0 {
            continue;
        }
        for d in tuple.deltas() {
            if let Some(s) = sums.get_mut(usize::from(d.position)) {
                *s += scalar * f64::from(d.value);
            }
        }
    }
    let mut out = cvt.to_vec();
    for (i, d) in sums.iter().enumerate() {
        if *d != 0.0 {
            let v = i16_at(&out, 2 * i).unwrap_or(0);
            if let Some(s) = out.get_mut(2 * i..2 * i + 2) {
                s.copy_from_slice(&clamp_i16(f64::from(v) + ot_round(*d)).to_be_bytes());
            }
        }
    }
    font.insert(*b"cvt ", out);
    Ok(())
}

/// The fields `MVAR` tags vary: table, byte offset, signed.
const MVAR_FIELDS: [(&[u8; 4], &[u8; 4], usize, bool); 28] = [
    (b"hasc", b"OS/2", 68, true),
    (b"hdsc", b"OS/2", 70, true),
    (b"hlgp", b"OS/2", 72, true),
    (b"hcla", b"OS/2", 74, false),
    (b"hcld", b"OS/2", 76, false),
    (b"vasc", b"vhea", 4, true),
    (b"vdsc", b"vhea", 6, true),
    (b"vlgp", b"vhea", 8, true),
    (b"hcrs", b"hhea", 18, true),
    (b"hcrn", b"hhea", 20, true),
    (b"hcof", b"hhea", 22, true),
    (b"vcrs", b"vhea", 18, true),
    (b"vcrn", b"vhea", 20, true),
    (b"vcof", b"vhea", 22, true),
    (b"xhgt", b"OS/2", 86, true),
    (b"cpht", b"OS/2", 88, true),
    (b"sbxs", b"OS/2", 10, true),
    (b"sbys", b"OS/2", 12, true),
    (b"sbxo", b"OS/2", 14, true),
    (b"sbyo", b"OS/2", 16, true),
    (b"spxs", b"OS/2", 18, true),
    (b"spys", b"OS/2", 20, true),
    (b"spxo", b"OS/2", 22, true),
    (b"spyo", b"OS/2", 24, true),
    (b"strs", b"OS/2", 26, true),
    (b"stro", b"OS/2", 28, true),
    (b"unds", b"post", 10, true),
    (b"undo", b"post", 8, true),
];

/// Adds the `MVAR` deltas at `coords` to the fields its records name. When `OS/2`'s typographic
/// ascender, descender and line gap equalled `hhea`'s before, `hhea` follows them.
pub(super) fn apply_mvar(font: &mut SfntFont, coords: &[f64]) -> Result<()> {
    let Some(data) = font.get(*b"MVAR") else { return Ok(()) };
    let record_size = usize::from(u16_at(data, 6).ok_or_else(|| bad("the MVAR table is truncated"))?);
    let count = usize::from(u16_at(data, 8).ok_or_else(|| bad("the MVAR table is truncated"))?);
    let store_offset = usize::from(u16_at(data, 10).ok_or_else(|| bad("the MVAR table is truncated"))?);
    if store_offset == 0 || count == 0 {
        return Ok(());
    }
    if record_size < 8 {
        return Err(bad("the MVAR value records are shorter than 8 bytes"));
    }
    let deltas = StoreDeltas::read(data, store_offset, coords)?;
    let mut changes: Vec<(Tag, usize, bool, f64)> = Vec::new();
    for i in 0..count {
        let at = 12 + i * record_size;
        let tag = data.get(at..at + 4).ok_or_else(|| bad("the MVAR value records are truncated"))?;
        let (Some(outer), Some(inner)) = (u16_at(data, at + 4), u16_at(data, at + 6)) else {
            return Err(bad("the MVAR value records are truncated"));
        };
        let Some(&(_, table, offset, signed)) = MVAR_FIELDS.iter().find(|f| f.0.as_slice() == tag) else { continue };
        let d = deltas.get(outer, inner);
        if d != 0.0 {
            changes.push((*table, offset, signed, ot_round(d)));
        }
    }
    let typo = |f: &SfntFont, tag: Tag, at: [usize; 3]| at.map(|a| get_i16(f, tag, a));
    let before_os2 = typo(font, *b"OS/2", [68, 70, 72]);
    let synced = before_os2.iter().all(Option::is_some) && before_os2 == typo(font, *b"hhea", [4, 6, 8]);
    for (table, offset, signed, d) in changes {
        let Some(v) = font.get(table).and_then(|t| u16_at(t, offset)) else { continue };
        if signed {
            put_i16(font, table, offset, clamp_i16(f64::from(v as i16) + d));
        } else {
            put_u16(font, table, offset, (f64::from(v) + d).clamp(0.0, 65535.0) as u16);
        }
    }
    let after_os2 = typo(font, *b"OS/2", [68, 70, 72]);
    if synced && after_os2 != before_os2 {
        for (v, at) in after_os2.iter().zip([4, 6, 8]) {
            if let Some(v) = v {
                put_i16(font, *b"hhea", at, *v);
            }
        }
    }
    Ok(())
}

/// The `wdth` value (in percent) at which each `OS/2` width class starts.
const WIDTH_CLASSES: [(f64, f64); 9] =
    [(50.0, 1.0), (62.5, 2.0), (75.0, 3.0), (87.5, 4.0), (100.0, 5.0), (112.5, 6.0), (125.0, 7.0), (150.0, 8.0), (200.0, 9.0)];

/// `usWeightClass`, `usWidthClass` and `italicAngle` from the pinned `wght`, `wdth` and `slnt`
/// values, as fontTools' `set_default_weight_width_slant` sets them.
pub(super) fn set_weight_width_slant(font: &mut SfntFont, user: &[(Tag, f64)]) {
    let value = |tag: &[u8; 4]| user.iter().find(|(t, _)| t == tag).map(|(_, v)| *v);
    if let Some(w) = value(b"wght") {
        put_u16(font, *b"OS/2", 4, ot_round(w.clamp(1.0, 1000.0)) as u16);
    }
    if let Some(w) = value(b"wdth") {
        let w = w.clamp(50.0, 200.0);
        let class = WIDTH_CLASSES
            .windows(2)
            .find_map(|pair| match pair {
                [(x0, c0), (x1, c1)] if w >= *x0 && w <= *x1 => Some(c0 + (w - x0) / (x1 - x0) * (c1 - c0)),
                _ => None,
            })
            .unwrap_or(5.0);
        put_u16(font, *b"OS/2", 6, ot_round(class) as u16);
    }
    if let Some(s) = value(b"slnt") {
        let fixed = ot_round(s.clamp(-90.0, 90.0) * 65536.0) as i32;
        if let Some(t) = font.tables.get_mut(b"post")
            && let Some(slot) = t.get_mut(4..8)
        {
            slot.copy_from_slice(&fixed.to_be_bytes());
        }
    }
}

/// `xAvgCharWidth`: the rounded mean of the advances above zero.
pub(super) fn recalc_avg_char_width(font: &mut SfntFont, hmtx: &Metrics) {
    let widths: Vec<f64> = hmtx.advances.iter().filter(|a| **a > 0).map(|a| f64::from(*a)).collect();
    let avg = if widths.is_empty() { 0.0 } else { ot_round(widths.iter().sum::<f64>() / widths.len() as f64) };
    put_i16(font, *b"OS/2", 2, clamp_i16(avg));
}

/// `head`'s bounds over the glyphs with outlines, and `hhea` (and `vhea`) extents, as
/// fontTools recomputes them when it saves with `recalcBBoxes`.
pub(super) fn recalc_bounds(font: &mut SfntFont, glyphs: &[Glyph], hmtx: &Metrics, vmtx: Option<&Metrics>) {
    let mut bounds: Option<[i16; 4]> = None;
    for g in glyphs.iter().filter(|g| has_outline(g)) {
        let b = g.bbox();
        bounds = Some(match bounds {
            None => b,
            Some(o) => [o[0].min(b[0]), o[1].min(b[1]), o[2].max(b[2]), o[3].max(b[3])],
        });
    }
    let [x0, y0, x1, y1] = bounds.unwrap_or([0; 4]);
    for (at, v) in [(36, x0), (38, y0), (40, x1), (42, y1)] {
        put_i16(font, *b"head", at, v);
    }
    let extents = |m: &Metrics, horizontal: bool| -> (u16, i16, i16, i16) {
        let max_adv = m.advances.iter().copied().max().unwrap_or(0);
        let (mut min_a, mut min_b, mut max_ext): (Option<f64>, Option<f64>, Option<f64>) = (None, None, None);
        for (i, g) in glyphs.iter().enumerate().filter(|(_, g)| has_outline(g)) {
            let b = g.bbox().map(f64::from);
            let size = if horizontal { b[2] - b[0] } else { b[3] - b[1] };
            let adv = f64::from(m.advances.get(i).copied().unwrap_or(0));
            let sb = f64::from(m.bearings.get(i).copied().unwrap_or(0));
            let other = adv - sb - size;
            let ext = sb + size;
            min_a = Some(min_a.map_or(sb, |v| v.min(sb)));
            min_b = Some(min_b.map_or(other, |v| v.min(other)));
            max_ext = Some(max_ext.map_or(ext, |v| v.max(ext)));
        }
        (max_adv, clamp_i16(min_a.unwrap_or(0.0)), clamp_i16(min_b.unwrap_or(0.0)), clamp_i16(max_ext.unwrap_or(0.0)))
    };
    let (adv, a, b, ext) = extents(hmtx, true);
    put_u16(font, *b"hhea", 10, adv);
    put_i16(font, *b"hhea", 12, a);
    put_i16(font, *b"hhea", 14, b);
    put_i16(font, *b"hhea", 16, ext);
    if let Some(v) = vmtx {
        let (adv, a, b, ext) = extents(v, false);
        put_u16(font, *b"vhea", 10, adv);
        put_i16(font, *b"vhea", 12, a);
        put_i16(font, *b"vhea", 14, b);
        put_i16(font, *b"vhea", 16, ext);
    }
}

/// The name IDs above 255 that `fvar` and `STAT` use.
pub(super) fn variation_name_ids(font: &SfntFont) -> Vec<u16> {
    let mut ids = Vec::new();
    if let Some(f) = font.get(*b"fvar") {
        let axes_at = usize::from(u16_at(f, 4).unwrap_or(0));
        let axis_count = usize::from(u16_at(f, 8).unwrap_or(0));
        let axis_size = usize::from(u16_at(f, 10).unwrap_or(20));
        let instance_count = usize::from(u16_at(f, 12).unwrap_or(0));
        let instance_size = usize::from(u16_at(f, 14).unwrap_or(0));
        for i in 0..axis_count {
            ids.extend(u16_at(f, axes_at + i * axis_size + 18));
        }
        let instances_at = axes_at + axis_count * axis_size;
        for i in 0..instance_count {
            let at = instances_at + i * instance_size;
            ids.extend(u16_at(f, at));
            if instance_size >= 6 + 4 * axis_count {
                ids.extend(u16_at(f, at + 4 + 4 * axis_count).filter(|id| *id != 0xFFFF));
            }
        }
    }
    if let Some(s) = font.get(*b"STAT") {
        let axis_size = usize::from(u16_at(s, 4).unwrap_or(8));
        let axis_count = usize::from(u16_at(s, 6).unwrap_or(0));
        let axes_at = u32_at(s, 8).unwrap_or(0) as usize;
        for i in 0..axis_count {
            ids.extend(u16_at(s, axes_at + i * axis_size + 4));
        }
        let value_count = usize::from(u16_at(s, 12).unwrap_or(0));
        let values_at = u32_at(s, 14).unwrap_or(0) as usize;
        if values_at != 0 {
            for i in 0..value_count {
                if let Some(o) = u16_at(s, values_at + 2 * i) {
                    ids.extend(u16_at(s, values_at + usize::from(o) + 6));
                }
            }
        }
        if u16_at(s, 2).unwrap_or(0) >= 1 {
            ids.extend(u16_at(s, 18));
        }
    }
    ids.retain(|id| *id > 255);
    ids.sort_unstable();
    ids.dedup();
    ids
}

fn fixed_at(b: &[u8], at: usize) -> Option<f64> {
    u32_at(b, at).map(|v| f64::from(v as i32) / 65536.0)
}

/// Keeps the `STAT` axis values that hold at the pinned location: those of axes not pinned, and
/// those whose value (nominal value for format 2; every record for format 4) equals the pinned
/// user value. The dropped subtables stay in the table's data, unreferenced.
pub(super) fn reduce_stat(font: &mut SfntFont, user: &[(Tag, f64)]) -> Result<()> {
    let Some(s) = font.get(*b"STAT") else { return Ok(()) };
    let mut s = s.to_vec();
    let axis_size = usize::from(u16_at(&s, 4).ok_or_else(|| bad("the STAT table is truncated"))?);
    let axis_count = usize::from(u16_at(&s, 6).ok_or_else(|| bad("the STAT table is truncated"))?);
    let axes_at = u32_at(&s, 8).ok_or_else(|| bad("the STAT table is truncated"))? as usize;
    let value_count = usize::from(u16_at(&s, 12).ok_or_else(|| bad("the STAT table is truncated"))?);
    let values_at = u32_at(&s, 14).ok_or_else(|| bad("the STAT table is truncated"))? as usize;
    if axis_count == 0 || value_count == 0 || values_at == 0 {
        return Ok(());
    }
    let axis_tag =
        |i: usize| -> Option<Tag> { s.get(axes_at + i * axis_size..axes_at + i * axis_size + 4).and_then(|t| <[u8; 4]>::try_from(t).ok()) };
    let outside = |i: u16, v: f64| -> bool {
        axis_tag(usize::from(i)).and_then(|t| user.iter().find(|(u, _)| *u == t)).is_some_and(|(_, pinned)| v != *pinned)
    };
    let mut kept: Vec<u16> = Vec::with_capacity(value_count);
    for i in 0..value_count {
        let Some(o) = u16_at(&s, values_at + 2 * i) else { return Err(bad("the STAT axis value offsets are truncated")) };
        let at = values_at + usize::from(o);
        let format = u16_at(&s, at).ok_or_else(|| bad("a STAT axis value is truncated"))?;
        let drop = match format {
            1..=3 => {
                let axis = u16_at(&s, at + 2).ok_or_else(|| bad("a STAT axis value is truncated"))?;
                let v = fixed_at(&s, at + 8).ok_or_else(|| bad("a STAT axis value is truncated"))?;
                outside(axis, v)
            }
            4 => {
                let n = usize::from(u16_at(&s, at + 2).ok_or_else(|| bad("a STAT axis value is truncated"))?);
                let mut drop = false;
                for r in 0..n {
                    let (Some(axis), Some(v)) = (u16_at(&s, at + 8 + 6 * r), fixed_at(&s, at + 10 + 6 * r)) else {
                        return Err(bad("a STAT axis value is truncated"));
                    };
                    if outside(axis, v) {
                        drop = true;
                        break;
                    }
                }
                drop
            }
            _ => false,
        };
        if !drop {
            kept.push(o);
        }
    }
    if kept.len() == value_count {
        return Ok(());
    }
    for (i, o) in kept.iter().enumerate() {
        if let Some(slot) = s.get_mut(values_at + 2 * i..values_at + 2 * i + 2) {
            slot.copy_from_slice(&o.to_be_bytes());
        }
    }
    let count = u16::try_from(kept.len()).unwrap_or(0);
    if let Some(slot) = s.get_mut(12..14) {
        slot.copy_from_slice(&count.to_be_bytes());
    }
    if count == 0
        && let Some(slot) = s.get_mut(14..18)
    {
        slot.copy_from_slice(&0u32.to_be_bytes());
    }
    font.insert(*b"STAT", s);
    Ok(())
}

/// Removes the `name` records with an ID in `drop`, laying the string storage out again; drops
/// `ltag` when no language-tagged Unicode record is left.
pub(super) fn prune_names(font: &mut SfntFont, drop: &[u16]) -> Result<()> {
    if drop.is_empty() {
        return Ok(());
    }
    let Some(n) = font.get(*b"name") else { return Ok(()) };
    let format = u16_at(n, 0).ok_or_else(|| bad("the name table is truncated"))?;
    let count = usize::from(u16_at(n, 2).ok_or_else(|| bad("the name table is truncated"))?);
    let storage = usize::from(u16_at(n, 4).ok_or_else(|| bad("the name table is truncated"))?);
    let string = |len: u16, off: u16| -> Result<&[u8]> {
        let start = storage + usize::from(off);
        n.get(start..start + usize::from(len)).ok_or_else(|| bad("a name record points past the end of the table"))
    };
    let mut records: Vec<([u16; 4], &[u8])> = Vec::with_capacity(count);
    for i in 0..count {
        let at = 6 + 12 * i;
        let f: Vec<u16> = (0..6).map(|k| u16_at(n, at + 2 * k)).collect::<Option<_>>().ok_or_else(|| bad("the name records are truncated"))?;
        let [p, e, l, id, len, off] = <[u16; 6]>::try_from(f).unwrap_or([0; 6]);
        if !drop.contains(&id) {
            records.push(([p, e, l, id], string(len, off)?));
        }
    }
    let mut lang_tags: Vec<&[u8]> = Vec::new();
    if format == 1 {
        let at = 6 + 12 * count;
        let tags = usize::from(u16_at(n, at).ok_or_else(|| bad("the name language tags are truncated"))?);
        for i in 0..tags {
            let (Some(len), Some(off)) = (u16_at(n, at + 2 + 4 * i), u16_at(n, at + 4 + 4 * i)) else {
                return Err(bad("the name language tags are truncated"));
            };
            lang_tags.push(string(len, off)?);
        }
    }
    let header = 6 + 12 * records.len() + if format == 1 { 2 + 4 * lang_tags.len() } else { 0 };
    let mut data: Vec<u8> = Vec::new();
    let mut placed: std::collections::HashMap<Vec<u8>, usize> = std::collections::HashMap::new();
    let mut place = |bytes: &[u8]| -> Result<(u16, u16)> {
        let off = match placed.get(bytes) {
            Some(p) => *p,
            None => {
                let p = data.len();
                data.extend_from_slice(bytes);
                placed.insert(bytes.to_vec(), p);
                p
            }
        };
        let len = u16::try_from(bytes.len()).map_err(|_| bad("a name is too long"))?;
        Ok((len, u16::try_from(off).map_err(|_| bad("the name strings are too long"))?))
    };
    let mut out = Vec::with_capacity(header);
    out.extend_from_slice(&format.to_be_bytes());
    out.extend_from_slice(&u16::try_from(records.len()).unwrap_or(0).to_be_bytes());
    out.extend_from_slice(&u16::try_from(header).map_err(|_| bad("the name table is too large"))?.to_be_bytes());
    for (ids, bytes) in &records {
        let (len, off) = place(bytes)?;
        for v in ids.iter().chain([len, off].iter()) {
            out.extend_from_slice(&v.to_be_bytes());
        }
    }
    if format == 1 {
        out.extend_from_slice(&u16::try_from(lang_tags.len()).unwrap_or(0).to_be_bytes());
        for bytes in &lang_tags {
            let (len, off) = place(bytes)?;
            out.extend_from_slice(&len.to_be_bytes());
            out.extend_from_slice(&off.to_be_bytes());
        }
    }
    let unicode_tagged = records.iter().any(|([platform, _, language, _], _)| *platform == 0 && *language != 0xFFFF);
    out.extend_from_slice(&data);
    font.insert(*b"name", out);
    if font.get(*b"ltag").is_some() && !unicode_tagged {
        font.remove(*b"ltag");
    }
    Ok(())
}
