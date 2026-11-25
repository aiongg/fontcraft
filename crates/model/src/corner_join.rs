//! Crossing paths with their overlap removed, for corner components on extra nodes.
//!
//! An extra node is the crossing of two closed paths. A corner on it is fitted on the outline the
//! two paths have together: their segments are cut where they cross or touch, the parts with
//! filled area on one side only are kept, and the kept parts are linked into contours again, so
//! the crossing becomes an ordinary node. Paths that share no corner on an extra node are left
//! alone, and so are the overlaps they have with other paths.
//!
//! The general overlap removal lives above this crate and may not finish on rare inputs, which a
//! drawing that is redone at every frame cannot afford. This one is bounded by [`MAX_JOIN_WORK`]
//! and [`MAX_JOIN_SPLITS`]; where it cannot tell the outline (paths that only run along each
//! other, two contours through one point, a counter crossing the edge of its shape), the paths
//! stay as they are and their corners on extra nodes are left out.

use std::collections::{BTreeSet, HashMap, HashSet};

use fontcraft_geom::crossings::{SAME_T, segment_crossing_pairs_within, subpiece};
use fontcraft_geom::{BezPath, Piece, Point, Segment, Shape as _, Vec2};

use crate::{MAX_OUTLINE_ELEMENTS, Node, NodeKind, Path, Shape, in_range};

/// Most corners on extra nodes of one layer that are fitted.
pub const MAX_EXTRA_CORNERS: usize = 1_024;
/// Most work joining the paths of one layer may cost: one unit per pair of segments or pieces
/// compared, per subdivision step of two curves and per segment a point is tested against.
pub const MAX_JOIN_WORK: usize = 2_000_000;
/// Most places where the paths of one join cross or touch.
pub const MAX_JOIN_SPLITS: usize = 4_096;
/// A crossing this close to a node, in font units, is at the node.
const SNAP: f64 = 0.5;
/// How far, in font units, from a segment its two sides are tested for being filled.
const PROBE: f64 = 0.05;

/// A corner's extra node as its hint gives it: shape and node of one segment's end, then of the
/// other's, in file order.
pub(crate) type ExtraNode = [usize; 4];

/// The two segments' ends, the smaller first.
fn key(extra: ExtraNode) -> ExtraNode {
    let [a, b, c, d] = extra;
    if (a, b) <= (c, d) { extra } else { [c, d, a, b] }
}

/// What [`join`] did to a layer's shapes.
#[derive(Debug, Default)]
pub(crate) struct Joined {
    paths: HashSet<usize>,
    nodes: HashMap<(usize, usize), (usize, usize)>,
    extras: HashMap<ExtraNode, (usize, usize)>,
    lengths: HashMap<usize, usize>,
    /// Shapes left without nodes because the join drew fewer contours than it read paths, to be
    /// removed once nothing points at shapes by index; ascending.
    pub(crate) emptied: Vec<usize>,
}

impl Joined {
    /// Where node `node` of path `shape` is now; `None` when it is no longer on the outline.
    pub(crate) fn node(&self, shape: usize, node: usize) -> Option<(usize, usize)> {
        if !self.paths.contains(&shape) {
            return Some((shape, node));
        }
        let node = node.checked_rem(*self.lengths.get(&shape)?)?;
        self.nodes.get(&(shape, node)).copied()
    }

    /// The node an extra node became.
    pub(crate) fn extra(&self, extra: ExtraNode) -> Option<(usize, usize)> {
        self.extras.get(&key(extra)).copied()
    }
}

