//! Font units to screen points and back.
//!
//! Font coordinates grow upwards; the canvas's document space (`CanvasView`) grows downwards like the screen. The
//! document is the edited glyph's advance box from the ascender down to the descender, so document `(0, 0)` is font
//! `(0, top)`: the camera flips y about `top` and leaves pan, zoom and rotation to the view.

use craft::behavior::CanvasView;
use craft::behavior::kurbo::{Affine, Point};
use egui::Pos2;

/// The font-to-screen transform of one canvas pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub view: CanvasView,
    /// The font y at the top of the document (the ascender of the edited glyph's master).
    pub top: f64,
}

impl Camera {
    pub fn new(view: CanvasView, top: f64) -> Camera {
        Camera { view, top: if top.is_finite() { top } else { 0.0 } }
    }

    /// Font units to document units.
    pub fn font_to_doc(&self) -> Affine {
        Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, self.top])
    }

    /// Font units to screen points, for whole outlines.
    pub fn affine(&self) -> Affine {
        self.view.affine() * self.font_to_doc()
    }

    /// The screen point of the font point `p` (font units, y up).
    pub fn to_screen(&self, p: Point) -> Pos2 {
        self.view.to_screen(self.font_to_doc() * p)
    }

    /// The font point under the screen point `p`.
    pub fn to_font(&self, p: Pos2) -> Point {
        self.font_to_doc().inverse() * self.view.to_doc(p)
    }

    /// Screen points per font unit.
    pub fn scale(&self) -> f64 {
        self.view.zoom
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_points_round_trip_with_y_up() {
        let view = CanvasView::new(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0)), Point::new(300.0, 400.0), 0.5);
        let camera = Camera::new(view, 800.0);
        let base = camera.to_screen(Point::new(0.0, 0.0));
        let up = camera.to_screen(Point::new(0.0, 100.0));
        assert!(up.y < base.y, "a higher font point is higher on screen");
        assert!((base.y - up.y - 50.0).abs() < 1e-3);
        let back = camera.to_font(camera.to_screen(Point::new(120.0, -40.0)));
        assert!((back.x - 120.0).abs() < 1e-6 && (back.y + 40.0).abs() < 1e-6);
    }
}
