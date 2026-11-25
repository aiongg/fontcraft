//! The final glyph order (`docs/spec/40-compile.md` § 1.4).
//!
//! The font's own order, never re-sorted (`compile.glyphOrder`; M2 plan § 3, question 14), with
//! `.notdef` moved to glyph 0 (`compile.notdefFirst`). A font without a `.notdef` gets one drawn
//! as fontc and ufo2ft draw it (`compile.notdefSynthesis`, ported from fontc's
//! `fontir/src/glyph.rs`, `make_notdef_outline`, Apache-2.0): a hollow box half an em wide, its
//! stroke 5% of the em, from the descender to the ascender, with the ascender to the descender
//! as its vertical advance.
//!
//! TrueType Keep GlyphOrder puts `space` second in a TrueType font, the only other slot the
//! handbook fixes then. Without it the order stays the source's, as both reference compilers write
//! it: the handbook's move of `NULL`, `CR` and `space` to the front is not made
//! (`docs/spec/40-compile.md` § 1.4).
//!
//! Glyphs the compiler makes ([`MadeGlyph`]: colour layer glyphs) follow the source glyphs
//! ([`append_made`]).

use fontcraft_geom::BezPath;
use fontcraft_model::{Resolved, ResolvedGlyph};

use crate::outline::Flavour;
use crate::{CompileError, MAX_FONT_GLYPHS, export_params, ot_round};

/// The name of glyph 0.
pub const NOTDEF: &str = ".notdef";

const SPACE: &str = "space";

/// A glyph the compiler makes that the source does not have, appended after the source glyphs
/// in the order the list gives: a colour layer glyph (`docs/plans/M11.md` § 3 › Layer glyphs) or
/// a variable font's alternate glyph.
#[derive(Clone, Debug, PartialEq)]
pub struct MadeGlyph {
    pub name: String,
    pub source: MadeFrom,
}

/// What a [`MadeGlyph`] is drawn from.
#[derive(Clone, Debug, PartialEq)]
pub enum MadeFrom {
    /// The alternate layers of one range set of `glyph` (a variable font's
    /// `<glyph>.BRACKET.varAltNN`); `set` indexes `fontcraft_interp::GlyphSources::alternates`.
    Alternate { glyph: String, set: usize },
    /// Shapes of the colour layer `layer` (its id) of the source glyph `glyph`.
    ColorLayer { glyph: String, layer: String },
}

/// `glyphs` (already in their final order) with `made` appended in the order given, and the
/// made glyphs' descriptions in the same order. Refused beyond [`MAX_FONT_GLYPHS`].
pub fn append_made(
    mut glyphs: Vec<ResolvedGlyph>,
    made: Vec<(MadeGlyph, ResolvedGlyph)>,
) -> Result<(Vec<ResolvedGlyph>, Vec<MadeGlyph>), CompileError> {
    let total = glyphs.len().saturating_add(made.len());
    if total > MAX_FONT_GLYPHS {
        return Err(CompileError::TooManyGlyphs(total));
    }
    let mut described = Vec::with_capacity(made.len());
    for (m, g) in made {
        described.push(m);
        glyphs.push(g);
    }
    Ok((glyphs, described))
}

/// `glyphs` in their final order: `.notdef` first, drawn when absent; `space` second in a
/// TrueType font with TrueType Keep GlyphOrder on.
pub fn final_order(
    mut glyphs: Vec<ResolvedGlyph>,
    font: &Resolved,
    flavour: Flavour,
    warnings: &mut Vec<String>,
) -> Result<Vec<ResolvedGlyph>, CompileError> {
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
    if flavour == Flavour::TrueType
        && export_params::keeps_glyph_order(&font.params)
        && let Some(i) = glyphs.iter().position(|g| g.name == SPACE)
    {
        let space = glyphs.remove(i);
        glyphs.insert(1, space);
    }
    if glyphs.len() > MAX_FONT_GLYPHS {
        return Err(CompileError::TooManyGlyphs(glyphs.len()));
    }
    Ok(glyphs)
}

/// The `.notdef` drawn for a font that has none.
pub fn notdef(font: &Resolved) -> ResolvedGlyph {
    let upm = f64::from(font.units_per_em);
    // The metrics as the tables write them, rounded half up.
    let ascender = ot_round(font.metrics.ascender);
    let descender = ot_round(font.metrics.descender);
    let mut g = ResolvedGlyph::new(NOTDEF, ot_round(upm * 0.5));
    g.contours = notdef_outline(upm, ascender, descender);
    // Its vertical advance, should the font be vertical: the ascender to the descender.
    g.vert_width = Some(ascender - descender);
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
