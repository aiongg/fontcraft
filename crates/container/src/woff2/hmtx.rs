//! The WOFF2 `hmtx` transform (version 1), both ways: the left side bearings that equal the
//! glyphs' `xMin` are left out, and a reader takes them from `glyf`.
//!
//! Ported from fontTools' `WOFF2HmtxTable` (`ttLib/woff2.py`, MIT). The rebuilt table keeps the
//! `numberOfHMetrics` of `hhea`; fontTools' reader recompiles `hmtx` and may shorten it, without
//! changing `hhea`.

use crate::Error;

const FORMAT: &str = "WOFF2";

fn bad(what: impl Into<String>) -> Error {
    Error::invalid(FORMAT, what)
}

/// Flags bit 0: the proportional glyphs' side bearings are left out.
const NO_LSB: u8 = 1;
/// Flags bit 1: the monospaced glyphs' side bearings are left out.
const NO_LEFT_SIDE_BEARING: u8 = 2;

fn word(data: &[u8], at: usize) -> Option<[u8; 2]> {
    data.get(at..at.checked_add(2)?)?.try_into().ok()
}

/// The transformed `hmtx`, or `None` when the transform does not apply: the table's size does
/// not match `number_of_hmetrics` and the glyph count, or both side-bearing arrays would be needed.
pub(crate) fn transform(hmtx: &[u8], number_of_hmetrics: u16, x_mins: &[i16]) -> Option<Vec<u8>> {
    let glyphs = x_mins.len();
    let n = usize::from(number_of_hmetrics);
    if n == 0 || n > glyphs || hmtx.len() != 4 * n + 2 * (glyphs - n) {
        return None;
    }
    let bearing = |i: usize| -> Option<i16> {
        let at = if i < n { 4 * i + 2 } else { 4 * n + 2 * (i - n) };
        word(hmtx, at).map(i16::from_be_bytes)
    };
    let differs = |range: std::ops::Range<usize>| range.into_iter().any(|i| bearing(i) != x_mins.get(i).copied());
    let (lsb, left) = (differs(0..n), differs(n..glyphs));
    if lsb && left {
        return None;
    }
    let mut flags = 0;
    if !lsb {
        flags |= NO_LSB;
    }
    if !left {
        flags |= NO_LEFT_SIDE_BEARING;
    }
    let mut out = vec![flags];
    for i in 0..n {
        out.extend_from_slice(&word(hmtx, 4 * i)?);
    }
    let kept = if lsb {
        0..n
    } else if left {
        n..glyphs
    } else {
        0..0
    };
    for i in kept {
        out.extend_from_slice(&bearing(i)?.to_be_bytes());
    }
    Some(out)
}

/// Rebuilds `hmtx` from its transformed form, with `number_of_hmetrics` from `hhea` and every
/// glyph's `xMin`.
pub(crate) fn reconstruct(data: &[u8], number_of_hmetrics: u16, x_mins: &[i16]) -> Result<Vec<u8>, Error> {
    let flags = *data.first().ok_or_else(|| bad("the transformed hmtx table is empty"))?;
    if flags & !(NO_LSB | NO_LEFT_SIDE_BEARING) != 0 {
        return Err(bad("the transformed hmtx table uses reserved flags"));
    }
    let (lsb, left) = (flags & NO_LSB == 0, flags & NO_LEFT_SIDE_BEARING == 0);
    if lsb && left {
        return Err(bad("the transformed hmtx table leaves out no side bearings"));
    }
    let glyphs = x_mins.len();
    let n = usize::from(number_of_hmetrics);
    if n > glyphs {
        return Err(bad("hhea names more horizontal metrics than the font has glyphs"));
    }
    if n == 0 {
        return Err(bad("the transformed hmtx table has no metrics"));
    }
    let expected = 1 + 2 * n + if lsb { 2 * n } else { 0 } + if left { 2 * (glyphs - n) } else { 0 };
    if data.len() != expected {
        return Err(bad("the transformed hmtx table has the wrong size"));
    }
    let at = |i: usize| word(data, i).ok_or_else(|| bad("the transformed hmtx table is cut short"));
    let x_min = |i: usize| x_mins.get(i).copied().unwrap_or(0).to_be_bytes();
    let mut out = Vec::with_capacity(4 * n + 2 * (glyphs - n));
    for i in 0..n {
        out.extend_from_slice(&at(1 + 2 * i)?);
        out.extend_from_slice(&if lsb { at(1 + 2 * n + 2 * i)? } else { x_min(i) });
    }
    for i in n..glyphs {
        out.extend_from_slice(&if left { at(1 + 2 * n + 2 * (i - n))? } else { x_min(i) });
    }
    Ok(out)
}
