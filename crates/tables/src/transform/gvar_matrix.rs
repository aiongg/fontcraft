//! A matrix on the `gvar` deltas of a TrueType font. A delta moves its point by a vector, so it
//! takes the glyph's linear part only. A tuple that leaves points out is first filled in by
//! interpolation of untouched points (IUP) when the linear part mixes x and y on a simple glyph:
//! interpolation works on x and y apart, so it would infer other deltas after a turn or a shear.
//! A scale, a mirror or a shift keeps the tuple as sparse as it was. Each tuple is written with
//! its own point numbers.

use super::tuples::{
    COUNT_MASK, EMBEDDED_PEAK_TUPLE, INTERMEDIATE_REGION, PRIVATE_POINT_NUMBERS, SHARED_POINT_NUMBERS, cut, pack_deltas, read_deltas, u16_at, u32_at,
};
use super::{Linear, apply};
use crate::instance::iup::iup_delta;
use crate::limits::{MAX_SFNT, MAX_TUPLE_VARIATIONS};
use crate::{Result, TablesError};

/// One glyph as the deltas need it, read before the matrix changes it.
#[derive(Clone, Debug, Default)]
pub(super) struct GlyphDeltas {
    /// The default points, the four phantom points last.
    pub coords: Vec<(f64, f64)>,
    /// The last point of each contour; one per component for a composite.
    pub ends: Vec<usize>,
    /// The linear part each point's deltas take, phantom points left out; `None` leaves the
    /// glyph's deltas as they are.
    pub linear: Option<Vec<Linear>>,
    /// Fill in left-out points first.
    pub densify: bool,
    /// What the x deltas of the two horizontal phantom points are multiplied by; `None` keeps
    /// them.
    pub advance_factor: Option<f64>,
}

