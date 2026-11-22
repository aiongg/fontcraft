//! The numbers a drag shows beside the pointer ([`Overlay::Badge`]): a primitive's size, a move's position, a knob's
//! percentages and size, an angle. Each is a short label on a dark rounded box whose top-left corner sits
//! `badge_offset` points right of and below the pointer, in `badge_fill` and `badge_text`.

use egui::{FontId, Pos2, Rect, Shape, vec2};
use fontcraft_tools::Overlay;

use super::layers::PaintCx;
use super::tools::tokens::ToolTokens;

/// Most characters a badge shows; a longer text is cut.
const MAX_BADGE_CHARS: usize = 64;

pub fn paint(cx: &PaintCx<'_>) {
    cx.painter.extend(shapes(cx));
}

/// The box and the text of each badge the active tool asks for.
pub fn shapes(cx: &PaintCx<'_>) -> Vec<Shape> {
    let t = ToolTokens::standard();
    let mut out = Vec::new();
    for overlay in &cx.tools.overlays {
        let Overlay::Badge { at, text } = overlay else { continue };
        let at = cx.camera.to_screen(*at);
        if !(at.x.is_finite() && at.y.is_finite()) {
            continue;
        }
        let text: String = text.chars().take(MAX_BADGE_CHARS).collect();
        let galley = cx.painter.layout_no_wrap(text, FontId::proportional(t.badge_text_size), t.badge_text);
        let corner = Pos2::new(at.x + t.badge_offset, at.y + t.badge_offset);
        let text_at = corner + vec2(t.badge_padding, t.badge_padding);
        let size = galley.size() + vec2(2.0 * t.badge_padding, 2.0 * t.badge_padding);
        out.push(Shape::rect_filled(Rect::from_min_size(corner, size), t.badge_padding, t.badge_fill));
        out.push(Shape::galley(text_at, galley, t.badge_text));
    }
    out
}
