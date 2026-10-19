//! Subsetting the colour, bitmap, math and other glyph-indexed tables (M12.19), ported from
//! fontTools' subsetter (MIT, Copyright 2013 Google, Inc.):
//!
//! - `COLR` and `CPAL` ([`colr`]): the COLR closure adds the layer glyphs and every glyph the
//!   paint graphs draw; the paint graphs are rebuilt with layer reuse and the palettes keep
//!   only the entries still used.
//! - `MATH` ([`math`]): the MATH closure adds the size variants and assembly parts.
//! - `SVG ` ([`svg`]): documents keep the elements of the kept glyphs and what they reference.
//! - `sbix`, `CBDT`/`CBLC` (and `EBDT`/`EBLC` when they are not dropped): the bitmaps of the
//!   output glyphs; strikes left without bitmaps go from `CBLC`.
//! - `VORG`, `hdmx` and `kern` keep the entries of the output glyphs (`kern` those of the
//!   glyphs the GSUB closure reached, and only its format 0 subtables).
//! - `BASE` is copied, as pyftsubset leaves it.

pub(crate) mod colr;
pub(crate) mod math;
mod svg;

use std::collections::{BTreeMap, BTreeSet};

use super::Plan;
use crate::reorder::be::{Data, put_u16, put_u32, search_params, to_u16, to_u32};
use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

/// Writes the subset of the tables this module handles from `input` into `output`.
pub fn apply(plan: &Plan, input: &SfntFont, output: &mut SfntFont) -> Result<()> {
    if let Some(bytes) = input.get(*b"BASE") {
        output.insert(*b"BASE", bytes.to_vec());
    }
    if let Some(bytes) = input.get(*b"VORG") {
        output.insert(*b"VORG", subset_vorg(plan, bytes)?);
    }
    if let Some(bytes) = input.get(*b"hdmx")
        && let Some(table) = subset_hdmx(plan, bytes)?
    {
        output.insert(*b"hdmx", table);
    }
    if let Some(bytes) = input.get(*b"kern")
        && let Some(table) = subset_kern(plan, bytes)?
    {
        output.insert(*b"kern", table);
    }
    if let Some(bytes) = input.get(*b"sbix") {
        output.insert(*b"sbix", subset_sbix(plan, bytes)?);
    }
    for (loc, data) in [(*b"CBLC", *b"CBDT"), (*b"EBLC", *b"EBDT")] {
        if let (Some(l), Some(d)) = (input.get(loc), input.get(data)) {
            let (l, d) = subset_bitmaps(plan, loc, l, data, d)?;
            output.insert(loc, l);
            output.insert(data, d);
        }
    }
    if let Some(bytes) = input.get(*b"MATH") {
        output.insert(*b"MATH", math::subset(plan, bytes)?);
    }
    if let Some(bytes) = input.get(*b"SVG ")
        && let Some(table) = svg::subset(plan, bytes)?
    {
        output.insert(*b"SVG ", table);
    }
    // CPAL follows the palette entries the subset COLR still uses, unless SVG may use any.
    let prune_palettes = input.tables.contains_key(b"CPAL") && !output.tables.contains_key(b"SVG ");
    let mut palettes = None;
    if let Some(bytes) = input.get(*b"COLR")
        && let Some((table, used)) = colr::subset_colr(plan, bytes, prune_palettes)?
    {
        output.insert(*b"COLR", table);
        palettes = used;
    }
    if let Some(bytes) = input.get(*b"CPAL")
        && let Some(table) = colr::prune_cpal(output, bytes, palettes.as_ref())?
    {
        output.insert(*b"CPAL", table);
    }
    Ok(())
}

/// The output id of input glyph `old`.
fn new_id(plan: &Plan, old: u32) -> Result<u16> {
    let new = plan.glyph_map.get(&old).copied().ok_or_else(|| TablesError::invalid(format!("glyph {old} is not in the subset")))?;
    to_u16(new as usize, "a glyph id")
}

/// The input glyph is kept with its data (in the output and not emptied).
fn retained(plan: &Plan, old: u32) -> bool {
    plan.glyphs.contains(&old)
}

// ── VORG ──

fn subset_vorg(plan: &Plan, bytes: &[u8]) -> Result<Vec<u8>> {
    let d = Data::new(*b"VORG", bytes);
    let count = usize::from(d.u16(6)?);
    let mut records = BTreeMap::new();
    for i in 0..count {
        let g = u32::from(d.u16(8 + 4 * i)?);
        if retained(plan, g) {
            records.insert(new_id(plan, g)?, d.u16(10 + 4 * i)?);
        }
    }
    let mut out = d.slice(0, 6)?.to_vec();
    put_u16(&mut out, to_u16(records.len(), "the VORG record count")?);
    for (g, y) in records {
        put_u16(&mut out, g);
        put_u16(&mut out, y);
    }
    Ok(out)
}

