//! The components and corner components of the edited layer, over the glyph's outline.
//!
//! | Item | Drawn as |
//! |---|---|
//! | component aligned automatically | its outline in `component_aligned` (`components.autoAlignColors`) |
//! | component aligned horizontally only | its outline in `component_shiftable` |
//! | component placed by hand | its outline in `component_manual` |
//! | empty base layer, missing base glyph, circular reference | a crossed box with a label, `component_placeholder` (`components.placeholder*`) |
//! | corner or cap component | a wide, faint band along the outline fitting it put in, `attachment_band` (`corner.highlightAndSelection`) |
//!
//! The projection resolves outlines per engine revision, so composites follow a base glyph while it is being
//! edited (`components.liveUpdateFromBaseGlyph`).

use egui::{Align2, FontId, Shape, Stroke};

use super::super::layers::PaintCx;
use super::{outline_shapes, screen};
use crate::app::models::canvas::{Placeholder, State};

pub fn paint(cx: &PaintCx<'_>) {
    if cx.edit_background {
        return;
    }
    cx.painter.extend(shapes(cx));
    let Some(glyph) = cx.glyph() else { return };
    let t = cx.tokens;
    for mark in &glyph.components {
        let (State::Placeholder(kind), Some([.., top_left])) = (mark.state, mark.placeholder) else { continue };
        let Some(at) = screen(cx, top_left) else { continue };
        let why = match kind {
            Placeholder::Empty => &cx.words.empty,
            Placeholder::Missing => &cx.words.missing,
            Placeholder::Circular => &cx.words.circular,
        };
        let label = format!("{}: {}", mark.base, why.as_str());
        cx.painter.text(at, Align2::LEFT_BOTTOM, label, FontId::proportional(t.label_size), t.component_placeholder);
    }
}

/// The outlines and placeholder boxes of the components, then the corner-component discs.
pub fn shapes(cx: &PaintCx<'_>) -> Vec<Shape> {
    let Some(glyph) = cx.glyph() else { return Vec::new() };
    let t = cx.tokens;
    let mut out = Vec::new();
    for mark in &glyph.components {
        let color = match mark.state {
            State::Aligned => t.component_aligned,
            State::Shiftable => t.component_shiftable,
            State::Manual => t.component_manual,
            State::Placeholder(_) => t.component_placeholder,
        };
        let stroke = Stroke::new(t.component_outline_width, color);
        match mark.placeholder {
            Some(corners) => {
                let Some(points) = corners.iter().map(|p| screen(cx, *p)).collect::<Option<Vec<_>>>() else { continue };
                let [a, b, c, d] = [0, 1, 2, 3].map(|i| points.get(i).copied().unwrap_or_default());
                out.push(Shape::closed_line(points, stroke));
                out.push(Shape::line_segment([a, c], stroke));
                out.push(Shape::line_segment([b, d], stroke));
            }
            None => out.extend(outline_shapes(cx, &mark.outline, stroke)),
        }
    }
    for band in &glyph.attachments {
        out.extend(outline_shapes(cx, band, Stroke::new(t.attachment_band_width, t.attachment_band)));
    }
    out
}
