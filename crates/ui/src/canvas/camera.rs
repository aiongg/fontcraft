//! Font units to screen points and back.
//!
//! Font coordinates grow upwards; the canvas's document space (`CanvasView`) grows downwards like the screen. The
//! camera flips y between the two and leaves pan, zoom and rotation to the view.

use craft::behavior::CanvasView;
use craft::behavior::kurbo::Point;
use egui::Pos2;

/// The font-to-screen transform of one canvas pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub view: CanvasView,
}

impl Camera {
    pub fn new(view: CanvasView) -> Camera {
        Camera { view }
    }

    /// The screen point of the font point `p` (font units, y up).
    pub fn to_screen(&self, p: Point) -> Pos2 {
        self.view.to_screen(Point::new(p.x, -p.y))
    }

    /// The font point under the screen point `p`.
    pub fn to_font(&self, p: Pos2) -> Point {
        let doc = self.view.to_doc(p);
        Point::new(doc.x, -doc.y)
    }

    /// Screen points per font unit.
    pub fn scale(&self) -> f64 {
        self.view.zoom
    }
}
