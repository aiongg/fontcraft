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

/// The colour kind of `layer`; `None` for a master layer or any other layer. A layer that stores
/// more than one colour attribute (the file format allows it, nothing writes it) takes the first
/// in the order palette, full colour, iColor, svg.
pub fn kind(layer: &Layer) -> Option<ColorLayerKind> {
    if layer.is_master_layer() {
        return None;
    }
    let a = &layer.info.attrs;
    if let Some(entry) = a.color_palette {
        Some(ColorLayerKind::Palette(entry))
    } else if a.full_color {
        Some(ColorLayerKind::FullColor)
    } else if let Some(size) = a.sbix_size {
        Some(ColorLayerKind::IColor(size))
    } else if a.svg {
        Some(ColorLayerKind::Svg)
    } else {
        None
    }
}

/// The colour kind `layer` has in an export: [`kind`], and a master layer marked full colour is
/// painted as a full-colour layer, as both oracles read it.
pub fn export_kind(layer: &Layer) -> Option<ColorLayerKind> {
    if layer.is_master_layer() { layer.info.attrs.full_color.then_some(ColorLayerKind::FullColor) } else { kind(layer) }
}

/// The colour layers of `glyph` associated with `master_id`, in paint order (bottom first), at
/// most [`crate::MAX_COLOR_LAYERS`].
pub fn color_layers<'a>(glyph: &'a Glyph, master_id: &str) -> Vec<(&'a Layer, ColorLayerKind)> {
    glyph.layers.iter().filter(|l| l.master == master_id).filter_map(|l| kind(l).map(|k| (l, k))).take(crate::MAX_COLOR_LAYERS).collect()
}

/// The label of a colour layer in the Layers palette: `Color 2`, `Color *`, `Color`,
/// `iColor 64`, `svg`.
pub fn label(kind: &ColorLayerKind) -> String {
    match kind {
        ColorLayerKind::Palette(PaletteEntry::Index(i)) => format!("Color {i}"),
        ColorLayerKind::Palette(PaletteEntry::Foreground) => "Color *".to_string(),
        ColorLayerKind::FullColor => "Color".to_string(),
        ColorLayerKind::IColor(size) => format!("iColor {size}"),
        ColorLayerKind::Svg => "svg".to_string(),
    }
}

/// Which kinds of colour layer `glyph` has, over all masters.
pub fn glyph_color(glyph: &Glyph) -> GlyphColor {
    let mut out = GlyphColor::None;
    for k in glyph.layers.iter().filter_map(kind) {
        let this = match k {
            ColorLayerKind::Palette(_) => GlyphColor::Palette,
            ColorLayerKind::FullColor => GlyphColor::Full,
            ColorLayerKind::IColor(_) => GlyphColor::Bitmap,
            ColorLayerKind::Svg => GlyphColor::Svg,
        };
        out = match out {
            GlyphColor::None => this,
            same if same == this => same,
            _ => return GlyphColor::Mixed,
        };
    }
    out
}

/// `attrs` with `kind` set and the other colour attributes cleared; `None` clears all four.
/// Everything else (intermediate coordinates, alternate ranges, smart poles) is kept.
pub fn with_kind(attrs: &LayerAttrs, kind: Option<ColorLayerKind>) -> LayerAttrs {
    let mut out = attrs.clone();
    out.color_palette = None;
    out.full_color = false;
    out.sbix_size = None;
    out.svg = false;
    match kind {
        Some(ColorLayerKind::Palette(entry)) => out.color_palette = Some(entry),
        Some(ColorLayerKind::FullColor) => out.full_color = true,
        Some(ColorLayerKind::IColor(size)) => out.sbix_size = Some(size),
        Some(ColorLayerKind::Svg) => out.svg = true,
        None => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colour_layer(id: &str, master: &str, kind: Option<ColorLayerKind>) -> Layer {
        let mut l = Layer::new(master, 500.0);
        l.id = id.to_string();
        l.info.attrs = with_kind(&LayerAttrs::default(), kind);
        l
    }

    #[test]
    fn kinds_labels_and_paint_order_follow_the_layer_attributes() {
        let mut g = Glyph::new("A");
        let mut master = Layer::new("m1", 500.0);
        // A master layer is never a colour layer, whatever it stores.
        master.info.attrs.full_color = true;
        g.layers.push(master);
        g.layers.push(colour_layer("c1", "m1", Some(ColorLayerKind::Palette(PaletteEntry::Index(2)))));
        g.layers.push(colour_layer("b1", "m1", None));
        g.layers.push(colour_layer("c2", "m2", Some(ColorLayerKind::Svg)));
        g.layers.push(colour_layer("c3", "m1", Some(ColorLayerKind::Palette(PaletteEntry::Foreground))));
        g.layers.push(colour_layer("c4", "m1", Some(ColorLayerKind::IColor(64))));

        let layers = color_layers(&g, "m1");
        let ids: Vec<&str> = layers.iter().map(|(l, _)| l.id.as_str()).collect();
        assert_eq!(ids, ["c1", "c3", "c4"], "file order, this master's colour layers only");
        let labels: Vec<String> = layers.iter().map(|(_, k)| label(k)).collect();
        assert_eq!(labels, ["Color 2", "Color *", "iColor 64"]);
        assert_eq!(label(&ColorLayerKind::FullColor), "Color");
        assert_eq!(label(&ColorLayerKind::Svg), "svg");
        assert_eq!(glyph_color(&g), GlyphColor::Mixed);

        let mut palette_only = Glyph::new("B");
        palette_only.layers.push(Layer::new("m1", 500.0));
        palette_only.layers.push(colour_layer("c1", "m1", Some(ColorLayerKind::Palette(PaletteEntry::Index(0)))));
        palette_only.layers.push(colour_layer("c2", "m1", Some(ColorLayerKind::Palette(PaletteEntry::Index(1)))));
        assert_eq!(glyph_color(&palette_only), GlyphColor::Palette);
        assert_eq!(glyph_color(&Glyph::new("C")), GlyphColor::None);
    }

    #[test]
    fn with_kind_sets_one_colour_attribute_and_keeps_the_rest() {
        let mut attrs = LayerAttrs { coordinates: vec![Some(400.0)], full_color: true, svg: true, ..LayerAttrs::default() };
        attrs.sbix_size = Some(32);
        let out = with_kind(&attrs, Some(ColorLayerKind::IColor(128)));
        assert_eq!(out.sbix_size, Some(128));
        assert!(!out.full_color && !out.svg && out.color_palette.is_none());
        assert_eq!(out.coordinates, vec![Some(400.0)], "an intermediate layer stays intermediate");
        let cleared = with_kind(&out, None);
        assert_eq!(cleared, LayerAttrs { coordinates: vec![Some(400.0)], ..LayerAttrs::default() });
    }
}
