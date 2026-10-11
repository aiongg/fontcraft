//! `CBDT` and `CBLC` (M11.11), an export option the reference app does not have: strikes from the
//! same iColor layers as sbix, PNG data in glyph bitmap format 17 with small metrics, index
//! subtable format 1, as nanoemoji writes them.
//!
//! Rows: `CBDT`, `color.export.cbdtNotInReference`.

use fontcraft_color::AssetStore;
use fontcraft_color::export::CbdtPlan;

use super::GlyphOrder;
use crate::CompileError;

/// The two tables built together.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CbdtOutput {
    pub cbdt: Vec<u8>,
    pub cblc: Vec<u8>,
}

/// The tables. A missing or unreadable image leaves the glyph out of its strike, with a warning.
pub fn build(
    plan: &CbdtPlan,
    order: &GlyphOrder,
    assets: &dyn AssetStore,
    units_per_em: u16,
    warnings: &mut Vec<String>,
) -> Result<CbdtOutput, CompileError> {
    // M11.11
    let _ = (plan, order, assets, units_per_em, warnings);
    Err(CompileError::NotYetImplemented("the CBDT table"))
}
