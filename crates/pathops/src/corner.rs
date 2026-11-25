//! Open Corner and Reconnect Nodes (`path.openCorner`, `path.reconnectNodes`,
//! `openCorner.overlapSize`, docs/spec/03a › questions 14 and 15).

use std::collections::{HashMap, HashSet};

use fontcraft_geom::{Point, Vec2};

use crate::contour::{Contour, Node, NodeKind, RNode, Ring, round_to_grid};
use crate::graph::{Graph, Rewired, assemble};
use crate::{MAX_COORDINATE, PathOpError, check};

/// Most nodes Reconnect Nodes pairs in one call; the candidate pairs grow with the square.
pub const MAX_RECONNECT_NODES: usize = 1_000;
/// Two directions closer than this many degrees (or than this to opposite) make no corner to
/// open or restore.
const MIN_CORNER_DEGREES: f64 = 1.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Opened {
    pub contour: Contour,
    /// The nodes of the opened corners in the result, two for each corner.
    pub nodes: Vec<usize>,
    /// The corners that opened, as indices into the input's nodes.
    pub corners: Vec<usize>,
}

/// One pair of nodes Reconnect Nodes reconnects, as `(contour, node)` addresses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reconnection {
    pub a: (usize, usize),
    pub b: (usize, usize),
    /// Whether the two are an open corner, which becomes one node again; otherwise their paths are rewired.
    pub corner: bool,
}

/// Open Corner on the corner nodes `nodes` (indices into `contour.nodes`) of one contour. Each
/// becomes two nodes: the incoming segment goes on past the corner along its own direction, a
/// straight segment goes back, and the outgoing segment starts behind the corner on its own
/// direction, so the two segments cross in a small triangle. Moving an end node along its handle
/// keeps the direction at that end. How far each segment goes past the corner depends on its
/// direction: `overlap.x` for a horizontal one, `overlap.y` for a vertical one
/// (`sqrt((dx·ox)² + (dy·oy)²)` for a unit direction `(dx, dy)`); the caller passes half the
/// master's first vertical and horizontal stems. Smooth nodes, the ends of an open contour, and
/// nodes whose two directions are nearly the same or opposite are left alone. The two new points
/// are rounded to `grid`.
pub fn open_corners(contour: &Contour, nodes: &[usize], overlap: Vec2, grid: f64) -> Result<Opened, PathOpError> {
    if !(overlap.x.is_finite() && overlap.y.is_finite()) {
        return Err(PathOpError::NotFinite);
    }
    if !(0.0..=MAX_COORDINATE).contains(&overlap.x) || !(0.0..=MAX_COORDINATE).contains(&overlap.y) {
        return Err(PathOpError::Invalid(format!("the overlap ({}, {}) must be between 0 and {MAX_COORDINATE} units", overlap.x, overlap.y)));
    }
    let ring = Ring::from_contour(contour)?;
    let wanted: HashSet<usize> = nodes.iter().copied().collect();
    let mut out = Vec::with_capacity(2 * ring.len());
    let mut corners = Vec::new();
    for (j, r) in ring.nodes.iter().enumerate() {
        let open = r.src.is_some_and(|s| wanted.contains(&s)) && !r.node.smooth;
        match open.then(|| corner_pair(&ring, j, overlap, grid)).flatten() {
            Some((c1, c2)) => {
                corners.extend(r.src);
                out.push(RNode { node: Node { point: c1, smooth: false, ..r.node }, ctrl: r.ctrl.clone(), src: r.src, mark: true });
                out.push(RNode { node: Node::new(c2, NodeKind::Line), ctrl: Vec::new(), src: None, mark: true });
            }
            None => out.push(RNode { mark: false, ..r.clone() }),
        }
    }
    let ring = Ring { closed: ring.closed, nodes: out };
    let nodes = (0..ring.len()).filter(|&k| ring.nodes.get(k).is_some_and(|r| r.mark)).map(|k| ring.list_index(k)).collect();
    Ok(Opened { contour: ring.to_contour(), nodes, corners })
}

