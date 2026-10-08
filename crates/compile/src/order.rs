//! The final glyph order (`docs/spec/40-compile.md` § 1.4).
//!
//! The font's own order, never re-sorted (`compile.glyphOrder`; M2 plan § 3, question 14), with
//! `.notdef` moved to glyph 0 (`compile.notdefFirst`). A font without a `.notdef` gets one drawn
//! as fontc and ufo2ft draw it (`compile.notdefSynthesis`, ported from fontc's
//! `fontir/src/glyph.rs`, `make_notdef_outline`, Apache-2.0): a hollow box half an em wide, its
//! stroke 5% of the em, from the descender to the ascender.

use fontcraft_geom::BezPath;
use fontcraft_model::{Resolved, ResolvedGlyph};

use crate::{CompileError, MAX_FONT_GLYPHS, ot_round};

/// The name of glyph 0.
pub const NOTDEF: &str = ".notdef";

/// `glyphs` in their final order: `.notdef` first, drawn when absent.
pub fn final_order(mut glyphs: Vec<ResolvedGlyph>, font: &Resolved, warnings: &mut Vec<String>) -> Result<Vec<ResolvedGlyph>, CompileError> {
    match glyphs.iter().position(|g| g.name == NOTDEF) {
        Some(0) => {}
        Some(i) => {
            let notdef = glyphs.remove(i);
            glyphs.insert(0, notdef);
        }
        None => {
            warnings.push("the font has no `.notdef` glyph; glyph 0 was drawn as a hollow box".to_string());
            glyphs.insert(0, notdef(font));
        }
    }
    if glyphs.len() > MAX_FONT_GLYPHS {
        return Err(CompileError::TooManyGlyphs(glyphs.len()));
    }
    Ok(glyphs)
}

/// The `.notdef` drawn for a font that has none.
pub fn notdef(font: &Resolved) -> ResolvedGlyph {
    let upm = f64::from(font.units_per_em);
    let ascender = font.metrics.ascender.unwrap_or(0.8 * upm);
    let descender = font.metrics.descender.unwrap_or(-0.2 * upm);
    let mut g = ResolvedGlyph::new(NOTDEF, ot_round(upm * 0.5));
    g.contours = notdef_outline(upm, ascender, descender);
    g
}

/// The box: an outer contour counter-clockwise and an inner one clockwise, each closed with an
/// explicit line back to its start, as fontc builds them.
pub fn notdef_outline(upm: f64, ascender: f64, descender: f64) -> Vec<BezPath> {
    let width = ot_round(upm * 0.5);
    let stroke = ot_round(upm * 0.05);
    let (x_min, x_max, y_min, y_max) = (stroke, width - stroke, descender, ascender);
    let mut outer = BezPath::new();
    outer.move_to((x_min, y_min));
    outer.line_to((x_max, y_min));
    outer.line_to((x_max, y_max));
    outer.line_to((x_min, y_max));
    outer.line_to((x_min, y_min));
    outer.close_path();
    let (x_min, x_max, y_min, y_max) = (x_min + stroke, x_max - stroke, y_min + stroke, y_max - stroke);
    let mut inner = BezPath::new();
    inner.move_to((x_min, y_min));
    inner.line_to((x_min, y_max));
    inner.line_to((x_max, y_max));
    inner.line_to((x_max, y_min));
    inner.line_to((x_min, y_min));
    inner.close_path();
    vec![outer, inner]
}
