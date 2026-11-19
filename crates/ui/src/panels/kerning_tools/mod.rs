//! The Kerning Pairs Preview (`pairs_preview`) and the Kerning Groups Editor (`groups_editor`), and the glyph
//! drawing they share.

pub mod groups_editor;
pub mod pairs_preview;

use craft::ui::primitives::{ContentPainterCx, StrokeWidth};
use egui::{Rect, Shape, Stroke, pos2};

use super::font_view::cells;
use crate::app::models::font_view::CellArt;

/// The share of a picture's height the em takes, as a Font View cell draws it.
const EM_SHARE: f32 = 0.8;
/// The ascender and descender a run is laid out with, as shares of the em.
const ASCENDER: f32 = 0.8;
const DESCENDER: f32 = -0.2;

/// Pixels per font unit for a picture of `rect` and an em of `upm` units.
pub fn scale_for(rect: Rect, upm: f32) -> f32 {
    if upm > 0.0 { rect.height() * EM_SHARE / upm } else { 0.0 }
}

/// Fills `glyphs` along one baseline in `rect`, each at its origin in font units from the run's start, the run
/// centred and shrunk to fit the width.
pub fn paint_run(cx: &ContentPainterCx<'_>, rect: Rect, upm: f32, glyphs: &[(&CellArt, f32)]) {
    let end = glyphs.iter().map(|(art, x)| x + art.width.max(0.0)).fold(0.0_f32, f32::max);
    let mut scale = scale_for(rect, upm);
    if end > 0.0 && end * scale > rect.width() * EM_SHARE {
        scale = rect.width() * EM_SHARE / end;
    }
    if !(scale.is_finite() && scale > 0.0) {
        return;
    }
    // `cells::paint` scales a cell's height to the em; a band of the run's height around the baseline gives it the
    // run's scale.
    let height = upm * scale / EM_SHARE;
    let top = rect.center().y - height / 2.0;
    let start = rect.center().x - end * scale / 2.0;
    for (art, x) in glyphs {
        let width = (art.width.max(0.0) * scale / EM_SHARE).max(f32::EPSILON);
        let centre = start + (x + art.width.max(0.0) / 2.0) * scale;
        let cell = Rect::from_min_max(pos2(centre - width / 2.0, top), pos2(centre + width / 2.0, top + height));
        cells::paint(cx, cell, art, upm * ASCENDER, upm * DESCENDER);
    }
}

/// Strokes the outline of `art` with its origin at `origin` (on the baseline, in points), `scale` points per unit.
pub fn stroke_glyph(cx: &ContentPainterCx<'_>, art: &CellArt, origin: egui::Pos2, scale: f32) {
    let stroke = Stroke::new(cx.stroke_width(StrokeWidth::Hairline), cx.ink());
    for contour in &art.contours {
        let points: Vec<egui::Pos2> = contour.iter().map(|[x, y]| pos2(origin.x + x * scale, origin.y - y * scale)).collect();
        cx.painter.add(Shape::closed_line(points, stroke));
    }
}

/// The baseline of a picture of `rect` laid out like [`paint_run`].
pub fn baseline(rect: Rect, upm: f32) -> f32 {
    rect.center().y + (ASCENDER + DESCENDER) / 2.0 * upm * scale_for(rect, upm)
}

#[cfg(test)]
mod tests;
