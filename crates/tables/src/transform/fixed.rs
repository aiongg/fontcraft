//! Fixed-layout tables for [`super::transform`]: values patched in place in `head`, `hhea`,
//! `vhea`, `OS/2`, `post`, `VORG`, `kern` and `cvt `, and the metrics tables read and written
//! whole.

use super::layout::Scaler;
use crate::{Result, TablesError};

pub(super) fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

pub(super) fn i16_at(b: &[u8], at: usize) -> Option<i16> {
    u16_at(b, at).map(|v| v as i16)
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes([*b.get(at)?, *b.get(at + 1)?, *b.get(at + 2)?, *b.get(at + 3)?]))
}

pub(super) fn put_u16(b: &mut [u8], at: usize, v: u16) {
    if let Some(slot) = b.get_mut(at..at + 2) {
        slot.copy_from_slice(&v.to_be_bytes());
    }
}

pub(super) fn put_i16(b: &mut [u8], at: usize, v: i16) {
    put_u16(b, at, v as u16);
}

/// Scales the signed 16-bit field at `at` when the table is long enough to hold it.
pub(super) fn scale_i16(b: &mut [u8], at: usize, s: Scaler, what: &str) -> Result<()> {
    if let Some(v) = i16_at(b, at) {
        put_i16(b, at, s.i16(v, what)?);
    }
    Ok(())
}

/// Scales the unsigned 16-bit field at `at` when the table is long enough to hold it.
pub(super) fn scale_u16(b: &mut [u8], at: usize, s: Scaler, what: &str) -> Result<()> {
    if let Some(v) = u16_at(b, at) {
        let r = s.i32(i32::from(v));
        let r = u16::try_from(r).map_err(|_| TablesError::Invalid(format!("{what} would be {r}, outside 0 to 65,535")))?;
        put_u16(b, at, r);
    }
    Ok(())
}

/// `hhea` and `vhea` share their layout: the line metrics and caret offset scale, the extremes
/// are worked out again by the caller.
pub(super) const HHEA_SCALED: [(usize, &str); 4] = [(4, "the ascender"), (6, "the descender"), (8, "the line gap"), (22, "the caret offset")];

/// The `OS/2` fields scaled by units per em, as fontTools scales them.
pub(super) const OS2_SIGNED: [(usize, &str); 16] = [
    (2, "xAvgCharWidth"),
    (10, "ySubscriptXSize"),
    (12, "ySubscriptYSize"),
    (14, "ySubscriptXOffset"),
    (16, "ySubscriptYOffset"),
    (18, "ySuperscriptXSize"),
    (20, "ySuperscriptYSize"),
    (22, "ySuperscriptXOffset"),
    (24, "ySuperscriptYOffset"),
    (26, "yStrikeoutSize"),
    (28, "yStrikeoutPosition"),
    (68, "sTypoAscender"),
    (70, "sTypoDescender"),
    (72, "sTypoLineGap"),
    (86, "sxHeight"),
    (88, "sCapHeight"),
];
pub(super) const OS2_UNSIGNED: [(usize, &str); 2] = [(74, "usWinAscent"), (76, "usWinDescent")];

/// Long metrics and trailing side bearings of `hmtx` or `vmtx`, one entry per glyph.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Metrics {
    pub advances: Vec<u16>,
    pub bearings: Vec<i16>,
    /// The number of long metrics the input stored (`numberOf…Metrics`).
    pub long: usize,
}

pub(super) fn read_metrics(table: &[u8], long: usize, glyphs: usize) -> Result<Metrics> {
    let long = long.min(glyphs);
    if long == 0 && glyphs > 0 {
        return Err(TablesError::invalid("the metrics header says there are no long metrics"));
    }
    let mut advances = Vec::with_capacity(glyphs);
    let mut bearings = Vec::with_capacity(glyphs);
    for g in 0..glyphs {
        let (adv, bearing) = if g < long {
            (u16_at(table, 4 * g), i16_at(table, 4 * g + 2))
        } else {
            (advances.last().copied(), i16_at(table, 4 * long + 2 * (g - long)))
        };
        // fontTools reads a cut-short table with zeros for the missing side bearings.
        advances.push(adv.ok_or_else(|| TablesError::invalid("the metrics table is shorter than its header says"))?);
        bearings.push(bearing.unwrap_or(0));
    }
    Ok(Metrics { advances, bearings, long })
}

