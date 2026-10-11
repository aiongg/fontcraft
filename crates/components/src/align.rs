//! Automatic alignment (M6.2): which components of a layer are placed automatically, and where.
//!
//! Rows: `components.autoAlign*`, `components.pathOnLayerDisablesAutoAlign`,
//! `components.underscoreAnchorSnapsToPlainAnchor`, `components.autoAlignBaseThenMarks`,
//! `components.marksStackInOrder`, `components.alternativeAnchorSuffix`,
//! `components.multipleBasesSetSideBySide`, `components.marksBelongToPrecedingBase`,
//! `components.exitEntryAnchorsConnectBases`, `components.singleNumberComponentShiftsVertically`,
//! `components.fontWideDisableAutoAlign`, `fontInfo.disableAutomaticAlignment`.
//!
//! Contract (`docs/plans/M6.md` § 3 › Design decisions):
//! - A component is aligned when the font's Disable Automatic Alignment setting is off and its
//!   stored alignment is `Forced`, or `Default` on a layer that holds only components;
//!   `Horizontal` aligns it horizontally only; `Disabled` never. The single Number-category
//!   component behaves as `Horizontal` (a guess).
//! - Aligned positions are stored in the font. [`align_layer`] computes them and returns the
//!   layer with the new transforms and width written; it reads the targets from
//!   [`crate::propagate`]. The engine runs it through [`crate::metrics::realign`] after every
//!   font-changing command; opening a file does not align.
//! - Placement only moves components: scale, rotation, flip and slant are kept.
//! - Pure functions; a failure changes nothing.

use fontcraft_model::{Font, Layer, Transform};

use crate::ComponentsError;

/// How a component is placed automatically.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlignMode {
    /// Both directions.
    Full,
    /// Horizontally only; the vertical position is the designer's.
    Horizontal,
}

/// The automatic placement of one component.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    /// Index into `Layer::shapes`.
    pub shape: usize,
    pub mode: AlignMode,
    /// The component's transform with the aligned position.
    pub transform: Transform,
}

/// How the component at shape index `shape` of `layer` is aligned; `None` when it is not
/// aligned or `shape` is not a component.
pub fn component_alignment(font: &Font, layer: &Layer, shape: usize) -> Option<AlignMode> {
    // M6.2
    let _ = (font, layer, shape);
    None
}

/// The aligned components of `layer` as (shape index, mode), in shape order.
pub fn aligned_components(font: &Font, layer: &Layer) -> Vec<(usize, AlignMode)> {
    // M6.2
    let _ = (font, layer);
    Vec::new()
}

/// Whether any component of `layer` is aligned. Commands that move components or set a width
/// refuse on such layers (except the vertical part of `Horizontal`).
pub fn is_aligned(font: &Font, layer: &Layer) -> bool {
    !aligned_components(font, layer).is_empty()
}

/// The aligned placement of the components of `glyph`'s layer `layer`, in shape order; empty
/// when none is aligned.
pub fn aligned_placement(font: &Font, glyph: &str, layer: &str) -> Result<Vec<Placement>, ComponentsError> {
    // M6.2
    let _ = (font, glyph, layer);
    Ok(Vec::new())
}

/// `glyph`'s layer `layer` with its aligned components placed and its width and sidebearings
/// derived ([`crate::metrics::derived_metrics`]); `None` when nothing would change.
pub fn align_layer(font: &Font, glyph: &str, layer: &str) -> Result<Option<Layer>, ComponentsError> {
    // M6.2
    let _ = (font, glyph, layer);
    Ok(None)
}
