//! Glyph bounds as the reference computes them for FontBBox: every point the charstring draws,
//! including the start of each contour, and the true extremes of curves whose control points lie
//! outside the box so far.
//!
//! A port of fontTools `pens/boundsPen.py` (`BoundsPen`), `misc/bezierTools.py`
//! (`calcCubicBounds`, `calcCubicParameters`, `solveQuadratic`) and `misc/arrayTools.py`
//! (`intRect`); MIT, Copyright (c) 2017 Just van Rossum and contributors. The arithmetic follows
//! the same order so the results agree to the last bit.

/// A bounding box in font units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub x_min: f64,
    pub y_min: f64,
    pub x_max: f64,
    pub y_max: f64,
}

impl Bounds {
    fn point(x: f64, y: f64) -> Self {
        Self { x_min: x, y_min: y, x_max: x, y_max: y }
    }

    fn add(self, x: f64, y: f64) -> Self {
        Self { x_min: self.x_min.min(x), y_min: self.y_min.min(y), x_max: self.x_max.max(x), y_max: self.y_max.max(y) }
    }

    pub(crate) fn union(self, o: Bounds) -> Self {
        Self { x_min: self.x_min.min(o.x_min), y_min: self.y_min.min(o.y_min), x_max: self.x_max.max(o.x_max), y_max: self.y_max.max(o.y_max) }
    }

    fn contains(self, x: f64, y: f64) -> bool {
        self.x_min <= x && x <= self.x_max && self.y_min <= y && y <= self.y_max
    }

    /// The smallest integer box that contains this one (floor the minima, ceiling the maxima), as
    /// FontBBox is written.
    pub fn round_out(self) -> [i64; 4] {
        [self.x_min.floor() as i64, self.y_min.floor() as i64, self.x_max.ceil() as i64, self.y_max.ceil() as i64]
    }

    /// Integer bounds as the reference compiler rounds glyph boxes for the other tables (`head`,
    /// `hmtx`): half-up rounding when the tolerance is 0.5 or more or the rounded value is within
    /// it, otherwise outwards (floor the minima, ceiling the maxima).
    pub fn round_with_tolerance(self, tolerance: f64) -> [i64; 4] {
        let to_int = |v: f64, outward: fn(f64) -> f64| {
            let rounded = crate::number::ot_round(v);
            if tolerance >= 0.5 || (rounded - v).abs() <= tolerance { rounded as i64 } else { outward(v) as i64 }
        };
        [to_int(self.x_min, f64::floor), to_int(self.y_min, f64::floor), to_int(self.x_max, f64::ceil), to_int(self.y_max, f64::ceil)]
    }
}

/// Collects bounds from absolute drawing calls.
#[derive(Default)]
pub(crate) struct BoundsPen {
    bounds: Option<Bounds>,
    start: Option<(f64, f64)>,
    current: (f64, f64),
}

impl BoundsPen {
    pub(crate) fn bounds(&self) -> Option<Bounds> {
        self.bounds
    }

    fn add_move_to(&mut self) {
        if let Some((x, y)) = self.start.take() {
            self.bounds = Some(match self.bounds {
                Some(b) => b.add(x, y),
                None => Bounds::point(x, y),
            });
        }
    }

    fn add(&mut self, x: f64, y: f64) -> Bounds {
        let b = match self.bounds {
            Some(b) => b.add(x, y),
            None => Bounds::point(x, y),
        };
        self.bounds = Some(b);
        b
    }

    pub(crate) fn move_to(&mut self, p: (f64, f64)) {
        self.start = Some(p);
        self.add_move_to();
        self.current = p;
    }

    pub(crate) fn line_to(&mut self, p: (f64, f64)) {
        self.add_move_to();
        self.add(p.0, p.1);
        self.current = p;
    }

