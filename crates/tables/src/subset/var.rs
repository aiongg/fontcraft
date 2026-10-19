//! Subsetting the variation tables (M12.19), ported from fontTools' subsetter (MIT, Copyright
//! 2013 Google, Inc.):
//!
//! - `fvar`, `avar`, `STAT`, `MVAR` and `cvar` are copied, as pyftsubset leaves them
//!   (`--no-subset-tables`); `cvar` goes with the other hinting tables under `hinting: false`.
//! - `gvar` keeps the variation data of the output glyphs; emptied glyphs and an emptied
//!   `.notdef` have none. The shared tuples are copied, so every glyph's data stays valid.
//! - `HVAR` and `VVAR` keep the index map entries of the output glyphs and the delta sets they
//!   use ([`store`]); without an advance map, the store's first subtable is reordered so that
//!   the implicit glyph → row mapping still holds.
//! - The `GDEF` store is pruned to the delta sets `GDEF` and `GPOS` still use
//!   ([`prune_layout_store`], called by the layout module).
//!
//! [`name_ids_in_use`] lists the name IDs the output's `fvar`, `STAT`, `CPAL` and layout feature
//! parameters refer to, which the name table keeps besides the requested IDs.

pub(crate) mod store;

use std::collections::{BTreeMap, BTreeSet};

use read_fonts::tables::layout::{FeatureList, FeatureParams};
use read_fonts::{FontData, FontRead};

use self::store::{NO_VARIATION, Store, read_index_map, write_glyph_map};
use super::Plan;
use crate::Result;
use crate::reorder::be::{Data, put_u16, put_u32, to_u16, to_u32};
use crate::sfnt::{SfntFont, Tag};

/// Tables copied unchanged.
const COPIED: [Tag; 5] = [*b"fvar", *b"avar", *b"STAT", *b"MVAR", *b"cvar"];

/// Writes the subset of the tables this module handles from `input` into `output`.
pub fn apply(plan: &Plan, input: &SfntFont, output: &mut SfntFont) -> Result<()> {
    for tag in COPIED {
        if let Some(bytes) = input.get(tag) {
            output.insert(tag, bytes.to_vec());
        }
    }
    if let Some(bytes) = input.get(*b"gvar") {
        output.insert(*b"gvar", subset_gvar(plan, bytes)?);
    }
    for tag in [*b"HVAR", *b"VVAR"] {
        if let Some(bytes) = input.get(tag) {
            output.insert(tag, subset_metrics_variations(plan, tag, bytes)?);
        }
    }
    Ok(())
}

// ── gvar ──

fn subset_gvar(plan: &Plan, bytes: &[u8]) -> Result<Vec<u8>> {
    let d = Data::new(*b"gvar", bytes);
    let (major, minor) = (d.u16(0)?, d.u16(2)?);
    let axis_count = d.u16(4)?;
    let shared_count = d.u16(6)?;
    let shared_at = d.u32_usize(8)?;
    let glyph_count = usize::from(d.u16(12)?);
    let flags = d.u16(14)?;
    let data_at = d.u32_usize(16)?;
    let long = flags & 1 != 0;
    let offset = |g: usize| -> Result<usize> { if long { d.u32_usize(20 + 4 * g) } else { Ok(2 * usize::from(d.u16(20 + 2 * g)?)) } };
    let shared = d.slice(shared_at, 2 * usize::from(axis_count) * usize::from(shared_count))?;
    let empty_notdef = plan.options.notdef_glyph && !plan.options.notdef_outline;
    let mut glyphs: Vec<&[u8]> = Vec::with_capacity(plan.new_to_old.len());
    for old in &plan.new_to_old {
        let g = *old as usize;
        if plan.emptied.contains(old) || (g == 0 && empty_notdef) || g >= glyph_count {
            glyphs.push(&[]);
            continue;
        }
        let (start, end) = (offset(g)?, offset(g + 1)?);
        let len = end.checked_sub(start).ok_or_else(|| d.bad("has decreasing glyph variation offsets"))?;
        glyphs.push(d.slice(data_at.checked_add(start).ok_or_else(|| d.bad("has an offset past its end"))?, len)?);
    }
    let total: usize = glyphs.iter().map(|g| g.len() + g.len() % 2).sum();
    let long = total > 2 * usize::from(u16::MAX);
    let n = glyphs.len();
    let offsets_len = (n + 1) * if long { 4 } else { 2 };
    let new_shared_at = 20 + offsets_len;
    let new_data_at = new_shared_at + shared.len();
    let mut out = Vec::with_capacity(new_data_at + total);
    put_u16(&mut out, major);
    put_u16(&mut out, minor);
    put_u16(&mut out, axis_count);
    put_u16(&mut out, shared_count);
    put_u32(&mut out, to_u32(new_shared_at, "the gvar table")?);
    put_u16(&mut out, to_u16(n, "the glyph count")?);
    put_u16(&mut out, (flags & !1) | u16::from(long));
    put_u32(&mut out, to_u32(new_data_at, "the gvar table")?);
    let mut at = 0usize;
    for g in glyphs.iter().map(|g| g.len() + g.len() % 2).chain(std::iter::once(0)) {
        if long {
            put_u32(&mut out, to_u32(at, "the gvar table")?);
        } else {
            put_u16(&mut out, to_u16(at / 2, "the gvar table")?);
        }
        at += g;
    }
    out.extend_from_slice(shared);
    for g in glyphs {
        out.extend_from_slice(g);
        if g.len() % 2 == 1 {
            out.push(0);
        }
    }
    Ok(out)
}