// ── hdmx ──

/// `hdmx` with a width per output glyph (0 for emptied glyphs) and each record's maximum
/// recomputed; `None` when it has no records.
fn subset_hdmx(plan: &Plan, bytes: &[u8]) -> Result<Option<Vec<u8>>> {
    let d = Data::new(*b"hdmx", bytes);
    let count = usize::from(d.u16(2)?);
    let size = d.u32_usize(4)?;
    if count == 0 {
        return Ok(None);
    }
    if size < 2 + plan.num_glyphs {
        return Err(d.bad("has device records shorter than the glyph count"));
    }
    d.slice(8, size.saturating_mul(count))?;
    let n = plan.new_to_old.len();
    let new_size = 4 * (2 + n).div_ceil(4);
    let mut records = BTreeMap::new();
    for r in 0..count {
        let at = 8 + r * size;
        let ppem = d.u8(at)?;
        let mut widths = Vec::with_capacity(n);
        for old in &plan.new_to_old {
            widths.push(if plan.emptied.contains(old) { 0 } else { d.u8(at + 2 + *old as usize)? });
        }
        records.insert(ppem, widths);
    }
    let mut out = Vec::with_capacity(8 + new_size * records.len());
    put_u16(&mut out, 0);
    put_u16(&mut out, to_u16(records.len(), "the hdmx record count")?);
    put_u32(&mut out, to_u32(new_size, "the hdmx record size")?);
    for (ppem, widths) in records {
        out.push(ppem);
        out.push(widths.iter().copied().max().unwrap_or(0));
        out.extend_from_slice(&widths);
        out.resize(out.len() + new_size - 2 - n, 0);
    }
    Ok(Some(out))
}

// ── kern ──

/// `kern` with the format 0 subtables only, each keeping the pairs of two glyphs the GSUB
/// closure reached; `None` when no pair is left.
fn subset_kern(plan: &Plan, bytes: &[u8]) -> Result<Option<Vec<u8>>> {
    let d = Data::new(*b"kern", bytes);
    let apple = d.u16(0)? == 1;
    let (count, mut at) = if apple { (d.u32_usize(4)?, 8) } else { (usize::from(d.u16(2)?), 4) };
    let mut tables: Vec<Vec<u8>> = Vec::new();
    for _ in 0..count {
        if at >= bytes.len() {
            break;
        }
        let (header_len, format, length) = if apple {
            (8, d.u8(at + 5)?, d.u32_usize(at)?)
        } else {
            let format = d.u8(at + 4)?;
            // A format 0 subtable's length follows its pair count; the stored length can overflow.
            let length = if format == 0 { 14 + 6 * usize::from(d.u16(at + 6)?) } else { usize::from(d.u16(at + 2)?) };
            (6, format, length)
        };
        if length < header_len {
            return Err(d.bad("has a subtable shorter than its header"));
        }
        if format == 0 {
            let pairs_at = at + header_len;
            let n = usize::from(d.u16(pairs_at)?);
            let mut pairs: BTreeMap<(u16, u16), u16> = BTreeMap::new();
            for i in 0..n {
                let p = pairs_at + 8 + 6 * i;
                let (l, r) = (u32::from(d.u16(p)?), u32::from(d.u16(p + 2)?));
                if plan.layout_glyphs.contains(&l) && plan.layout_glyphs.contains(&r) {
                    pairs.insert((new_id(plan, l)?, new_id(plan, r)?), d.u16(p + 4)?);
                }
            }
            if !pairs.is_empty() {
                let mut t = d.slice(at, header_len)?.to_vec();
                let n_pairs = pairs.len().min(usize::from(u16::MAX));
                let (range, selector, shift) = search_params(n_pairs, 6);
                put_u16(&mut t, n_pairs as u16);
                put_u16(&mut t, range);
                put_u16(&mut t, selector);
                put_u16(&mut t, shift);
                for ((l, r), v) in pairs {
                    put_u16(&mut t, l);
                    put_u16(&mut t, r);
                    put_u16(&mut t, v);
                }
                let len = t.len();
                if apple {
                    let slot = t.get_mut(0..4).ok_or_else(|| d.bad("has a short subtable"))?;
                    slot.copy_from_slice(&to_u32(len, "a kern subtable")?.to_be_bytes());
                } else {
                    let slot = t.get_mut(2..4).ok_or_else(|| d.bad("has a short subtable"))?;
                    slot.copy_from_slice(&((len & 0xFFFF) as u16).to_be_bytes());
                }
                tables.push(t);
            }
        }
        at = at.checked_add(length).ok_or_else(|| d.bad("has a subtable past its end"))?;
    }
    if tables.is_empty() {
        return Ok(None);
    }
    let mut out = Vec::new();
    if apple {
        put_u32(&mut out, 0x0001_0000);
        put_u32(&mut out, to_u32(tables.len(), "the kern subtable count")?);
    } else {
        put_u16(&mut out, 0);
        put_u16(&mut out, to_u16(tables.len(), "the kern subtable count")?);
    }
    for t in tables {
        out.extend(t);
    }
    Ok(Some(out))
}

