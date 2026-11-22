//! Where a dragged point lands.
//!
//! Priority (`docs/spec/03a` question 28): nodes, then handles, then the background's points,
//! then guides ([`crate::snap_guides`]), then metric lines.
//! A point that finds none of them can still line up on one axis with another node (the magnetic
//! alignment lines). Points that move with the drag are never targets. The snap distance is in
//! screen points; the shell passes its token value. `Ctrl` suspends snapping: the tools then do
//! not call [`snap`].

use std::collections::BTreeSet;

use fontcraft_geom::Point;
use fontcraft_model::NodeKind;
use serde::Serialize;

use crate::geometry::{dist, finite};
use crate::scene::Scene;
use crate::selection::Selection;
use crate::view::View;

/// A node address: shape index, node index.
type NodeRef = (usize, usize);

/// What a point snapped to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SnapKind {
    Node,
    Handle,
    Guide,
    Metric,
    /// Lined up with another node on one axis.
    Align,
}

/// The result of snapping one point.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Snap {
    /// Where the point lands (the input when nothing is near).
    pub point: Point,
    pub kind: Option<SnapKind>,
    /// The path node it snapped onto, when it snapped to one.
    pub node: Option<(usize, usize)>,
    /// Where to draw a target marker.
    pub marker: Option<Point>,
    /// Alignment and metric lines to draw.
    pub lines: Vec<(Point, Point)>,
}

impl Snap {
    fn none(p: Point) -> Self {
        Self { point: p, kind: None, node: None, marker: None, lines: Vec::new() }
    }
}

/// The points and shapes that move with a drag and so cannot be snap targets.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Moving {
    pub points: BTreeSet<(usize, usize)>,
    pub shapes: BTreeSet<usize>,
}

impl Moving {
    /// What dragging `selection` moves: its points, the handles of its on-curve nodes, and every
    /// point of its paths.
    pub fn from_selection(scene: &Scene, selection: &Selection) -> Self {
        let mut points = selection.points.clone();
        for &(s, n) in &selection.points {
            let Some(path) = scene.path(s) else { continue };
            if !path.nodes.get(n).is_some_and(|node| node.on_curve()) {
                continue;
            }
            let count = path.nodes.len();
            let wrap = if path.closed { count.checked_sub(1) } else { None };
            let prev = n.checked_sub(1).or(wrap);
            let next = if n + 1 < count {
                Some(n + 1)
            } else if path.closed {
                Some(0)
            } else {
                None
            };
            for i in [prev, next].into_iter().flatten() {
                if path.nodes.get(i).is_some_and(|node| node.kind == NodeKind::OffCurve) {
                    points.insert((s, i));
                }
            }
        }
        for &s in &selection.shapes {
            if let Some(path) = scene.path(s) {
                points.extend((0..path.nodes.len()).map(|n| (s, n)));
            }
        }
        Self { points, shapes: selection.shapes.clone() }
    }
}