/// The table bytes, with as many long metrics as the trailing equal advances allow, as
/// fontTools writes them.
pub(super) fn write_metrics(m: &Metrics) -> (Vec<u8>, u16) {
    let mut long = m.advances.len();
    while long > 1 && m.advances.get(long - 1) == m.advances.get(long - 2) {
        long -= 1;
    }
    let mut out = Vec::with_capacity(4 * long + 2 * (m.advances.len() - long));
    for (g, (a, b)) in m.advances.iter().zip(&m.bearings).enumerate() {
        if g < long {
            out.extend_from_slice(&a.to_be_bytes());
        }
        out.extend_from_slice(&b.to_be_bytes());
    }
    (out, long as u16)
}

/// fontTools' `hhea.recalc` (and `vhea.recalc` with heights): the largest advance and the
/// extremes over glyphs with outlines. `extents[g]` is the glyph's (min, max) along the axis.
pub(super) fn recalc_extremes(header: &mut [u8], m: &Metrics, extents: &[Option<(i32, i32)>]) -> Result<()> {
    let max_advance = m.advances.iter().copied().max().unwrap_or(0);
    put_u16(header, 10, max_advance);
    let mut found = false;
    let (mut min_first, mut min_second, mut max_extent) = (i32::MAX, i32::MAX, i32::MIN);
    for (g, e) in extents.iter().enumerate() {
        let Some((lo, hi)) = e else { continue };
        let (Some(&adv), Some(&bearing)) = (m.advances.get(g), m.bearings.get(g)) else { continue };
        let size = hi - lo;
        let (adv, bearing) = (i32::from(adv), i32::from(bearing));
        min_first = min_first.min(bearing);
        min_second = min_second.min(adv - bearing - size);
        max_extent = max_extent.max(bearing + size);
        found = true;
    }
    let fit = |v: i32, what: &str| i16::try_from(v).map_err(|_| TablesError::Invalid(format!("{what} would be {v}, outside ±32,767")));
    let (a, b, c) = if found {
        (fit(min_first, "the smallest side bearing")?, fit(min_second, "the smallest side bearing")?, fit(max_extent, "the largest extent")?)
    } else {
        (0, 0, 0)
    };
    put_i16(header, 12, a);
    put_i16(header, 14, b);
    put_i16(header, 16, c);
    Ok(())
}

/// `VORG`: the default origin and every glyph's.
pub(super) fn scale_vorg(b: &mut [u8], s: Scaler) -> Result<()> {
    scale_i16(b, 4, s, "the default vertical origin")?;
    let count = usize::from(u16_at(b, 6).unwrap_or(0));
    for i in 0..count {
        scale_i16(b, 8 + 4 * i + 2, s, "a vertical origin")?;
    }
    Ok(())
}

/// `kern`: the pair values of every format 0 subtable, in the OpenType and in the Apple
/// header. Gives how many subtables of other formats were left unscaled.
pub(super) fn scale_kern(b: &mut [u8], s: Scaler) -> Result<usize> {
    let version = u16_at(b, 0).ok_or_else(|| TablesError::invalid("the kern table is cut short"))?;
    let (apple, count, mut at) =
        if version == 0 { (false, usize::from(u16_at(b, 2).unwrap_or(0)), 4usize) } else { (true, u32_at(b, 4).unwrap_or(0) as usize, 8usize) };
    let mut skipped = 0;
    for _ in 0..count.min(crate::limits::MAX_TABLES) {
        let (length, format, body) = if apple {
            let length = u32_at(b, at).map(|v| v as usize);
            let coverage = u16_at(b, at + 4).unwrap_or(0);
            (length, coverage & 0xFF, at + 8)
        } else {
            let length = u16_at(b, at + 2).map(usize::from);
            let coverage = u16_at(b, at + 4).unwrap_or(0);
            (length, coverage >> 8, at + 6)
        };
        let Some(length) = length else { break };
        if format == 0 {
            let pairs = usize::from(u16_at(b, body).unwrap_or(0));
            for p in 0..pairs {
                let at = body + 8 + 6 * p + 4;
                if at + 2 > b.len() {
                    break;
                }
                scale_i16(b, at, s, "a kerning value")?;
            }
        } else {
            skipped += 1;
        }
        // Old fonts store a wrong length in a single format 0 subtable; nothing follows it.
        if length < 6 {
            break;
        }
        at = at.saturating_add(length);
    }
    Ok(skipped)
}

/// `cvt `: every value.
pub(super) fn scale_cvt(b: &mut [u8], s: Scaler) -> Result<()> {
    for at in (0..b.len() / 2).map(|i| 2 * i) {
        scale_i16(b, at, s, "a control value")?;
    }
    Ok(())
}
