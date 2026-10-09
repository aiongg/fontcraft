//! Small plane-geometry helpers the tools share. Every function accepts any input, NaN
//! included, and answers without panicking.

use fontcraft_geom::Point;
use fontcraft_model::MAX_COORDINATE;

/// Both coordinates are numbers within the model's coordinate limit.
pub fn usable(p: Point) -> bool {
    p.x.is_finite() && p.y.is_finite() && p.x.abs() <= MAX_COORDINATE && p.y.abs() <= MAX_COORDINATE
}

/// Both coordinates are finite.
pub fn finite(p: Point) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

pub fn dist(a: Point, b: Point) -> f64 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    (dx * dx + dy * dy).sqrt()
}

/// Whether `p` lies inside the polygon, by the even-odd rule. Fewer than three corners enclose
/// nothing.
pub fn point_in_polygon(p: Point, poly: &[Point]) -> bool {
    if poly.len() < 3 || !finite(p) {
        return false;
    }
    let mut inside = false;
    let mut prev = poly.last().copied();
    for a in poly {
        if let Some(b) = prev
            && (a.y > p.y) != (b.y > p.y)
        {
            let x = (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x;
            if p.x < x {
                inside = !inside;
            }
        }
        prev = Some(*a);
    }
    inside
}

/// The winding number of closed polygons around `p` (nonzero: inside).
pub fn winding(p: Point, polygons: &[Vec<Point>]) -> i64 {
    let mut w: i64 = 0;
    for poly in polygons {
        let mut prev = poly.last().copied();
        for b in poly {
            if let Some(a) = prev {
                let cross = (b.x - a.x) * (p.y - a.y) - (p.x - a.x) * (b.y - a.y);
                if a.y <= p.y {
                    if b.y > p.y && cross > 0.0 {
                        w = w.saturating_add(1);
                    }
                } else if b.y <= p.y && cross < 0.0 {
                    w = w.saturating_sub(1);
                }
            }
            prev = Some(*b);
        }
    }
    w
}

/// The unit direction of an angle in degrees, counter-clockwise from the x axis.
pub fn direction(angle_degrees: f64) -> (f64, f64) {
    let a = angle_degrees.to_radians();
    (a.cos(), a.sin())
}

/// The distance from `p` to the infinite line through `origin` at `angle_degrees`, and the
/// nearest point on it. `None` when the input is not finite.
pub fn line_distance(p: Point, origin: Point, angle_degrees: f64) -> Option<(f64, Point)> {
    if !(finite(p) && finite(origin) && angle_degrees.is_finite()) {
        return None;
    }
    let (ux, uy) = direction(angle_degrees);
    let (dx, dy) = (p.x - origin.x, p.y - origin.y);
    let along = dx * ux + dy * uy;
    let foot = Point::new(origin.x + along * ux, origin.y + along * uy);
    let d = dist(p, foot);
    d.is_finite().then_some((d, foot))
}

/// The four corners of a rectangle that starts at `start`, turned by `angle` radians, `u` long
/// along its turned x axis and `v` along its turned y axis.
pub fn rect_corners(start: Point, angle: f64, u: f64, v: f64) -> [Point; 4] {
    let (c, s) = (angle.cos(), angle.sin());
    let ax = (c * u, s * u);
    let ay = (-s * v, c * v);
    [
        start,
        Point::new(start.x + ax.0, start.y + ax.1),
        Point::new(start.x + ax.0 + ay.0, start.y + ax.1 + ay.1),
        Point::new(start.x + ay.0, start.y + ay.1),
    ]
}

/// The smallest axis-aligned box around the finite points, as `(min, max)`.
pub fn bounds(points: impl IntoIterator<Item = Point>) -> Option<(Point, Point)> {
    let mut out: Option<(Point, Point)> = None;
    for p in points.into_iter().filter(|p| finite(*p)) {
        out = Some(match out {
            None => (p, p),
            Some((lo, hi)) => (Point::new(lo.x.min(p.x), lo.y.min(p.y)), Point::new(hi.x.max(p.x), hi.y.max(p.y))),
        });
    }
    out
}

/// Whether `p` is within `margin` of the box `(min, max)`.
pub fn near_box(p: Point, (lo, hi): (Point, Point), margin: f64) -> bool {
    (lo.x - margin..=hi.x + margin).contains(&p.x) && (lo.y - margin..=hi.y + margin).contains(&p.y)
}
