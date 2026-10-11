//! Which composite glyphs a variable font decomposes (M9.5).
//!
//! Rows: `compile.inconsistentComponentsDecompose`, parameters `Decompose Brace Layers`,
//! `Decompose Components in Variable Font`.
//!
//! Contract: a composite is kept wherever `gvar` can vary it. It is decomposed when its
//! component transforms other than the offsets differ between sources, when Decompose
//! Components in Variable Font is on, and, with Decompose Brace Layers on, when a component's
//! base glyph has intermediate layers the composite lacks. With the parameter off or absent such
//! composites stay composites, as the oracles write them (`docs/plans/M9.md` § 7 question 4,
//! decided).

use fontcraft_interp::GlyphSources;
use fontcraft_model::Font;

/// The font parameter that decomposes every composite of a variable font.
pub const DECOMPOSE_COMPONENTS: &str = "Decompose Components in Variable Font";
/// The font parameter that decomposes composites of glyphs with intermediate layers.
pub const DECOMPOSE_BRACE_LAYERS: &str = "Decompose Brace Layers";

/// Why a composite is decomposed.
#[derive(Clone, Debug, PartialEq)]
pub enum DecomposeReason {
    /// The component at this shape index scales, rotates or slants differently between sources.
    InconsistentTransform { shape: usize },
    /// Decompose Components in Variable Font is on.
    Parameter,
    /// Decompose Brace Layers is on and this base glyph has intermediate layers.
    BraceBase { base: String },
}

/// Why `sources`' glyph must be decomposed in a variable font; `None` to keep it a composite.
pub fn decompose_reason(font: &Font, sources: &GlyphSources) -> Option<DecomposeReason> {
    // M9.5
    let _ = (font, sources);
    None
}