// ── sbix ──

/// `sbix` with every strike's bitmaps in the output order; emptied and dropped glyphs have
/// none, and a `dupe` or `flip` bitmap whose glyph left the subset goes too.
fn subset_sbix(plan: &Plan, bytes: &[u8]) -> Result<Vec<u8>> {
    let d = Data::new(*b"sbix", bytes);
    let (version, flags) = (d.u16(0)?, d.u16(2)?);
    let count = d.u32_usize(4)?;
    // Each strike offset takes 4 bytes, so the table length bounds the count.
    d.slice(8, count.saturating_mul(4))?;
    let mut strikes: BTreeMap<u16, Vec<u8>> = BTreeMap::new();
    for s in 0..count {
        let at = d.u32_usize(8 + 4 * s)?;
        let (ppem, ppi) = (d.u16(at)?, d.u16(at + 2)?);
        let offset = |g: usize| -> Result<usize> { d.u32_usize(at + 4 + 4 * g) };
        let header = 4 + 4 * (plan.new_to_old.len() + 1);
        let mut data: Vec<u8> = Vec::new();
        let mut offsets = Vec::with_capacity(plan.new_to_old.len() + 1);
        for old in &plan.new_to_old {
            offsets.push(header + data.len());
            if !retained(plan, *old) {
                continue;
            }
            let g = *old as usize;
            let (start, end) = (offset(g)?, offset(g + 1)?);
            let len = end.checked_sub(start).ok_or_else(|| d.bad("has decreasing glyph data offsets"))?;
            let glyph = d.slice(at + start, len)?;
            match glyph.get(4..8) {
                Some(b"dupe") | Some(b"flip") => {
                    let target = u32::from(Data::new(*b"sbix", glyph).u16(8)?);
                    if !retained(plan, target) {
                        continue;
                    }
                    data.extend_from_slice(glyph.get(..8).unwrap_or_default());
                    put_u16(&mut data, new_id(plan, target)?);
                }
                _ => data.extend_from_slice(glyph),
            }
        }
        offsets.push(header + data.len());
        let mut strike = Vec::with_capacity(header + data.len());
        put_u16(&mut strike, ppem);
        put_u16(&mut strike, ppi);
        for o in offsets {
            put_u32(&mut strike, to_u32(o, "an sbix strike")?);
        }
        strike.extend(data);
        strikes.insert(ppem, strike);
    }
    let mut out = Vec::new();
    put_u16(&mut out, version);
    put_u16(&mut out, flags);
    put_u32(&mut out, to_u32(strikes.len(), "the sbix strike count")?);
    let mut at = 8 + 4 * strikes.len();
    for s in strikes.values() {
        put_u32(&mut out, to_u32(at, "the sbix table")?);
        at += s.len();
    }
    for s in strikes.into_values() {
        out.extend(s);
    }
    Ok(out)
}

// ── CBLC and CBDT ──