// ── HVAR and VVAR ──

fn subset_metrics_variations(plan: &Plan, tag: Tag, bytes: &[u8]) -> Result<Vec<u8>> {
    let d = Data::new(tag, bytes);
    let (major, minor) = (d.u16(0)?, d.u16(2)?);
    // The store, then the advance map, then the side-bearing maps (and VVAR's origin map).
    let map_count = if &tag == b"HVAR" { 3 } else { 4 };
    let store_at = d.u32_usize(4)?;
    let store_bytes = bytes.get(store_at..).ok_or_else(|| d.bad("has its variation store past its end"))?;
    let store = Store::read(tag, store_bytes)?;
    let n = plan.num_glyphs;
    let mut maps: Vec<Option<Vec<u32>>> = Vec::with_capacity(map_count);
    for i in 0..map_count {
        let at = d.u32_usize(8 + 4 * i)?;
        maps.push(if at == 0 { None } else { Some(read_index_map(d, at, n)?) });
    }

    // The output glyphs other than the emptied ones, as pyftsubset's `glyphs_retained`.
    let retained: Vec<u32> = plan.new_to_old.iter().copied().filter(|g| !plan.emptied.contains(g)).collect();
    let mut used = BTreeSet::new();
    let mut first = BTreeSet::new();
    let mut retain_first = false;
    match maps.first().and_then(Option::as_ref) {
        Some(map) => used.extend(retained.iter().filter_map(|g| map.get(*g as usize).copied())),
        None => {
            used.extend(retained.iter().copied());
            first.extend(retained.iter().map(|g| *g as u16));
            retain_first = plan.options.retain_gids;
        }
    }
    for map in maps.iter().skip(1).flatten() {
        used.extend(retained.iter().filter_map(|g| map.get(*g as usize).copied()));
    }
    let (subset, varidx_map) = store.subset(&used, retain_first, &first)?;

    let mut new_maps: Vec<Option<Vec<u8>>> = Vec::with_capacity(map_count);
    for map in &maps {
        let Some(map) = map else {
            new_maps.push(None);
            continue;
        };
        let mut mapping = Vec::with_capacity(plan.new_to_old.len());
        for old in &plan.new_to_old {
            let v = if plan.emptied.contains(old) {
                0
            } else {
                let v = map.get(*old as usize).copied().unwrap_or(NO_VARIATION);
                varidx_map.get(&v).copied().unwrap_or(NO_VARIATION)
            };
            mapping.push(v);
        }
        new_maps.push(Some(write_glyph_map(&mapping)?));
    }

    let store_bytes = subset.write()?;
    let header = 4 + 4 * (map_count + 1);
    let mut out = Vec::new();
    put_u16(&mut out, major);
    put_u16(&mut out, minor);
    put_u32(&mut out, to_u32(header, "the metrics variations table")?);
    let mut at = header + store_bytes.len();
    for m in &new_maps {
        match m {
            Some(m) => {
                put_u32(&mut out, to_u32(at, "the metrics variations table")?);
                at += m.len();
            }
            None => put_u32(&mut out, 0),
        }
    }
    out.extend(store_bytes);
    for m in new_maps.into_iter().flatten() {
        out.extend(m);
    }
    Ok(out)
}

// ── Layout variation store ──

/// The delta sets a layout table's devices use, renumbered: GDEF's store keeps only the delta
/// sets in `used`, as `_pruneGDEF` does. Returns the new store bytes (`None` when no
/// variation data is left) and the old → new variation index map; an index not in the map
/// becomes "no variation".
pub(crate) fn prune_layout_store(gdef: &[u8], used: &BTreeSet<u32>) -> Result<Option<(Vec<u8>, BTreeMap<u32, u32>)>> {
    let d = Data::new(*b"GDEF", gdef);
    if d.u16(0)? != 1 || d.u16(2)? < 3 {
        return Ok(None);
    }
    let at = d.u32_usize(14)?;
    if at == 0 {
        return Ok(None);
    }
    let bytes = gdef.get(at..).ok_or_else(|| d.bad("has its variation store past its end"))?;
    let store = Store::read(*b"GDEF", bytes)?;
    let (subset, map) = store.subset(used, false, &BTreeSet::new())?;
    if subset.is_empty() {
        return Ok(Some((Vec::new(), map)));
    }
    Ok(Some((subset.write()?, map)))
}

