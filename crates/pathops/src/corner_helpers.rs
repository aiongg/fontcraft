//! Corner and point helpers from the secondary reference: ink traps, loop corners, Replace with
//! Corner, Align Points to Background, Collapse Points and Same Width / Same Height
//! (`docs/spec/90-secondary-references.md`). Every function works on one contour or on plain
//! points and returns new values; the caller decides what to keep.

use std::collections::{HashMap, HashSet};

use fontcraft_geom::{Point, Rect, Vec2};

use crate::contour::{Contour, Node, NodeKind, RNode, Ring, round_to_grid};
use crate::{MAX_COORDINATE, PathOpError, open_corners};

/// Largest ink trap width, in units.
pub const MAX_INK_TRAP_WIDTH: f64 = 10_000.0;
/// Largest loop size (how far each segment goes past the corner), in units.
pub const MAX_LOOP_SIZE: f64 = 10_000.0;
/// Range of Align Points to Background's tolerance, in units.
pub const MIN_ALIGN_TOLERANCE: f64 = 0.01;
pub const MAX_ALIGN_TOLERANCE: f64 = 10_000.0;
/// Most point-to-point distances one alignment may measure. Points spread over the grid of
/// tolerance-sized cells need few; many points in one cell are refused past this.
pub const MAX_ALIGN_COMPARISONS: usize = 50_000_000;
/// Replace with Corner refuses neighbours whose directions are closer than this many degrees
/// (or this close to opposite): their extensions meet far away or not at all.
pub const MIN_REPLACE_DEGREES: f64 = 5.0;
/// Largest factor Same Width / Same Height applies to one path.
pub const MAX_SCALE_FACTOR: f64 = 1_000.0;
/// A corner whose two directions are closer than this many degrees (or than this to opposite)
/// takes no ink trap.
const MIN_CORNER_DEGREES: f64 = 1.0;

/// A contour after an edit, with the nodes the caller should select (indices into its list).
#[derive(Clone, Debug, PartialEq)]
pub struct Changed {
    pub contour: Contour,
    pub nodes: Vec<usize>,
}

/// Which extent Same Width / Same Height matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dimension {
    Width,
    Height,
}

fn in_range(p: Point) -> bool {
    p.x.is_finite() && p.y.is_finite() && p.x.abs() <= MAX_COORDINATE && p.y.abs() <= MAX_COORDINATE
}

/// Refuses a contour with a node beyond the coordinate limit.
fn check_range(c: &Contour) -> Result<(), PathOpError> {
    match c.nodes.iter().find(|n| !in_range(n.point)) {
        Some(n) => Err(PathOpError::OutOfRange(if n.point.x.abs() > n.point.y.abs() { n.point.x } else { n.point.y })),
        None => Ok(()),
    }
}

fn marked(ring: &Ring) -> Vec<usize> {
    (0..ring.len()).filter(|&k| ring.nodes.get(k).is_some_and(|r| r.mark)).map(|k| ring.list_index(k)).collect()
}

// ── Ink traps ───────────────────────────────────────────────────────────────────────────────────

/// Make Ink Trap on the corner nodes `nodes` (indices into `contour.nodes`). Each becomes two
/// nodes `width` apart: the copy that ends the incoming segment moves half the width towards
/// the incoming side, perpendicular to the corner's bisector, and the other copy moves half the
/// width the other way and starts the outgoing segment; a straight segment joins them across the
/// corner. At an inside corner this cuts a flat-bottomed notch into the ink. The handle next to
/// each copy moves with it, so the segments keep their directions at the copies. Smooth nodes,
/// off-curve nodes, the ends of an open contour and nearly straight corners are left alone. The
/// copies are rounded to `grid`.
pub fn make_ink_traps(contour: &Contour, nodes: &[usize], width: f64, grid: f64) -> Result<Changed, PathOpError> {
    if !width.is_finite() {
        return Err(PathOpError::NotFinite);
    }
    if width <= 0.0 || width > MAX_INK_TRAP_WIDTH {
        return Err(PathOpError::Invalid(format!("the ink trap width ({width}) must be above 0 and at most {MAX_INK_TRAP_WIDTH} units")));
    }
    let ring = Ring::from_contour(contour)?;
    let wanted: HashSet<usize> = nodes.iter().copied().collect();
    let traps: Vec<Option<(Point, Point)>> = ring
        .nodes
        .iter()
        .enumerate()
        .map(|(j, r)| if r.src.is_some_and(|s| wanted.contains(&s)) && !r.node.smooth { trap_pair(&ring, j, width, grid) } else { None })
        .collect();
    let trap = |j: usize| traps.get(j).copied().flatten();
    let mut out = Vec::with_capacity(ring.len() + traps.iter().flatten().count());
    for (j, r) in ring.nodes.iter().enumerate() {
        let mut ctrl = r.ctrl.clone();
        if let Some((a, _)) = trap(j)
            && let Some(last) = ctrl.last_mut()
        {
            last.point += a - r.node.point;
        }
        if let Some(i) = ring.prev(j)
            && let (Some((_, b)), Some(prev)) = (trap(i), ring.nodes.get(i))
            && let Some(first) = ctrl.first_mut()
        {
            first.point += b - prev.node.point;
        }
        match trap(j) {
            Some((a, b)) => {
                out.push(RNode { node: Node { point: a, smooth: false, ..r.node }, ctrl, src: r.src, mark: true });
                out.push(RNode { node: Node::new(b, NodeKind::Line), ctrl: Vec::new(), src: None, mark: true });
            }
            None => out.push(RNode { ctrl, mark: false, ..r.clone() }),
        }
    }
    let ring = Ring { closed: ring.closed, nodes: out };
    let contour = ring.to_contour();
    check_range(&contour)?;
    Ok(Changed { nodes: marked(&ring), contour })
}