/// The two points that replace corner node `j`: past it on the incoming direction, and behind it
/// on the outgoing direction.
fn corner_pair(ring: &Ring, j: usize, overlap: Vec2, grid: f64) -> Option<(Point, Point)> {
    let tin = ring.tangent_in(j)?;
    let tout = ring.tangent_out(j)?;
    if tin.cross(tout).abs() < MIN_CORNER_DEGREES.to_radians().sin() {
        return None;
    }
    let reach = |d: Vec2| ((d.x * overlap.x).powi(2) + (d.y * overlap.y).powi(2)).sqrt();
    let c = ring.nodes.get(j)?.node.point;
    let c1 = round_to_grid(c + tin * reach(tin), grid);
    let c2 = round_to_grid(c - tout * reach(tout), grid);
    let in_range = |p: Point| p.x.abs() <= MAX_COORDINATE && p.y.abs() <= MAX_COORDINATE;
    if !(in_range(c1) && in_range(c2)) || c1 == c || c2 == c || c1 == c2 {
        return None;
    }
    Some((c1, c2))
}

/// Reconnect Nodes on the on-curve nodes `nodes` (`(contour, node)` addresses) of one layer's
/// contours. The nodes are paired, nearest first (ties go to the lower addresses). A pair that is
/// an open corner (two nodes joined by a straight segment, the incoming direction of the first
/// and the outgoing direction of the second meeting behind them) becomes one corner node where
/// the two directions meet. Any other pair is rewired: the path that reached one node goes on to
/// the other node's place and continues along the other node's path, which joins two contours
/// into one, or splits one into two, with overlapping corners. Needs an even number of nodes.
pub fn reconnect_nodes(contours: &[Contour], nodes: &[(usize, usize)], grid: f64) -> Result<Rewired, PathOpError> {
    Ok(reconnect_nodes_paired(contours, nodes, grid)?.0)
}

/// [`reconnect_nodes`], and what it did with each pair: [`reconnect_pairs`] does the same on contours with the same
/// node types drawn differently.
pub fn reconnect_nodes_paired(contours: &[Contour], nodes: &[(usize, usize)], grid: f64) -> Result<(Rewired, Vec<Reconnection>), PathOpError> {
    let mut chosen = nodes.to_vec();
    chosen.sort_unstable();
    chosen.dedup();
    if chosen.len() > MAX_RECONNECT_NODES {
        return Err(PathOpError::Invalid(format!("Reconnect Nodes takes at most {MAX_RECONNECT_NODES} nodes, not {}", chosen.len())));
    }
    if chosen.len() < 2 || !chosen.len().is_multiple_of(2) {
        return Err(PathOpError::Invalid("Reconnect Nodes needs an even number of nodes".into()));
    }
    let mut points: Vec<((usize, usize), Point)> = Vec::with_capacity(chosen.len());
    for &(c, n) in &chosen {
        let node = contours.get(c).and_then(|c| c.nodes.get(n)).filter(|n| n.on_curve()).ok_or_else(|| off_curve(c, n))?;
        check(node.point.x)?;
        check(node.point.y)?;
        points.push(((c, n), node.point));
    }
    reconnect(contours, pairs(&points).into_iter().map(|(a, b)| (a, b, None)).collect(), grid)
}

/// Reconnects `pairs`, each as it says. A pair that is to become a corner and is no open corner is refused.
pub fn reconnect_pairs(contours: &[Contour], pairs: &[Reconnection], grid: f64) -> Result<Rewired, PathOpError> {
    if pairs.len() > MAX_RECONNECT_NODES {
        return Err(PathOpError::Invalid(format!("Reconnect Nodes takes at most {MAX_RECONNECT_NODES} nodes, not {}", pairs.len())));
    }
    Ok(reconnect(contours, pairs.iter().map(|p| (p.a, p.b, Some(p.corner))).collect(), grid)?.0)
}

fn off_curve(contour: usize, node: usize) -> PathOpError {
    PathOpError::Invalid(format!("node {node} of path {contour} is not an on-curve node"))
}

/// A pair of addresses, and whether they are to become a corner (`None`: when they are an open one).
type Wanted = ((usize, usize), (usize, usize), Option<bool>);

