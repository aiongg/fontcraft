//! Rescaling the deltas of tuple variation stores: `gvar` per glyph and `cvar`. Point numbers
//! and tuple records are copied; each tuple's packed deltas are decoded, changed and packed
//! again, so only the serialized data sizes change.

use crate::limits::{MAX_SFNT, MAX_TUPLE_VARIATIONS};
use crate::{Result, TablesError};

pub(super) const SHARED_POINT_NUMBERS: u16 = 0x8000;
pub(super) const COUNT_MASK: u16 = 0x0FFF;
pub(super) const EMBEDDED_PEAK_TUPLE: u16 = 0x8000;
pub(super) const INTERMEDIATE_REGION: u16 = 0x4000;
pub(super) const PRIVATE_POINT_NUMBERS: u16 = 0x2000;

const DELTAS_ARE_ZERO: u8 = 0x80;
const DELTAS_ARE_WORDS: u8 = 0x40;
const DELTA_RUN_COUNT_MASK: u8 = 0x3F;

pub(super) fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

pub(super) fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes([*b.get(at)?, *b.get(at + 1)?, *b.get(at + 2)?, *b.get(at + 3)?]))
}

pub(super) fn cut() -> TablesError {
    TablesError::invalid("the variation data is cut short")
}

/// Reads packed point numbers at `at`; gives how many points they name (`all` for "every
/// point") and where they end.
fn point_numbers(b: &[u8], mut at: usize, all: usize) -> Result<(usize, usize)> {
    let first = *b.get(at).ok_or_else(cut)?;
    at += 1;
    if first == 0 {
        return Ok((all, at));
    }
    let count = if first & 0x80 != 0 {
        let second = *b.get(at).ok_or_else(cut)?;
        at += 1;
        (usize::from(first & 0x7F) << 8) | usize::from(second)
    } else {
        usize::from(first)
    };
    let mut seen = 0;
    while seen < count {
        let control = *b.get(at).ok_or_else(cut)?;
        at += 1;
        let run = usize::from(control & 0x7F) + 1;
        let width = if control & 0x80 != 0 { 2 } else { 1 };
        at = at.checked_add(run * width).ok_or_else(cut)?;
        if at > b.len() {
            return Err(cut());
        }
        seen += run;
    }
    Ok((count, at))
}

/// Reads `count` packed deltas at `at`; gives them and where they end.
pub(super) fn read_deltas(b: &[u8], mut at: usize, count: usize) -> Result<(Vec<i32>, usize)> {
    let mut out = Vec::with_capacity(count.min(b.len().saturating_mul(64)));
    while out.len() < count {
        let control = *b.get(at).ok_or_else(cut)?;
        at += 1;
        let run = usize::from(control & DELTA_RUN_COUNT_MASK) + 1;
        let take = run.min(count - out.len());
        match control & (DELTAS_ARE_ZERO | DELTAS_ARE_WORDS) {
            DELTAS_ARE_ZERO => out.extend(std::iter::repeat_n(0, take)),
            DELTAS_ARE_WORDS => {
                for i in 0..run {
                    let v = u16_at(b, at + 2 * i).ok_or_else(cut)? as i16;
                    if i < take {
                        out.push(i32::from(v));
                    }
                }
                at += 2 * run;
            }
            0 => {
                for i in 0..run {
                    let v = *b.get(at + i).ok_or_else(cut)? as i8;
                    if i < take {
                        out.push(i32::from(v));
                    }
                }
                at += run;
            }
            _ => {
                // 32-bit deltas.
                for i in 0..run {
                    let v = u32_at(b, at + 4 * i).ok_or_else(cut)? as i32;
                    if i < take {
                        out.push(v);
                    }
                }
                at += 4 * run;
            }
        }
    }
    Ok((out, at))
}

