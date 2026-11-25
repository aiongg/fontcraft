//! Snapping to guides and to the background (`docs/spec/03a` § 20, question 28;
//! `background.snapping`).
//!
//! [`crate::snap::snap`] asks these after the layer's own nodes and handles:
//!
//! | Target | Lands on |
//! |---|---|
//! | a node or handle of the background | that point (nodes before handles) |
//! | a guide that crosses another guide, a metric line or a sidebearing near the point | the crossing |
//! | a guide | the nearest point of the guide |
//!
//! A crossing beats the guide alone, so a point dragged near where a guide meets the baseline
//! lands exactly on both. Metric lines and sidebearings on their own stay with
//! [`crate::snap::snap`], after guides.

use fontcraft_geom::Point;
use fontcraft_model::Shape;

use crate::geometry::{direction, dist, finite};
use crate::scene::Scene;
use crate::snap::{HoldAxis, Snap, SnapKind};

/// Lines closer to parallel than this (the sine of the angle between them) do not cross.
const PARALLEL: f64 = 1e-9;

/// A straight line: a point on it and its direction (a unit vector).
#[derive(Clone, Copy, Debug)]
struct Line {
    origin: Point,
    dir: (f64, f64),
}

impl Line {
    fn new(origin: Point, angle_degrees: f64) -> Option<Self> {
        let dir = direction(angle_degrees);
        (finite(origin) && dir.0.is_finite() && dir.1.is_finite()).then_some(Self { origin, dir })
    }

    /// The distance from `p` to the line and the nearest point on it.
    fn nearest(&self, p: Point) -> Option<(f64, Point)> {
        let along = (p.x - self.origin.x) * self.dir.0 + (p.y - self.origin.y) * self.dir.1;
        let foot = Point::new(self.origin.x + along * self.dir.0, self.origin.y + along * self.dir.1);
        let d = dist(p, foot);
        (d.is_finite() && finite(foot)).then_some((d, foot))
    }

    /// Where this line crosses `other`, or `None` for parallel lines.
    fn crossing(&self, other: &Line) -> Option<Point> {
        let (u, v) = (self.dir, other.dir);
        let cross = u.0 * v.1 - u.1 * v.0;
        if cross.abs() < PARALLEL {
            return None;
        }
        let (bx, by) = (other.origin.x - self.origin.x, other.origin.y - self.origin.y);
        let s = (bx * v.1 - by * v.0) / cross;
        let p = Point::new(self.origin.x + s * u.0, self.origin.y + s * u.1);
        finite(p).then_some(p)
    }
}

/// The nearest node, then the nearest handle, of the background's paths within `tol` font units
/// of `p`. Background components are left out: their points are not drawn as nodes.
pub fn background(scene: &Scene, p: Point, tol: f64) -> Option<Snap> {
    let bg = scene.layer.info.background.as_ref()?;
    for want_on_curve in [true, false] {
        let mut best: Option<(f64, Point)> = None;
        for shape in &bg.shapes {
            let Shape::Path(path) = shape else { continue };
            for node in path.nodes.iter().filter(|n| n.on_curve() == want_on_curve) {
                let q = node.point();
                let d = dist(p, q);
                if finite(q) && d <= tol && best.is_none_or(|b| d < b.0) {
                    best = Some((d, q));
                }
            }
        }
        if let Some((_, q)) = best {
            let kind = if want_on_curve { SnapKind::Node } else { SnapKind::Handle };
            return Some(Snap { point: q, kind: Some(kind), node: None, marker: Some(q), lines: Vec::new() });
        }
    }
    None
}

/// The guides that show in the scene, as lines.
fn lines<'s>(scene: &'s Scene<'_>) -> impl Iterator<Item = Line> + 's {
    scene.local_guides().iter().chain(scene.shown_global_guides().map(|(_, g)| g)).filter_map(move |g| Line::new(scene.knob(g), g.angle))
}

/// Where the guides cross the line through `p` along `axis`.
pub fn crossings<'s>(scene: &'s Scene<'_>, p: Point, axis: HoldAxis) -> impl Iterator<Item = Point> + 's {
    let held = Line::new(p, if axis == HoldAxis::Horizontal { 0.0 } else { 90.0 });
    lines(scene).filter_map(move |guide| held?.crossing(&guide))
}

/// The guide nearest to `p` within `tol` font units: where it crosses another guide, a metric
/// line or a sidebearing, when that crossing is also within `tol`, else the nearest point of the
/// guide.
pub fn guides(scene: &Scene, p: Point, tol: f64) -> Option<Snap> {
    let all: Vec<Line> = lines(scene).collect();
    let mut nearest: Option<(f64, usize, Point)> = None;
    for (i, line) in all.iter().enumerate() {
        let Some((d, foot)) = line.nearest(p) else { continue };
        if d <= tol && nearest.is_none_or(|b| d < b.0) {
            nearest = Some((d, i, foot));
        }
    }
    let (_, index, foot) = nearest?;
    let guide = all.get(index)?;

    let width = if scene.layer.width.is_finite() { scene.layer.width } else { 0.0 };
    let others = all
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != index)
        .map(|(_, l)| *l)
        .chain(scene.metric_lines.iter().filter_map(|&y| Line::new(Point::new(0.0, y), 0.0)))
        .chain([0.0, width].into_iter().filter_map(|x| Line::new(Point::new(x, 0.0), 90.0)));
    let mut crossing: Option<(f64, Point)> = None;
    for other in others {
        let Some(q) = guide.crossing(&other) else { continue };
        let d = dist(p, q);
        if d <= tol && crossing.is_none_or(|b| d < b.0) {
            crossing = Some((d, q));
        }
    }
    let point = crossing.map_or(foot, |(_, q)| q);
    Some(Snap { point, kind: Some(SnapKind::Guide), node: None, marker: Some(point), lines: Vec::new() })
}
