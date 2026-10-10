//! Tidy up Paths (`path.tidyUpPaths`, docs/spec/03a › Tidy up Paths and question 9).

use std::collections::HashSet;

use crate::PathOpError;
use crate::boolean::SMOOTH_MAX_DEGREES;
use crate::contour::{Contour, NodeKind, RNode, Ring, Seg, dist_to_segment};

/// Points within this many units of a line, or of each other, count as on it, or as one point.
pub const TIDY_TOLERANCE: f64 = 0.01;
/// The most [`tidy_up_straightening`] straightens: the largest `maxDeviation` the audit takes.
pub const MAX_STRAIGHTEN: f64 = 100.0;
/// Points sampled on each piece of a curve to measure how far it strays from its chord.
const STRAIGHTEN_SAMPLES: u32 = 32;

/// Tidy up Paths on one contour, on the on-curve nodes `selected` (indices into
/// `contour.nodes`), or on all of them. In this order it
/// 1. turns curves whose off-curve nodes lie on the line between their ends into lines (both
///    end nodes selected),
/// 2. merges nodes that sit on the node before them, when the segment between them is straight
///    or has no extent (both selected),
/// 3. removes a node between two lines when it lies on the line between its neighbours (the
///    node selected; the ends of an open contour stay),
/// 4. makes each selected node smooth when a curve meets it and its two directions are within
///    1°, and a corner otherwise.
///
/// Removing points can break compatibility with other masters (`tidyUp.mayBreakCompatibility`);
/// that check belongs to the caller.
pub fn tidy_up(contour: &Contour, selected: Option<&[usize]>) -> Result<Contour, PathOpError> {
    tidy_up_straightening(contour, selected, 0.0)
}

/// [`tidy_up`] that also turns into lines the curves (both end nodes selected) whose every point
/// lies less than `straighten` units from the line between their ends: the repair for the audit's
/// almost-straight curves. Values up to [`TIDY_TOLERANCE`] add nothing; at most
/// [`MAX_STRAIGHTEN`] is used.
pub fn tidy_up_straightening(contour: &Contour, selected: Option<&[usize]>, straighten: f64) -> Result<Contour, PathOpError> {
    let straighten = if straighten.is_finite() { straighten.clamp(0.0, MAX_STRAIGHTEN) } else { 0.0 };
    let mut ring = Ring::from_contour(contour)?;
    let set: Option<HashSet<usize>> = selected.map(|s| s.iter().copied().collect());
    let chosen = |r: &RNode| set.as_ref().is_none_or(|s| r.src.is_some_and(|i| s.contains(&i)));

    for j in ring.segment_ends() {
        let flat = ring.prev(j).and_then(|p| ring.nodes.get(p)).is_some_and(chosen)
            && ring.nodes.get(j).is_some_and(chosen)
            && ring.seg(j).is_some_and(|s| {
                s.is_curve()
                    && (s.ctrl.iter().all(|c| dist_to_segment(c.point, s.from, s.to) <= TIDY_TOLERANCE)
                        || (straighten > TIDY_TOLERANCE && s.from != s.to && strays_less(&s, straighten)))
            });
        if flat && let Some(r) = ring.nodes.get_mut(j) {
            r.ctrl.clear();
            r.node.kind = NodeKind::Line;
        }
    }

    ring.remove_zero_length(TIDY_TOLERANCE, |a, b| chosen(a) && chosen(b));

    for _pass in 0..2 {
        let mut j = usize::from(!ring.closed);
        // Each step removes a node and steps back by at most one, or moves on.
        while j < ring.len() && ring.len() > 2 {
            if on_line_between(&ring, j) && ring.nodes.get(j).is_some_and(chosen) {
                ring.nodes.remove(j);
                j = j.saturating_sub(1).max(usize::from(!ring.closed));
            } else {
                j += 1;
            }
        }
    }

    let last = ring.len().saturating_sub(1);
    for j in 0..ring.len() {
        if !ring.nodes.get(j).is_some_and(chosen) {
            continue;
        }
        let end = !ring.closed && (j == 0 || j == last);
        let smooth = !end && ring.looks_smooth(j, SMOOTH_MAX_DEGREES).unwrap_or(false);
        if let Some(r) = ring.nodes.get_mut(j) {
            r.node.smooth = smooth;
        }
    }
    Ok(ring.to_contour())
}

/// Whether every sampled point of the segment lies less than `limit` units from the line between
/// its ends.
fn strays_less(seg: &Seg<'_>, limit: f64) -> bool {
    seg.pieces().iter().all(|piece| {
        (0..=STRAIGHTEN_SAMPLES).all(|i| dist_to_segment(piece.eval(f64::from(i) / f64::from(STRAIGHTEN_SAMPLES)), seg.from, seg.to) < limit)
    })
}

/// Is node `j` between two straight segments and on the line between its neighbours?
fn on_line_between(ring: &Ring, j: usize) -> bool {
    let (Some(seg_in), Some(seg_out)) = (ring.seg(j), ring.next(j).and_then(|n| ring.seg(n))) else { return false };
    if seg_in.is_curve() || seg_out.is_curve() {
        return false;
    }
    let (a, p, b) = (seg_in.from, seg_in.to, seg_out.to);
    let ab = b - a;
    let len2 = ab.hypot2();
    if len2 <= 0.0 {
        return false;
    }
    let t = (p - a).dot(ab) / len2;
    t > 0.0 && t < 1.0 && dist_to_segment(p, a, b) <= TIDY_TOLERANCE
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    use crate::contour::{Node, contours_from_bez};
    use fontcraft_geom::{BezPath, Point};

    #[test]
    fn redundant_points_go_and_node_types_are_set() {
        // A square drawn with: a flat curve on the bottom, a point in the middle of the right
        // edge, a doubled corner at the top right, and a round top left corner.
        let path = BezPath::from_svg("M0,0 C30,0 70,0 100,0 L100,50 L100,100 L100,100 L40,100 C20,100 0,80 0,60 Z").unwrap();
        let c = contours_from_bez(&path).remove(0).tagged();
        let out = tidy_up(&c, None).unwrap();
        let points: Vec<Point> = out.nodes.iter().filter(|n| n.on_curve()).map(|n| n.point).collect();
        assert_eq!(
            points,
            vec![Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, 100.0), Point::new(40.0, 100.0), Point::new(0.0, 60.0)]
        );
        assert_eq!(out.nodes[1].kind, NodeKind::Line);
        // The line meets the round corner along its handle: smooth. Lines meet at corners.
        let on: Vec<&Node> = out.nodes.iter().filter(|n| n.on_curve()).collect();
        assert!(on[3].smooth && on[4].smooth);
        assert!(!on[0].smooth && !on[1].smooth && !on[2].smooth);
        // A second run changes nothing.
        assert_eq!(tidy_up(&out, None).unwrap(), out);
    }

    #[test]
    fn only_selected_nodes_are_tidied() {
        let path = BezPath::from_svg("M0,0 L50,0 L100,0 L100,100 L50,100 L0,100 Z").unwrap();
        let c = contours_from_bez(&path).remove(0);
        // Node 1 (50,0) is selected; node 4 (50,100) is not.
        let out = tidy_up(&c, Some(&[1])).unwrap();
        assert_eq!(out.nodes.len(), 5);
        assert!(out.nodes.iter().all(|n| n.point != Point::new(50.0, 0.0)));
        assert!(out.nodes.iter().any(|n| n.point == Point::new(50.0, 100.0)));
    }
}
