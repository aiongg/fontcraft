//! The glyph status tint behind the edited glyph: the advance box, from the descender to the
//! ascender, filled with the colour of the status the glyph is in (its stored status, or the
//! font's default status). Glyphs in no defined status get no tint.

use egui::{Color32, Pos2, Rect};
use fontcraft_engine::cmd::status::effective_status;
use fontcraft_geom::Point;

use super::{Painting, Scene, view};
use crate::panels::Pending;

/// The share of the status colour's own opacity the tint uses, so the outline stays readable over
/// an opaque status colour.
const TINT_OPACITY: f64 = 0.25;

/// Draws this overlay for the edited layer.
pub fn draw(ui: &egui::Ui, c: &Painting, scene: &Scene, pending: &mut Pending) {
    let _ = (ui, pending);
    let Some(glyph) = scene.tab.edited_glyph() else { return };
    let Some(def) = effective_status(scene.font, glyph) else { return };
    let Some(master) = scene.font.masters.iter().find(|m| m.id == scene.layer.master) else { return };
    let (descender, ascender) = view::vertical_span(scene.font, master);
    let width = if scene.layer.width.is_finite() { scene.layer.width.max(0.0) } else { 0.0 };
    let a = c.view.to_screen(Point::new(0.0, descender));
    let b = c.view.to_screen(Point::new(width, ascender));
    let rect = Rect::from_two_pos(Pos2::new(a.x as f32, a.y as f32), Pos2::new(b.x as f32, b.y as f32));
    if !rect.is_finite() {
        return;
    }
    c.painter.rect_filled(rect.intersect(c.rect), 0.0, tint(def.color));
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
