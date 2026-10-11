//! The `avar` table (M9.6).
//!
//! Rows: the `avar` table row, `compile.avarMaps`.
//!
//! Contract: one segment map per axis from [`VarAxis::avar`](super::VarAxis::avar), an identity
//! map for axes without one; no table when every axis is identity, as fontc writes it.

use write_fonts::tables::avar::Avar;

use super::VarAxes;
use crate::CompileError;

/// The `avar` table of `axes`; `None` when every map is identity.
pub fn build_avar(axes: &VarAxes) -> Result<Option<Avar>, CompileError> {
    // M9.6
    let _ = axes;
    Err(CompileError::NotYetImplemented("the avar table"))
}
