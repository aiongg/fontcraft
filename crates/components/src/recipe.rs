//! Glyph recipes (M6.3): the `component=glyph`, `base+mark=glyph` and `base+base=glyph` forms
//! that Add Glyphs accepts.
//!
//! Rows: `components.recipeSingleComponent`, `components.recipeBasePlusMarks`,
//! `components.recipeMultipleBases`, `addGlyphs.recipe` (resolution).
//!
//! Contract:
//! - [`parse_recipe`] reads one recipe: part names joined by `+`, then `=`, then the new glyph's
//!   name. Hostile input is refused with [`ComponentsError::BadRecipe`] or
//!   [`ComponentsError::LimitExceeded`]: more than [`crate::MAX_RECIPE_BYTES`] bytes, more than
//!   [`crate::MAX_RECIPE_PARTS`] parts, an empty or over-long name.
//! - [`is_recipe`] tells a recipe from a plain glyph name in the Add Glyphs field.
//! - [`classify`] says which parts are bases and which are marks, from the glyph's category in
//!   the font, else in the glyph database.

use fontcraft_glyphdata::GlyphData;
use fontcraft_model::Font;

use crate::ComponentsError;

/// One parsed recipe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recipe {
    /// The components, in the order written.
    pub parts: Vec<String>,
    /// The glyph to create.
    pub target: String,
}

/// What a recipe part becomes in the composite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartRole {
    /// Set side by side with the other bases.
    Base,
    /// Attached by anchors to the preceding base.
    Mark,
}

/// Whether `text` has the shape of a recipe (contains `=`), without checking it further.
pub fn is_recipe(text: &str) -> bool {
    text.contains('=')
}

/// The recipe in `text`; see the module doc.
pub fn parse_recipe(text: &str) -> Result<Recipe, ComponentsError> {
    // M6.3
    let _ = text;
    Err(ComponentsError::NotYetImplemented("recipes"))
}

/// The role of each part of `recipe`, parallel to `recipe.parts`.
pub fn classify(font: &Font, data: &GlyphData, recipe: &Recipe) -> Vec<PartRole> {
    // M6.3
    let _ = (font, data);
    vec![PartRole::Base; recipe.parts.len()]
}
