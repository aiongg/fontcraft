//! Alternate layers as feature variations (M9.8).
//!
//! Rows: `alternateLayer.variableExport`, `compile.featureVariations`,
//! `compile.bracketLayerGlyphs`, `switching.threeMethods` (alternate half), parameter
//! `Feature for Feature Variations`.
//!
//! Contract:
//! - Each alternate set of a glyph becomes a glyph of its own, named
//!   `<glyph>.BRACKET.varAltNN` ([`bracket_glyph_name`]), appended to the glyph order as a
//!   [`MadeGlyph`] with the parent's production name and the same suffix.
//! - The sets' boxes go through `fontcraft_interp::overlay`; each record becomes a
//!   [`FeatureVariation`] of single substitutions under the feature the Feature for Feature
//!   Variations parameter names, [`DEFAULT_FEATURE`] when it names none (`docs/plans/M9.md` § 7
//!   question 2, decided).
//! - Conditions are in normalized coordinates after `avar`, as `GSUB` stores them.
//! - Until M8's compile merges them into the feature code's `GSUB`, [`build_gsub`] writes a
//!   `GSUB` holding only these lookups.
//!
//! Where this meets M8: [`FeatureVariation`] has the fields of
//! `fea::GeneratedLookups::feature_variations`' entries; when M8 is in, the records are handed to
//! `fea` and [`build_gsub`] is no longer called.

use fontcraft_interp::{Designspace, OverlayRecord};
use fontcraft_model::Font;
use write_fonts::tables::gsub::Gsub;
use write_fonts::types::{GlyphId16, Tag};

use super::VarAxes;
use crate::CompileError;
use crate::order::MadeGlyph;

/// The feature that switches alternate layers when the font names none.
pub const DEFAULT_FEATURE: &str = "rvrn";
/// The font parameter naming the feature for feature variations.
pub const FEATURE_PARAMETER: &str = "Feature for Feature Variations";
/// What an alternate glyph's name adds to its parent's, before the two-digit number.
pub const BRACKET_INFIX: &str = ".BRACKET.varAlt";

/// One feature variation: where it applies and what it substitutes.
#[derive(Clone, Debug, PartialEq)]
pub struct FeatureVariation {
    /// The feature whose lookups the record replaces.
    pub tag: Tag,
    /// `(axis tag, min, max)` in normalized coordinates; all must hold.
    pub conditions: Vec<(Tag, f64, f64)>,
    pub substitutions: Vec<(GlyphId16, GlyphId16)>,
}

/// The name of the `number`th alternate glyph of `parent` (`a.BRACKET.varAlt01`).
pub fn bracket_glyph_name(parent: &str, number: usize) -> String {
    format!("{parent}{BRACKET_INFIX}{number:02}")
}

/// The alternate glyphs of `font` in `designspace`, in glyph order then set order.
pub fn bracket_glyphs(font: &Font, designspace: &Designspace) -> Result<Vec<MadeGlyph>, CompileError> {
    // M9.8
    let _ = (font, designspace);
    Ok(Vec::new())
}

/// The feature variations of the overlay's `records` (each payload a list of (glyph, alternate
/// glyph) names), with glyph ids from `glyph_order` and condition axes from `axes`.
pub fn feature_variations(
    records: &[OverlayRecord<Vec<(String, String)>>],
    axes: &VarAxes,
    feature: Tag,
    glyph_order: &[String],
) -> Result<Vec<FeatureVariation>, CompileError> {
    // M9.8
    let _ = (records, axes, feature, glyph_order);
    Err(CompileError::NotYetImplemented("feature variations"))
}

/// A `GSUB` holding only `variations`; `None` when there are none.
pub fn build_gsub(variations: &[FeatureVariation]) -> Result<Option<Gsub>, CompileError> {
    // M9.8
    let _ = variations;
    Err(CompileError::NotYetImplemented("the GSUB of feature variations"))
}
