//! The mark writer (M6.17): `mark`, `mkmk`, `abvm`, `blwm` and `curs` attachment, the `GDEF`
//! glyph classes and the ligature caret list, generated from propagated anchors.
//!
//! Rows: `compile.markFeatureWriter`, `compile.gdefClasses`, `compile.gdefLigatureCarets`,
//! `anchors.markToBase`, `anchors.markToMark`, `anchors.cursiveAttachment`,
//! `anchors.ligatureCarets`, `autoFeature.mark`, `autoFeature.mkmk`, `autoFeature.curs`,
//! `autoFeature.abvm`, `autoFeature.blwm`.
//!
//! Contract:
//! - The input is plain data ([`MarkInput`]): per glyph, in glyph-id order, its code points, its
//!   category and subcategory (which decide the `GDEF` class) and its anchors per master after
//!   propagation. This crate does not depend on `fontcraft-components`; the compiler builds the
//!   input from `fontcraft_components::propagate::effective_anchors_for_master`.
//! - [`mark_features`] writes one master ([`MarkRequest::master`]). A feature whose
//!   [`MarkTags`] flag is off (the source has a hand-written one without `# Automatic Code`)
//!   is not generated.
//! - The `GPOS` result holds only the generated mark lookups; the compiler merges it with the
//!   kerning of [`crate::kern_feature`].
//! - Every count the work grows with is capped ([`MarkLimits`]).

use write_fonts::tables::gdef::Gdef;
use write_fonts::tables::gpos::Gpos;

use crate::FeatureError;

/// Most steps (glyphs, anchors and attachment rules visited) the writer may take for one master.
pub const MAX_MARK_WORK: usize = 100_000_000;

/// One anchor after propagation, in font units.
#[derive(Clone, Debug, PartialEq)]
pub struct MarkAnchor {
    pub name: String,
    pub x: f64,
    pub y: f64,
}

/// A glyph's anchors on one master.
#[derive(Clone, Debug, PartialEq)]
pub struct MasterAnchors {
    pub master: String,
    pub anchors: Vec<MarkAnchor>,
}

/// One glyph of the compiled font.
#[derive(Clone, Debug, PartialEq)]
pub struct MarkGlyph {
    pub name: String,
    pub codepoints: Vec<u32>,
    /// The effective category and subcategory (`Mark`, `Nonspacing`; `Letter`, `Ligature`…).
    pub category: Option<String>,
    pub sub_category: Option<String>,
    /// The effective anchors, one entry per master that has a layer.
    pub anchors: Vec<MasterAnchors>,
}

/// What the writer reads: the glyphs in the compiled font's order (a glyph's index is its id).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MarkInput {
    pub glyphs: Vec<MarkGlyph>,
}

/// Which generated features to write. Each is on unless the source has a hand-written feature
/// of that tag without `# Automatic Code`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MarkTags {
    pub mark: bool,
    pub mkmk: bool,
    pub abvm: bool,
    pub blwm: bool,
    pub curs: bool,
}

impl Default for MarkTags {
    fn default() -> Self {
        Self { mark: true, mkmk: true, abvm: true, blwm: true, curs: true }
    }
}

/// The caps [`mark_features`] works under. The default is the `MAX_…` constants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MarkLimits {
    pub work: usize,
}

impl Default for MarkLimits {
    fn default() -> Self {
        Self { work: MAX_MARK_WORK }
    }
}

/// What [`mark_features`] writes from.
#[derive(Clone, Copy, Debug)]
pub struct MarkRequest<'a> {
    pub input: &'a MarkInput,
    /// The master compiled; anchors of other masters are ignored.
    pub master: &'a str,
    pub tags: MarkTags,
    pub limits: MarkLimits,
}

/// The generated attachment and glyph classes.
#[derive(Debug, Default)]
pub struct MarkFeatures {
    /// `None` when no attachment rule is generated.
    pub gpos: Option<Gpos>,
    /// The glyph class definitions and the ligature caret list; `None` when no glyph has a class.
    pub gdef: Option<Gdef>,
    /// How many lookups `gpos` has.
    pub lookups: usize,
    /// What the user should know: anchors left out, values that did not fit.
    pub warnings: Vec<String>,
}

/// The mark attachment, cursive attachment and `GDEF` of `req.master`.
pub fn mark_features(req: &MarkRequest) -> Result<MarkFeatures, FeatureError> {
    // M6.17: port fontc's marks.rs and the GDEF class rules (credit in the header and NOTICE).
    let _ = req;
    Ok(MarkFeatures::default())
}
