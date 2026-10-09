//! The view transform between font units and screen points.

use fontcraft_geom::Point;

/// How font units map to the screen: `screen = origin + (x · scale, −y · scale)`. The y axis
/// flips because font units grow upwards and screen points downwards.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// Screen points per font unit. Read it through [`View::scale`], which repairs bad values.
    pub scale: f64,
    /// Where the font origin (0, 0) sits on screen.
    pub origin: Point,
}

impl Default for View {
    fn default() -> Self {
        Self { scale: 1.0, origin: Point::ZERO }
    }
}

impl View {
    /// Smallest and largest scale the tools work with; anything else is clamped.
    pub const SCALE_RANGE: std::ops::RangeInclusive<f64> = 1e-4..=1e4;

    pub fn new(scale: f64, origin: Point) -> Self {
        Self { scale, origin }
    }

    /// The scale, clamped to [`View::SCALE_RANGE`]; 1 when it is not a positive number.
    pub fn scale(&self) -> f64 {
        if self.scale.is_finite() && self.scale > 0.0 { self.scale.clamp(*Self::SCALE_RANGE.start(), *Self::SCALE_RANGE.end()) } else { 1.0 }
    }

    fn origin(&self) -> Point {
        if self.origin.x.is_finite() && self.origin.y.is_finite() { self.origin } else { Point::ZERO }
    }

    /// Font units to screen points.
    pub fn to_screen(self, p: Point) -> Point {
        let s = self.scale();
        let o = self.origin();
        Point::new(o.x + p.x * s, o.y - p.y * s)
    }

    /// Screen points to font units.
    pub fn to_font(self, p: Point) -> Point {
        let s = self.scale();
        let o = self.origin();
        Point::new((p.x - o.x) / s, (o.y - p.y) / s)
    }

    /// A distance in screen points as font units. Negative or non-finite distances count as 0.
    pub fn units(&self, screen_points: f64) -> f64 {
        if screen_points.is_finite() && screen_points > 0.0 { screen_points / self.scale() } else { 0.0 }
    }
}