/// Joins the paths that cross at `extras`. The closed paths of `shapes` are in file order, the
/// start node last, and so are the joined ones; contours beyond the paths read are added at the
/// end.
pub(crate) fn join(shapes: &mut Vec<Shape>, extras: &[ExtraNode]) -> Joined {
    let mut groups: Vec<(BTreeSet<usize>, Vec<ExtraNode>)> = Vec::new();
    for &extra in extras.iter().take(MAX_EXTRA_CORNERS) {
        let [a, _, b, _] = extra;
        if !(joinable(shapes, a) && joinable(shapes, b)) {
            continue;
        }
        let mut group = (BTreeSet::from([a, b]), vec![key(extra)]);
        let mut kept = Vec::with_capacity(groups.len() + 1);
        for g in groups {
            if g.0.contains(&a) || g.0.contains(&b) {
                group.0.extend(g.0);
                group.1.extend(g.1);
            } else {
                kept.push(g);
            }
        }
        kept.push(group);
        groups = kept;
    }
    let mut out = Joined::default();
    let mut work = MAX_JOIN_WORK;
    for (paths, extras) in groups {
        if let Some(outline) = outline_of(shapes, &paths, &extras, &mut work) {
            place(shapes, &paths, outline, &mut out);
        }
    }
    out.emptied.sort_unstable();
    out
}

/// Whether the shape is a closed path that only fills: a stroke or a mask is not its outline.
fn joinable(shapes: &[Shape], shape: usize) -> bool {
    let Some(Shape::Path(p)) = shapes.get(shape) else { return false };
    let a = &p.info.attrs;
    p.closed
        && a.stroke_width.is_none()
        && a.stroke_height.is_none()
        && a.mask.is_none()
        && p.nodes.iter().all(|n| in_range(n.x) && in_range(n.y))
        && p.nodes.iter().any(Node::on_curve)
}

/// A place on the outline, by the exact coordinates.
type Place = (u64, u64);

fn place_of(p: Point) -> Place {
    // Adding zero makes −0 the same place as 0.
    ((p.x + 0.0).to_bits(), (p.y + 0.0).to_bits())
}

/// Rounds half up, as the corner fitting does.
fn round(p: Point) -> Point {
    Point::new((p.x + 0.5).floor(), (p.y + 0.5).floor())
}

/// The part of a path's segment between two places.
struct Edge {
    shape: usize,
    /// The index of the segment in the group's list.
    segment: usize,
    from: Point,
    to: Point,
    pieces: Vec<Piece>,
    /// The part starts and ends where its segment does.
    whole: bool,
    /// The part ends at its segment's end node.
    at_end: bool,
    /// Filled on the left; kept edges only.
    filled_left: bool,
}

/// The joined contours of one group of paths.
struct Outline {
    contours: Vec<Contour>,
    /// The places of the paths' on-curve nodes, then of the extra nodes.
    nodes: Vec<((usize, usize), Place)>,
    extras: Vec<(ExtraNode, Place)>,
}

struct Contour {
    /// The shape whose attributes it takes.
    shape: usize,
    nodes: Vec<Node>,
    /// Where each edge ends: the place and the node.
    ends: Vec<(Place, usize)>,
}

fn eval(pieces: &[Piece], t: f64) -> Option<Point> {
    let n = pieces.len();
    if n == 0 {
        return None;
    }
    let x = t * n as f64;
    let i = (x.floor().max(0.0) as usize).min(n - 1);
    pieces.get(i).map(|p| p.eval(x - i as f64))
}

fn piece_start(p: &Piece) -> Point {
    match p {
        Piece::Line(l) => l.p0,
        Piece::Quad(q) => q.p0,
        Piece::Cubic(c) => c.p0,
    }
}

fn piece_end(p: &Piece) -> Point {
    match p {
        Piece::Line(l) => l.p1,
        Piece::Quad(q) => q.p2,
        Piece::Cubic(c) => c.p3,
    }
}

/// The part of `pieces` between the segment parameters `t0` and `t1`.
fn part(pieces: &[Piece], t0: f64, t1: f64) -> Vec<Piece> {
    let n = pieces.len() as f64;
    let mut out = Vec::new();
    for (i, p) in pieces.iter().enumerate() {
        let local = |t: f64| (t * n - i as f64).clamp(0.0, 1.0);
        let (a, b) = (local(t0), local(t1));
        if b - a > SAME_T {
            out.push(if a == 0.0 && b == 1.0 { *p } else { subpiece(p, a, b) });
        }
    }
    out
}

