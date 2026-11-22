//! What the canvas draws behind the edited layer (`docs/spec/03b` › Background, Palette › Layers).
//!
//! | Item | Drawn as | Shown when |
//! |---|---|---|
//! | the canvas | its colour, `canvas` (`appearance.color.canvas`) | [`canvas`], always |
//! | the active master's background | `master_canvas` (Master Background Color, or its Dark form on a dark canvas) | the master sets one |
//! | another layer of the glyph whose eye is on | thin outline, `visible_layer_outline` (`layers.visibleLayerOutline`) | always |
//! | the other side of the layer | thin outline, `background_outline` (`background.outlineDisplay`) | Show Background |
//! | its nodes and handles | as the edited side's nodes | Show Background and Show Nodes › In Background |
//! | editing the background | the canvas a little darker, `background_tint` (`background.darkenedCanvas`) | [`tint`] |

use egui::{Shape, Stroke};

use super::super::layers::PaintCx;
use super::{nodes, outline_shapes};

/// Fills the canvas with its colour (`appearance.color.canvas`) of the appearance it is drawn in, and the active
/// master's background over it.
pub fn canvas(cx: &PaintCx<'_>) {
    cx.painter.extend(canvas_shapes(cx));
}

/// The canvas colour, then the active master's background over it.
pub fn canvas_shapes(cx: &PaintCx<'_>) -> Vec<Shape> {
    let mut out = vec![Shape::rect_filled(cx.rect, 0.0, cx.tokens.canvas)];
    if let Some(master) = cx.tokens.master_canvas {
        out.push(Shape::rect_filled(cx.rect, 0.0, master));
    }
    out
}

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