/// Packs deltas as runs of zeros, bytes and words; a delta outside ±32,767 is refused.
pub(super) fn pack_deltas(values: &[i32], out: &mut Vec<u8>) -> Result<()> {
    let mut i = 0;
    while let Some(&v) = values.get(i) {
        if v.unsigned_abs() > 32_767 {
            return Err(TablesError::Invalid(format!("a variation delta would be {v}, outside ±32,767")));
        }
        let kind = |v: i32| {
            if v == 0 {
                0u8
            } else if (-128..=127).contains(&v) {
                1
            } else {
                2
            }
        };
        let k = kind(v);
        let mut run = 1;
        while run < 64 {
            match values.get(i + run) {
                Some(&n) if kind(n) == k && n.unsigned_abs() <= 32_767 => run += 1,
                _ => break,
            }
        }
        let slice = values.get(i..i + run).unwrap_or_default();
        match k {
            0 => out.push(DELTAS_ARE_ZERO | (run as u8 - 1)),
            1 => {
                out.push(run as u8 - 1);
                out.extend(slice.iter().map(|&v| v as i8 as u8));
            }
            _ => {
                out.push(DELTAS_ARE_WORDS | (run as u8 - 1));
                for &v in slice {
                    out.extend_from_slice(&(v as i16).to_be_bytes());
                }
            }
        }
        i += run;
    }
    Ok(())
}

/// Rewrites one tuple variation store. `b[start..]` begins with `tupleVariationCount`;
/// `dataOffset` counts from `b[0]`. `all` is the number of points "every point" means; `dims`
/// is 2 for `gvar` (x and y deltas), 1 for `cvar`. `change` gets each tuple's deltas, one list
/// per dimension, and changes them in place.
pub(super) fn rewrite_store(
    b: &[u8],
    start: usize,
    axes: usize,
    all: usize,
    dims: usize,
    change: &mut dyn FnMut(&mut [Vec<i32>]) -> Result<()>,
) -> Result<Vec<u8>> {
    let count_word = u16_at(b, start).ok_or_else(cut)?;
    let data_offset = usize::from(u16_at(b, start + 2).ok_or_else(cut)?);
    let count = usize::from(count_word & COUNT_MASK);
    if count > MAX_TUPLE_VARIATIONS {
        return Err(TablesError::TooLarge { what: "tuple variations", limit: MAX_TUPLE_VARIATIONS });
    }
    let mut headers = Vec::with_capacity(count);
    let mut at = start + 4;
    for _ in 0..count {
        let size = usize::from(u16_at(b, at).ok_or_else(cut)?);
        let index = u16_at(b, at + 2).ok_or_else(cut)?;
        let mut len = 4;
        if index & EMBEDDED_PEAK_TUPLE != 0 {
            len += 2 * axes;
        }
        if index & INTERMEDIATE_REGION != 0 {
            len += 4 * axes;
        }
        if at + len > b.len() {
            return Err(cut());
        }
        headers.push((at, len, size, index));
        at += len;
    }
    let headers_end = at;
    let mut data_at = data_offset;
    let shared = if count_word & SHARED_POINT_NUMBERS != 0 {
        let (n, end) = point_numbers(b, data_at, all)?;
        let bytes = b.get(data_at..end).ok_or_else(cut)?;
        data_at = end;
        Some((n, bytes))
    } else {
        None
    };

    let mut out = b.get(..headers_end).ok_or_else(cut)?.to_vec();
    // The headers keep their length, so the serialized data starts where it did, after any gap
    // between the headers and the data.
    let gap = b.get(headers_end..data_offset).ok_or_else(cut)?;
    out.extend_from_slice(gap);
    if let Some((_, bytes)) = shared {
        out.extend_from_slice(bytes);
    }
    for (header_at, _, size, index) in headers {
        let tuple = b.get(data_at..data_at + size).ok_or_else(cut)?;
        data_at += size;
        let (points, mut pos) = if index & PRIVATE_POINT_NUMBERS != 0 { point_numbers(tuple, 0, all)? } else { (shared.map_or(all, |(n, _)| n), 0) };
        let before = out.len();
        out.extend_from_slice(tuple.get(..pos).ok_or_else(cut)?);
        let mut lists = Vec::with_capacity(dims);
        for _ in 0..dims {
            let (d, end) = read_deltas(tuple, pos, points)?;
            lists.push(d);
            pos = end;
        }
        change(&mut lists)?;
        for list in &lists {
            pack_deltas(list, &mut out)?;
        }
        let new_size =
            u16::try_from(out.len() - before).map_err(|_| TablesError::invalid("a tuple's variation data would be longer than 65,535 bytes"))?;
        if let Some(slot) = out.get_mut(header_at..header_at + 2) {
            slot.copy_from_slice(&new_size.to_be_bytes());
        }
        if out.len() > MAX_SFNT {
            return Err(TablesError::TooLarge { what: "the variation data", limit: MAX_SFNT });
        }
    }
    Ok(out)
}

