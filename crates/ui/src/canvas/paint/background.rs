//! What the canvas draws behind the edited layer (`docs/spec/03b` › Background, Palette › Layers).
//!
//! | Item | Drawn as | Shown when |
//! |---|---|---|
//! | another layer of the glyph whose eye is on | thin outline, `visible_layer_outline` (`layers.visibleLayerOutline`) | always |
//! | the other side of the layer | thin outline, `background_outline` (`background.outlineDisplay`) | Show Background |
//! | its nodes and handles | as the edited side's nodes | Show Background and Show Nodes › In Background |
//! | editing the background | the canvas a little darker, `background_tint` (`background.darkenedCanvas`) | [`tint`] |

use egui::{Shape, Stroke};

use super::super::layers::PaintCx;
use super::{nodes, outline_shapes};

/// Darkens the canvas a little while the background is the side being edited (`background.darkenedCanvas`).
pub fn tint(cx: &PaintCx<'_>) {
    if cx.edit_background && cx.glyph().is_some() {
        cx.painter.rect_filled(cx.rect, 0.0, cx.tokens.background_tint);
    }
}

pub fn paint(cx: &PaintCx<'_>) {
    cx.painter.extend(shapes(cx));
}

/// The other visible layers, then the other side of the layer and, under Show Nodes › In Background, its nodes.
pub fn shapes(cx: &PaintCx<'_>) -> Vec<Shape> {
    let Some(glyph) = cx.glyph() else { return Vec::new() };
    let t = cx.tokens;
    let mut out = Vec::new();
    for other in &glyph.other_layers {
        out.extend(outline_shapes(cx, other, Stroke::new(t.visible_layer_outline_width, t.visible_layer_outline)));
    }
    let Some((outline, layer)) = glyph.background.as_ref().filter(|_| cx.display.background) else { return out };
    out.extend(outline_shapes(cx, outline, Stroke::new(t.background_outline_width, t.background_outline)));
    if cx.display.nodes_in_background {
        out.extend(nodes::layer_shapes(cx, layer, &nodes::Metrics::default()));
    }
    out
}
