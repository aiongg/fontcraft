//! Correct Path Direction: directions from nesting, start nodes and shape order
//! (`path.correctPathDirection`, `paths.directionConvention`, docs/spec/03a › question 16).

use fontcraft_geom::{BezPath, Point, Rect, Shape};

use crate::PathOpError;
use crate::contour::{Contour, Ring};

/// Most closed contours whose nesting one call works out; the pairs grow with the square.
pub const MAX_NESTING_CONTOURS: usize = 1_000;
/// Points of a contour tested for lying inside another.
const NESTING_SAMPLES: usize = 16;
/// A closed contour with less area than this, in square units, has no direction.
const MIN_AREA: f64 = 1e-9;

#[derive(Clone, Debug, PartialEq)]
pub struct Directed {
    /// The contours in their new order.
    pub contours: Vec<Contour>,
    /// For each result contour, the index of the input contour it is.
    pub order: Vec<usize>,
}

/// Correct Path Direction on one layer's contours. A closed contour inside an even number of
/// others (an outer contour) runs counter-clockwise, y up; one inside an odd number (a counter)
/// runs clockwise. A contour is inside another when that one is larger, its box holds the
/// contour's box, and most of the contour's sample points (nodes and segment midpoints) lie
/// inside it, so two contours that only overlap are both outer. Each closed contour then starts
/// at its lowest, then leftmost, on-curve node, and the contours are sorted by [`shape_order`].
/// Open contours keep their direction and start and are sorted with the rest. Contours with no
/// area keep their direction.
pub fn correct_direction(contours: &[Contour]) -> Result<Directed, PathOpError> {
    let total: usize = contours.iter().map(|c| c.nodes.len()).sum();
    if total > crate::MAX_NODES {
        return Err(PathOpError::TooManyNodes(total));
    }
    let mut rings = contours.iter().map(Ring::from_contour).collect::<Result<Vec<_>, _>>()?;
    let closed = rings.iter().filter(|r| r.closed).count();
    if closed > MAX_NESTING_CONTOURS {
        return Err(PathOpError::TooManyContours(closed));
    }
    let info: Vec<Option<Nesting>> = rings.iter().map(Nesting::new).collect();
    for (b, ring) in rings.iter_mut().enumerate() {
        let Some(Some(nb)) = info.get(b) else { continue };
        if nb.area.abs() < MIN_AREA {
            continue;
        }
        let depth = info.iter().enumerate().filter(|(a, na)| *a != b && na.as_ref().is_some_and(|na| na.holds(*a, nb, b))).count();
        let want_ccw = depth.is_multiple_of(2);
        if (nb.area > 0.0) != want_ccw {
            *ring = ring.reversed();
        }
    }
    for ring in rings.iter_mut().filter(|r| r.closed) {
        let lowest = ring
            .nodes
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| a.node.point.y.total_cmp(&b.node.point.y).then(a.node.point.x.total_cmp(&b.node.point.x)))
            .map(|(i, _)| i);
        if let Some(i) = lowest {
            ring.nodes.rotate_left(i);
        }
    }
    let boxes: Vec<Option<Rect>> = rings.iter().map(Ring::bounds).collect();
    let mut order: Vec<usize> = (0..rings.len()).collect();
    let place = |i: usize| shape_order(boxes.get(i).copied().flatten());
    order.sort_by(|&a, &b| place(a).0.total_cmp(&place(b).0).then(place(a).1.total_cmp(&place(b).1)));
    let contours = order.iter().filter_map(|&i| rings.get(i)).map(Ring::to_contour).collect();
    Ok(Directed { contours, order })
}

/// Where a shape with these bounds comes in the shape order, from the bottom left to the top right:
/// by the bottom-left corner's `x + y`, then by its `x`. Shapes without bounds come last.
pub fn shape_order(bounds: Option<Rect>) -> (f64, f64) {
    bounds.map_or((f64::INFINITY, f64::INFINITY), |r| (r.x0 + r.y0, r.x0))
}

/// What the nesting test needs of one closed contour.
struct Nesting {
    path: BezPath,
    area: f64,
    bounds: Rect,
    samples: Vec<Point>,
}