/// The two copies of corner node `j`: the one that ends the incoming segment, then the other.
fn trap_pair(ring: &Ring, j: usize, width: f64, grid: f64) -> Option<(Point, Point)> {
    let tin = ring.tangent_in(j)?;
    let tout = ring.tangent_out(j)?;
    if tin.cross(tout).abs() < MIN_CORNER_DEGREES.to_radians().sin() {
        return None;
    }
    // Back along the incoming segment minus forward along the outgoing one: perpendicular to
    // the bisector of the two, on the incoming segment's side.
    let side: Vec2 = (-tin - tout).normalize();
    let c = ring.nodes.get(j)?.node.point;
    let a = round_to_grid(c + side * (width / 2.0), grid);
    let b = round_to_grid(c - side * (width / 2.0), grid);
    (in_range(a) && in_range(b) && a != b).then_some((a, b))
}

// ── Loop corners ────────────────────────────────────────────────────────────────────────────────

/// Loop Corner on the corner nodes `nodes`: each becomes two nodes whose segments go on `size`
/// units past the corner along their own directions and cross in a small loop (the geometry of
/// [`open_corners`] with the same reach in every direction). Smooth nodes, the ends of an open
/// contour and nearly straight corners are left alone.
pub fn loop_corners(contour: &Contour, nodes: &[usize], size: f64, grid: f64) -> Result<Changed, PathOpError> {
    if !size.is_finite() {
        return Err(PathOpError::NotFinite);
    }
    if size <= 0.0 || size > MAX_LOOP_SIZE {
        return Err(PathOpError::Invalid(format!("the loop size ({size}) must be above 0 and at most {MAX_LOOP_SIZE} units")));
    }
    let opened = open_corners(contour, nodes, Vec2::new(size, size), grid)?;
    check_range(&opened.contour)?;
    Ok(Changed { contour: opened.contour, nodes: opened.nodes })
}

// ── Replace with Corner ─────────────────────────────────────────────────────────────────────────

