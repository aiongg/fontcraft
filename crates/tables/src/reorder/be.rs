//! Big-endian reads that fail with a message instead of panicking, and the small writers the
//! glyph-order tools share.

use crate::sfnt::{Tag, tag_str};
use crate::{Result, TablesError};

/// A view of one table's bytes with checked reads.
#[derive(Clone, Copy)]
pub(crate) struct Data<'a> {
    pub bytes: &'a [u8],
    pub tag: Tag,
}

impl<'a> Data<'a> {
    pub fn new(tag: Tag, bytes: &'a [u8]) -> Self {
        Data { bytes, tag }
    }

    fn short(&self, at: usize) -> TablesError {
        TablesError::invalid(format!("the '{}' table is cut short at byte {at}", tag_str(self.tag)))
    }

    pub fn slice(&self, at: usize, len: usize) -> Result<&'a [u8]> {
        let end = at.checked_add(len).ok_or_else(|| self.short(at))?;
        self.bytes.get(at..end).ok_or_else(|| self.short(at))
    }

    pub fn u8(&self, at: usize) -> Result<u8> {
        self.bytes.get(at).copied().ok_or_else(|| self.short(at))
    }

    pub fn u16(&self, at: usize) -> Result<u16> {
        let s = self.slice(at, 2)?;
        Ok(u16::from_be_bytes([s.first().copied().unwrap_or(0), s.get(1).copied().unwrap_or(0)]))
    }

    pub fn i16(&self, at: usize) -> Result<i16> {
        Ok(self.u16(at)? as i16)
    }

    pub fn u32(&self, at: usize) -> Result<u32> {
        Ok(u32::from(self.u16(at)?) << 16 | u32::from(self.u16(at + 2)?))
    }

    /// An offset or count read as a `usize`.
    pub fn u32_usize(&self, at: usize) -> Result<usize> {
        usize::try_from(self.u32(at)?).map_err(|_| self.short(at))
    }

    /// An error that names this table.
    pub fn bad(&self, what: impl std::fmt::Display) -> TablesError {
        TablesError::invalid(format!("the '{}' table {what}", tag_str(self.tag)))
    }
}

pub(crate) fn put_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_be_bytes());
}

pub(crate) fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_be_bytes());
}

/// Overwrites two bytes at `at`; a write past the end is refused.
pub(crate) fn set_u16(out: &mut [u8], at: usize, v: u16) -> Result<()> {
    let slot = out.get_mut(at..at.saturating_add(2)).ok_or_else(|| TablesError::invalid(format!("no room for a value at byte {at}")))?;
    slot.copy_from_slice(&v.to_be_bytes());
    Ok(())
}

/// A count or offset as `u16`, or an error naming what overflowed.
pub(crate) fn to_u16(v: usize, what: &'static str) -> Result<u16> {
    u16::try_from(v).map_err(|_| TablesError::TooLarge { what, limit: usize::from(u16::MAX) })
}

/// A count or offset as `u32`, or an error naming what overflowed.
pub(crate) fn to_u32(v: usize, what: &'static str) -> Result<u32> {
    u32::try_from(v).map_err(|_| TablesError::TooLarge { what, limit: u32::MAX as usize })
}

/// `searchRange`, `entrySelector` and `rangeShift` for `count` items of `size` bytes.
pub(crate) fn search_params(count: usize, size: usize) -> (u16, u16, u16) {
    let mut power = 1usize;
    let mut log = 0u16;
    while power * 2 <= count {
        power *= 2;
        log += 1;
    }
    let range = power * size;
    let shift = (count * size).saturating_sub(range);
    (range.min(usize::from(u16::MAX)) as u16, log, shift.min(usize::from(u16::MAX)) as u16)
}