    pub(crate) fn curve_to(&mut self, b1: (f64, f64), b2: (f64, f64), p: (f64, f64)) {
        self.add_move_to();
        let mut b = self.add(p.0, p.1);
        if !b.contains(b1.0, b1.1) || !b.contains(b2.0, b2.1) {
            b = b.union(cubic_bounds(self.current, b1, b2, p));
        }
        self.bounds = Some(b);
        self.current = p;
    }
}

/// The box of one cubic segment, from its end points and its extremes.
fn cubic_bounds(p1: (f64, f64), p2: (f64, f64), p3: (f64, f64), p4: (f64, f64)) -> Bounds {
    let (dx, dy) = p1;
    let cx = (p2.0 - dx) * 3.0;
    let cy = (p2.1 - dy) * 3.0;
    let bx = (p3.0 - p2.0) * 3.0 - cx;
    let by = (p3.1 - p2.1) * 3.0 - cy;
    let ax = p4.0 - dx - cx - bx;
    let ay = p4.1 - dy - cy - by;
    let mut roots: Vec<f64> = solve_quadratic(ax * 3.0, bx * 2.0, cx).into_iter().filter(|t| (0.0..1.0).contains(t)).collect();
    roots.extend(solve_quadratic(ay * 3.0, by * 2.0, cy).into_iter().filter(|t| (0.0..1.0).contains(t)));
    let mut b = Bounds::point(p1.0, p1.1).add(p4.0, p4.1);
    for t in roots {
        let x = ax * t * t * t + bx * t * t + cx * t + dx;
        let y = ay * t * t * t + by * t * t + cy * t + dy;
        b = b.add(x, y);
    }
    b
}

fn solve_quadratic(a: f64, b: f64, c: f64) -> Vec<f64> {
    const EPSILON: f64 = 1e-10;
    if a.abs() < EPSILON {
        if b.abs() < EPSILON { Vec::new() } else { vec![-c / b] }
    } else {
        let dd = b * b - 4.0 * a * c;
        if dd >= 0.0 {
            let r = dd.sqrt();
            vec![(-b + r) / 2.0 / a, (-b - r) / 2.0 / a]
        } else {
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cubic_extremes_match_the_reference() {
        // fontTools' doctests for calcCubicBounds.
        let b = cubic_bounds((0.0, 0.0), (25.0, 100.0), (75.0, 100.0), (100.0, 0.0));
        assert_eq!((b.x_min, b.y_min, b.x_max, b.y_max), (0.0, 0.0, 100.0, 75.0));
        let b = cubic_bounds((50.0, 0.0), (0.0, 100.0), (100.0, 100.0), (50.0, 0.0));
        assert_eq!(format!("{:.6} {:.6} {:.6} {:.6}", b.x_min, b.y_min, b.x_max, b.y_max), "35.566243 0.000000 64.433757 75.000000");
    }

    #[test]
    fn a_lone_move_counts_and_inner_controls_skip_the_extremes() {
        let mut pen = BoundsPen::default();
        pen.move_to((5.0, 5.0));
        assert_eq!(pen.bounds().unwrap().round_out(), [5, 5, 5, 5]);
        pen.move_to((0.0, 0.0));
        pen.line_to((100.0, 0.0));
        pen.line_to((100.0, 100.0));
        pen.curve_to((90.0, 90.0), (10.0, 90.0), (0.0, 100.0));
        assert_eq!(pen.bounds().unwrap().round_out(), [0, 0, 100, 100]);
        pen.curve_to((-30.0, 50.0), (-30.0, 50.0), (0.0, 0.0));
        assert_eq!(pen.bounds().unwrap().round_out(), [-23, 0, 100, 100]);
    }

    #[test]
    fn rounding_with_a_tolerance() {
        let b = Bounds { x_min: 1.4, y_min: -0.3, x_max: 10.6, y_max: 9.2 };
        assert_eq!(b.round_with_tolerance(0.5), [1, 0, 11, 9]);
        assert_eq!(b.round_with_tolerance(0.0), [1, -1, 11, 10]);
        assert_eq!(b.round_out(), [1, -1, 11, 10]);
    }
}