/// Replace with Corner: removes the segment that ends at node `segment` (an index into
/// `contour.nodes`), or with `None` the gap between the last and first nodes of an open contour,
/// and extends the two neighbouring segments along their end directions until they meet. The
/// meeting point, rounded to `grid`, becomes one corner node; the gap's contour becomes closed.
/// Refused when a neighbour is missing, when the two directions are within
/// [`MIN_REPLACE_DEGREES`] of parallel, or when the directions meet behind the segments.
pub fn replace_with_corner(contour: &Contour, segment: Option<usize>, grid: f64) -> Result<Changed, PathOpError> {
    let bad = |m: &str| PathOpError::Invalid(m.to_string());
    let ring = Ring::from_contour(contour)?;
    let m = ring.len();
    if m < 3 {
        return Err(bad("the path has too few nodes to keep a corner"));
    }
    let (a, b) = match segment {
        None if ring.closed => return Err(bad("a closed path has no gap between its ends")),
        None => (m - 1, 0),
        Some(end) => {
            let j = ring.nodes.iter().position(|r| r.src == Some(end)).ok_or_else(|| bad("the node does not end a segment"))?;
            (ring.prev(j).ok_or_else(|| bad("the node does not end a segment"))?, j)
        }
    };
    let din = ring.tangent_in(a).ok_or_else(|| bad("no segment comes before the replaced one"))?;
    let dout = ring.tangent_out(b).ok_or_else(|| bad("no segment comes after the replaced one"))?;
    let (pa, pb) = match (ring.nodes.get(a), ring.nodes.get(b)) {
        (Some(x), Some(y)) => (x.node.point, y.node.point),
        _ => return Err(bad("the segment does not resolve")),
    };
    let cross = din.cross(dout);
    if cross.abs() < MIN_REPLACE_DEGREES.to_radians().sin() {
        return Err(bad("the neighbouring segments are nearly parallel: their extensions do not meet"));
    }
    // pa + s·din = pb − t·dout, both s and t ahead of the segments.
    let d = pb - pa;
    let s = d.cross(dout) / cross;
    let t = din.cross(d) / cross;
    if s < -1e-9 || t < -1e-9 {
        return Err(bad("the extensions of the neighbouring segments meet behind them"));
    }
    let x = round_to_grid(pa + din * s, grid);
    if !in_range(x) {
        return Err(PathOpError::OutOfRange(if x.x.abs() > x.y.abs() { x.x } else { x.y }));
    }
    let corner = |r: &RNode| RNode { node: Node { point: x, smooth: false, ..r.node }, ctrl: r.ctrl.clone(), src: r.src, mark: true };
    let nodes: Vec<RNode> = ring
        .nodes
        .iter()
        .enumerate()
        .filter(|&(k, _)| k != b)
        .map(|(k, r)| if k == a { corner(r) } else { RNode { mark: false, ..r.clone() } })
        .collect();
    // An open contour's gap: its first node goes (merged into the last), and the segment that
    // left it now leaves the corner, which closes the contour.
    let ring = Ring { closed: segment.is_none() || ring.closed, nodes };
    let contour = ring.to_contour();
    check_range(&contour)?;
    Ok(Changed { nodes: marked(&ring), contour })
}

// ── Align Points to Background ──────────────────────────────────────────────────────────────────

/// For each of `points`, the nearest of `targets` within `tolerance` units (the first one found
/// at equal distance), or `None` when none is that close.
pub fn nearest_within(points: &[Point], targets: &[Point], tolerance: f64) -> Result<Vec<Option<Point>>, PathOpError> {
    if !tolerance.is_finite() {
        return Err(PathOpError::NotFinite);
    }
    if !(MIN_ALIGN_TOLERANCE..=MAX_ALIGN_TOLERANCE).contains(&tolerance) {
        return Err(PathOpError::Invalid(format!(
            "the tolerance ({tolerance}) must be between {MIN_ALIGN_TOLERANCE} and {MAX_ALIGN_TOLERANCE} units"
        )));
    }
    // Cells as large as the tolerance: a target within reach lies in one of the nine cells
    // around a point's own.
    let cell = |p: Point| ((p.x / tolerance).floor() as i64, (p.y / tolerance).floor() as i64);
    let mut grid: HashMap<(i64, i64), Vec<Point>> = HashMap::new();
    for &t in targets.iter().filter(|t| in_range(**t)) {
        grid.entry(cell(t)).or_default().push(t);
    }
    let mut work = 0usize;
    let mut out = Vec::with_capacity(points.len());
    for &p in points {
        if !in_range(p) {
            out.push(None);
            continue;
        }
        let (cx, cy) = cell(p);
        let mut best: Option<(f64, Point)> = None;
        for dx in -1..=1 {
            for dy in -1..=1 {
                let Some(list) = grid.get(&(cx.saturating_add(dx), cy.saturating_add(dy))) else { continue };
                work = work.saturating_add(list.len());
                if work > MAX_ALIGN_COMPARISONS {
                    return Err(PathOpError::Invalid(format!(
                        "too many points lie close together: aligning them would take more than {MAX_ALIGN_COMPARISONS} comparisons"
                    )));
                }
                for &t in list {
                    let dist = p.distance(t);
                    if dist <= tolerance && best.is_none_or(|(d, _)| dist < d) {
                        best = Some((dist, t));
                    }
                }
            }
        }
        out.push(best.map(|(_, t)| t));
    }
    Ok(out)
}

// ── Collapse Points ─────────────────────────────────────────────────────────────────────────────