/// One index subtable: its formats, the constant image size and metrics of formats 2 and 5,
/// and its glyphs (output id, image data).
struct IndexSubtable<'a> {
    index_format: u16,
    image_format: u16,
    fixed: Option<(u32, &'a [u8])>,
    glyphs: Vec<(u16, &'a [u8])>,
}

/// The location and data tables keeping the bitmaps of the output glyphs, as pyftsubset's
/// `EBLC` and `EBDT` methods do: index subtables and strikes left empty go.
fn subset_bitmaps(plan: &Plan, loc_tag: Tag, loc: &[u8], data_tag: Tag, data: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    let l = Data::new(loc_tag, loc);
    let bd = Data::new(data_tag, data);
    let count = l.u32_usize(4)?;
    l.slice(8, count.saturating_mul(48))?;
    let mut out_data = bd.slice(0, 4)?.to_vec();
    let mut strikes: Vec<(&[u8], Vec<IndexSubtable<'_>>)> = Vec::new();
    for s in 0..count {
        let rec = 8 + 48 * s;
        let array_at = l.u32_usize(rec)?;
        let subtables = l.u32_usize(rec + 8)?;
        l.slice(array_at, subtables.saturating_mul(8))?;
        let mut kept = Vec::new();
        for i in 0..subtables {
            let a = array_at + 8 * i;
            let (first, last) = (usize::from(l.u16(a)?), usize::from(l.u16(a + 2)?));
            let at = array_at + l.u32_usize(a + 4)?;
            if last < first {
                return Err(l.bad("has an index subtable whose last glyph comes before its first"));
            }
            let index_format = l.u16(at)?;
            let image_format = l.u16(at + 2)?;
            if matches!(image_format, 8 | 9) {
                return Err(TablesError::Unsupported("subsetting composite bitmap glyphs".into()));
            }
            let image_at = l.u32_usize(at + 4)?;
            let mut entries: Vec<(u32, usize, usize)> = Vec::new();
            let mut fixed = None;
            match index_format {
                1 | 3 => {
                    for (k, g) in (first..=last).enumerate() {
                        let off = |j: usize| -> Result<usize> {
                            if index_format == 1 { l.u32_usize(at + 8 + 4 * j) } else { Ok(usize::from(l.u16(at + 8 + 2 * j)?)) }
                        };
                        entries.push((g as u32, off(k)?, off(k + 1)?));
                    }
                }
                2 => {
                    let size = l.u32_usize(at + 8)?;
                    fixed = Some((size as u32, l.slice(at + 12, 8)?));
                    for (k, g) in (first..=last).enumerate() {
                        entries.push((g as u32, k * size, (k + 1) * size));
                    }
                }
                4 => {
                    let n = l.u32_usize(at + 8)?;
                    l.slice(at + 12, n.saturating_add(1).saturating_mul(4))?;
                    for k in 0..n {
                        let p = at + 12 + 4 * k;
                        entries.push((u32::from(l.u16(p)?), usize::from(l.u16(p + 2)?), usize::from(l.u16(p + 6)?)));
                    }
                }
                5 => {
                    let size = l.u32_usize(at + 8)?;
                    fixed = Some((size as u32, l.slice(at + 12, 8)?));
                    let n = l.u32_usize(at + 20)?;
                    l.slice(at + 24, n.saturating_mul(2))?;
                    for k in 0..n {
                        entries.push((u32::from(l.u16(at + 24 + 2 * k)?), k * size, (k + 1) * size));
                    }
                }
                _ => return Err(l.bad(format!("has an index subtable of format {index_format}"))),
            }
            let mut glyphs = Vec::new();
            for (g, start, end) in entries {
                // Glyphs without data are not listed, as fontTools reads them.
                if end <= start || !retained(plan, g) {
                    continue;
                }
                glyphs.push((new_id(plan, g)?, bd.slice(image_at.saturating_add(start), end - start)?));
            }
            glyphs.sort_by_key(|(g, _)| *g);
            glyphs.dedup_by_key(|(g, _)| *g);
            if !glyphs.is_empty() {
                kept.push(IndexSubtable { index_format, image_format, fixed, glyphs });
            }
        }
        if !kept.is_empty() {
            strikes.push((l.slice(rec, 48)?, kept));
        }
    }

    let mut out = l.slice(0, 4)?.to_vec();
    put_u32(&mut out, to_u32(strikes.len(), "the bitmap strike count")?);
    let mut arrays: Vec<Vec<u8>> = Vec::with_capacity(strikes.len());
    let mut records: Vec<Vec<u8>> = Vec::with_capacity(strikes.len());
    for (record, subtables) in &strikes {
        let mut array = Vec::new();
        let mut bodies: Vec<Vec<u8>> = Vec::new();
        let mut body_at = 8 * subtables.len();
        let (mut start, mut end) = (u16::MAX, 0u16);
        for st in subtables {
            let first = st.glyphs.first().map_or(0, |g| g.0);
            let last = st.glyphs.last().map_or(0, |g| g.0);
            start = start.min(first);
            end = end.max(last);
            let image_at = out_data.len();
            let body = index_body(st, first, last, image_at, &mut out_data)?;
            put_u16(&mut array, first);
            put_u16(&mut array, last);
            put_u32(&mut array, to_u32(body_at, "a bitmap index subtable")?);
            body_at += body.len();
            bodies.push(body);
        }
        for b in bodies {
            array.extend(b);
        }
        let mut r = record.to_vec();
        r.get_mut(8..12)
            .ok_or_else(|| l.bad("has a short strike"))?
            .copy_from_slice(&to_u32(subtables.len(), "the index subtable count")?.to_be_bytes());
        r.get_mut(4..8).ok_or_else(|| l.bad("has a short strike"))?.copy_from_slice(&to_u32(array.len(), "the index tables")?.to_be_bytes());
        r.get_mut(40..42).ok_or_else(|| l.bad("has a short strike"))?.copy_from_slice(&start.to_be_bytes());
        r.get_mut(42..44).ok_or_else(|| l.bad("has a short strike"))?.copy_from_slice(&end.to_be_bytes());
        records.push(r);
        arrays.push(array);
    }
    let mut at = 8 + 48 * records.len();
    for (r, a) in records.iter_mut().zip(&arrays) {
        r.get_mut(0..4).ok_or_else(|| l.bad("has a short strike"))?.copy_from_slice(&to_u32(at, "the bitmap location table")?.to_be_bytes());
        at += a.len();
    }
    for r in records {
        out.extend(r);
    }
    for a in arrays {
        out.extend(a);
    }
    Ok((out, out_data))
}

/// The index subtable `st` (from its header on), its images appended to `data` at `image_at`.
fn index_body(st: &IndexSubtable<'_>, first: u16, last: u16, image_at: usize, data: &mut Vec<u8>) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    let consecutive = st.glyphs.windows(2).all(|w| matches!(w, [a, b] if b.0 == a.0 + 1));
    // Format 2 needs consecutive glyphs; a subset with gaps is written as format 5.
    let index_format = if st.index_format == 2 && !consecutive { 5 } else { st.index_format };
    put_u16(&mut body, index_format);
    put_u16(&mut body, st.image_format);
    put_u32(&mut body, to_u32(image_at, "the bitmap data table")?);
    match index_format {
        1 | 3 => {
            let mut offsets = Vec::with_capacity(usize::from(last - first) + 2);
            let mut glyphs = st.glyphs.iter().peekable();
            let mut rel = 0usize;
            for g in first..=last {
                offsets.push(rel);
                if let Some((_, image)) = glyphs.next_if(|(id, _)| *id == g) {
                    data.extend_from_slice(image);
                    rel += image.len();
                }
            }
            offsets.push(rel);
            for o in &offsets {
                if index_format == 1 {
                    put_u32(&mut body, to_u32(*o, "a bitmap offset")?);
                } else {
                    put_u16(&mut body, to_u16(*o, "a bitmap offset")?);
                }
            }
            if index_format == 3 && offsets.len() % 2 == 1 {
                put_u16(&mut body, 0);
            }
        }
        2 | 5 => {
            let (size, metrics) = st.fixed.ok_or_else(|| TablesError::invalid("a fixed-size bitmap index subtable lost its metrics"))?;
            put_u32(&mut body, size);
            body.extend_from_slice(metrics);
            if index_format == 5 {
                put_u32(&mut body, to_u32(st.glyphs.len(), "the bitmap glyph count")?);
                for (g, _) in &st.glyphs {
                    put_u16(&mut body, *g);
                }
                if st.glyphs.len() % 2 == 1 {
                    put_u16(&mut body, 0);
                }
            }
            for (_, image) in &st.glyphs {
                data.extend_from_slice(image);
            }
        }
        _ => {
            put_u32(&mut body, to_u32(st.glyphs.len(), "the bitmap glyph count")?);
            let mut rel = 0usize;
            for (g, image) in &st.glyphs {
                put_u16(&mut body, *g);
                put_u16(&mut body, to_u16(rel, "a bitmap offset")?);
                data.extend_from_slice(image);
                rel += image.len();
            }
            put_u16(&mut body, 0);
            put_u16(&mut body, to_u16(rel, "a bitmap offset")?);
        }
    }
    Ok(body)
}

/// Glyphs `parts` reaches from `glyphs`, in one pass or until nothing is added, each glyph
/// asked once.
pub(crate) fn close_over(glyphs: &mut BTreeSet<u32>, n: usize, repeat: bool, mut parts: impl FnMut(u32, &mut Vec<u32>) -> Result<()>) -> Result<()> {
    let mut todo: Vec<u32> = glyphs.iter().copied().collect();
    let mut found = Vec::new();
    while !todo.is_empty() {
        let mut next = Vec::new();
        for g in todo {
            found.clear();
            parts(g, &mut found)?;
            for p in found.iter().copied() {
                if (p as usize) < n && glyphs.insert(p) {
                    next.push(p);
                }
            }
        }
        if !repeat {
            break;
        }
        todo = next;
    }
    Ok(())
}

#[cfg(test)]
#[path = "other_tests.rs"]
mod tests;
