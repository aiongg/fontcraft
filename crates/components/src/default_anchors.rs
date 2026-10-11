//! Default anchors (M6.3): the anchors Set Anchors adds to a layer, from our own table.
//!
//! Rows: the Set Anchors default table (`glyph.setAnchors`), `segment.startEndAnchors` and
//! `brush.startEndAnchors` (the anchor rules only).
//!
//! Contract:
//! - [`default_anchor_names`] lists the anchors a glyph should have, by its name and category:
//!   a custom glyph data file's `anchors` first, then our table.
//! - [`default_anchors`] places them on one layer from its outline bounds and the master's
//!   metrics, and returns only the names the layer does not have yet (Set Anchors adds the
//!   missing ones and keeps the rest). At most [`crate::MAX_LAYER_ANCHORS`] in total.

use fontcraft_glyphdata::GlyphData;
use fontcraft_model::{Anchor, Font};

use crate::ComponentsError;

/// The anchor names a glyph called `glyph` should have.
pub fn default_anchor_names(font: &Font, data: &GlyphData, glyph: &str) -> Vec<String> {
    // M6.3
    let _ = (font, data, glyph);
    Vec::new()
}

/// The default anchors `glyph`'s layer `layer` lacks, placed on that layer.
pub fn default_anchors(font: &Font, data: &GlyphData, glyph: &str, layer: &str) -> Result<Vec<Anchor>, ComponentsError> {
    // M6.3
    let _ = (font, data, glyph, layer);
    Ok(Vec::new())
}
