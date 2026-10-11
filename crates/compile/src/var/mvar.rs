//! The `MVAR` table (M9.6).
//!
//! Rows: the `MVAR` table row.
//!
//! Contract: one record per value tag (`hasc`, `xhgt`, `undo`…) whose value varies, sorted by
//! tag; no table when nothing varies.

use write_fonts::tables::mvar::Mvar;
use write_fonts::types::Tag;

use super::VarValue;
use crate::CompileError;

/// The `MVAR` table of `values` (value tag, value); `None` when no value varies.
pub fn build_mvar(axis_count: u16, values: &[(Tag, VarValue)]) -> Result<Option<Mvar>, CompileError> {
    // M9.6
    let _ = (axis_count, values);
    Err(CompileError::NotYetImplemented("the MVAR table"))
}