impl Nesting {
    fn new(ring: &Ring) -> Option<Self> {
        if !ring.closed || ring.nodes.is_empty() {
            return None;
        }
        let path = ring.to_bez();
        let area = path.area();
        let bounds = ring.bounds()?;
        let mut points: Vec<Point> = Vec::new();
        for j in ring.segment_ends() {
            if let Some(r) = ring.nodes.get(j) {
                points.push(r.node.point);
            }
            if let Some(seg) = ring.seg(j)
                && let Some(piece) = seg.pieces().first()
            {
                points.push(piece.eval(0.5));
            }
        }
        let step = points.len().div_ceil(NESTING_SAMPLES).max(1);
        let samples = points.into_iter().step_by(step).collect();
        Some(Self { path, area, bounds, samples })
    }

    /// Does this contour (input `a`) hold contour `b`?
    fn holds(&self, a: usize, b: &Nesting, b_index: usize) -> bool {
        let (sa, sb) = (self.area.abs(), b.area.abs());
        if sa < MIN_AREA || sa < sb || (sa == sb && a > b_index) {
            return false;
        }
        let eps = 1e-9;
        let r = self.bounds;
        let q = b.bounds;
        if q.x0 < r.x0 - eps || q.y0 < r.y0 - eps || q.x1 > r.x1 + eps || q.y1 > r.y1 + eps {
            return false;
        }
        let inside = b.samples.iter().filter(|p| self.path.winding(**p) != 0).count();
        2 * inside > b.samples.len()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    use crate::fixtures::rect;

    #[test]
    fn a_counter_drawn_the_wrong_way_is_turned_round() {
        // Both squares counter-clockwise: the inner one is a counter and must run clockwise.
        let inner = rect(100.0, 100.0, 200.0, 200.0).tagged();
        let outer = rect(0.0, 0.0, 300.0, 300.0);
        let out = correct_direction(&[inner, outer]).unwrap();
        assert_eq!(out.order, vec![1, 0], "the outer contour is nearer the bottom left");
        let areas: Vec<f64> = out.contours.iter().map(|c| c.to_bez().unwrap().area()).collect();
        assert_eq!(areas, vec![90_000.0, -10_000.0]);
        // The counter starts at its lowest, leftmost node and keeps its node tags.
        let counter = &out.contours[1];
        assert_eq!(counter.nodes[0].point, Point::new(100.0, 100.0));
        assert_eq!(counter.nodes[0].tag, Some(0));
        let mut tags: Vec<_> = counter.nodes.iter().map(|n| n.tag.unwrap()).collect();
        tags.sort();
        assert_eq!(tags, vec![0, 1, 2, 3]);
        // A second run changes nothing.
        assert_eq!(correct_direction(&out.contours).unwrap().contours, out.contours);
    }

    #[test]
    fn contours_run_from_the_bottom_left_to_the_top_right() {
        // The bar of a t reaches past its stem's left edge, high up: the stem comes first.
        let stem = rect(60.0, 0.0, 160.0, 800.0);
        let bar = rect(-5.0, 600.0, 310.0, 680.0);
        assert_eq!(correct_direction(&[bar, stem]).unwrap().order, vec![1, 0]);
        // Two dots side by side: the left one first.
        let dots = [rect(250.0, 660.0, 380.0, 800.0), rect(40.0, 660.0, 170.0, 800.0)];
        assert_eq!(correct_direction(&dots).unwrap().order, vec![1, 0]);
    }

    #[test]
    fn overlapping_contours_are_both_outer_and_a_clockwise_outer_is_turned() {
        let a = rect(0.0, 0.0, 100.0, 100.0).reversed().unwrap();
        let b = rect(50.0, 50.0, 150.0, 150.0);
        let out = correct_direction(&[b, a]).unwrap();
        assert_eq!(out.order, vec![1, 0]);
        for c in &out.contours {
            assert!(c.to_bez().unwrap().area() > 0.0);
        }
        // Three levels: the island inside the counter is outer again.
        let three = [rect(0.0, 0.0, 300.0, 300.0), rect(50.0, 50.0, 250.0, 250.0), rect(100.0, 100.0, 200.0, 200.0)];
        let out = correct_direction(&three).unwrap();
        let signs: Vec<bool> = out.contours.iter().map(|c| c.to_bez().unwrap().area() > 0.0).collect();
        assert_eq!(signs, vec![true, false, true]);
    }
}
