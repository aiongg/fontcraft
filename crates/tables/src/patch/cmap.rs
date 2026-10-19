//! `tables.rebuildCmap`: the cmap subtables written again from one mapping.
//!
//! The mapping is the union of the Unicode subtables (platform 0, and platform 3 encodings 1
//! and 10); where they disagree, platform 3 wins, and among those the later encoding record.
//! The new table has format 4 subtables for (0, 3) and (3, 1), format 12 subtables for (0, 4)
//! and (3, 10) when a code point lies beyond the Basic Multilingual Plane, and the format 14
//! variation sequences (0, 5) copied byte for byte. Other subtables (Macintosh, symbol) are
//! left out.

use std::collections::BTreeMap;

use read_fonts::tables::cmap::Cmap;
use read_fonts::{FontData, FontRead};

use super::{be_u32, missing};
use crate::limits::{MAX_CODE_POINTS, MAX_TTX_CMAP_ENTRIES};
use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

const CMAP: Tag = *b"cmap";

/// A code point the subtables mapped to different glyphs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CmapConflict {
    pub codepoint: u32,
    pub kept: u16,
    pub dropped: u16,
}

/// The result of [`rebuild_cmap`].
#[derive(Clone, Debug, PartialEq)]
pub struct CmapRebuild {
    pub font: SfntFont,
    pub mappings: usize,
    pub conflicts: Vec<CmapConflict>,
}

fn invalid(what: impl std::fmt::Display) -> TablesError {
    TablesError::invalid(format!("the cmap table cannot be read: {what}"))
}

/// The font with its cmap rebuilt.
pub fn rebuild_cmap(font: &SfntFont) -> Result<CmapRebuild> {
    let data = font.get(CMAP).ok_or_else(|| missing(CMAP))?;
    let cmap = Cmap::read(FontData::new(data)).map_err(invalid)?;
    let mut records: Vec<(u16, u16, u32)> =
        cmap.encoding_records().iter().map(|r| (r.platform_id() as u16, r.encoding_id(), r.subtable_offset().to_u32())).collect();
    // Platform 0 first, so platform 3 overrides it.
    records.sort_by_key(|(p, e, _)| (u16::from(*p == 3), *e));
    let mut map: BTreeMap<u32, u16> = BTreeMap::new();
    let mut conflicts = Vec::new();
    let mut variations: Option<Vec<u8>> = None;
    let mut read = 0usize;
    for (platform, encoding, offset) in records {
        let unicode = platform == 0 || (platform == 3 && matches!(encoding, 1 | 10));
        if !unicode {
            continue;
        }
        let at = usize::try_from(offset).map_err(invalid)?;
        if platform == 0 && encoding == 5 {
            let len =
                be_u32(data, at.saturating_add(2)).and_then(|l| usize::try_from(l).ok()).ok_or_else(|| invalid("a format 14 subtable is cut off"))?;
            let bytes = data.get(at..at.saturating_add(len)).ok_or_else(|| invalid("a format 14 subtable is cut off"))?;
            variations.get_or_insert_with(|| bytes.to_vec());
            continue;
        }
        let sub = read_fonts::tables::cmap::CmapSubtable::read(FontData::new(data.get(at..).ok_or_else(|| invalid("an offset is past the end"))?))
            .map_err(invalid)?;
        for (cp, gid) in sub.iter().take(MAX_CODE_POINTS) {
            read = read.saturating_add(1);
            if read > MAX_TTX_CMAP_ENTRIES {
                return Err(TablesError::TooLarge { what: "the cmap mappings", limit: MAX_TTX_CMAP_ENTRIES });
            }
            let Ok(gid) = u16::try_from(gid.to_u32()) else { continue };
            if let Some(old) = map.insert(cp, gid)
                && old != gid
            {
                conflicts.retain(|c: &CmapConflict| c.codepoint != cp);
                conflicts.push(CmapConflict { codepoint: cp, kept: gid, dropped: old });
            }
        }
    }
    let format4 = format_4(&map)?;
    let beyond = map.keys().any(|cp| *cp > 0xFFFF);
    let format12 = if beyond { Some(format_12(&map)?) } else { None };
    let mut subtables: Vec<(u16, u16, &[u8])> = vec![(0, 3, &format4)];
    if let Some(f12) = &format12 {
        subtables.push((0, 4, f12));
    }
    if let Some(f14) = &variations {
        subtables.push((0, 5, f14));
    }
    subtables.push((3, 1, &format4));
    if let Some(f12) = &format12 {
        subtables.push((3, 10, f12));
    }
    let table = assemble(&subtables)?;
    let mut out = font.clone();
    out.insert(CMAP, table);
    Ok(CmapRebuild { font: out, mappings: map.len(), conflicts })
}

