//! The `HVAR` and `VVAR` tables (M9.6).
//!
//! Rows: the `HVAR` and `VVAR` table rows, `compile.hvarStoreChoice`.
//!
//! Contract: the advance of each glyph (in glyph order) as a [`VarValue`]; the deltas go into an
//! item variation store, written with an advance-width mapping or as direct rows, whichever is
//! smaller, as fontc chooses. Side-bearing variations are left to `gvar`'s phantom points.

use write_fonts::tables::hvar::Hvar;
use write_fonts::tables::vvar::Vvar;

use super::VarValue;
use crate::CompileError;

/// The `HVAR` table of `advances` (one per glyph, in glyph order).
pub fn build_hvar(axis_count: u16, advances: &[VarValue]) -> Result<Hvar, CompileError> {
    // M9.6
    let _ = (axis_count, advances);
    Err(CompileError::NotYetImplemented("the HVAR table"))
}

/// The `VVAR` table of vertical `advances` (one per glyph, in glyph order).
pub fn build_vvar(axis_count: u16, advances: &[VarValue]) -> Result<Vvar, CompileError> {
    // M9.6
    let _ = (axis_count, advances);
    Err(CompileError::NotYetImplemented("the VVAR table"))
}