/// The `gvar` table with every glyph's deltas changed by `change`; `points[g]` is the number of
/// points (phantom points included) of glyph `g`.
pub(super) fn rewrite_gvar(b: &[u8], points: &[usize], change: &mut dyn FnMut(&mut [Vec<i32>]) -> Result<()>) -> Result<Vec<u8>> {
    let axes = usize::from(u16_at(b, 4).ok_or_else(cut)?);
    let shared_count = usize::from(u16_at(b, 6).ok_or_else(cut)?);
    let shared_offset = u32_at(b, 8).ok_or_else(cut)? as usize;
    let glyph_count = usize::from(u16_at(b, 12).ok_or_else(cut)?);
    let flags = u16_at(b, 14).ok_or_else(cut)?;
    let array_offset = u32_at(b, 16).ok_or_else(cut)? as usize;
    let long = flags & 1 != 0;
    let shared_len = shared_count.checked_mul(axes).and_then(|n| n.checked_mul(2)).ok_or_else(cut)?;
    let shared = b.get(shared_offset..shared_offset.checked_add(shared_len).ok_or_else(cut)?).ok_or_else(cut)?;
    let offset = |i: usize| -> Result<usize> {
        let v = if long { u32_at(b, 20 + 4 * i).map(|v| v as usize) } else { u16_at(b, 20 + 2 * i).map(|v| usize::from(v) * 2) };
        v.ok_or_else(cut)
    };
    let mut data = Vec::new();
    let mut offsets = Vec::with_capacity(glyph_count + 1);
    for g in 0..glyph_count {
        offsets.push(data.len());
        let (s, e) = (offset(g)?, offset(g + 1)?);
        if e <= s {
            continue;
        }
        let glyph = b.get(array_offset.checked_add(s).ok_or_else(cut)?..array_offset.checked_add(e).ok_or_else(cut)?).ok_or_else(cut)?;
        let all = points.get(g).copied().unwrap_or(4);
        let rewritten = rewrite_store(glyph, 0, axes, all, 2, change).map_err(|e| TablesError::Invalid(format!("gvar, glyph {g}: {e}")))?;
        data.extend_from_slice(&rewritten);
        if data.len() % 2 != 0 {
            data.push(0);
        }
        if data.len() > MAX_SFNT {
            return Err(TablesError::TooLarge { what: "the gvar table", limit: MAX_SFNT });
        }
    }
    offsets.push(data.len());
    let long = data.len() > 0x1FFFE;
    let offsets_len = (glyph_count + 1) * if long { 4 } else { 2 };
    let new_shared = 20 + offsets_len;
    let new_array = new_shared + shared.len();
    let mut out = Vec::with_capacity(new_array + data.len());
    out.extend_from_slice(b.get(..8).ok_or_else(cut)?);
    out.extend_from_slice(&(new_shared as u32).to_be_bytes());
    out.extend_from_slice(&(glyph_count as u16).to_be_bytes());
    out.extend_from_slice(&((flags & !1) | u16::from(long)).to_be_bytes());
    out.extend_from_slice(&(new_array as u32).to_be_bytes());
    for o in offsets {
        if long {
            out.extend_from_slice(&(o as u32).to_be_bytes());
        } else {
            out.extend_from_slice(&((o / 2) as u16).to_be_bytes());
        }
    }
    out.extend_from_slice(shared);
    out.extend_from_slice(&data);
    Ok(out)
}

/// The `cvar` table with its deltas changed by `change`; `cvt_count` is the number of `cvt`
/// values and `axes` the `fvar` axis count.
pub(super) fn rewrite_cvar(b: &[u8], axes: usize, cvt_count: usize, change: &mut dyn FnMut(&mut [Vec<i32>]) -> Result<()>) -> Result<Vec<u8>> {
    rewrite_store(b, 4, axes, cvt_count, 1, change).map_err(|e| TablesError::Invalid(format!("cvar: {e}")))
}