fn reconnect(contours: &[Contour], wanted: Vec<Wanted>, grid: f64) -> Result<(Rewired, Vec<Reconnection>), PathOpError> {
    let total: usize = contours.iter().map(|c| c.nodes.len()).sum();
    if total > crate::MAX_NODES {
        return Err(PathOpError::TooManyNodes(total));
    }
    let chosen: HashSet<usize> = wanted.iter().flat_map(|(a, b, _)| [a.0, b.0]).collect();
    let involved: Vec<bool> = (0..contours.len()).map(|i| chosen.contains(&i)).collect();
    let mut graph = Graph::default();
    let mut ids: HashMap<(usize, usize), usize> = HashMap::new();
    for (i, c) in contours.iter().enumerate() {
        if !involved.get(i).copied().unwrap_or(false) {
            continue;
        }
        let ring = Ring::from_contour(c)?;
        let base = graph.add_ring(i, &ring);
        for (k, r) in ring.nodes.iter().enumerate() {
            if let Some(src) = r.src {
                ids.insert((i, src), base + k);
            }
        }
    }
    let id_of = |at: (usize, usize)| ids.get(&at).copied().ok_or_else(|| off_curve(at.0, at.1));
    let mut done = Vec::with_capacity(wanted.len());
    for (at_a, at_b, corner) in wanted {
        let (a, b) = (id_of(at_a)?, id_of(at_b)?);
        for id in [a, b] {
            if let Some(g) = graph.get_mut(id) {
                g.r.mark = true;
            }
        }
        let restored = corner != Some(false) && restore_corner(&mut graph, a, b, grid);
        if corner == Some(true) && !restored {
            return Err(PathOpError::Invalid(format!("nodes {} and {} of path {} are not an open corner", at_a.1, at_b.1, at_a.0)));
        }
        if !restored {
            graph.rewire(a, b);
        }
        done.push(Reconnection { a: at_a, b: at_b, corner: restored });
    }
    Ok((assemble(contours, &involved, graph.trace()), done))
}

/// Pairs of keys, nearest first; ties go to the pair that comes first in `points`.
fn pairs<K: Copy>(points: &[(K, Point)]) -> Vec<(K, K)> {
    let mut candidates: Vec<(f64, usize, usize)> = Vec::new();
    for (i, (_, p)) in points.iter().enumerate() {
        for (k, (_, q)) in points.iter().enumerate().skip(i + 1) {
            candidates.push((p.distance(*q), i, k));
        }
    }
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let mut used = vec![false; points.len()];
    let mut out = Vec::with_capacity(points.len() / 2);
    for (_, i, k) in candidates {
        if used.get(i).copied().unwrap_or(true) || used.get(k).copied().unwrap_or(true) {
            continue;
        }
        if let (Some(&(a, _)), Some(&(b, _))) = (points.get(i), points.get(k)) {
            out.push((a, b));
        }
        for x in [i, k] {
            if let Some(u) = used.get_mut(x) {
                *u = true;
            }
        }
    }
    out
}

