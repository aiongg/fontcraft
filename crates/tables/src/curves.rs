//! Curve conversion and componentizing on compiled fonts (M12.29; rows `outlines to-quadratic`,
//! `to-cubic`, `componentize`, `outlines.convertCurves`, `outlines.componentize`). Oracles: cu2qu,
//! qu2cu, ttfcomponentizer. The converters live in `fontcraft-geom`.

use crate::outlines::OutlineResult;
use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

/// The direction of a curve conversion.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CurveTarget {
    /// Cubic to quadratic (`glyf`), within `max_error` font units.
    Quadratic { max_error: f64, reverse_direction: bool },
    /// Quadratic to cubic, within `max_error` font units.
    Cubic { max_error: f64 },
}

/// What [`convert_curves`] does.
#[derive(Clone, Debug, PartialEq)]
pub struct CurveOptions {
    pub target: CurveTarget,
    /// Write `glyf` version 1 (cubic curves in `glyf`) instead of moving to `CFF `.
    pub glyf_cubic: bool,
}

/// The font with its curves converted.
pub fn convert_curves(font: &SfntFont, options: &CurveOptions) -> Result<OutlineResult> {
    // M12.29
    let _ = (font, options);
    Err(TablesError::NotBuilt("curve conversion"))
}

/// The font with repeated outlines turned into components.
pub fn componentize(font: &SfntFont) -> Result<OutlineResult> {
    // M12.29
    let _ = font;
    Err(TablesError::NotBuilt("componentize"))
}
