//! Extra nodes: the places where segments of a layer's paths cross, away from their nodes. View
//! › Show Nodes › Extra Nodes draws them; the TrueType Instructor always draws them and selects
//! them as points.
//!
//! An extra node is named by the two segments that cross there, each as the node it ends at (a
//! segment's address, decision 0004): `[shape, node, shape, node]` in a
//! [`Selection`](crate::Selection), and `[path, node, path, node]`, paths counted without
//! components, for the hinting commands. Two segments that cross more than once name one extra
//! node: the crossing nearest to where their chords cross, which is the place a static export
//! with overlap removal moves an instruction on it to (`compile/src/points.rs`).
//!
//! [`crossing_points`] lists every crossing as a place, for drawing: two segments that cross
//! twice give two places there.

use std::collections::{BTreeMap, HashSet};

use fontcraft_geom::{Piece, Point};
use fontcraft_model::{Layer, Shape};
use fontcraft_pathops::geometry_helpers::{MAX_CROSSING_SEGMENTS, MAX_CROSSING_WORK, segment_crossing_pairs_within};

use crate::geometry::dist;
use crate::segment::{self, Segment};

/// How far, in font units, a crossing must lie from the end nodes of both its segments to be an
/// extra node (the tolerance `cornerComponent.add` uses for extra nodes).
pub const NODE_DISTANCE: f64 = 1.0;
/// Most extra nodes listed for one layer; past it, the rest are left out.
pub const MAX_EXTRA_NODES: usize = 4_096;
/// Most work one [`crossing_points`] call does, in the units of [`MAX_CROSSING_WORK`]. The canvas
/// asks for them at every revision of the edited layer, so the budget is a tenth of a command's.
pub const MAX_CROSSING_POINT_WORK: usize = MAX_CROSSING_WORK / 10;
/// Two crossings closer than this on both axes, in font units, are drawn as one place.
const SAME_PLACE: f64 = 0.01;

/// One extra node.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExtraNode {
    /// The crossing segments as `(shape, end node)`, `a` the smaller.
    pub a: (usize, usize),
    pub b: (usize, usize),
    /// Where they cross, in font units.
    pub at: Point,
}

impl ExtraNode {
    /// The selection's address: `[shape, node, shape, node]`.
    pub fn key(&self) -> [usize; 4] {
        [self.a.0, self.a.1, self.b.0, self.b.1]
    }
}

/// The extra nodes of `layer`'s paths, ordered by their address. None when the layer has too
/// many segments or crossings to compare.
pub fn extra_nodes(layer: &Layer) -> Vec<ExtraNode> {
    let mut segments = Vec::new();
    for (shape, s) in layer.shapes.iter().enumerate() {
        let Shape::Path(path) = s else { continue };
        segments.extend(segment::segments(shape, path));
        if segments.len() > MAX_CROSSING_SEGMENTS {
            return Vec::new();
        }
    }
    crossings(&segments)
}

/// Every place where `layer`'s paths cross one another or themselves, at least [`NODE_DISTANCE`]
/// from the end nodes of both segments, each place once, in the order found, at most
/// [`MAX_EXTRA_NODES`]: what View › Show Nodes › Extra Nodes marks. Empty when the layer has more
/// segments than can be compared or the comparison runs past [`MAX_CROSSING_POINT_WORK`].
pub fn crossing_points(layer: &Layer) -> Vec<Point> {
    let mut segments = Vec::new();
    for (shape, s) in layer.shapes.iter().enumerate() {
        let Shape::Path(path) = s else { continue };
        segments.extend(segment::segments(shape, path));
        if segments.len() > MAX_CROSSING_SEGMENTS {
            return Vec::new();
        }
    }
    let all: Vec<Vec<Piece>> = segments.iter().map(pieces).collect();
    let mut work = MAX_CROSSING_POINT_WORK;
    let Ok(found) = segment_crossing_pairs_within(&all, &mut work) else { return Vec::new() };
    let mut seen: HashSet<(i64, i64)> = HashSet::new();
    let mut out = Vec::new();
    for c in found {
        let (Some(pa), Some(pb)) = (all.get(c.a), all.get(c.b)) else { continue };
        let (Some(at), Some(a0), Some(a1), Some(b0), Some(b1)) = (eval(pa, c.ta), eval(pa, 0.0), eval(pa, 1.0), eval(pb, 0.0), eval(pb, 1.0)) else {
            continue;
        };
        if !(at.x.is_finite() && at.y.is_finite()) || [a0, a1, b0, b1].iter().any(|e| dist(*e, at) <= NODE_DISTANCE) {
            continue;
        }
        // Saturating casts: a coordinate too large for the key still gives a key.
        #[allow(clippy::cast_possible_truncation)]
        let key = ((at.x / SAME_PLACE).round() as i64, (at.y / SAME_PLACE).round() as i64);
        if seen.insert(key) {
            out.push(at);
            if out.len() >= MAX_EXTRA_NODES {
                break;
            }
        }
    }
    out
}