/// Where the chords of two segments cross, or halfway between their ends when the chords are
/// parallel: two segments that cross twice name the crossing nearest to it.
fn chord_crossing((p1, p2): (Point, Point), (p3, p4): (Point, Point)) -> Point {
    let d = (p1.x - p2.x) * (p3.y - p4.y) - (p1.y - p2.y) * (p3.x - p4.x);
    if d.abs() < 1e-9 {
        return p2.midpoint(p4);
    }
    let t = ((p1.x - p3.x) * (p3.y - p4.y) - (p1.y - p3.y) * (p3.x - p4.x)) / d;
    p1.lerp(p2, t)
}

/// The outline of the paths `group` together; `None` when none of `extras` is a crossing of them
/// or the outline cannot be told.
fn outline_of(shapes: &[Shape], group: &BTreeSet<usize>, extras: &[ExtraNode], work: &mut usize) -> Option<Outline> {
    let mut segments: Vec<(usize, Segment)> = Vec::new();
    let mut fill = BezPath::new();
    let mut nodes = Vec::new();
    for &shape in group {
        let Some(Shape::Path(p)) = shapes.get(shape) else { return None };
        fill.extend(p.to_bez().ok()?);
        nodes.extend(p.nodes.iter().enumerate().filter(|(_, n)| n.on_curve()).map(|(i, n)| ((shape, i), place_of(n.point()))));
        let mut own = p.segments();
        // The segment that leaves the start node first, so a contour that keeps it starts there.
        own.rotate_right(usize::from(own.len() > 1));
        segments.extend(own.into_iter().map(|s| (shape, s)));
    }
    let pieces: Vec<Vec<Piece>> = segments.iter().map(|(_, s)| s.pieces.clone()).collect();
    let ends: Vec<(Point, Point)> =
        pieces.iter().map(|p| Some((p.first().map(piece_start)?, p.last().map(piece_end)?))).collect::<Option<Vec<_>>>()?;

    // Where the segments are cut, and where the extra nodes are.
    let mut cuts: Vec<Vec<(f64, Point)>> = vec![Vec::new(); segments.len()];
    let mut best: HashMap<ExtraNode, (f64, Point)> = HashMap::new();
    let crossings = segment_crossing_pairs_within(&pieces, work).ok()?;
    if crossings.len() > MAX_JOIN_SPLITS {
        return None;
    }
    for c in &crossings {
        let (pa, pb) = (pieces.get(c.a)?, pieces.get(c.b)?);
        let (&(a0, a1), &(b0, b1)) = (ends.get(c.a)?, ends.get(c.b)?);
        let at = eval(pa, c.ta)?.midpoint(eval(pb, c.tb)?);
        if !at.is_finite() {
            continue;
        }
        let node = [a0, a1, b0, b1].into_iter().filter(|e| e.distance(at) <= SNAP).min_by(|x, y| x.distance(at).total_cmp(&y.distance(at)));
        let place = node.unwrap_or_else(|| round(at));
        for (segment, t, (start, end)) in [(c.a, c.ta, (a0, a1)), (c.b, c.tb, (b0, b1))] {
            let at_own_end = place == start || place == end;
            if !at_own_end && t > SAME_T && t < 1.0 - SAME_T {
                cuts.get_mut(segment)?.push((t, place));
            }
        }
        let (&(sa, ref ea), &(sb, ref eb)) = (segments.get(c.a)?, segments.get(c.b)?);
        let name = key([sa, ea.end, sb, eb.end]);
        if extras.contains(&name) {
            let off = chord_crossing((a0, a1), (b0, b1)).distance(at);
            let off = if off.is_finite() { off } else { f64::MAX };
            if best.get(&name).is_none_or(|b| off < b.0) {
                best.insert(name, (off, place));
            }
        }
    }
    if best.is_empty() {
        return None;
    }

    let edges = kept_edges(&segments, &pieces, &ends, cuts, &fill, work)?;
    let contours = link(shapes, &segments, &edges)?;
    Some(Outline { contours, nodes, extras: best.into_iter().map(|(k, (_, p))| (k, place_of(p))).collect() })
}

