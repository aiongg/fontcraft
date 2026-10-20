//! Variable `COLR` (version 1) in instances. fontTools' instancer leaves `COLR` as it is; FontCraft
//! instances it as the instancer instances `GDEF`: the item variation store is rebased (or goes
//! with the last axis), and each varied field of a paint, colour stop or clip box takes the
//! delta the location folds into the default, rounded, in the field's own units. While the
//! store keeps regions, the fields keep their variation indexes; when it goes, every index
//! becomes "no variation" and the delta-set index map goes too.
//!
//! The paint formats and their fields follow the OpenType `COLR` chapter.

use std::collections::HashSet;

use read_fonts::tables::variations::DeltaSetIndexMap;
use read_fonts::{FontData, FontRead};
use write_fonts::from_obj::ToOwnedTable;
use write_fonts::tables::variations::ItemVariationStore as WriteStore;

use super::ivs::StoreDeltas;
use super::ot_round;
use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

/// Most paint tables, colour lines and clip boxes visited in one `COLR` table.
const MAX_NODES: usize = 1 << 20;
const NO_VARIATION: u32 = 0xFFFF_FFFF;

fn bad(what: impl Into<String>) -> TablesError {
    TablesError::invalid(what)
}

fn u8_at(d: &[u8], at: usize) -> Result<usize> {
    d.get(at).map(|b| usize::from(*b)).ok_or_else(|| bad("the COLR table is truncated"))
}

fn u16_at(d: &[u8], at: usize) -> Result<usize> {
    d.get(at..at.saturating_add(2))
        .and_then(|b| <[u8; 2]>::try_from(b).ok())
        .map(|b| usize::from(u16::from_be_bytes(b)))
        .ok_or_else(|| bad("the COLR table is truncated"))
}

fn u24_at(d: &[u8], at: usize) -> Result<usize> {
    Ok((u8_at(d, at)? << 16) | u16_at(d, at.saturating_add(1))?)
}

fn u32_at(d: &[u8], at: usize) -> Result<u32> {
    d.get(at..at.saturating_add(4))
        .and_then(|b| <[u8; 4]>::try_from(b).ok())
        .map(u32::from_be_bytes)
        .ok_or_else(|| bad("the COLR table is truncated"))
}

/// How a varied field is stored.
#[derive(Clone, Copy)]
enum Kind {
    /// FWORD or F2Dot14.
    Signed16,
    /// UFWORD.
    Unsigned16,
    /// Fixed.
    Fixed,
}

/// One record whose fields vary: their positions and kinds, then the position of its
/// `varIndexBase`.
struct Record {
    fields: Vec<(usize, Kind)>,
    base: usize,
}