/// Collapse Points: moves the on-curve nodes `nodes` (`(contour, node)` addresses into
/// `contours`) to the centre of their bounding box, rounded to `grid`, and moves the off-curve
/// nodes next to each of them onto it, so its handles have zero length. Addresses that do not
/// name an on-curve node are ignored; at least one must. Returns the centre.
pub fn collapse_points(contours: &mut [Contour], nodes: &[(usize, usize)], grid: f64) -> Result<Point, PathOpError> {
    let on: Vec<(usize, usize, Point)> = nodes
        .iter()
        .filter_map(|&(c, n)| contours.get(c).and_then(|c| c.nodes.get(n)).filter(|n| n.on_curve()).map(|node| (c, n, node.point)))
        .collect();
    let Some(&(_, _, first)) = on.first() else {
        return Err(PathOpError::Invalid("no on-curve node to collapse".into()));
    };
    let bounds = on.iter().fold(Rect::from_points(first, first), |r, &(_, _, p)| r.union_pt(p));
    let centre = round_to_grid(bounds.center(), grid);
    if !in_range(centre) {
        return Err(PathOpError::OutOfRange(centre.x));
    }
    for (c, n, _) in on {
        let Some(contour) = contours.get_mut(c) else { continue };
        let len = contour.nodes.len();
        let prev = if n > 0 {
            Some(n - 1)
        } else if contour.closed && len > 1 {
            Some(len - 1)
        } else {
            None
        };
        let next = if n + 1 < len {
            Some(n + 1)
        } else if contour.closed && len > 1 {
            Some(0)
        } else {
            None
        };
        if let Some(node) = contour.nodes.get_mut(n) {
            node.point = centre;
        }
        for k in [prev, next].into_iter().flatten() {
            if let Some(node) = contour.nodes.get_mut(k).filter(|node| !node.on_curve()) {
                node.point = centre;
            }
        }
    }
    Ok(centre)
}

// ── Same Width / Same Height ────────────────────────────────────────────────────────────────────

/// The bounding box of a contour's outline (curve extremes included), `None` for an empty one.
pub fn contour_bounds(contour: &Contour) -> Result<Option<Rect>, PathOpError> {
    Ok(Ring::from_contour(contour)?.bounds())
}

/// The extent of `bounds` along `dimension`.
pub fn extent(bounds: Rect, dimension: Dimension) -> f64 {
    match dimension {
        Dimension::Width => bounds.width(),
        Dimension::Height => bounds.height(),
    }
}

