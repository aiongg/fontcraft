//! The `fvar` table (M9.6).
//!
//! Rows: the `fvar` table row, `compile.fvarInstances`, `instances.namedInstancesInVariable`.
//!
//! Contract: one axis record per [`VarAxis`](super::VarAxis) (hidden axes flagged), one instance
//! record per named instance with its subfamily name and, when the font has any PostScript
//! names for instances, a PostScript name ID; at most 65,535 instances. Names come from the
//! shared [`NameAllocator`].

use write_fonts::tables::fvar::Fvar;

use super::names::NameAllocator;
use super::{VarAxes, VarInstance};
use crate::CompileError;

/// The `fvar` table of `axes` and `instances`.
pub fn build_fvar(axes: &VarAxes, instances: &[VarInstance], names: &mut NameAllocator) -> Result<Fvar, CompileError> {
    // M9.6
    let _ = (axes, instances, names);
    Err(CompileError::NotYetImplemented("the fvar table"))
}
