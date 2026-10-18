//! Curve conversion and componentizing on compiled fonts (M12.29; rows `outlines to-quadratic`,
//! `to-cubic`, `componentize`, `outlines.convertCurves`, `outlines.componentize`). Oracles: cu2qu,
//! qu2cu, ttfcomponentizer. The converters live in `fontcraft-geom`; the `glyf` work is in
//! `outlines` (its `outlines_curves.rs` part), next to the other outline edits.
//!
//! | Action | What changes |
//! |---|---|
//! | To cubic | As fontTools' qu2cu: runs of quadratic curves become cubic curves where that needs fewer points (every curve with `all_cubic`), within `max_error` units; `glyf` keeps them as cubic off-curve points and `head.glyphDataFormat` becomes 1 |
//! | To quadratic | Cubic curves in `glyf` become quadratic splines within `max_error` units (fontTools' cu2qu); with `mixed`, a curve that one quadratic cannot replace stays cubic. `head.glyphDataFormat` goes back to 0 once no cubic curve is left |
//! | Componentize | Glyphs whose contours are other glyphs' contours moved by whole units become composites: as a reference source builds them (AFDKO ttfcomponentizer), or found by matching contours |
//!
//! Only TrueType (`glyf`) fonts without `gvar` are converted; CFF outlines change format through
//! `convert`. Untouched tables are copied byte for byte.

use std::collections::BTreeMap;

use crate::outlines::{GlyphFailure, OutlineResult};
use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

/// Largest error bound accepted, in font units.
pub const MAX_ERROR_UNITS: f64 = 1_000.0;

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
    /// To cubic: every quadratic curve becomes cubic, not only where that saves points.
    pub all_cubic: bool,
    /// To quadratic: keep a cubic curve that one quadratic curve cannot replace.
    pub mixed: bool,
    /// Glyphs to convert; empty converts every glyph.
    pub glyphs: Vec<String>,
    /// Glyphs left alone even when `glyphs` names them.
    pub exclude_glyphs: Vec<String>,
    /// Leave a glyph that cannot be converted as it was and report it, instead of failing.
    pub ignore_errors: bool,
}

impl CurveOptions {
    /// `target` on every glyph, writing cubic curves into `glyf`.
    pub fn new(target: CurveTarget) -> Self {
        Self { target, glyf_cubic: true, all_cubic: false, mixed: false, glyphs: Vec::new(), exclude_glyphs: Vec::new(), ignore_errors: false }
    }
}

/// What [`convert_curves_report`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CurveReport {
    pub result: OutlineResult,
    /// Glyphs left as they were (only with `ignore_errors`).
    pub failed: Vec<GlyphFailure>,
    /// Curves converted: cubic curves made (to cubic) or cubic curves replaced (to quadratic).
    pub segments: usize,
}

/// The font with its curves converted.
pub fn convert_curves(font: &SfntFont, options: &CurveOptions) -> Result<OutlineResult> {
    convert_curves_report(font, options).map(|r| r.result)
}

/// The font with its curves converted, and what changed.
pub fn convert_curves_report(font: &SfntFont, options: &CurveOptions) -> Result<CurveReport> {
    let max_error = match options.target {
        CurveTarget::Quadratic { max_error, reverse_direction } => {
            if reverse_direction {
                return Err(TablesError::Unsupported(
                    "reversing contours while converting a compiled font: its contours already run as TrueType expects".into(),
                ));
            }
            if options.all_cubic {
                return Err(TablesError::BadOptions("allCubic belongs to the conversion to cubic curves".into()));
            }
            max_error
        }
        CurveTarget::Cubic { max_error } => {
            if !options.glyf_cubic {
                return Err(TablesError::Unsupported("moving outlines to CFF: use `convert`".into()));
            }
            if options.mixed {
                return Err(TablesError::BadOptions("mixed belongs to the conversion to quadratic curves".into()));
            }
            max_error
        }
    };
    if !(max_error.is_finite() && max_error > 0.0 && max_error <= MAX_ERROR_UNITS) {
        return Err(TablesError::BadOptions(format!("the conversion error must be above 0 and at most {MAX_ERROR_UNITS} units")));
    }
    crate::outlines::curve_work::convert(font, options)
}

/// One component of a composite in a reference source.
#[derive(Clone, Debug, PartialEq)]
pub struct ReferenceComponent {
    /// The component's glyph, by its name in the compiled font.
    pub glyph: String,
    /// `xx, xy, yx, yy, dx, dy` in fontTools' order: `x' = xx·x + yx·y + dx`, `y' = xy·x + yy·y + dy`.
    pub transform: [f64; 6],
}

/// How a reference source builds one glyph.
#[derive(Clone, Debug, PartialEq)]
pub struct ReferenceGlyph {
    pub components: Vec<ReferenceComponent>,
    /// The glyph has contours of its own too (it is skipped).
    pub has_contours: bool,
    /// The glyph's advance equals its first component's in the source.
    pub same_advance_as_first: bool,
}

/// What [`componentize_report`] does.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ComponentizeOptions {
    /// Composites by glyph name in the compiled font; `None` finds components by matching contours.
    pub reference: Option<BTreeMap<String, ReferenceGlyph>>,
    /// Use scaled, rotated or flipped components of the reference too.
    pub allow_transforms: bool,
    /// Glyphs to rebuild; empty considers every glyph.
    pub glyphs: Vec<String>,
    /// Glyphs left alone even when `glyphs` names them.
    pub exclude_glyphs: Vec<String>,
}

/// What [`componentize_report`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentizeReport {
    pub result: OutlineResult,
    /// Glyphs the reference makes composites that were left alone, and why.
    pub skipped: Vec<GlyphFailure>,
}

/// The font with repeated outlines turned into components, found by matching contours.
pub fn componentize(font: &SfntFont) -> Result<OutlineResult> {
    componentize_report(font, &ComponentizeOptions::default()).map(|r| r.result)
}

/// The font with glyphs rebuilt from components, and what was skipped.
pub fn componentize_report(font: &SfntFont, options: &ComponentizeOptions) -> Result<ComponentizeReport> {
    crate::outlines::curve_work::componentize(font, options)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bad_options_are_refused_before_reading_the_font() {
        let font = SfntFont::new(crate::sfnt::Flavour::TrueType);
        let mut o = CurveOptions::new(CurveTarget::Cubic { max_error: 0.0 });
        assert!(matches!(convert_curves(&font, &o), Err(TablesError::BadOptions(_))));
        o.target = CurveTarget::Cubic { max_error: f64::NAN };
        assert!(matches!(convert_curves(&font, &o), Err(TablesError::BadOptions(_))));
        o.target = CurveTarget::Cubic { max_error: 1.0 };
        o.glyf_cubic = false;
        assert!(matches!(convert_curves(&font, &o), Err(TablesError::Unsupported(_))));
        let o = CurveOptions { all_cubic: true, ..CurveOptions::new(CurveTarget::Quadratic { max_error: 1.0, reverse_direction: false }) };
        assert!(matches!(convert_curves(&font, &o), Err(TablesError::BadOptions(_))));
        let o = CurveOptions::new(CurveTarget::Quadratic { max_error: 1.0, reverse_direction: false });
        assert!(matches!(convert_curves(&font, &o), Err(TablesError::MissingTable(_))));
    }
}
