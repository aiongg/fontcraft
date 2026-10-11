//! FontCraft components (L2): the reverse component index, anchor propagation, automatic
//! alignment and derived metrics, and composition from the glyph database and recipes.
//! Pure functions over a [`fontcraft_model::Font`]; no I/O. Plan: `docs/plans/M6.md` § 3 and
//! § 4 (M6.1 to M6.3). Behaviour notes: `docs/spec/09-reusing-shapes.md`; rows in
//! `spec/inventory/09-reusing-shapes.toml`.
//!
//! | Module | Package | What it gives |
//! |---|---|---|
//! | [`index`] | M6.1 | glyph → glyphs that use it, dependency order, cycles |
//! | [`propagate`] | M6.1 | the effective anchors of a layer, the Propagate Anchors flag, a cache |
//! | [`align`] | M6.2 | which components are aligned and where they go |
//! | [`metrics`] | M6.2 | derived width and sidebearings, `=+N` / `=-N`, the engine's re-alignment hook |
//! | [`compose`] | M6.3 | the parts of a glyph name, a new composite glyph |
//! | [`recipe`] | M6.3 | recipe parsing and part roles |
//! | [`default_anchors`] | M6.3 | the anchors Set Anchors adds |
//!
//! Who calls what: the engine runs [`metrics::realign`] after every font-changing command, inside
//! the same undo step. Commands use [`align`], [`compose`], [`recipe`] and [`default_anchors`].
//! The compiler reads [`propagate::effective_anchors_for_master`] and hands the result to the
//! mark writer in `fontcraft-features` as plain data.
//!
//! Limits are named in [`error`]; every function returns [`ComponentsError`] instead of
//! panicking.
#![forbid(unsafe_code)]

pub mod align;
pub mod compose;
pub mod default_anchors;
pub mod error;
pub mod index;
pub mod metrics;
pub mod propagate;
pub mod recipe;

pub use error::{
    ComponentsError, MAX_ANCHOR_NAME_LEN, MAX_COMPONENT_DEPTH, MAX_COMPOSE_GLYPHS, MAX_LAYER_ANCHORS, MAX_LAYER_COMPONENTS, MAX_RECIPE_BYTES,
    MAX_RECIPE_PARTS,
};
pub use index::ComponentIndex;
pub use propagate::{AnchorCache, AnchorSource, EffectiveAnchor};

#[cfg(test)]
mod tests {
    use fontcraft_model::{Font, GlyphTable};

    use crate::ComponentIndex;

    fn empty_font() -> Font {
        let mut font = Font::new("Empty");
        font.glyphs = GlyphTable::from_glyphs(Vec::new());
        font
    }

    #[test]
    fn the_index_of_an_empty_font_is_empty() {
        let index = ComponentIndex::new(&empty_font());
        assert!(index.is_empty());
        assert!(index.dependency_order().is_empty());
        assert!(index.cycles().is_empty());
    }
}
