//! Variable kerning and mark positions: per-master values as a default plus deltas (M9.9).
//!
//! Rows: `compile.instanceLayoutInterpolation` (variable half), `masters.metricsInterpolate`
//! (kerning and anchors in variable fonts).
//!
//! Contract (`docs/plans/M9.md` § 3 › Design decisions):
//! - Every value is turned into a [`VarValue`] through the font's variation model: the origin
//!   master's value, then one delta per region. A pair a master lacks counts as 0 there, as fontc
//!   does.
//! - The deltas go into one `VariationStoreBuilder` shared by kerning and marks; the `GPOS` holds
//!   `PendingVariationIndex` device tables until [`finish_store`] builds the store, writes it to
//!   the `GDEF` and remaps the indices.
//! - The variable writers keep the static writers' lookups, features and limits
//!   ([`crate::KernLimits`], [`crate::marks::MarkLimits`]); only the values change.
//!
//! Where this meets M8: the neutral generated lookups of the `fea` crate carry values as
//! `fea::Metric { default, deltas: Vec<(Region, i16)> }` with regions keyed by axis tag;
//! [`VarValue`] converts to it once both are in, with the axis tags of the variable font.

use fontcraft_geom::variation::{Region, VariationModel};
use fontcraft_model::Font;
use fontcraft_spacing::kerning::KernIndex;
use write_fonts::tables::gdef::Gdef;
use write_fonts::tables::gpos::Gpos;
use write_fonts::tables::variations::ivs_builder::VariationStoreBuilder;

use crate::marks::{MarkFeatures, MarkInput, MarkLimits, MarkTags};
use crate::{FeatureError, KernFeature, KernGlyph, KernLimits, KernTags};

/// A value that varies: the origin's value and one delta per region, in font units.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VarValue {
    pub default: i32,
    pub deltas: Vec<(Region, i32)>,
}

/// A master of the variable font and its normalized location (one value per axis of the font, in
/// `fvar` order).
#[derive(Clone, Copy, Debug)]
pub struct MasterAt<'a> {
    pub id: &'a str,
    pub location: &'a [f64],
}

/// What [`variable_kern_feature`] writes from: [`crate::KernRequest`] with every master.
pub struct VarKernRequest<'a> {
    pub font: &'a Font,
    pub index: &'a KernIndex,
    /// The variable font's masters, the origin among them.
    pub masters: &'a [MasterAt<'a>],
    /// The glyphs in the compiled font's order: a glyph's index is its glyph id.
    pub glyphs: &'a [KernGlyph<'a>],
    pub tags: KernTags,
    pub limits: KernLimits,
}

/// What [`variable_mark_features`] writes from: [`crate::marks::MarkRequest`] with every master.
pub struct VarMarkRequest<'a> {
    pub input: &'a MarkInput,
    pub masters: &'a [MasterAt<'a>],
    pub tags: MarkTags,
    pub limits: MarkLimits,
}

/// `values` (one per master, in the model's master order) as a default plus rounded deltas.
pub fn var_value(model: &VariationModel, values: &[f64]) -> Result<VarValue, FeatureError> {
    // M9.9
    let _ = (model, values);
    Err(FeatureError::NotYetImplemented("variable values"))
}

/// The variable `GPOS` of the font's kerning, its deltas added to `store`.
pub fn variable_kern_feature(req: &VarKernRequest, store: &mut VariationStoreBuilder) -> Result<KernFeature, FeatureError> {
    // M9.9
    let _ = (req, store);
    Err(FeatureError::NotYetImplemented("variable kerning"))
}

/// The variable mark attachment lookups and `GDEF`, their deltas added to `store`.
pub fn variable_mark_features(req: &VarMarkRequest, store: &mut VariationStoreBuilder) -> Result<MarkFeatures, FeatureError> {
    // M9.9
    let _ = (req, store);
    Err(FeatureError::NotYetImplemented("variable mark attachment"))
}

/// Builds `store`, writes it to `gdef` and replaces the pending variation indices in `gpos` with
/// the final ones.
pub fn finish_store(store: VariationStoreBuilder, gpos: Option<&mut Gpos>, gdef: &mut Gdef) -> Result<(), FeatureError> {
    // M9.9
    let _ = (store, gpos, gdef);
    Err(FeatureError::NotYetImplemented("the variation store of the layout tables"))
}
