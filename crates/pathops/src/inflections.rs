//! Add Inflections (`path.addInflections`): a node at each inflection of a cubic segment, the
//! repair for the audit's `check.outlines.inflections`.

use std::collections::HashSet;

use fontcraft_geom::{CubicBez, ParamCurve, Point};

use crate::PathOpError;
use crate::contour::{Contour, Piece, RNode, Ring, round_to_grid, split_segment};

/// An inflection closer than this many units to an end of its segment gets no node; the audit
/// check ignores it too.
pub const MIN_INFLECTION_DISTANCE: f64 = 1.0;

/// Add Inflections on one contour, on the segments whose two end nodes are in `selected` (indices
/// into `contour.nodes`), or on all of them. Each cubic piece is split at every inflection that
/// lies more than [`MIN_INFLECTION_DISTANCE`] units from both ends of its segment. The new nodes
/// are smooth; they and the new handles are rounded to `grid` (0 leaves them unrounded). Returns
/// the contour and the number of nodes added.
pub fn add_inflections(contour: &Contour, selected: Option<&[usize]>, grid: f64) -> Result<(Contour, usize), PathOpError> {
    let mut ring = Ring::from_contour(contour)?;
    let set: Option<HashSet<usize>> = selected.map(|s| s.iter().copied().collect());
    let chosen = |r: &RNode| set.as_ref().is_none_or(|s| r.src.is_some_and(|i| s.contains(&i)));

    let mut added = 0;
    let mut out: Vec<RNode> = Vec::with_capacity(ring.len());
    let mut tail: Vec<RNode> = Vec::new();
    for j in 0..ring.len() {
        let Some(original) = ring.nodes.get(j) else { continue };
        let ends_chosen = chosen(original) && ring.prev(j).and_then(|p| ring.nodes.get(p)).is_some_and(chosen);
        let cuts = if ends_chosen { inflections(&ring, j) } else { Vec::new() };
        let Some(mut nodes) = split_segment(&ring, j, &cuts).filter(|_| !cuts.is_empty()) else {
            out.push(original.clone());
            continue;
        };
        let new = nodes.len().saturating_sub(1);
        added += new;
        let exact = nodes.clone();
        for (i, r) in nodes.iter_mut().enumerate() {
            for c in &mut r.ctrl {
                c.point = round_to_grid(c.point, grid);
            }
            if i < new {
                r.node.point = round_to_grid(r.node.point, grid);
            }
        }
        let from = ring.seg(j).map(|s| s.from);
        for i in 0..new {
            if let Some(from) = from {
                straighten(&exact, &mut nodes, from, i, grid);
            }
        }
        if j == 0 && ring.closed {
            // The start stays first; the closing segment's new nodes go at the end.
            let end = nodes.pop();
            tail = nodes;
            out.extend(end);
        } else {
            out.extend(nodes);
        }
    }
    out.extend(tail);
    ring.nodes = out;
    Ok((ring.to_contour(), added))
}

