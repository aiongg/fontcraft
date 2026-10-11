//! The crate's error type and its named limits (`docs/plans/M6.md` § 3 › Limits).
//!
//! Every count read from a font, a recipe or a command parameter is checked against one of these
//! limits before the work it bounds.

use fontcraft_model::ModelError;

/// Deepest component nesting followed; deeper is reported as [`ComponentsError::TooDeep`]. The
/// model's outline resolution uses the same limit.
pub const MAX_COMPONENT_DEPTH: usize = fontcraft_model::MAX_COMPONENT_DEPTH;
/// Most components on one layer (the compiler's limit, `fontcraft_compile::MAX_COMPONENTS`).
pub const MAX_LAYER_COMPONENTS: usize = 65_535;
/// Most anchors on one layer, own and propagated together.
pub const MAX_LAYER_ANCHORS: usize = 4_096;
/// Most steps of aligning one layer: each component costs one plus the anchors of its base. A
/// layer may hold [`MAX_LAYER_COMPONENTS`] components of bases with [`MAX_LAYER_ANCHORS`] anchors
/// each; this bounds the product.
pub const MAX_ALIGN_WORK: usize = 1_000_000;
/// Longest anchor name, in bytes.
pub const MAX_ANCHOR_NAME_LEN: usize = fontcraft_model::MAX_NAME_LEN;
/// Longest recipe, in bytes.
pub const MAX_RECIPE_BYTES: usize = 4_096;
/// Most components a recipe may name.
pub const MAX_RECIPE_PARTS: usize = 64;
/// Most glyphs one composition request may build (Create Composite on a selection).
pub const MAX_COMPOSE_GLYPHS: usize = fontcraft_model::MAX_GLYPHS;

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum ComponentsError {
    /// The glyphs, in order, of a chain of components that returns to its start.
    #[error("components refer to each other in a circle: {}", .0.join(" → "))]
    Cycle(Vec<String>),
    #[error("the components of `{glyph}` nest more than {limit} deep")]
    TooDeep { glyph: String, limit: usize },
    #[error("there is no glyph called `{0}`")]
    MissingGlyph(String),
    #[error("there is no master or layer `{layer}` in `{glyph}`")]
    MissingLayer { glyph: String, layer: String },
    /// A composition needs glyphs the font does not have; nothing is built.
    #[error("`{glyph}` needs glyphs the font does not have: {}", .missing.join(", "))]
    MissingParts { glyph: String, missing: Vec<String> },
    #[error("the recipe cannot be read: {0}")]
    BadRecipe(String),
    #[error("more than {limit} {what}")]
    LimitExceeded { what: &'static str, limit: usize },
    /// A transform or position that is not a finite number, or one outside the model's range.
    #[error("`{glyph}` has a component placement that is not a usable number")]
    BadPlacement { glyph: String },
    #[error(transparent)]
    Model(#[from] ModelError),
    /// A function of this crate whose M6 package has not been written yet.
    #[error("{0} is not available yet")]
    NotYetImplemented(&'static str),
}
