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
use fontcraft_model::{Font, MAX_NAME_LEN};

use crate::compose::is_mark;
use crate::{ComponentsError, MAX_RECIPE_BYTES, MAX_RECIPE_PARTS};

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

/// The recipe in `text`; see the module doc. Leading and trailing white space is ignored; a
/// recipe holds no other white space, because spaces separate items in the Add Glyphs field.
pub fn parse_recipe(text: &str) -> Result<Recipe, ComponentsError> {
    if text.len() > MAX_RECIPE_BYTES {
        return Err(ComponentsError::LimitExceeded { what: "bytes in a recipe", limit: MAX_RECIPE_BYTES });
    }
    let text = text.trim();
    if text.chars().any(char::is_whitespace) {
        return Err(ComponentsError::BadRecipe("a recipe has no spaces".into()));
    }
    let Some((left, target)) = text.split_once('=') else {
        return Err(ComponentsError::BadRecipe("a recipe needs `=` before the new glyph's name".into()));
    };
    if target.contains('=') {
        return Err(ComponentsError::BadRecipe("a recipe has one `=`".into()));
    }
    if target.contains('+') {
        return Err(ComponentsError::BadRecipe("the new glyph's name after `=` cannot contain `+`".into()));
    }
    check_name(target, "the new glyph's name")?;
    let mut parts = Vec::new();
    for part in left.split('+') {
        if parts.len() == MAX_RECIPE_PARTS {
            return Err(ComponentsError::LimitExceeded { what: "components in a recipe", limit: MAX_RECIPE_PARTS });
        }
        check_name(part, "a component name")?;
        if part == target {
            return Err(ComponentsError::BadRecipe(format!("`{target}` cannot be a component of itself")));
        }
        parts.push(part.to_string());
    }
    Ok(Recipe { parts, target: target.to_string() })
}

fn check_name(name: &str, what: &str) -> Result<(), ComponentsError> {
    if name.is_empty() {
        return Err(ComponentsError::BadRecipe(format!("{what} is empty")));
    }
    if name.len() > MAX_NAME_LEN {
        return Err(ComponentsError::LimitExceeded { what: "bytes in a glyph name", limit: MAX_NAME_LEN });
    }
    fontcraft_glyphdata::validate(name, false).map_err(|e| ComponentsError::BadRecipe(format!("{what} `{name}`: {e}")))
}

/// The role of each part of `recipe`, parallel to `recipe.parts`: a part whose glyph has the
/// category Mark (stored in the font, else in the glyph database) is a mark, except the first,
/// which has no base before it to attach to; every other part is a base.
pub fn classify(font: &Font, data: &GlyphData, recipe: &Recipe) -> Vec<PartRole> {
    recipe.parts.iter().enumerate().map(|(i, part)| if i > 0 && is_mark(font, data, part) { PartRole::Mark } else { PartRole::Base }).collect()
}
