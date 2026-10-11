//! Transforming every glyph and scaling units per em (M12.24; rows `transform`,
//! `font.transform`). Oracles: AFDKO rotatefont, fontTools `ttLib.scaleUpem`.

use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

/// What [`transform`] does: a matrix, or the named parts multiplied in field order (scale,
/// slant, rotate), then optionally new units per em.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TransformOptions {
    /// `[a, b, c, d, e, f]` as in PostScript; values finite. Wins over the named parts.
    pub matrix: Option<[f64; 6]>,
    pub scale: Option<(f64, f64)>,
    /// Slant angle in degrees.
    pub slant: Option<f64>,
    /// Rotation in degrees, counter-clockwise.
    pub rotate: Option<f64>,
    /// New units per em, from [`crate::limits::MIN_UNITS_PER_EM`] to
    /// [`crate::limits::MAX_UNITS_PER_EM`]; every metric scales with it.
    pub units_per_em: Option<u16>,
}

/// The transformed font. Refuses a result whose `glyf` coordinates pass
/// [`crate::limits::MAX_COORDINATE`].
pub fn transform(font: &SfntFont, options: &TransformOptions) -> Result<SfntFont> {
    // M12.24
    let _ = (font, options);
    Err(TablesError::NotBuilt("transform"))
}