/// Rounding the two handles of new node `i` to the grid can leave them off one line through the
/// node, which puts a small new inflection next to it. When it does, this tries the other grid
/// points around each handle's exact position (nearest first) and keeps the first choice that
/// leaves neither neighbouring segment with an inflection away from its ends. `from` is where the
/// first of `nodes` starts.
fn straighten(exact: &[RNode], nodes: &mut [RNode], from: Point, i: usize, grid: f64) {
    let start = |k: usize| if k == 0 { Some(from) } else { nodes.get(k - 1).map(|r| r.node.point) };
    let clean = |nodes: &[RNode], k: usize| {
        let (Some(a), Some(r)) = (if k == 0 { Some(from) } else { nodes.get(k - 1).map(|r| r.node.point) }, nodes.get(k)) else { return true };
        match r.ctrl.as_slice() {
            [b, c] => !has_inner_inflection(CubicBez::new(a, b.point, c.point, r.node.point)),
            _ => true,
        }
    };
    if grid <= 0.0 || (clean(nodes, i) && clean(nodes, i + 1)) || start(i).is_none() {
        return;
    }
    let (Some(h_in), Some(h_out)) =
        (exact.get(i).and_then(|r| r.ctrl.last()).map(|n| n.point), exact.get(i + 1).and_then(|r| r.ctrl.first()).map(|n| n.point))
    else {
        return;
    };
    let around = |p: Point| -> Vec<Point> {
        let xs = [(p.x / grid).floor() * grid, (p.x / grid).ceil() * grid];
        let ys = [(p.y / grid).floor() * grid, (p.y / grid).ceil() * grid];
        let mut out: Vec<Point> = xs.iter().flat_map(|&x| ys.iter().map(move |&y| Point::new(x, y))).collect();
        out.sort_by(|a, b| a.distance(p).total_cmp(&b.distance(p)));
        out.dedup();
        out
    };
    let mut choices: Vec<(f64, Point, Point)> =
        around(h_in).into_iter().flat_map(|a| around(h_out).into_iter().map(move |b| (a.distance(h_in) + b.distance(h_out), a, b))).collect();
    choices.sort_by(|x, y| x.0.total_cmp(&y.0));
    for (_, a, b) in choices {
        let mut trial = nodes.to_vec();
        if let Some(h) = trial.get_mut(i).and_then(|r| r.ctrl.last_mut()) {
            h.point = a;
        }
        if let Some(h) = trial.get_mut(i + 1).and_then(|r| r.ctrl.first_mut()) {
            h.point = b;
        }
        if clean(&trial, i) && clean(&trial, i + 1) {
            nodes.clone_from_slice(&trial);
            return;
        }
    }
}

/// Whether the cubic has an inflection more than [`MIN_INFLECTION_DISTANCE`] from both its ends.
fn has_inner_inflection(c: CubicBez) -> bool {
    c.inflections()
        .into_iter()
        .filter(|t| t.is_finite() && *t > 0.0 && *t < 1.0)
        .map(|t| c.eval(t))
        .any(|p| p.distance(c.p0) > MIN_INFLECTION_DISTANCE && p.distance(c.p3) > MIN_INFLECTION_DISTANCE)
}

/// The inflections inside the segment that ends at node `j`, as (piece, parameter) in path order,
/// without those too close to its ends.
fn inflections(ring: &Ring, j: usize) -> Vec<(usize, f64)> {
    let Some(seg) = ring.seg(j) else { return Vec::new() };
    let mut cuts = Vec::new();
    for (i, piece) in seg.pieces().iter().enumerate() {
        let Piece::Cubic(c) = piece else { continue };
        let mut ts: Vec<f64> = c.inflections().into_iter().filter(|t| t.is_finite() && *t > 0.0 && *t < 1.0).collect();
        ts.sort_by(f64::total_cmp);
        ts.dedup();
        for t in ts {
            let p = piece.eval(t);
            if p.distance(seg.from) > MIN_INFLECTION_DISTANCE && p.distance(seg.to) > MIN_INFLECTION_DISTANCE {
                cuts.push((i, t));
            }
        }
    }
    cuts
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    use crate::contour::contours_from_bez;
    use crate::coverage_difference;
    use fontcraft_geom::{BezPath, Point};

    #[test]
    fn an_s_curve_gets_a_node_at_its_inflection() {
        // The S runs from (0, 0) to (300, 0) and turns its bend at its middle, (150, 0).
        let path = BezPath::from_svg("M0,0 C100,200 200,-200 300,0 L300,-300 L0,-300 Z").unwrap();
        let c = contours_from_bez(&path).remove(0);
        let (out, added) = add_inflections(&c, None, 1.0).unwrap();
        assert_eq!(added, 1);
        let node = out.nodes.iter().find(|n| n.on_curve() && n.point == Point::new(150.0, 0.0)).expect("a node at the inflection");
        assert!(node.smooth);
        assert_eq!(out.nodes[0].point, c.nodes[0].point, "the start node stays first");
        let d = coverage_difference(&[c.to_bez().unwrap()], &[out.to_bez().unwrap()]).unwrap();
        assert!(d.within(0.5, 0.005), "{d:?}");
        // A second run adds nothing; an unselected segment is left alone.
        assert_eq!(add_inflections(&out, None, 1.0).unwrap().1, 0);
        assert_eq!(add_inflections(&c, Some(&[4, 5]), 1.0).unwrap().1, 0);
    }
}
