//! Cut with Line, the Knife tool's operation (`path.cutWithLine`, docs/spec/03a › Knife).

use std::collections::HashMap;

use fontcraft_geom::{Point, Vec2};

use crate::contour::{Contour, Piece, RNode, Ring, round_to_grid, split_segment, unit_roots};
use crate::graph::{Graph, Rewired, assemble};
use crate::{PathOpError, check};

/// A crossing this close to the end of a segment's parameter range is at the node there.
const T_NODE: f64 = 1e-9;
/// Bisection steps when locating a crossing; 2⁻⁶⁰ of a segment is far below any unit.
const BISECTION_STEPS: usize = 60;

/// Where a crossing lies on its contour.
#[derive(Clone, Copy, Debug, PartialEq)]
enum At {
    /// At a ring node.
    Node(usize),
    /// Inside the segment that ends at a ring node: piece and parameter.
    Inside { seg: usize, piece: usize, t: f64 },
}

#[derive(Clone, Copy, Debug)]
struct Crossing {
    contour: usize,
    closed: bool,
    at: At,
    /// +1 where the contour crosses from the right of the line to its left, −1 the other way.
    sign: i32,
    /// Position along the cut: 0 at `from`, 1 at `to`.
    s: f64,
}

/// Cut with Line on one layer's contours: the segment from `from` to `to` cuts every contour it
/// crosses. Along the line, the crossings with closed contours are walked in order, counting the
/// nonzero winding; each stretch that goes inside and comes back out pairs its crossings in
/// order, and each pair is joined by straight segments along the cut, so the pieces close along
/// the line and pieces of overlapping contours join one another. A stretch that starts or ends
/// inside a shape (the cut stops inside it) is not cut. An open contour is split where the line
/// crosses it. New points are rounded to `grid`. Points where the line only touches a contour
/// are not crossings.
pub fn cut_with_line(contours: &[Contour], from: Point, to: Point, grid: f64) -> Result<Rewired, PathOpError> {
    for v in [from.x, from.y, to.x, to.y] {
        check(v)?;
    }
    let total: usize = contours.iter().map(|c| c.nodes.len()).sum();
    if total > crate::MAX_NODES {
        return Err(PathOpError::TooManyNodes(total));
    }
    let dir = to - from;
    let len2 = dir.hypot2();
    if len2 < 1e-12 {
        return Ok(Rewired::unchanged(contours));
    }
    let rings = contours.iter().map(Ring::from_contour).collect::<Result<Vec<_>, _>>()?;
    let side = |p: Point| dir.cross(p - from);
    let mut crossings = Vec::new();
    for (ci, ring) in rings.iter().enumerate() {
        for j in ring.segment_ends() {
            let Some(seg) = ring.seg(j) else { continue };
            let pieces = seg.pieces();
            let last = pieces.len().saturating_sub(1);
            for (pi, piece) in pieces.iter().enumerate() {
                for (t, sign) in piece_crossings(piece, dir, &side) {
                    let at = if pi == 0 && t <= T_NODE {
                        At::Node(ring.prev(j).unwrap_or(j))
                    } else if pi == last && t >= 1.0 - T_NODE {
                        At::Node(j)
                    } else {
                        At::Inside { seg: j, piece: pi, t }
                    };
                    let s = (piece.eval(t) - from).dot(dir) / len2;
                    crossings.push(Crossing { contour: ci, closed: ring.closed, at, sign, s });
                }
            }
        }
    }
    cancel_touches(&mut crossings);

    // Pairs of crossings of closed contours, and lone crossings of open ones.
    let mut closed: Vec<&Crossing> = crossings.iter().filter(|x| x.closed).collect();
    closed.sort_by(|a, b| a.s.total_cmp(&b.s));
    let mut pairs: Vec<(Crossing, Crossing)> = Vec::new();
    let mut winding = 0i32;
    let mut run: Vec<Crossing> = Vec::new();
    let mut in_run = false;
    for x in closed {
        if x.s < 0.0 {
            winding += x.sign;
            continue;
        }
        if x.s > 1.0 {
            break;
        }
        let before = winding;
        winding += x.sign;
        if before == 0 && winding != 0 {
            run.clear();
            run.push(*x);
            in_run = true;
        } else if in_run {
            run.push(*x);
            if winding == 0 {
                for &[a, b] in run.as_chunks::<2>().0 {
                    pairs.push((a, b));
                }
                in_run = false;
            }
        }
    }
    let splits: Vec<Crossing> = crossings.iter().filter(|x| !x.closed && (0.0..=1.0).contains(&x.s)).copied().collect();
    if pairs.is_empty() && splits.is_empty() {
        return Ok(Rewired::unchanged(contours));
    }

    // New nodes where the used crossings lie inside segments.
    let used: Vec<Crossing> = pairs.iter().flat_map(|(a, b)| [*a, *b]).chain(splits.iter().copied()).collect();
    let mut involved = vec![false; contours.len()];
    for x in &used {
        if let Some(v) = involved.get_mut(x.contour) {
            *v = true;
        }
    }
    let mut graph = Graph::default();
    let mut ids: HashMap<(usize, usize), usize> = HashMap::new();
    let mut inside_ids: HashMap<(usize, usize, usize, u64), usize> = HashMap::new();
    for (ci, ring) in rings.iter().enumerate() {
        if !involved.get(ci).copied().unwrap_or(false) {
            continue;
        }
        let cuts: Vec<(usize, usize, f64)> = used
            .iter()
            .filter(|x| x.contour == ci)
            .filter_map(|x| match x.at {
                At::Inside { seg, piece, t } => Some((seg, piece, t)),
                At::Node(_) => None,
            })
            .collect();
        let (cut_ring, old_index, new_index) = insert_cuts(ring, &cuts, grid);
        let base = graph.add_ring(ci, &cut_ring);
        for (old, new) in old_index.iter().enumerate() {
            ids.insert((ci, old), base + new);
        }
        for ((seg, piece, t), new) in new_index {
            inside_ids.insert((ci, seg, piece, t), base + new);
        }
    }
    let id_of = |x: &Crossing| match x.at {
        At::Node(k) => ids.get(&(x.contour, k)).copied(),
        At::Inside { seg, piece, t } => inside_ids.get(&(x.contour, seg, piece, t.to_bits())).copied(),
    };
    for x in &used {
        if let Some(g) = id_of(x).and_then(|id| graph.get_mut(id)) {
            g.r.mark = true;
        }
    }
    for (a, b) in &pairs {
        if let (Some(a), Some(b)) = (id_of(a), id_of(b)) {
            graph.rewire(a, b);
        }
    }
    for x in &splits {
        if let Some(id) = id_of(x) {
            graph.split(id);
        }
    }
    Ok(assemble(contours, &involved, graph.trace()))
}