/// The parts of the segments between their cuts that have filled area on one side only, each
/// once.
fn kept_edges(
    segments: &[(usize, Segment)],
    pieces: &[Vec<Piece>],
    ends: &[(Point, Point)],
    cuts: Vec<Vec<(f64, Point)>>,
    fill: &BezPath,
    work: &mut usize,
) -> Option<Vec<Edge>> {
    let cost = fill.elements().len();
    let mut edges: Vec<Edge> = Vec::new();
    let mut same: HashMap<(Place, Place), Vec<Point>> = HashMap::new();
    for (i, ((shape, _), mut cuts)) in segments.iter().zip(cuts).enumerate() {
        let (pieces, &(start, end)) = (pieces.get(i)?, ends.get(i)?);
        cuts.sort_by(|a, b| a.0.total_cmp(&b.0));
        cuts.dedup_by(|a, b| a.1 == b.1);
        let mut from = (0.0, start);
        let last = cuts.len();
        for (k, to) in cuts.into_iter().chain([(1.0, end)]).enumerate() {
            let (a, b) = (from, to);
            from = to;
            let part = part(pieces, a.0, b.0);
            // The fill is tested beside the middle of the part, in the direction it runs.
            let (Some(mid), Some(before), Some(after)) = (eval(&part, 0.5), eval(&part, 0.5 - 1e-3), eval(&part, 0.5 + 1e-3)) else { continue };
            let run: Vec2 = after - before;
            if a.1 == b.1 && mid.distance(a.1) <= 2.0 * SNAP {
                continue;
            }
            if !(run.hypot() > 0.0 && run.hypot().is_finite()) {
                return None;
            }
            let side = Vec2::new(-run.y, run.x) * (PROBE / run.hypot());
            *work = work.checked_sub(cost.saturating_mul(2))?;
            let (left, right) = (fill.winding(mid + side) != 0, fill.winding(mid - side) != 0);
            if left == right {
                continue;
            }
            // Parts that lie on each other are drawn once.
            let twins = same.entry((place_of(a.1), place_of(b.1))).or_default();
            if twins.iter().any(|m| m.distance(mid) <= 2.0 * PROBE) {
                continue;
            }
            twins.push(mid);
            edges.push(Edge {
                shape: *shape,
                segment: i,
                from: a.1,
                to: b.1,
                pieces: part,
                whole: k == 0 && last == 0,
                at_end: k == last,
                filled_left: left,
            });
        }
    }
    Some(edges)
}

/// The edges linked into contours; `None` when a place has more or fewer than one edge in and
/// one out.
fn link(shapes: &[Shape], segments: &[(usize, Segment)], edges: &[Edge]) -> Option<Vec<Contour>> {
    let mut next: HashMap<Place, usize> = HashMap::with_capacity(edges.len());
    let mut arrives: HashSet<Place> = HashSet::with_capacity(edges.len());
    for (i, e) in edges.iter().enumerate() {
        if next.insert(place_of(e.from), i).is_some() || !arrives.insert(place_of(e.to)) {
            return None;
        }
    }
    let mut used = vec![false; edges.len()];
    let mut contours = Vec::new();
    let mut total = 0usize;
    for first in 0..edges.len() {
        if used.get(first).copied() != Some(false) {
            continue;
        }
        let mut contour = Contour { shape: edges.get(first)?.shape, nodes: Vec::new(), ends: Vec::new() };
        let mut i = first;
        loop {
            let (e, slot) = (edges.get(i)?, used.get_mut(i)?);
            *slot = true;
            append(shapes, &segments.get(e.segment)?.1, e, &mut contour)?;
            let following = *next.get(&place_of(e.to))?;
            if edges.get(following)?.filled_left != e.filled_left {
                return None;
            }
            if following == first {
                break;
            }
            // One edge in and one out of every place: the walk closes at its first edge.
            if used.get(following).copied() != Some(false) {
                return None;
            }
            i = following;
        }
        total = total.saturating_add(contour.nodes.len());
        if total > MAX_OUTLINE_ELEMENTS {
            return None;
        }
        contours.push(contour);
    }
    (!contours.is_empty()).then_some(contours)
}