/// Restores the open corner that `a` and `b` make, if they make one.
fn restore_corner(graph: &mut Graph, a: usize, b: usize, grid: f64) -> bool {
    let succ = |id: usize| graph.get(id).and_then(|g| g.succ);
    let (n1, n2) = if succ(a) == Some(b) {
        (a, b)
    } else if succ(b) == Some(a) {
        (b, a)
    } else {
        return false;
    };
    let (Some(g1), Some(g2)) = (graph.get(n1), graph.get(n2)) else { return false };
    if !g2.r.ctrl.is_empty() {
        return false;
    }
    let (p1, p2) = (g1.r.node.point, g2.r.node.point);
    let (Some(tin), Some(tout)) = (graph.tangent_in(n1), graph.tangent_out(n2)) else { return false };
    let cross = tin.cross(tout);
    if cross.abs() < MIN_CORNER_DEGREES.to_radians().sin() {
        return false;
    }
    // p1 − s·tin = p2 + u·tout, with s and u positive: the corner lies behind p1 on the incoming
    // direction and ahead of p2 on the outgoing direction.
    let d = p1 - p2;
    let s = d.cross(tout) / cross;
    let u = tin.cross(d) / cross;
    if !(s > 0.0 && u > 0.0) {
        return false;
    }
    let corner = round_to_grid(p1 - tin * s, grid);
    if !(corner.x.abs() <= MAX_COORDINATE && corner.y.abs() <= MAX_COORDINATE) {
        return false;
    }
    if let Some(g) = graph.get_mut(n1) {
        g.r.node.point = corner;
        g.r.node.smooth = false;
    }
    if let Some(g) = graph.get_mut(n2) {
        g.r.mark = false;
    }
    graph.drop_next(n1).is_some()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    use crate::coverage_difference;
    use crate::fixtures::rect;
    use fontcraft_geom::{BezPath, Shape};

    fn bez(cs: &[Contour]) -> Vec<BezPath> {
        cs.iter().map(|c| c.to_bez().unwrap()).collect()
    }

    #[test]
    fn a_corner_opens_and_reconnects() {
        let square = rect(0.0, 0.0, 100.0, 100.0).tagged();
        let opened = open_corners(&square, &[1], Vec2::new(10.0, 20.0), 1.0).unwrap();
        let c = &opened.contour;
        assert_eq!(c.nodes.len(), 5);
        // The bottom edge goes on to the right by 10, the right edge starts 20 below the corner.
        assert_eq!(c.nodes[1].point, Point::new(110.0, 0.0));
        assert_eq!(c.nodes[1].tag, Some(1));
        assert_eq!(c.nodes[2].point, Point::new(100.0, -20.0));
        assert_eq!(opened.nodes, vec![1, 2]);
        // Smooth nodes stay, and so does the square's ink (the triangle outside it is extra).
        let mut smooth = square.clone();
        smooth.nodes[1].smooth = true;
        assert_eq!(open_corners(&smooth, &[1], Vec2::new(10.0, 20.0), 1.0).unwrap().contour, smooth);

        let back = reconnect_nodes(std::slice::from_ref(c), &[(0, 1), (0, 2)], 1.0).unwrap();
        assert_eq!(back.contours, vec![square]);
        assert_eq!(back.selection, vec![(0, 1)]);
    }

    #[test]
    fn a_curved_corner_opens_along_its_handles() {
        let path = BezPath::from_svg("M0,0 L100,0 C100,60 60,100 0,100 Z").unwrap();
        let c = crate::contour::contours_from_bez(&path).remove(0);
        // Node 1 is the corner between the bottom line and the curve, which leaves it going up.
        let opened = open_corners(&c, &[1], Vec2::new(10.0, 10.0), 0.0).unwrap();
        assert_eq!(opened.contour.nodes[1].point, Point::new(110.0, 0.0));
        assert_eq!(opened.contour.nodes[2].point, Point::new(100.0, -10.0));
        // The curve's first handle did not move, so it still leaves straight up.
        assert_eq!(opened.contour.nodes[3].point, Point::new(100.0, 60.0));
        let back = reconnect_nodes(&[opened.contour], &[(0, 1), (0, 2)], 0.0).unwrap();
        assert_eq!(back.contours, vec![c]);
    }

    #[test]
    fn reconnecting_two_squares_joins_them_into_one_contour() {
        // Nodes 1 of each square sit near each other; rewiring them joins the outlines.
        let a = rect(0.0, 0.0, 100.0, 100.0);
        let b = rect(90.0, -10.0, 200.0, 100.0);
        let input = [a, b];
        let out = reconnect_nodes(&input, &[(0, 1), (1, 0)], 1.0).unwrap();
        assert_eq!(out.contours.len(), 1);
        assert_eq!(out.sources, vec![0]);
        assert!(out.contours[0].closed);
        assert_eq!(out.contours[0].nodes.len(), 10);
        let area: f64 = out.contours.iter().map(|c| c.to_bez().unwrap().area()).sum();
        let before: f64 = input.iter().map(|c| c.to_bez().unwrap().area()).sum();
        assert!((area - before).abs() < 1e-6);
        assert_eq!(out.selection.len(), 4);
        // Rewiring the same pair again splits them apart.
        let sel = out.selection.clone();
        let again = reconnect_nodes(&out.contours, &[sel[0], sel[2]], 1.0);
        assert!(again.is_ok());
        let d = coverage_difference(&bez(&input), &bez(&out.contours)).unwrap();
        assert!(d.max_pixel < 1e-9, "{d:?}");
    }

    #[test]
    fn reconnect_refuses_odd_and_off_curve_selections() {
        let a = rect(0.0, 0.0, 100.0, 100.0);
        assert!(matches!(reconnect_nodes(std::slice::from_ref(&a), &[(0, 1)], 1.0), Err(PathOpError::Invalid(_))));
        assert!(matches!(reconnect_nodes(std::slice::from_ref(&a), &[(0, 1), (0, 9)], 1.0), Err(PathOpError::Invalid(_))));
        assert!(matches!(open_corners(&a, &[1], Vec2::new(f64::NAN, 1.0), 1.0), Err(PathOpError::NotFinite)));
    }
}