/// The cmap table holding these subtables; identical subtables are stored once.
fn assemble(subtables: &[(u16, u16, &[u8])]) -> Result<Vec<u8>> {
    let mut header: Vec<u8> = Vec::new();
    header.extend(0u16.to_be_bytes());
    header.extend((subtables.len() as u16).to_be_bytes());
    let mut body: Vec<u8> = Vec::new();
    let mut placed: Vec<(&[u8], u32)> = Vec::new();
    let start = 4usize.saturating_add(subtables.len().saturating_mul(8));
    for (platform, encoding, bytes) in subtables {
        let offset = match placed.iter().find(|(b, _)| b == bytes) {
            Some((_, off)) => *off,
            None => {
                let off = u32::try_from(start.saturating_add(body.len()))
                    .map_err(|_| TablesError::TooLarge { what: "the cmap table", limit: u32::MAX as usize })?;
                body.extend_from_slice(bytes);
                placed.push((bytes, off));
                off
            }
        };
        header.extend(platform.to_be_bytes());
        header.extend(encoding.to_be_bytes());
        header.extend(offset.to_be_bytes());
    }
    header.extend(body);
    Ok(header)
}

/// A format 4 subtable of the Basic Multilingual Plane part of `map`: one segment per run of
/// consecutive code points, with a glyph array when the run's glyphs are not consecutive.
fn format_4(map: &BTreeMap<u32, u16>) -> Result<Vec<u8>> {
    // (start, end, glyphs)
    let mut runs: Vec<(u16, u16, Vec<u16>)> = Vec::new();
    for (&cp, &gid) in map.range(..0xFFFF) {
        let Ok(cp) = u16::try_from(cp) else { continue };
        match runs.last_mut() {
            Some((_, end, glyphs)) if end.checked_add(1) == Some(cp) => {
                *end = cp;
                glyphs.push(gid);
            }
            _ => runs.push((cp, cp, vec![gid])),
        }
    }
    let seg_count = runs.len().saturating_add(1);
    let too_big = || TablesError::Unsupported("the Basic Multilingual Plane mapping does not fit a format 4 cmap subtable".into());
    let seg_u16 = u16::try_from(seg_count).ok().filter(|n| *n <= 0x7FFF).ok_or_else(too_big)?;
    let consecutive = |glyphs: &[u16]| glyphs.windows(2).all(|w| matches!(w, [a, b] if a.checked_add(1) == Some(*b)));
    let (mut ends, mut starts, mut deltas, mut range_offsets, mut array) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::<u16>::new());
    for (i, (start, end, glyphs)) in runs.iter().enumerate() {
        ends.push(*end);
        starts.push(*start);
        if consecutive(glyphs) {
            let first = glyphs.first().copied().unwrap_or(0);
            deltas.push(first.wrapping_sub(*start));
            range_offsets.push(0u16);
        } else {
            deltas.push(0);
            // From this idRangeOffset entry to the run's first entry of the glyph array.
            let offset = seg_count.saturating_sub(i).saturating_add(array.len()).saturating_mul(2);
            range_offsets.push(u16::try_from(offset).map_err(|_| too_big())?);
            array.extend_from_slice(glyphs);
        }
    }
    ends.push(0xFFFF);
    starts.push(0xFFFF);
    deltas.push(1);
    range_offsets.push(0);
    let length = 16usize.saturating_add(seg_count.saturating_mul(8)).saturating_add(array.len().saturating_mul(2));
    let length = u16::try_from(length).map_err(|_| too_big())?;
    let selector = (usize::BITS - 1).saturating_sub(seg_count.leading_zeros());
    let search = 2u16.checked_shl(selector).unwrap_or(0);
    let mut out = Vec::with_capacity(usize::from(length));
    for v in [4u16, length, 0, seg_u16.saturating_mul(2), search, selector as u16, seg_u16.saturating_mul(2).saturating_sub(search)] {
        out.extend(v.to_be_bytes());
    }
    let mut words = |vs: &[u16]| vs.iter().for_each(|v| out.extend(v.to_be_bytes()));
    words(&ends);
    words(&[0]);
    words(&starts);
    words(&deltas);
    words(&range_offsets);
    words(&array);
    Ok(out)
}

/// A format 12 subtable of all of `map`: one group per run of consecutive code points with
/// consecutive glyphs.
fn format_12(map: &BTreeMap<u32, u16>) -> Result<Vec<u8>> {
    let mut groups: Vec<(u32, u32, u32)> = Vec::new();
    for (&cp, &gid) in map {
        let gid = u32::from(gid);
        match groups.last_mut() {
            Some((start, end, first)) if end.checked_add(1) == Some(cp) && first.checked_add(cp.saturating_sub(*start)) == Some(gid) => *end = cp,
            _ => groups.push((cp, cp, gid)),
        }
    }
    let n = u32::try_from(groups.len()).map_err(|_| TablesError::TooLarge { what: "the cmap groups", limit: u32::MAX as usize })?;
    let length =
        n.checked_mul(12).and_then(|v| v.checked_add(16)).ok_or(TablesError::TooLarge { what: "the cmap table", limit: u32::MAX as usize })?;
    let mut out = Vec::new();
    out.extend(12u16.to_be_bytes());
    out.extend(0u16.to_be_bytes());
    out.extend(length.to_be_bytes());
    out.extend(0u32.to_be_bytes());
    out.extend(n.to_be_bytes());
    for (start, end, gid) in groups {
        out.extend(start.to_be_bytes());
        out.extend(end.to_be_bytes());
        out.extend(gid.to_be_bytes());
    }
    Ok(out)
}
