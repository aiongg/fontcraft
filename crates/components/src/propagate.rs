//! Anchor propagation (M6.1): the effective anchors of a glyph's layer, its own anchors plus the
//! anchors its components pass on.
//!
//! Rows: `components.topBottomAnchorsPropagate`, `components.ownAnchorOverridesPropagated`,
//! `components.propagateAnchorsParameter`, `components.ligatureNumberedAnchors`,
//! `components.verticalFlipSwapsTopBottomAnchors`, `transform.mirrorComponentAnchors`,
//! `Propagate Anchors`, `compile.glyphsOutlinePreSteps` (the propagation half).
//!
//! Provenance: this module is written from the reference handbook's description, the behaviour
//! facts in `docs/plans/M6.md` § 2, and the outputs of the reference compilers observed as a
//! black box (§ 5's propagation measure is the specification). It is never written from fontc's
//! or glyphsLib's propagation source, which derive from the reference app's code; neither is read
//! or ported.
//!
//! Contract:
//! - [`effective_anchors`] gives a layer's anchors after propagation: the layer's own anchors,
//!   then the names its components pass on that no own anchor overrides, with the name swaps of
//!   flipped components, ligature numbering, the component `anchor` attribute and `*origin`
//!   applied. Positions are in the composite's coordinates. Order: own anchors in stored order,
//!   then propagated ones in component order.
//! - Layer choice for a component's base: the base's layer with the same id, else the base's
//!   master layer of the same master.
//! - When [`propagate_anchors_enabled`] is false the result is the layer's own anchors.
//! - Errors: a cycle ([`ComponentsError::Cycle`]), nesting past [`MAX_COMPONENT_DEPTH`], more
//!   than [`MAX_LAYER_ANCHORS`] anchors, a missing layer. A missing base glyph passes nothing
//!   on and is not an error (compile's preflight reports it).
//! - Propagated anchors are computed, never stored in the font.
//! - [`AnchorCache`] keeps results across edits. An entry stays valid while the glyph's `Arc`
//!   and the `Arc`s of every glyph it reaches through components are pointer-equal to those it
//!   was computed from; the font has no revision counter, so pointers are the key.
//!
//! [`MAX_COMPONENT_DEPTH`]: crate::MAX_COMPONENT_DEPTH
//! [`MAX_LAYER_ANCHORS`]: crate::MAX_LAYER_ANCHORS

use std::collections::HashMap;
use std::sync::Arc;

use fontcraft_model::{Font, Glyph};

use crate::ComponentsError;

/// The font custom parameter that turns propagation off when its checkbox is off.
pub const PROPAGATE_ANCHORS_PARAM: &str = "Propagate Anchors";

/// Where an effective anchor comes from.
#[derive(Clone, Debug, PartialEq)]
pub enum AnchorSource {
    /// Stored on the layer itself.
    Own,
    /// Passed on by the component at shape index `component` (an index into `Layer::shapes`),
    /// whose base glyph is `base`.
    Component { component: usize, base: String },
}

/// One anchor of a layer after propagation, in the composite's coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectiveAnchor {
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub source: AnchorSource,
}

/// Whether the font propagates anchors: true unless the font's Propagate Anchors parameter is
/// present and off.
pub fn propagate_anchors_enabled(font: &Font) -> bool {
    // M6.1: read `PROPAGATE_ANCHORS_PARAM` from `font.info.params`.
    let _ = font;
    true
}

/// The effective anchors of `glyph`'s layer `layer` (a layer id; a master layer's id is its
/// master's id). See the module doc.
pub fn effective_anchors(font: &Font, glyph: &str, layer: &str) -> Result<Vec<EffectiveAnchor>, ComponentsError> {
    // M6.1
    let _ = (font, glyph, layer);
    Ok(Vec::new())
}

/// The effective anchors of every glyph's master layer of `master`, by glyph name, computed in
/// dependency order so each base is worked out once. For compile and for whole-font checks.
pub fn effective_anchors_for_master(font: &Font, master: &str) -> Result<HashMap<String, Vec<EffectiveAnchor>>, ComponentsError> {
    // M6.1
    let _ = (font, master);
    Ok(HashMap::new())
}

/// One cached result and the glyph pointers it was computed from.
// M6.1: `AnchorCache::get` reads the fields once it reuses entries.
#[allow(dead_code)]
#[derive(Clone, Debug)]
struct CacheEntry {
    glyphs: Vec<Arc<Glyph>>,
    anchors: Arc<[EffectiveAnchor]>,
}

/// A cache of [`effective_anchors`] results. See the module doc for when an entry is valid.
#[derive(Clone, Debug, Default)]
pub struct AnchorCache {
    /// Keyed by (glyph, layer id).
    entries: HashMap<(String, String), CacheEntry>,
}

impl AnchorCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// [`effective_anchors`], from the cache when the entry is still valid.
    pub fn get(&mut self, font: &Font, glyph: &str, layer: &str) -> Result<Arc<[EffectiveAnchor]>, ComponentsError> {
        // M6.1: check the stored pointers before reusing an entry.
        let anchors = effective_anchors(font, glyph, layer)?;
        Ok(anchors.into())
    }

    /// Drops the entries `font` no longer matches.
    pub fn retain_current(&mut self, font: &Font) {
        // M6.1
        let _ = font;
        self.entries.clear();
    }
}
