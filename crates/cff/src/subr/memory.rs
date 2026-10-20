//! The subroutiniser's working-memory budget. Each large array is charged when it is made and
//! released when it is dropped; a charge past the limit stops the work with an error. Small,
//! bounded allocations (one glyph's encoding, the traversal stack) are not counted.

use super::SubrError;

pub(super) struct Memory {
    used: usize,
    peak: usize,
    limit: usize,
}

impl Memory {
    pub fn new(limit: usize) -> Self {
        Memory { used: 0, peak: 0, limit }
    }

    pub fn charge(&mut self, bytes: usize) -> Result<(), SubrError> {
        let used = self.used.checked_add(bytes).filter(|&u| u <= self.limit);
        let used = used.ok_or(SubrError::TooLarge { what: "bytes of working memory to subroutinise", limit: self.limit })?;
        self.used = used;
        self.peak = self.peak.max(used);
        Ok(())
    }

    /// Charges `count` values of `T`.
    pub fn charge_for<T>(&mut self, count: usize) -> Result<(), SubrError> {
        self.charge(count.saturating_mul(size_of::<T>()))
    }

    pub fn release(&mut self, bytes: usize) {
        self.used = self.used.saturating_sub(bytes);
    }

    /// Releases `count` values of `T`.
    pub fn release_for<T>(&mut self, count: usize) {
        self.release(count.saturating_mul(size_of::<T>()));
    }

    /// The most charged at once so far.
    pub fn peak(&self) -> usize {
        self.peak
    }
}
