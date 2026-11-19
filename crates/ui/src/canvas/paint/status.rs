//! The glyph status tint behind the edited glyph: the advance box, from the descender to the ascender, filled with
//! the colour of the status the glyph is in (its stored status, or the font's default status). Glyphs in no defined
//! status get no tint.

use egui::{Color32, Rect};
use fontcraft_geom::Point;

use super::super::layers::PaintCx;

/// The share of the status colour's own opacity the tint uses, so the outline stays readable over an opaque status
/// colour.
const TINT_OPACITY: f64 = 0.25;

pub fn paint(cx: &PaintCx<'_>) {
    let Some(glyph) = cx.glyph() else { return };
    let Some(color) = glyph.status else { return };
    let width = if glyph.width.is_finite() { glyph.width.max(0.0) } else { 0.0 };
    let a = cx.camera.to_screen(Point::new(0.0, glyph.descender));
    let b = cx.camera.to_screen(Point::new(width, glyph.ascender));
    let rect = Rect::from_two_pos(a, b);
    if rect.is_finite() {
        cx.painter.rect_filled(rect.intersect(cx.rect), 0.0, tint(color));
    }
}

/// The fill colour for a status colour (RGBA from 0 to 1).
fn tint(color: [f64; 4]) -> Color32 {
    let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let [r, g, b, a] = color;
    Color32::from_rgba_unmultiplied(byte(r), byte(g), byte(b), byte(a * TINT_OPACITY))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tint_keeps_the_hue_and_lowers_the_opacity() {
        let c = tint([1.0, 0.0, 0.0, 1.0]);
        assert_eq!(c.a(), 64);
        assert!(c.r() > c.g() && c.r() > c.b());
        assert_eq!(tint([0.0, 1.0, 0.0, 0.0]).a(), 0);
    }
}
