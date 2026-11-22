//! The edited side's outline: the composite (`compositing.shapesCombineInOrder`) in the foreground stroke colour and,
//! under it, when strokes or masks make it differ, the paths as drawn in light grey (`stroke.display`,
//! `mask.display`). Fill Preview fills its closed contours while the Text, Hand or Zoom tool is active.

use egui::{Shape, Stroke};

use super::super::layers::PaintCx;
use super::{fill_shapes, outline_shapes};

pub fn paint(cx: &PaintCx<'_>) {
    cx.painter.extend(shapes(cx));
}

pub fn shapes(cx: &PaintCx<'_>) -> Vec<Shape> {
    let Some(glyph) = cx.glyph() else { return Vec::new() };
    let t = cx.tokens;
    let mut out = Vec::new();
    if let Some(source) = &glyph.source {
        out.extend(outline_shapes(cx, source, Stroke::new(t.source_outline_width, t.source_outline)));
    }
    if cx.display.fill_preview && cx.tools.fills {
        out.extend(fill_shapes(cx, &glyph.outline, t.outline));
    }
    out.extend(outline_shapes(cx, &glyph.outline, Stroke::new(t.outline_width, t.outline)));
    out
}