/// Every varied record of the table, each once.
fn var_records(data: &[u8]) -> Result<Vec<Record>> {
    use Kind::{Fixed, Signed16 as S, Unsigned16 as U};
    let mut paints: Vec<usize> = Vec::new();
    let base_list = u32_at(data, 14)? as usize;
    if base_list != 0 {
        let count = u32_at(data, base_list)? as usize;
        for k in 0..count.min(MAX_NODES) {
            let at = base_list + 4 + 6 * k;
            paints.push(base_list + u32_at(data, at + 2)? as usize);
        }
    }
    let layer_list = u32_at(data, 18)? as usize;
    if layer_list != 0 {
        let count = u32_at(data, layer_list)? as usize;
        for k in 0..count.min(MAX_NODES) {
            paints.push(layer_list + u32_at(data, layer_list + 4 + 4 * k)? as usize);
        }
    }
    let mut records: Vec<Record> = Vec::new();
    let mut seen: HashSet<usize> = HashSet::new();
    let mut lines: Vec<usize> = Vec::new();
    let mut nodes = 0usize;
    let mut visit = |seen: &mut HashSet<usize>, at: usize| -> Result<bool> {
        nodes += 1;
        if nodes > MAX_NODES {
            return Err(TablesError::TooLarge { what: "the paint tables of a COLR table", limit: MAX_NODES });
        }
        Ok(seen.insert(at))
    };
    while let Some(at) = paints.pop() {
        if !visit(&mut seen, at)? {
            continue;
        }
        let format = u8_at(data, at)?;
        let child = |rel: usize| -> Result<usize> { Ok(at + u24_at(data, at + rel)?) };
        let fields = |list: &[(usize, Kind)], base: usize| Record { fields: list.iter().map(|(o, k)| (at + o, *k)).collect(), base: at + base };
        match format {
            3 => records.push(fields(&[(3, S)], 5)),
            5 => {
                lines.push(child(1)?);
                records.push(fields(&[(4, S), (6, S), (8, S), (10, S), (12, S), (14, S)], 16));
            }
            7 => {
                lines.push(child(1)?);
                records.push(fields(&[(4, S), (6, S), (8, U), (10, S), (12, S), (14, U)], 16));
            }
            9 => {
                lines.push(child(1)?);
                records.push(fields(&[(4, S), (6, S), (8, S), (10, S)], 12));
            }
            10 | 12 | 14 | 16 | 18 | 20 | 22 | 24 | 26 | 28 | 30 => paints.push(child(1)?),
            13 => {
                paints.push(child(1)?);
                let affine = child(4)?;
                if visit(&mut seen, affine)? {
                    let list: Vec<(usize, Kind)> = (0..6).map(|k| (affine + 4 * k, Fixed)).collect();
                    records.push(Record { fields: list, base: affine + 24 });
                }
            }
            15 | 17 | 29 => {
                paints.push(child(1)?);
                records.push(fields(&[(4, S), (6, S)], 8));
            }
            19 | 31 => {
                paints.push(child(1)?);
                records.push(fields(&[(4, S), (6, S), (8, S), (10, S)], 12));
            }
            21 | 25 => {
                paints.push(child(1)?);
                records.push(fields(&[(4, S)], 6));
            }
            23 | 27 => {
                paints.push(child(1)?);
                records.push(fields(&[(4, S), (6, S), (8, S)], 10));
            }
            32 => {
                paints.push(child(1)?);
                paints.push(child(5)?);
            }
            _ => {}
        }
    }
    for line in lines {
        if !visit(&mut seen, line)? {
            continue;
        }
        let stops = u16_at(data, line + 1)?;
        for k in 0..stops {
            let stop = line + 3 + 10 * k;
            records.push(Record { fields: vec![(stop, S), (stop + 4, S)], base: stop + 6 });
        }
    }
    let clip_list = u32_at(data, 22)? as usize;
    if clip_list != 0 {
        let count = u32_at(data, clip_list + 1)? as usize;
        for k in 0..count.min(MAX_NODES) {
            let clip = clip_list + 5 + 7 * k;
            let bx = clip_list + u24_at(data, clip + 4)?;
            if visit(&mut seen, bx)? && u8_at(data, bx)? == 2 {
                records.push(Record { fields: vec![(bx + 1, S), (bx + 3, S), (bx + 5, S), (bx + 7, S)], base: bx + 9 });
            }
        }
    }
    Ok(records)
}

fn patch(table: &mut [u8], at: usize, kind: Kind, delta: f64) -> Result<()> {
    let delta = ot_round(delta);
    let width = if matches!(kind, Kind::Fixed) { 4 } else { 2 };
    let slot = table.get_mut(at..at + width).ok_or_else(|| bad("the COLR table is truncated"))?;
    match kind {
        Kind::Signed16 => {
            let v = f64::from(i16::from_be_bytes([slot.first().copied().unwrap_or(0), slot.get(1).copied().unwrap_or(0)]));
            slot.copy_from_slice(&((v + delta).clamp(-32768.0, 32767.0) as i16).to_be_bytes());
        }
        Kind::Unsigned16 => {
            let v = f64::from(u16::from_be_bytes([slot.first().copied().unwrap_or(0), slot.get(1).copied().unwrap_or(0)]));
            slot.copy_from_slice(&((v + delta).clamp(0.0, 65535.0) as u16).to_be_bytes());
        }
        Kind::Fixed => {
            let bytes = <[u8; 4]>::try_from(&*slot).map_err(|_| bad("the COLR table is truncated"))?;
            let v = f64::from(i32::from_be_bytes(bytes));
            slot.copy_from_slice(&((v + delta).clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32).to_be_bytes());
        }
    }
    Ok(())
}

