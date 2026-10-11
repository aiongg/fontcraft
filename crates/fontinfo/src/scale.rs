//! A font scaled to a new units-per-em: every value measured in font units, as the
//! `font.info.scaleToUnitsPerEm` row's guess lists them (outlines, component offsets, anchors,
//! guides, hints, backgrounds, special layers, kerning, metrics, zones, stems, and the parameters
//! whose catalogue entry has [`crate::ParamSpec::units`]), rounded to the font's grid.
//!
//! A scaling that would move any value past [`crate::MAX_SCALED_COORDINATE`] is refused, and the
//! font is unchanged. The factor is `units_per_em / font.units_per_em`, always finite.
//!
//! Rows: `font.info.scaleToUnitsPerEm`, `fontInfo.unitsPerEm.resizeTrick` (M7.4); the `Scale to
//! UPM` parameter at export (`engine/src/export_steps.rs`, M7.10).

use fontcraft_model::Font;

use crate::error::ScaleError;

/// `font` with units per em `units_per_em` and every value in font units scaled to match.
pub fn scale_font(font: &Font, units_per_em: u16) -> Result<Font, ScaleError> {
    if !crate::UPM_RANGE.contains(&units_per_em) {
        return Err(ScaleError::UnitsPerEm(u32::from(units_per_em)));
    }
    if units_per_em == font.units_per_em {
        return Ok(font.clone());
    }
    // M7.4: the scaling.
    Err(ScaleError::NotYetImplemented("scaling to a new units per em"))
}