/// The extra node where the segment ending at node `a.1` of shape `a.0` crosses the one ending
/// at node `b.1` of shape `b.0`, in either order; `None` when they do not cross away from their
/// nodes.
pub fn extra_node_at(layer: &Layer, a: (usize, usize), b: (usize, usize)) -> Option<ExtraNode> {
    let find = |(shape, node): (usize, usize)| -> Option<Segment> {
        let Some(Shape::Path(path)) = layer.shapes.get(shape) else { return None };
        segment::segments(shape, path).into_iter().find(|s| s.end == node)
    };
    let pair = [find(a)?, find(b)?];
    crossings(&pair).into_iter().next()
}

/// [`extra_node_at`] for a point as the hinting commands name it, `[path, node, path, node]`
/// with paths counted without components: where an instruction on an extra node is drawn.
pub fn hint_extra_node_at(layer: &Layer, point: [usize; 4]) -> Option<ExtraNode> {
    let [p1, n1, p2, n2] = point;
    extra_node_at(layer, (path_shape(layer, p1)?, n1), (path_shape(layer, p2)?, n2))
}

/// The shape index of the layer's path number `path`, paths counted without components.
pub fn path_shape(layer: &Layer, path: usize) -> Option<usize> {
    layer.shapes.iter().enumerate().filter(|(_, s)| matches!(s, Shape::Path(_))).nth(path).map(|(i, _)| i)
}

/// The path number, components not counted, of the path at shape index `shape`.
pub fn shape_path(layer: &Layer, shape: usize) -> Option<usize> {
    if !matches!(layer.shapes.get(shape)?, Shape::Path(_)) {
        return None;
    }
    Some(layer.shapes.iter().take(shape).filter(|s| matches!(s, Shape::Path(_))).count())
}

fn pieces(s: &Segment) -> Vec<Piece> {
    s.pieces
        .iter()
        .map(|p| match *p {
            segment::Piece::Line(l) => Piece::Line(l),
            segment::Piece::Quad(q) => Piece::Quad(q),
            segment::Piece::Cubic(c) => Piece::Cubic(c),
        })
        .collect()
}

/// The point at parameter `t` across all of a segment's pieces.
fn eval(pieces: &[Piece], t: f64) -> Option<Point> {
    let n = pieces.len();
    if n == 0 || !t.is_finite() {
        return None;
    }
    let x = t.clamp(0.0, 1.0) * n as f64;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let i = (x.floor() as usize).min(n - 1);
    pieces.get(i).map(|p| p.eval(x - i as f64))
}

/// Where the chords of two segments cross, or halfway between their ends when the chords are
/// parallel (the rule `compile/src/points.rs` follows).
fn chord_crossing((p1, p2): (Point, Point), (p3, p4): (Point, Point)) -> Point {
    let d = (p1.x - p2.x) * (p3.y - p4.y) - (p1.y - p2.y) * (p3.x - p4.x);
    if d.abs() < 1e-9 {
        return p2.midpoint(p4);
    }
    let t = ((p1.x - p3.x) * (p3.y - p4.y) - (p1.y - p3.y) * (p3.x - p4.x)) / d;
    p1.lerp(p2, t)
}

fn crossings(segments: &[Segment]) -> Vec<ExtraNode> {
    let all: Vec<Vec<Piece>> = segments.iter().map(pieces).collect();
    let mut work = MAX_CROSSING_WORK;
    let Ok(found) = segment_crossing_pairs_within(&all, &mut work) else { return Vec::new() };
    // For each pair of segments, the crossing nearest to their chords' crossing and its distance.
    let mut best: BTreeMap<[usize; 4], (f64, Point)> = BTreeMap::new();
    for c in found {
        let (Some(sa), Some(sb), Some(pa), Some(pb)) = (segments.get(c.a), segments.get(c.b), all.get(c.a), all.get(c.b)) else { continue };
        let (Some(at), Some(a0), Some(a1), Some(b0), Some(b1)) = (eval(pa, c.ta), eval(pa, 0.0), eval(pa, 1.0), eval(pb, 0.0), eval(pb, 1.0)) else {
            continue;
        };
        if !(at.x.is_finite() && at.y.is_finite()) || [a0, a1, b0, b1].iter().any(|e| dist(*e, at) <= NODE_DISTANCE) {
            continue;
        }
        let (ka, kb) = ((sa.shape, sa.end), (sb.shape, sb.end));
        let key = if ka <= kb { [ka.0, ka.1, kb.0, kb.1] } else { [kb.0, kb.1, ka.0, ka.1] };
        let off = dist(at, chord_crossing((a0, a1), (b0, b1)));
        let off = if off.is_finite() { off } else { f64::MAX };
        if let Some(b) = best.get_mut(&key) {
            if off < b.0 {
                *b = (off, at);
            }
        } else if best.len() < MAX_EXTRA_NODES {
            best.insert(key, (off, at));
        }
    }
    best.into_iter().map(|([s1, n1, s2, n2], (_, at))| ExtraNode { a: (s1, n1), b: (s2, n2), at }).collect()
}

#[cfg(test)]
mod tests {
    use fontcraft_model::{Component, Node, NodeKind, Path};

    use super::*;