/// The offset of the item variation store of a `COLR` table with variation data.
pub(crate) fn store_offset(font: &SfntFont) -> Option<usize> {
    let data = font.get(*b"COLR")?;
    if u16_at(data, 0).ok()? < 1 {
        return None;
    }
    let offset = u32_at(data, 30).ok()? as usize;
    (offset != 0).then_some(offset)
}

/// Adds `deltas` (by outer and inner index) to every varied field of the `COLR` table of
/// `out`, and puts `store` in place of its item variation store; without one, the store, the
/// index map and every variation index go.
pub(crate) fn apply(out: &mut SfntFont, deltas: &StoreDeltas, store: Option<WriteStore>) -> Result<()> {
    if store_offset(out).is_none() {
        return Ok(());
    }
    let Some(data) = out.get(*b"COLR") else { return Ok(()) };
    let mut table = data.to_vec();
    let map_offset = u32_at(data, 26)? as usize;
    let map = if map_offset == 0 {
        None
    } else {
        let bytes = data.get(map_offset..).ok_or_else(|| bad("the COLR index map offset points past the end of the table"))?;
        Some(DeltaSetIndexMap::read(FontData::new(bytes)).map_err(|e| bad(format!("the COLR index map cannot be read: {e}")))?)
    };
    let keep = store.is_some();
    for record in var_records(data)? {
        let base = u32_at(data, record.base)?;
        if base == NO_VARIATION {
            continue;
        }
        for (k, (at, kind)) in record.fields.iter().enumerate() {
            let idx = base.saturating_add(k as u32);
            let (outer, inner) = match &map {
                Some(m) => match m.get(idx) {
                    Ok(i) => (i.outer, i.inner),
                    Err(_) => continue,
                },
                None => ((idx >> 16) as u16, (idx & 0xFFFF) as u16),
            };
            patch(&mut table, *at, *kind, deltas.get(outer, inner))?;
        }
        if !keep && let Some(slot) = table.get_mut(record.base..record.base + 4) {
            slot.copy_from_slice(&NO_VARIATION.to_be_bytes());
        }
    }
    let read = read_fonts::tables::colr::Colr::read(FontData::new(&table)).map_err(|e| bad(format!("the COLR table cannot be read: {e}")))?;
    let mut colr: write_fonts::tables::colr::Colr = read.to_owned_table();
    if keep {
        colr.item_variation_store = write_fonts::NullableOffsetMarker::new(store);
    } else {
        colr.item_variation_store = write_fonts::NullableOffsetMarker::new(None);
        colr.var_index_map = write_fonts::NullableOffsetMarker::new(None);
    }
    let bytes = write_fonts::dump_table(&colr).map_err(|e| bad(format!("the COLR table could not be written again: {e}")))?;
    out.insert(*b"COLR", bytes);
    Ok(())
}

/// Each varied record's raw field values and variation index base, in the order they are found.
#[cfg(test)]
pub(super) fn varied_values(data: &[u8]) -> Vec<(Vec<i64>, u32)> {
    #![allow(clippy::unwrap_used)]
    var_records(data)
        .unwrap()
        .iter()
        .map(|r| {
            let values = r
                .fields
                .iter()
                .map(|(at, kind)| match kind {
                    Kind::Signed16 => i64::from(u16_at(data, *at).unwrap() as i16),
                    Kind::Unsigned16 => u16_at(data, *at).unwrap() as i64,
                    Kind::Fixed => i64::from(u32_at(data, *at).unwrap() as i32),
                })
                .collect();
            (values, u32_at(data, r.base).unwrap())
        })
        .collect()
}