/// Adds the nodes of `edge`, all but the one it starts at, to `contour`.
fn append(shapes: &[Shape], segment: &Segment, edge: &Edge, contour: &mut Contour) -> Option<()> {
    let Some(Shape::Path(path)) = shapes.get(edge.shape) else { return None };
    let end = path.nodes.get(segment.end)?;
    if edge.whole {
        for &h in &segment.handles {
            contour.nodes.push(path.nodes.get(h)?.clone());
        }
        contour.nodes.push(end.clone());
    } else {
        let count = edge.pieces.len();
        for (k, piece) in edge.pieces.iter().enumerate() {
            let mut handle = |p: Point| {
                let p = round(p);
                contour.nodes.push(Node::new(p.x, p.y, NodeKind::OffCurve));
            };
            let kind = match piece {
                Piece::Line(_) => NodeKind::Line,
                Piece::Quad(q) => {
                    handle(q.p1);
                    NodeKind::QCurve
                }
                Piece::Cubic(c) => {
                    handle(c.p1);
                    handle(c.p2);
                    NodeKind::Curve
                }
            };
            let last = k + 1 == count;
            let mut node = if last && edge.at_end {
                end.clone()
            } else {
                let at = if last { edge.to } else { round(piece_end(piece)) };
                let mut n = Node::new(at.x, at.y, kind);
                n.smooth = !last && kind == NodeKind::QCurve;
                n
            };
            node.kind = kind;
            contour.nodes.push(node);
        }
    }
    contour.ends.push((place_of(edge.to), contour.nodes.len().checked_sub(1)?));
    Some(())
}

/// Puts the contours of `outline` in the place of the paths `group` and records where the nodes
/// went.
fn place(shapes: &mut Vec<Shape>, group: &BTreeSet<usize>, outline: Outline, out: &mut Joined) {
    let sources: HashMap<usize, Path> =
        group.iter().filter_map(|&i| if let Some(Shape::Path(p)) = shapes.get(i) { Some((i, p.clone())) } else { None }).collect();
    let mut at: HashMap<Place, (usize, usize)> = HashMap::new();
    let mut slots = group.iter().copied();
    for contour in outline.contours {
        let mut path = sources.get(&contour.shape).cloned().unwrap_or_else(|| Path::new(true, Vec::new()));
        path.nodes = contour.nodes;
        let slot = slots.next().unwrap_or(shapes.len());
        match shapes.get_mut(slot) {
            Some(s) => *s = Shape::Path(path),
            None => shapes.push(Shape::Path(path)),
        }
        for (p, node) in contour.ends {
            at.entry(p).or_insert((slot, node));
        }
    }
    for slot in slots {
        if let Some(Shape::Path(p)) = shapes.get_mut(slot) {
            p.nodes.clear();
            out.emptied.push(slot);
        }
    }
    out.lengths.extend(sources.iter().map(|(&i, p)| (i, p.nodes.len())));
    out.paths.extend(group.iter().copied());
    out.nodes.extend(outline.nodes.into_iter().filter_map(|(n, p)| at.get(&p).map(|&to| (n, to))));
    out.extras.extend(outline.extras.into_iter().filter_map(|(x, p)| at.get(&p).map(|&to| (x, to))));
}
