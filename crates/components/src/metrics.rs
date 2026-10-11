//! Derived metrics of aligned composites, and the re-alignment step the engine runs after each
//! font-changing command (M6.2).
//!
//! Rows: `components.anchoredMarksDoNotAffectSidebearings`, `metricsKey.autoAlignedOffset` (the
//! offset arithmetic), the re-alignment half of `components.liveUpdateFromBaseGlyph`.
//!
//! Contract:
//! - [`derived_metrics`]: an aligned layer's width and sidebearings come from its base
//!   components (advances summed, no kerning; marks attached by anchors do not count), plus the
//!   offsets of `=+N` / `=-N` metrics keys ([`parse_offset`]).
//! - [`realign`] is the engine hook. After a font-changing command it takes the font before and
//!   after, and re-aligns the glyphs that changed and every glyph that depends on them, in
//!   dependency order ([`crate::ComponentIndex`]), each layer at most once. It returns the font
//!   to keep, pointer-equal to `new` when nothing moves, so the engine can put it in the same
//!   undo step. An error means the command fails and the engine rolls back.
//! - Glyphs in a cycle are skipped, not an error: the editor shows them as placeholders.

use fontcraft_model::Font;

use crate::ComponentsError;

/// The width and sidebearings automatic alignment gives a layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DerivedMetrics {
    pub width: f64,
    /// `None` for a layer without outlines.
    pub lsb: Option<f64>,
    pub rsb: Option<f64>,
}

/// The amount of an offset key: `=+18` → `Some(18.0)`, `=-12` → `Some(-12.0)`. `None` for any
/// other text, and for numbers that are not finite.
pub fn parse_offset(key: &str) -> Option<f64> {
    // M6.2
    let _ = key;
    None
}

/// The derived metrics of `glyph`'s layer `layer`, with the layer's offset keys applied; `None`
/// when the layer has no aligned component.
pub fn derived_metrics(font: &Font, glyph: &str, layer: &str) -> Result<Option<DerivedMetrics>, ComponentsError> {
    // M6.2
    let _ = (font, glyph, layer);
    Ok(None)
}

/// The names of the glyphs whose `Arc` differs between `old` and `new`, those added, and those
/// removed, in `new`'s order then removed ones. Every glyph when the masters or the font's
/// alignment setting changed.
pub fn changed_glyphs(old: &Font, new: &Font) -> Vec<String> {
    // M6.2
    let _ = (old, new);
    Vec::new()
}

/// `font` with `changed` and every glyph that depends on them re-aligned, in dependency order.
pub fn realign_glyphs(font: Font, changed: &[String]) -> Result<Font, ComponentsError> {
    // M6.2
    let _ = changed;
    Ok(font)
}

/// The engine hook: `new` with the glyphs changed since `old`, and their dependents,
/// re-aligned ([`changed_glyphs`], then [`realign_glyphs`]).
pub fn realign(old: &Font, new: Font) -> Result<Font, ComponentsError> {
    let changed = changed_glyphs(old, &new);
    if changed.is_empty() {
        return Ok(new);
    }
    realign_glyphs(new, &changed)
}