/// The `gvar` table with each glyph's deltas transformed as `glyphs` says.
pub(super) fn transform_gvar(b: &[u8], glyphs: &[GlyphDeltas]) -> Result<Vec<u8>> {
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
        match glyphs.get(g).filter(|d| d.linear.is_some()) {
            Some(d) => {
                let rewritten = transform_glyph(glyph, axes, d).map_err(|e| TablesError::Invalid(format!("gvar, glyph {g}: {e}")))?;
                data.extend_from_slice(&rewritten);
            }
            None => data.extend_from_slice(glyph),
        }
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

/// Reads packed point numbers at `at`: `None` for every point, and where they end.
fn read_points(b: &[u8], mut at: usize, all: usize) -> Result<(Option<Vec<usize>>, usize)> {
    let first = *b.get(at).ok_or_else(cut)?;
    at += 1;
    if first == 0 {
        return Ok((None, at));
    }
    let count = if first & 0x80 != 0 {
        let second = *b.get(at).ok_or_else(cut)?;
        at += 1;
        (usize::from(first & 0x7F) << 8) | usize::from(second)
    } else {
        usize::from(first)
    };
    let mut points = Vec::with_capacity(count.min(all));
    let mut last = 0usize;
    while points.len() < count {
        let control = *b.get(at).ok_or_else(cut)?;
        at += 1;
        let run = usize::from(control & 0x7F) + 1;
        for _ in 0..run {
            let step = if control & 0x80 != 0 {
                let v = u16_at(b, at).ok_or_else(cut)?;
                at += 2;
                usize::from(v)
            } else {
                let v = *b.get(at).ok_or_else(cut)?;
                at += 1;
                usize::from(v)
            };
            last += step;
            if points.len() < count {
                if last >= all {
                    return Err(TablesError::invalid(format!("a tuple names point {last} of a glyph with {all}")));
                }
                points.push(last);
            }
        }
    }
    Ok((Some(points), at))
}

/// Packs point numbers, `None` for every point.
fn pack_points(points: Option<&[usize]>, out: &mut Vec<u8>) -> Result<()> {
    let Some(points) = points else {
        out.push(0);
        return Ok(());
    };
    let count = points.len();
    if count == 0 || count > 0x7FFF {
        return Err(TablesError::invalid(format!("a tuple of {count} explicit points")));
    }
    if count < 0x80 {
        out.push(count as u8);
    } else {
        out.extend_from_slice(&(count as u16 | 0x8000).to_be_bytes());
    }
    let mut steps = Vec::with_capacity(count);
    let mut last = 0usize;
    for p in points {
        steps.push(p.checked_sub(last).ok_or_else(|| TablesError::invalid("point numbers out of order"))?);
        last = *p;
    }
    for chunk in steps.chunks(128) {
        let words = chunk.iter().any(|s| *s > 0xFF);
        out.push(((chunk.len() - 1) as u8) | if words { 0x80 } else { 0 });
        for s in chunk {
            let s = u16::try_from(*s).map_err(|_| TablesError::invalid("a point number past 65,535"))?;
            if words {
                out.extend_from_slice(&s.to_be_bytes());
            } else {
                out.push(s as u8);
            }
        }
    }
    Ok(())
}

/// One glyph's variation data with its deltas transformed.
fn transform_glyph(b: &[u8], axes: usize, d: &GlyphDeltas) -> Result<Vec<u8>> {
    let all = d.coords.len();
    let count_word = u16_at(b, 0).ok_or_else(cut)?;
    let data_offset = usize::from(u16_at(b, 2).ok_or_else(cut)?);
    let count = usize::from(count_word & COUNT_MASK);
    if count > MAX_TUPLE_VARIATIONS {
        return Err(TablesError::TooLarge { what: "tuple variations", limit: MAX_TUPLE_VARIATIONS });
    }
    let mut headers = Vec::with_capacity(count);
    let mut at = 4;
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
        let rest = b.get(at + 4..at + len).ok_or_else(cut)?;
        headers.push((size, index, rest));
        at += len;
    }
    let mut data_at = data_offset;
    let shared = if count_word & SHARED_POINT_NUMBERS != 0 {
        let (points, end) = read_points(b, data_at, all)?;
        data_at = end;
        Some(points)
    } else {
        None
    };

    let phantom_start = all.saturating_sub(4);
    let mut out_headers = Vec::new();
    let mut out_data = Vec::new();
    for (size, index, rest) in headers {
        let tuple = b.get(data_at..data_at.checked_add(size).ok_or_else(cut)?).ok_or_else(cut)?;
        data_at += size;
        let (points, pos) = if index & PRIVATE_POINT_NUMBERS != 0 { read_points(tuple, 0, all)? } else { (shared.clone().flatten(), 0) };
        let n = points.as_ref().map_or(all, Vec::len);
        let (xs, pos) = read_deltas(tuple, pos, n)?;
        let (ys, _) = read_deltas(tuple, pos, n)?;
        let mut explicit: Vec<Option<(f64, f64)>> = vec![None; all];
        let named: Vec<usize> = points.clone().unwrap_or_else(|| (0..all).collect());
        for ((p, x), y) in named.iter().zip(&xs).zip(&ys) {
            if let Some(slot) = explicit.get_mut(*p) {
                *slot = Some((f64::from(*x), f64::from(*y)));
            }
        }
        let (named, deltas): (Option<Vec<usize>>, Vec<(f64, f64)>) = if d.densify && points.is_some() {
            (None, iup_delta(&explicit, &d.coords, &d.ends))
        } else {
            (points, named.iter().map(|p| explicit.get(*p).copied().flatten().unwrap_or((0.0, 0.0))).collect())
        };
        let order: Vec<usize> = named.clone().unwrap_or_else(|| (0..all).collect());
        let mut new_x = Vec::with_capacity(order.len());
        let mut new_y = Vec::with_capacity(order.len());
        for (p, (dx, dy)) in order.iter().zip(deltas) {
            let (nx, ny) = if *p < phantom_start {
                let l = d.linear.as_ref().and_then(|l| l.get(*p)).copied().unwrap_or([1.0, 0.0, 0.0, 1.0]);
                apply(l, (dx, dy))
            } else if *p < phantom_start + 2 {
                (d.advance_factor.map_or(dx, |f| f * dx), dy)
            } else {
                (dx, dy)
            };
            let round = |v: f64| -> Result<i32> {
                let r = (v + 0.5).floor();
                if !r.is_finite() || r.abs() > 32_767.0 {
                    return Err(TablesError::Invalid(format!("a delta would be {r}, outside ±32,767")));
                }
                Ok(r as i32)
            };
            new_x.push(round(nx)?);
            new_y.push(round(ny)?);
        }
        let start = out_data.len();
        pack_points(named.as_deref(), &mut out_data)?;
        pack_deltas(&new_x, &mut out_data)?;
        pack_deltas(&new_y, &mut out_data)?;
        let new_size =
            u16::try_from(out_data.len() - start).map_err(|_| TablesError::invalid("a tuple's variation data would be longer than 65,535 bytes"))?;
        out_headers.extend_from_slice(&new_size.to_be_bytes());
        out_headers.extend_from_slice(&(index | PRIVATE_POINT_NUMBERS).to_be_bytes());
        out_headers.extend_from_slice(rest);
        if out_data.len() > MAX_SFNT {
            return Err(TablesError::TooLarge { what: "the variation data", limit: MAX_SFNT });
        }
    }
    let data_start = u16::try_from(4 + out_headers.len()).map_err(|_| TablesError::invalid("the tuple headers would pass 65,535 bytes"))?;
    let mut out = Vec::with_capacity(4 + out_headers.len() + out_data.len());
    out.extend_from_slice(&(count_word & !SHARED_POINT_NUMBERS).to_be_bytes());
    out.extend_from_slice(&data_start.to_be_bytes());
    out.extend_from_slice(&out_headers);
    out.extend_from_slice(&out_data);
    Ok(out)
}