/// Parameters where `piece` crosses the line, with the sign of each crossing. A point on the line
/// counts as left of it, so each change of side is found once, at a node or inside one piece.
fn piece_crossings(piece: &Piece, dir: Vec2, side: &impl Fn(Point) -> f64) -> Vec<(f64, i32)> {
    let (a, b, c) = piece.deriv_coeffs();
    let mut ts = vec![0.0];
    ts.extend(unit_roots(dir.cross(a), dir.cross(b), dir.cross(c)));
    ts.push(1.0);
    let value = |t: f64| {
        if t <= 0.0 {
            side(piece.start())
        } else if t >= 1.0 {
            side(piece.end())
        } else {
            side(piece.eval(t))
        }
    };
    let mut out = Vec::new();
    for w in ts.windows(2) {
        let [t0, t1] = w else { continue };
        let (left0, left1) = (value(*t0) >= 0.0, value(*t1) >= 0.0);
        if left0 == left1 {
            continue;
        }
        let (mut lo, mut hi) = (*t0, *t1);
        for _ in 0..BISECTION_STEPS {
            let mid = 0.5 * (lo + hi);
            if (value(mid) >= 0.0) == left0 {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        out.push((0.5 * (lo + hi), if left1 { 1 } else { -1 }));
    }
    out
}

/// Removes pairs of opposite crossings at the same node: the contour touches the line there and
/// goes back.
fn cancel_touches(crossings: &mut Vec<Crossing>) {
    let mut at_node: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    for (i, x) in crossings.iter().enumerate() {
        if let At::Node(k) = x.at {
            at_node.entry((x.contour, k)).or_default().push(i);
        }
    }
    let mut gone = vec![false; crossings.len()];
    for list in at_node.values() {
        let (mut ups, mut downs): (Vec<usize>, Vec<usize>) = list.iter().copied().partition(|&i| crossings.get(i).is_some_and(|x| x.sign > 0));
        while let (Some(u), Some(d)) = (ups.pop(), downs.pop()) {
            for i in [u, d] {
                if let Some(g) = gone.get_mut(i) {
                    *g = true;
                }
            }
        }
    }
    let mut i = 0;
    crossings.retain(|_| {
        let keep = !gone.get(i).copied().unwrap_or(false);
        i += 1;
        keep
    });
}

/// `ring` with a node at each cut (segment, piece, parameter), rounded to `grid`. Returns the new
/// ring, the new index of each old node, and the new index of each cut, keyed by the cut with
/// its parameter's bits.
#[allow(clippy::type_complexity)]
fn insert_cuts(ring: &Ring, cuts: &[(usize, usize, f64)], grid: f64) -> (Ring, Vec<usize>, Vec<((usize, usize, u64), usize)>) {
    let mut out: Vec<RNode> = Vec::with_capacity(ring.len() + cuts.len());
    let mut old_index = Vec::with_capacity(ring.len());
    let mut new_index = Vec::with_capacity(cuts.len());
    let mut tail: Vec<(RNode, Option<(usize, usize, u64)>)> = Vec::new();
    for j in 0..ring.len() {
        let mut here: Vec<(usize, f64)> = cuts.iter().filter(|(s, _, _)| *s == j).map(|(_, p, t)| (*p, *t)).collect();
        here.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
        here.dedup();
        let nodes = if here.is_empty() { None } else { split_segment(ring, j, &here) };
        let Some(mut nodes) = nodes else {
            old_index.push(out.len());
            if let Some(r) = ring.nodes.get(j) {
                out.push(r.clone());
            }
            continue;
        };
        let new = nodes.len().saturating_sub(1);
        for (i, r) in nodes.iter_mut().enumerate() {
            for c in &mut r.ctrl {
                c.point = round_to_grid(c.point, grid);
            }
            if i < new {
                r.node.point = round_to_grid(r.node.point, grid);
            }
        }
        // split_segment makes one node per cut, in the order of `here`.
        let mut labels: Vec<Option<(usize, usize, u64)>> = here.iter().map(|(p, t)| Some((j, *p, t.to_bits()))).collect();
        labels.push(None);
        if j == 0 && ring.closed {
            let end = nodes.pop();
            labels.pop();
            tail = nodes.into_iter().zip(labels).collect();
            old_index.push(out.len());
            if let Some(end) = end {
                out.push(end);
            }
        } else {
            for (r, label) in nodes.into_iter().zip(labels) {
                match label {
                    Some(l) => new_index.push((l, out.len())),
                    None => old_index.push(out.len()),
                }
                out.push(r);
            }
        }
    }
    for (r, label) in tail {
        if let Some(l) = label {
            new_index.push((l, out.len()));
        }
        out.push(r);
    }
    (Ring { closed: ring.closed, nodes: out }, old_index, new_index)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    use crate::contour::contours_from_bez;
    use crate::coverage_difference;
    use crate::fixtures::rect;
    use fontcraft_geom::{BezPath, Shape};

    fn bez(cs: &[Contour]) -> Vec<BezPath> {
        cs.iter().map(|c| c.to_bez().unwrap()).collect()
    }

    #[test]
    fn a_cut_through_an_o_gives_two_closed_halves() {
        // An O: a circle-like outer contour and a counter drawn the other way.
        let svg = "M0,-100 C55,-100 100,-55 100,0 C100,55 55,100 0,100 C-55,100 -100,55 -100,0 C-100,-55 -55,-100 0,-100 Z \
                   M0,-50 C-28,-50 -50,-28 -50,0 C-50,28 -28,50 0,50 C28,50 50,28 50,0 C50,-28 28,-50 0,-50 Z";
        let o = contours_from_bez(&BezPath::from_svg(svg).unwrap());
        let out = cut_with_line(&o, Point::new(10.0, -150.0), Point::new(10.0, 150.0), 1.0).unwrap();
        assert_eq!(out.contours.len(), 2);
        assert!(out.contours.iter().all(|c| c.closed));
        for c in &out.contours {
            assert!(c.to_bez().unwrap().area() > 0.0);
        }
        // Four crossings, each a node on both sides of the cut.
        assert_eq!(out.selection.len(), 8);
        let d = coverage_difference(&bez(&o), &bez(&out.contours)).unwrap();
        assert!(d.within(0.5, 0.005), "{d:?}");
    }

    #[test]
    fn a_square_cut_in_two() {
        let out = cut_with_line(&[rect(0.0, 0.0, 100.0, 100.0)], Point::new(40.0, -10.0), Point::new(40.0, 110.0), 1.0).unwrap();
        assert_eq!(out.contours.len(), 2);
        assert_eq!(out.sources, vec![0, 0]);
        let mut areas: Vec<f64> = out.contours.iter().map(|c| c.to_bez().unwrap().area()).collect();
        areas.sort_by(f64::total_cmp);
        assert_eq!(areas, vec![4_000.0, 6_000.0]);
        assert!(out.contours.iter().all(|c| c.nodes.len() == 4));
    }

    #[test]
    fn a_cut_that_stops_inside_or_misses_changes_nothing() {
        let square = [rect(0.0, 0.0, 100.0, 100.0)];
        let inside = cut_with_line(&square, Point::new(40.0, -10.0), Point::new(40.0, 50.0), 1.0).unwrap();
        assert_eq!(inside.contours, square.to_vec());
        let miss = cut_with_line(&square, Point::new(200.0, -10.0), Point::new(200.0, 50.0), 1.0).unwrap();
        assert_eq!(miss.contours, square.to_vec());
        // A line through a corner only touches the square.
        let touch = cut_with_line(&square, Point::new(-50.0, 50.0), Point::new(50.0, -50.0), 1.0).unwrap();
        assert_eq!(touch.contours, square.to_vec());
    }

    #[test]
    fn an_open_path_is_split_where_the_line_crosses_it() {
        let open = contours_from_bez(&BezPath::from_svg("M0,0 L100,0 L100,100").unwrap());
        let out = cut_with_line(&open, Point::new(50.0, -10.0), Point::new(50.0, 10.0), 1.0).unwrap();
        assert_eq!(out.contours.len(), 2);
        assert!(out.contours.iter().all(|c| !c.closed));
        assert_eq!(out.contours[0].nodes.len(), 2);
        assert_eq!(out.contours[1].nodes.len(), 3);
    }
}