// ── Name IDs in use ──

/// Name IDs the tables of `font` refer to: `fvar` axes and instances, `STAT` axes, values and
/// elided fallback name, `CPAL` palette and entry labels, and the `size`, stylistic set and
/// character variant parameters of GSUB and GPOS features, as fontTools' `NameRecordVisitor`
/// collects them. Unreadable parts add nothing.
pub(crate) fn name_ids_in_use(font: &SfntFont) -> BTreeSet<u16> {
    let mut ids = BTreeSet::new();
    if let Some(bytes) = font.get(*b"fvar") {
        let _ = fvar_names(Data::new(*b"fvar", bytes), &mut ids);
    }
    if let Some(bytes) = font.get(*b"STAT") {
        let _ = stat_names(Data::new(*b"STAT", bytes), &mut ids);
    }
    if let Some(bytes) = font.get(*b"CPAL") {
        let _ = cpal_names(Data::new(*b"CPAL", bytes), &mut ids);
    }
    for tag in [*b"GSUB", *b"GPOS"] {
        if let Some(bytes) = font.get(tag) {
            feature_names(bytes, &mut ids);
        }
    }
    ids
}

fn fvar_names(d: Data<'_>, ids: &mut BTreeSet<u16>) -> Result<()> {
    let axes_at = usize::from(d.u16(4)?);
    let axis_count = usize::from(d.u16(8)?);
    let axis_size = usize::from(d.u16(10)?);
    let instance_count = usize::from(d.u16(12)?);
    let instance_size = usize::from(d.u16(14)?);
    for a in 0..axis_count {
        ids.insert(d.u16(axes_at + a * axis_size + 18)?);
    }
    let instances_at = axes_at + axis_count * axis_size;
    for i in 0..instance_count {
        let at = instances_at + i * instance_size;
        ids.insert(d.u16(at)?);
        if instance_size >= 4 * axis_count + 6 {
            let ps = d.u16(at + 4 + 4 * axis_count)?;
            if ps != 0xFFFF {
                ids.insert(ps);
            }
        }
    }
    Ok(())
}

fn stat_names(d: Data<'_>, ids: &mut BTreeSet<u16>) -> Result<()> {
    let minor = d.u16(2)?;
    let axis_size = usize::from(d.u16(4)?);
    let axis_count = usize::from(d.u16(6)?);
    let axes_at = d.u32_usize(8)?;
    let value_count = usize::from(d.u16(12)?);
    let values_at = d.u32_usize(14)?;
    if minor >= 1 {
        ids.insert(d.u16(18)?);
    }
    for a in 0..axis_count {
        ids.insert(d.u16(axes_at + a * axis_size + 4)?);
    }
    for v in 0..value_count {
        let at = values_at + usize::from(d.u16(values_at + 2 * v)?);
        ids.insert(d.u16(at + 6)?);
    }
    Ok(())
}

fn cpal_names(d: Data<'_>, ids: &mut BTreeSet<u16>) -> Result<()> {
    if d.u16(0)? != 1 {
        return Ok(());
    }
    let entries = usize::from(d.u16(2)?);
    let palettes = usize::from(d.u16(4)?);
    let after = 12 + 2 * palettes;
    let labels_at = d.u32_usize(after + 4)?;
    let entry_labels_at = d.u32_usize(after + 8)?;
    if labels_at != 0 {
        for p in 0..palettes {
            ids.insert(d.u16(labels_at + 2 * p)?);
        }
    }
    if entry_labels_at != 0 {
        for e in 0..entries {
            ids.insert(d.u16(entry_labels_at + 2 * e)?);
        }
    }
    Ok(())
}

fn feature_names(bytes: &[u8], ids: &mut BTreeSet<u16>) {
    let d = Data::new(*b"GSUB", bytes);
    let Ok(at) = d.u16(6) else { return };
    let Some(list) = bytes.get(usize::from(at)..) else { return };
    let Ok(list) = FeatureList::read(FontData::new(list)) else { return };
    for record in list.feature_records() {
        let Ok(feature) = record.feature(list.offset_data()) else { continue };
        let Some(Ok(params)) = feature.feature_params() else { continue };
        match params {
            FeatureParams::Size(p) => {
                ids.insert(p.name_entry());
            }
            FeatureParams::StylisticSet(p) => {
                ids.insert(p.ui_name_id().to_u16());
            }
            FeatureParams::CharacterVariant(p) => {
                ids.extend([p.feat_ui_label_name_id().to_u16(), p.feat_ui_tooltip_text_name_id().to_u16(), p.sample_text_name_id().to_u16()]);
                let first = p.first_param_ui_label_name_id().to_u16();
                let count = p.num_named_parameters();
                if first != 0 && count != 0 {
                    ids.extend((u32::from(first)..(u32::from(first) + u32::from(count)).min(0x1_0000)).map(|i| i as u16));
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "var_tests.rs"]
mod tests;