/// Snaps `p` (font units) to what is within `distance` screen points of it.
pub fn snap(scene: &Scene, view: &View, p: Point, moving: &Moving, distance: f64) -> Snap {
    if !finite(p) {
        return Snap::none(p);
    }
    let tol = view.units(distance);
    if tol <= 0.0 {
        return Snap::none(p);
    }

    // Nodes, then handles: the nearest one within the distance.
    for want_on_curve in [true, false] {
        let mut best: Option<(f64, Point, Option<NodeRef>)> = None;
        let mut consider = |q: Point, node: Option<NodeRef>| {
            let d = dist(p, q);
            if d <= tol && best.is_none_or(|b| d < b.0) {
                best = Some((d, q, node));
            }
        };
        for (shape, path) in scene.layer.shapes.iter().enumerate().filter_map(|(i, s)| match s {
            fontcraft_model::Shape::Path(path) => Some((i, path)),
            _ => None,
        }) {
            for (n, node) in path.nodes.iter().enumerate() {
                if node.on_curve() == want_on_curve && !moving.points.contains(&(shape, n)) {
                    consider(node.point(), Some((shape, n)));
                }
            }
        }
        for c in scene.components().iter().filter(|c| !moving.shapes.contains(&c.shape)) {
            let list = if want_on_curve { &c.nodes } else { &c.handles };
            for q in list {
                consider(*q, None);
            }
        }
        if let Some((_, q, node)) = best {
            let kind = if want_on_curve { SnapKind::Node } else { SnapKind::Handle };
            return Snap { point: q, kind: Some(kind), node, marker: Some(q), lines: Vec::new() };
        }
    }

    // The background's points, then guides and their crossings, each only while it is showing.
    let background = scene.background_shown.then(|| crate::snap_guides::background(scene, p, tol)).flatten();
    if let Some(s) = background.or_else(|| scene.guides_shown.then(|| crate::snap_guides::guides(scene, p, tol)).flatten()) {
        return s;
    }

    // Metric lines on y, the sidebearings on x, then alignment with other nodes on each axis
    // that is still free.
    let width = if scene.layer.width.is_finite() { scene.layer.width } else { 0.0 };
    let mut out = Snap::none(p);
    let mut x_snapped = false;
    let mut y_snapped = false;
    if let Some(y) = nearest_value(p.y, scene.metric_lines.iter().copied(), tol) {
        out.point.y = y;
        out.kind = Some(SnapKind::Metric);
        out.lines.push((Point::new(p.x.min(0.0), y), Point::new(p.x.max(width), y)));
        y_snapped = true;
    }
    if let Some(x) = nearest_value(p.x, [0.0, width], tol) {
        out.point.x = x;
        out.kind = Some(SnapKind::Metric);
        x_snapped = true;
    }
    let mut align_x: Option<(f64, Point)> = None;
    let mut align_y: Option<(f64, Point)> = None;
    for (shape, path) in scene.layer.shapes.iter().enumerate().filter_map(|(i, s)| match s {
        fontcraft_model::Shape::Path(path) => Some((i, path)),
        _ => None,
    }) {
        for (n, node) in path.nodes.iter().enumerate() {
            if !node.on_curve() || moving.points.contains(&(shape, n)) {
                continue;
            }
            let q = node.point();
            if !finite(q) {
                continue;
            }
            let dx = (q.x - p.x).abs();
            if !x_snapped && dx <= tol && align_x.is_none_or(|b| dx < b.0) {
                align_x = Some((dx, q));
            }
            let dy = (q.y - p.y).abs();
            if !y_snapped && dy <= tol && align_y.is_none_or(|b| dy < b.0) {
                align_y = Some((dy, q));
            }
        }
    }
    if let Some((_, q)) = align_x {
        out.point.x = q.x;
        out.kind = out.kind.or(Some(SnapKind::Align));
    }
    if let Some((_, q)) = align_y {
        out.point.y = q.y;
        out.kind = out.kind.or(Some(SnapKind::Align));
    }
    if let Some((_, q)) = align_x {
        out.lines.push((q, Point::new(q.x, out.point.y)));
    }
    if let Some((_, q)) = align_y {
        out.lines.push((q, Point::new(out.point.x, q.y)));
    }
    out
}

/// How far, as a multiple of the snap distance, a component's node shows as a snap marker during a drag.
pub const COMPONENT_MARKER_REACH: f64 = 4.0;
/// Most component node markers one drag shows.
pub const MAX_COMPONENT_MARKERS: usize = 64;

/// The on-curve points of the components that do not move with the drag within
/// [`COMPONENT_MARKER_REACH`] snap distances of `p`, nearest first: the markers a drag shows near a
/// component (`snapping.toNodesAndHandles`), at most [`MAX_COMPONENT_MARKERS`].
pub fn component_markers(scene: &Scene, view: &View, p: Point, moving: &Moving, distance: f64) -> Vec<Point> {
    let reach = view.units(distance) * COMPONENT_MARKER_REACH;
    if !finite(p) || !reach.is_finite() || reach <= 0.0 {
        return Vec::new();
    }
    let mut near: Vec<(f64, Point)> = scene
        .components()
        .iter()
        .filter(|c| !moving.shapes.contains(&c.shape))
        .flat_map(|c| c.nodes.iter().copied())
        .filter(|&q| finite(q))
        .map(|q| (dist(p, q), q))
        .filter(|(d, _)| *d <= reach)
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    near.into_iter().take(MAX_COMPONENT_MARKERS).map(|(_, q)| q).collect()
}

fn nearest_value(v: f64, candidates: impl IntoIterator<Item = f64>, tol: f64) -> Option<f64> {
    let mut best: Option<(f64, f64)> = None;
    for c in candidates.into_iter().filter(|c| c.is_finite()) {
        let d = (c - v).abs();
        if d <= tol && best.is_none_or(|b| d < b.0) {
            best = Some((d, c));
        }
    }
    best.map(|b| b.1)
}