    /// Shape 0: a component; shape 1: a bar from (0, 100) to (300, 200); shape 2: a bar from
    /// (100, 0) to (200, 300). They cross four times.
    fn plus() -> Layer {
        let mut layer = Layer::new("m01", 600.0);
        layer.shapes.push(Shape::Component(Component::new("B", 0.0, 0.0)));
        layer.shapes.push(Shape::Path(Path::rect(0.0, 100.0, 300.0, 200.0)));
        layer.shapes.push(Shape::Path(Path::rect(100.0, 0.0, 200.0, 300.0)));
        layer
    }

    #[test]
    fn crossings_are_named_by_the_segments_that_end_at_their_nodes() {
        let layer = plus();
        let found: Vec<([usize; 4], (f64, f64))> = extra_nodes(&layer).iter().map(|e| (e.key(), (e.at.x.round(), e.at.y.round()))).collect();
        // Bar 1: node 1 ends the bottom edge, node 3 the top. Bar 2: node 2 ends the right
        // edge, node 0 the left one.
        assert_eq!(
            found,
            vec![([1, 1, 2, 0], (100.0, 100.0)), ([1, 1, 2, 2], (200.0, 100.0)), ([1, 3, 2, 0], (100.0, 200.0)), ([1, 3, 2, 2], (200.0, 200.0))]
        );
        let one = extra_node_at(&layer, (2, 2), (1, 1)).map(|e| e.key());
        assert_eq!(one, Some([1, 1, 2, 2]), "either order");
        assert_eq!(hint_extra_node_at(&layer, [0, 1, 1, 2]).map(|e| e.key()), Some([1, 1, 2, 2]), "paths skip the component");
        assert_eq!(extra_node_at(&layer, (1, 1), (1, 2)), None, "neighbouring segments meet at a node");
        assert_eq!((path_shape(&layer, 1), shape_path(&layer, 2), shape_path(&layer, 0)), (Some(2), Some(1), None));
    }

    #[test]
    fn two_segments_that_cross_twice_give_the_crossing_nearest_their_chords_crossing() {
        // An arch over a flat bar: the curve ending at node 0 crosses the bar's top edge twice.
        let mut layer = Layer::new("m01", 600.0);
        let n = Node::new;
        layer.shapes.push(Shape::Path(Path::new(
            true,
            vec![n(400.0, 0.0, NodeKind::Curve), n(0.0, 0.0, NodeKind::Line), n(0.0, 300.0, NodeKind::OffCurve), n(400.0, 300.0, NodeKind::OffCurve)],
        )));
        layer.shapes.push(Shape::Path(Path::rect(-100.0, 100.0, 500.0, 150.0)));
        let top: Vec<ExtraNode> = extra_nodes(&layer).into_iter().filter(|e| e.a == (0, 0) && e.b == (1, 3)).collect();
        assert_eq!(top.len(), 1, "one extra node per pair of segments");
        // The chords (0,0)-(400,0) and (500,150)-(-100,150) are parallel: halfway between the
        // ends (400,0) and (-100,150) is (150,75), nearer the left crossing.
        assert!(top.first().is_some_and(|e| e.at.x < 200.0 && (e.at.y - 150.0).abs() < 0.01), "{top:?}");
        let places = crossing_points(&layer).into_iter().filter(|p| (p.y - 150.0).abs() < 0.01).count();
        assert_eq!(places, 2, "the places to draw keep both crossings");
    }

    #[test]
    fn crossing_points_of_two_overlapping_squares_and_a_plus() {
        let mut squares = Layer::new("m01", 600.0);
        squares.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 200.0, 200.0)));
        squares.shapes.push(Shape::Path(Path::rect(100.0, 100.0, 300.0, 300.0)));
        let mut found: Vec<(f64, f64)> = crossing_points(&squares).iter().map(|p| (p.x.round(), p.y.round())).collect();
        found.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        assert_eq!(found, vec![(100.0, 200.0), (200.0, 100.0)]);
        assert_eq!(crossing_points(&plus()).len(), 4, "the component is no path");
    }

    #[test]
    fn crossing_points_stop_at_their_limits() {
        // 70 long thin bars each way cross 70 × 70 × 4 times: more places than are listed.
        let mut grid = Layer::new("m01", 600.0);
        for i in 0..70 {
            let at = f64::from(i) * 10.0;
            grid.shapes.push(Shape::Path(Path::rect(-10.0, at, 1000.0, at + 2.0)));
            grid.shapes.push(Shape::Path(Path::rect(at, -10.0, at + 2.0, 1000.0)));
        }
        assert_eq!(crossing_points(&grid).len(), MAX_EXTRA_NODES);
        // A path with more segments than can be compared gives none, at once.
        let n = Node::new;
        let zigzag: Vec<Node> = (0..MAX_CROSSING_SEGMENTS + 1).map(|i| n(i as f64, if i % 2 == 0 { 0.0 } else { 100.0 }, NodeKind::Line)).collect();
        let mut long = Layer::new("m01", 600.0);
        long.shapes.push(Shape::Path(Path::new(true, zigzag)));
        assert!(crossing_points(&long).is_empty());
    }
}
