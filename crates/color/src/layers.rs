//! Colour layer kinds (M11.3): which special layer a layer is, its label in the Layers palette,
//! and a glyph's colour layers for one master in paint order.
//!
//! Rows: `color.fourKinds`, `color.paletteLayer.naming`, `color.paletteLayer.foregroundColor`,
//! `color.paletteLayer.belongsToMaster`, `color.fullColor.layerNaming`,
//! `color.sbix.iColorNaming`, `color.sbix.sizeIsPixels`, `layer.listColorLayers`, and the query
//! tokens of `color.export.glyphFilterTokens` ([`GlyphColor`]).
//!
//! Contract (`docs/plans/M11.md` § 3):
//! - A layer's kind comes from its [`fontcraft_model::LayerAttrs`]: `color_palette`,
//!   `full_color`, `sbix_size` or `svg`, at most one of them. A master layer is never a colour
//!   layer.
//! - The colour layers of a glyph for a master are the non-master layers associated with that
//!   master that have a kind, in their order in the file: the first is painted first, at the
//!   bottom.
//! - Converting a layer sets the one attribute of the new kind and clears the others
//!   ([`with_kind`]).

use std::ops::RangeInclusive;

use fontcraft_model::{Glyph, Layer, LayerAttrs, PaletteEntry};

/// iColor sizes, in pixels per em.
pub const ICOLOR_SIZES: RangeInclusive<u16> = 1..=4_096;
/// The size of a new iColor layer (`layer.convertToIColor`).
pub const DEFAULT_ICOLOR_SIZE: u16 = 128;

/// What a colour layer paints.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColorLayerKind {
    /// Its outlines in one palette entry, or in the text colour.
    Palette(PaletteEntry),
    /// Its paths with their own fills, strokes and gradients.
    FullColor,
    /// One bitmap at this many pixels per em.
    IColor(u16),
    /// One referenced SVG file.
    Svg,
}

/// What kinds of colour layer a glyph has, for the glyph query tokens.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GlyphColor {
    #[default]
    None,
    /// Palette layers only.
    Palette,
    /// Full-colour layers only.
    Full,
    /// iColor layers only.
    Bitmap,
    /// svg layers only.
    Svg,
    /// More than one kind.
    Mixed,
}

/// The colour kind of `layer`; `None` for a master layer or any other layer.
pub fn kind(layer: &Layer) -> Option<ColorLayerKind> {
    // M11.3
    let _ = layer;
    None
}

/// The colour layers of `glyph` associated with `master_id`, in paint order (bottom first), at
/// most [`crate::MAX_COLOR_LAYERS`].
pub fn color_layers<'a>(glyph: &'a Glyph, master_id: &str) -> Vec<(&'a Layer, ColorLayerKind)> {
    // M11.3
    let _ = (glyph, master_id);
    Vec::new()
}

/// The label of a colour layer in the Layers palette: `Color 2`, `Color *`, `Color`,
/// `iColor 64`, `svg`.
pub fn label(kind: &ColorLayerKind) -> String {
    // M11.3
    let _ = kind;
    String::new()
}

/// Which kinds of colour layer `glyph` has, over all masters.
pub fn glyph_color(glyph: &Glyph) -> GlyphColor {
    // M11.3
    let _ = glyph;
    GlyphColor::None
}

/// `attrs` with `kind` set and the other colour attributes cleared; `None` clears all four.
pub fn with_kind(attrs: &LayerAttrs, kind: Option<ColorLayerKind>) -> LayerAttrs {
    // M11.3
    let _ = kind;
    attrs.clone()
}
