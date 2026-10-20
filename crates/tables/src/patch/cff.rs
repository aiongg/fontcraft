//! Top DICT values of a `CFF ` table that are derived from the rest of the font: `FontBBox`
//! from the charstrings, and any integer entry a caller recomputes (`isFixedPitch`). The table
//! is written again with the subsetter's CFF writer, which keeps every other entry and lays the
//! offsets out anew. `FontBBox` is computed as fontTools 4.66.1 `TopDict.recalcFontBBox` does (MIT,
//! Copyright 2013 Google, Inc.).

use super::be_u16;
use crate::reorder::cff;
use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

/// The Top DICT operator of `FontBBox`.
pub const CFF_FONT_BBOX: u16 = 5;
/// The Top DICT operator of `isFixedPitch` (escaped operators count as 1200 + their second byte).
pub const CFF_IS_FIXED_PITCH: u16 = 1201;

fn cff_table(font: &SfntFont) -> Result<(&[u8], usize)> {
    let data = font.get(*b"CFF ").ok_or_else(|| TablesError::MissingTable("CFF ".into()))?;
    let maxp = font.get(*b"maxp").ok_or_else(|| TablesError::MissingTable("maxp".into()))?;
    let count = be_u16(maxp, 4).ok_or_else(|| TablesError::invalid("the 'maxp' table is cut short"))?;
    Ok((data, usize::from(count)))
}

/// The integer operands of Top DICT operator `op`; `None` when the entry is absent or an operand
/// is not an integer.
pub fn cff_top_ints(font: &SfntFont, op: u16) -> Result<Option<Vec<i64>>> {
    let (data, count) = cff_table(font)?;
    Ok(cff::parse(data, count)?.top_ints(op))
}

/// The box of every charstring, as `FontBBox` stores it: the lowest coordinates rounded down and
/// the highest rounded up, all zero when no glyph has an outline (fontTools' `recalcFontBBox`).
pub fn cff_font_bbox(font: &SfntFont) -> Result<[i64; 4]> {
    let (data, _) = cff_table(font)?;
    let boxes = crate::ttx::read_cff::glyph_bounds(data)?;
    let union = boxes.into_iter().flatten().reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]);
    Ok(union.map_or([0; 4], |b| [b[0].floor() as i64, b[1].floor() as i64, b[2].ceil() as i64, b[3].ceil() as i64]))
}

/// The font with Top DICT operator `op` given the integer operands `values`.
pub fn set_cff_top_ints(font: &SfntFont, op: u16, values: &[i64]) -> Result<SfntFont> {
    let (data, count) = cff_table(font)?;
    let mut table = cff::parse(data, count)?;
    table.set_top_ints(op, values)?;
    let identity: Vec<u16> = (0..count).map(|g| u16::try_from(g).map_err(|_| TablesError::invalid("too many glyphs"))).collect::<Result<_>>()?;
    let rebuilt = table.rebuild(&identity, None)?;
    let mut out = font.clone();
    out.insert(*b"CFF ", rebuilt);
    Ok(out)
}
