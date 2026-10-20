//! What the PK, GF and Mac readers share: a bounds-checked big-endian reader, the scaling of TeX
//! design sizes to pixels, glyph names made from codes, and filling a run of pixels in a row.

use crate::{BitmapError, Result};

/// Reads big-endian numbers from a byte slice; every read past the end is an error.
pub(crate) struct Cursor<'a> {
    bytes: &'a [u8],
    pub(crate) pos: usize,
    format: &'static str,
}

impl<'a> Cursor<'a> {
    pub(crate) fn new(bytes: &'a [u8], format: &'static str) -> Self {
        Cursor { bytes, pos: 0, format }
    }

    pub(crate) fn at(bytes: &'a [u8], pos: usize, format: &'static str) -> Self {
        Cursor { bytes, pos, format }
    }

    pub(crate) fn error(&self, message: impl Into<String>) -> BitmapError {
        malformed(self.format, format!("at byte {}: {}", self.pos, message.into()))
    }

    pub(crate) fn len(&self) -> usize {
        self.bytes.len()
    }

    pub(crate) fn at_end(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    pub(crate) fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).filter(|end| *end <= self.bytes.len()).ok_or_else(|| self.error("the data ends early"))?;
        let out = self.bytes.get(self.pos..end).ok_or_else(|| self.error("the data ends early"))?;
        self.pos = end;
        Ok(out)
    }

    pub(crate) fn skip(&mut self, n: usize) -> Result<()> {
        self.take(n).map(|_| ())
    }

    /// An unsigned big-endian number of `n` bytes, 1 to 4.
    pub(crate) fn uint(&mut self, n: usize) -> Result<u32> {
        if !(1..=4).contains(&n) {
            return Err(self.error("a number wider than four bytes"));
        }
        Ok(self.take(n)?.iter().fold(0u32, |acc, b| (acc << 8) | u32::from(*b)))
    }

    pub(crate) fn u8(&mut self) -> Result<u8> {
        Ok(self.uint(1)? as u8)
    }

    pub(crate) fn i8(&mut self) -> Result<i8> {
        Ok(self.uint(1)? as u8 as i8)
    }

    pub(crate) fn u16(&mut self) -> Result<u16> {
        Ok(self.uint(2)? as u16)
    }

    pub(crate) fn i16(&mut self) -> Result<i16> {
        Ok(self.uint(2)? as u16 as i16)
    }

    pub(crate) fn u32(&mut self) -> Result<u32> {
        self.uint(4)
    }

    pub(crate) fn i32(&mut self) -> Result<i32> {
        Ok(self.uint(4)? as i32)
    }
}

pub(crate) fn malformed(format: &'static str, message: String) -> BitmapError {
    BitmapError::Malformed { format, message }
}

/// TeX points per inch.
pub(crate) const TEX_POINTS_PER_INCH: f64 = 72.27;
/// One in the fixed-point units of a TeX design size or width (2^20).
pub(crate) const FIX_WORD_ONE: f64 = 1_048_576.0;
/// One in the fixed-point units of a pixels-per-point ratio or an escapement (2^16).
pub(crate) const SCALED_ONE: f64 = 65_536.0;

/// The size of a PK or GF font: its design size in TeX points and its pixels per point.
pub(crate) struct Scale {
    pub(crate) design_points: f64,
    h_per_point: f64,
    v_per_point: f64,
}

impl Scale {
    pub(crate) fn new(format: &'static str, design_size: i32, hppp: i32, vppp: i32) -> Result<Scale> {
        if design_size <= 0 || hppp <= 0 || vppp <= 0 {
            return Err(malformed(format, "the design size and the pixels per point must be positive".into()));
        }
        Ok(Scale {
            design_points: f64::from(design_size) / FIX_WORD_ONE,
            h_per_point: f64::from(hppp) / SCALED_ONE,
            v_per_point: f64::from(vppp) / SCALED_ONE,
        })
    }

    /// Pixels per em: the design size times the vertical pixels per point, rounded, at least 1.
    pub(crate) fn pixel_size(&self) -> u16 {
        let pixels = (self.design_points * self.v_per_point).round();
        if pixels.is_finite() { pixels.clamp(1.0, f64::from(u16::MAX)) as u16 } else { 1 }
    }

    /// Dots per inch, horizontally and vertically.
    pub(crate) fn resolution(&self) -> (u32, u32) {
        let dpi = |per_point: f64| {
            let v = (per_point * TEX_POINTS_PER_INCH).round();
            if v.is_finite() { v.clamp(1.0, f64::from(u32::MAX)) as u32 } else { 72 }
        };
        (dpi(self.h_per_point), dpi(self.v_per_point))
    }

    /// A TeX width (a fraction of the design size, times 2^20) in thousandths of the design size.
    pub(crate) fn thousandths(width: i32) -> i32 {
        (f64::from(width) * 1000.0 / FIX_WORD_ONE).round() as i32
    }
}

/// The name of a glyph known only by its code: the character itself for an ASCII letter or digit,
/// else `prefix` and the code, `char65` or `uni00C4`.
pub(crate) fn code_name(code: u32, hex: bool) -> String {
    match char::from_u32(code) {
        Some(c) if c.is_ascii_alphanumeric() => c.to_string(),
        _ if hex => format!("uni{code:04X}"),
        _ => format!("char{code}"),
    }
}

/// Sets `len` bits of `row` from bit `start`, counted from the most significant bit of the first
/// byte. Bits past the row are ignored.
pub(crate) fn set_bits(row: &mut [u8], start: usize, len: usize) {
    let end = start.saturating_add(len).min(row.len().saturating_mul(8));
    let mut bit = start;
    while bit < end {
        let Some(byte) = row.get_mut(bit / 8) else { return };
        let offset = bit % 8;
        if offset == 0 && end - bit >= 8 {
            *byte = 0xff;
            bit += 8;
        } else {
            *byte |= 0x80 >> offset;
            bit += 1;
        }
    }
}

/// Whether bit `bit` of `bytes` is set, counted from the most significant bit of the first byte.
pub(crate) fn bit(bytes: &[u8], bit: usize) -> bool {
    bytes.get(bit / 8).is_some_and(|b| b & (0x80 >> (bit % 8)) != 0)
}
