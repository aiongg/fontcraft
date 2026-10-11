//! The `CFF2` flavour of the variable compile (M9.13).
//!
//! Rows: `compile.cff2VariableBuild`, `compile.cff2CompanionTables`.
//!
//! Contract: each glyph's cubic sources become blended charstrings for
//! `fontcraft_cff::cff2::write_cff2`, decomposed and with their region lists shared; `hmtx` comes
//! with `HVAR`, `post` holds the glyph names. Sources must be point-compatible.

use fontcraft_cff::cff2::Cff2Font;
use fontcraft_geom::BezPath;

use super::VarAxes;
use crate::CompileError;

/// One glyph's cubic outlines at each of its sources.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CubicSources {
    pub glyph: String,
    /// Normalized location of each source; the first is the origin.
    pub locations: Vec<Vec<f64>>,
    /// Per source, the glyph's contours, the same structure in each.
    pub paths: Vec<Vec<BezPath>>,
}

/// The `CFF2` contents of `glyphs` (in glyph order) for a font with `axes`.
pub fn cff2_font(axes: &VarAxes, glyphs: &[CubicSources], units_per_em: u16) -> Result<Cff2Font, CompileError> {
    // M9.13
    let _ = (axes, glyphs, units_per_em);
    Err(CompileError::NotYetImplemented("CFF2 variable fonts"))
}
