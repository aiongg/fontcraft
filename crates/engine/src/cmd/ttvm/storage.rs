// Ported from skrifa 0.48.0 (fontations, MIT OR Apache-2.0), src/outline/glyf/hint/storage.rs; see NOTICE.
//! Storage area.

use super::{HintErrorKind, cow_slice::CowSlice};

/// Backing store for the storage area.
///
/// This is just a wrapper for [`CowSlice`] that converts out of bounds
/// accesses to appropriate errors.
pub struct Storage<'a>(CowSlice<'a>);

impl Storage<'_> {
    pub fn get(&self, index: usize) -> Result<i32, HintErrorKind> {
        self.0.get(index).ok_or(HintErrorKind::InvalidStorageIndex(index))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn set(&mut self, index: usize, value: i32) -> Result<(), HintErrorKind> {
        self.0.set(index, value).ok_or(HintErrorKind::InvalidStorageIndex(index))
    }
}

impl<'a> From<CowSlice<'a>> for Storage<'a> {
    fn from(value: CowSlice<'a>) -> Self {
        Self(value)
    }
}