/// Scales `contour` about the centre of its bounding box so that its extent along `dimension`
/// becomes `target`: on both axes when `proportional`, on that axis only otherwise. Nodes are
/// rounded to `grid`. Refused for a contour without that extent, a target that is not above 0,
/// and a factor beyond [`MAX_SCALE_FACTOR`].
pub fn scale_to_size(contour: &Contour, dimension: Dimension, target: f64, proportional: bool, grid: f64) -> Result<Contour, PathOpError> {
    if !target.is_finite() {
        return Err(PathOpError::NotFinite);
    }
    if target <= 0.0 || target > 2.0 * MAX_COORDINATE {
        return Err(PathOpError::Invalid(format!("the target size ({target}) must be above 0")));
    }
    let bounds = contour_bounds(contour)?.ok_or_else(|| PathOpError::Invalid("the path is empty".into()))?;
    let current = extent(bounds, dimension);
    let what = match dimension {
        Dimension::Width => "width",
        Dimension::Height => "height",
    };
    if current <= 1e-9 {
        return Err(PathOpError::Invalid(format!("the path has no {what} to scale")));
    }
    let f = target / current;
    if !f.is_finite() || !(1.0 / MAX_SCALE_FACTOR..=MAX_SCALE_FACTOR).contains(&f) {
        return Err(PathOpError::Invalid(format!(
            "scaling the path's {what} from {current} to {target} units goes beyond a factor of {MAX_SCALE_FACTOR}"
        )));
    }
    let (fx, fy) = match (dimension, proportional) {
        (_, true) => (f, f),
        (Dimension::Width, false) => (f, 1.0),
        (Dimension::Height, false) => (1.0, f),
    };
    let c = bounds.center();
    let mut out = contour.clone();
    for n in &mut out.nodes {
        n.point = round_to_grid(Point::new(c.x + (n.point.x - c.x) * fx, c.y + (n.point.y - c.y) * fy), grid);
    }
    check_range(&out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::rect;

    fn pts(c: &Contour) -> Vec<(f64, f64, NodeKind)> {
        c.nodes.iter().map(|n| (n.point.x, n.point.y, n.kind)).collect()
    }

    /// An L: the inside corner is node 3 at (100, 100).
    fn ell() -> Contour {
        let n = |x: f64, y: f64| Node::new(Point::new(x, y), NodeKind::Line);
        Contour::new(true, vec![n(0.0, 0.0), n(300.0, 0.0), n(300.0, 100.0), n(100.0, 100.0), n(100.0, 400.0), n(0.0, 400.0)])
    }

    #[test]
    fn an_ink_trap_splits_the_inside_corner_into_a_notch_of_the_given_width() {
        let out = make_ink_traps(&ell(), &[3], 20.0, 0.0).unwrap();
        let p = pts(&out.contour);
        assert_eq!(p.len(), 7);
        let (a, b) = (Point::new(p[3].0, p[3].1), Point::new(p[4].0, p[4].1));
        assert!((a.distance(b) - 20.0).abs() < 1e-9, "{a:?} {b:?}");
        // Incoming from (300, 100) going left: its copy moves right and down, into the ink.
        assert!(a.x > 100.0 && a.y < 100.0, "{a:?}");
        assert!(b.x < 100.0 && b.y > 100.0, "{b:?}");
        assert_eq!(out.nodes, vec![3, 4]);
        // A straight run is not a corner.
        assert_eq!(make_ink_traps(&rect(0.0, 0.0, 10.0, 10.0), &[], 20.0, 0.0).unwrap().contour, rect(0.0, 0.0, 10.0, 10.0));
        assert!(make_ink_traps(&ell(), &[3], 0.0, 0.0).is_err());
    }

    #[test]
    fn replacing_a_chamfer_restores_the_sharp_corner() {
        let n = |x: f64, y: f64| Node::new(Point::new(x, y), NodeKind::Line);
        // A square with its top right corner cut off by the segment ending at node 3.
        let c = Contour::new(true, vec![n(0.0, 0.0), n(100.0, 0.0), n(100.0, 80.0), n(80.0, 100.0), n(0.0, 100.0)]);
        let out = replace_with_corner(&c, Some(3), 1.0).unwrap();
        assert_eq!(out.contour, rect(0.0, 0.0, 100.0, 100.0));
        assert_eq!(out.nodes, vec![2]);
        // An open path closes: its last segment and its first meet at (0, 100).
        let open = Contour::new(false, vec![n(0.0, 60.0), n(0.0, 0.0), n(100.0, 0.0), n(60.0, 40.0)]);
        let out = replace_with_corner(&open, None, 1.0).unwrap();
        assert_eq!(out.contour, Contour::new(true, vec![n(0.0, 0.0), n(100.0, 0.0), n(0.0, 100.0)]));
        assert_eq!(out.nodes, vec![2]);
        // The arms of a U are parallel.
        let u = Contour::new(false, vec![n(0.0, 50.0), n(0.0, 0.0), n(100.0, 0.0), n(100.0, 50.0)]);
        assert!(replace_with_corner(&u, None, 1.0).is_err());
        // Parallel neighbours never meet.
        let slot = Contour::new(true, vec![n(0.0, 0.0), n(100.0, 0.0), n(100.0, 10.0), n(0.0, 10.0)]);
        assert!(replace_with_corner(&slot, Some(2), 1.0).is_err());
    }

    #[test]
    fn nearest_within_takes_the_closest_target_in_reach() {
        let got = nearest_within(&[Point::new(0.0, 0.0), Point::new(500.0, 0.0)], &[Point::new(30.0, 0.0), Point::new(-10.0, 5.0)], 50.0).unwrap();
        assert_eq!(got, vec![Some(Point::new(-10.0, 5.0)), None]);
    }

    #[test]
    fn collapsing_moves_nodes_to_their_centre_and_retracts_their_handles() {
        let mut c = vec![Contour::new(
            false,
            vec![
                Node::new(Point::new(0.0, 0.0), NodeKind::Line),
                Node::off(Point::new(10.0, 40.0)),
                Node::off(Point::new(90.0, 40.0)),
                Node::new(Point::new(100.0, 0.0), NodeKind::Curve),
            ],
        )];
        let centre = collapse_points(&mut c, &[(0, 0), (0, 3)], 1.0).unwrap();
        assert_eq!(centre, Point::new(50.0, 0.0));
        assert!(c[0].nodes.iter().all(|n| n.point == centre));
        assert!(collapse_points(&mut c, &[(0, 1)], 1.0).is_err());
    }

    #[test]
    fn scaling_to_a_width_keeps_the_centre() {
        let out = scale_to_size(&rect(0.0, 0.0, 100.0, 50.0), Dimension::Width, 200.0, true, 0.0).unwrap();
        assert_eq!(out, rect(-50.0, -25.0, 150.0, 75.0));
        let out = scale_to_size(&rect(0.0, 0.0, 100.0, 50.0), Dimension::Width, 200.0, false, 0.0).unwrap();
        assert_eq!(out, rect(-50.0, 0.0, 150.0, 50.0));
    }
}
