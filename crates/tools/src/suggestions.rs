//! Snapping suggestions (`view.suggestHandles`, `view.suggestLines`, `view.suggestDistance`,
//! `view.suggestStems`): snap targets a drag offers that are not objects of the layer but follow
//! from it.
//!
//! | Switch | While dragging | Targets |
//! |---|---|---|
//! | handles | one handle | its node's horizontal and vertical, the direction of the segment's other handle, and the right angle to that direction |
//! | lines | nodes | the continuations of nearby straight segments and handles, the perpendiculars of diagonal guides, the centrelines between facing straight edges (stems and counters) |
//! | distance | nodes | the outlines parallel to the segments that stay put, at the master's `suggestDistance` |
//! | stems | a straight vertical or horizontal edge | the positions where the edge and a facing edge are one of the declared stems apart |
//!
//! Suggestions rank after the plain snap targets ([`crate::snap`]): a point that snaps to a node,
//! handle, guide or metric line keeps that snap, and one that only lined up with another node, or
//! found nothing, takes the nearest suggestion within the snap distance. A suggestion is reported
//! as [`SnapKind::Align`]. Two suggestion lines that cross near the pointer give their crossing.
//!
//! The shell resolves what the suggestions read from the document (the switches, the stems that
//! apply to the edited glyph, the master's distances) into a [`Suggest`] carried by the scene.
//! The work per pointer event is bounded by [`MAX_SEGMENTS`] and [`MAX_CENTRELINE_EDGES`].

use fontcraft_geom::{Piece, Point, Segment};
use fontcraft_model::{MAX_COORDINATE, Node};
use serde::{Deserialize, Serialize};

use crate::geometry::{bounds, dist, finite, usable};
use crate::scene::Scene;
use crate::selection::Selection;
use crate::snap::{Moving, Snap, SnapKind};
use crate::view::View;

/// Most declared stems a scene keeps.
pub const MAX_DECLARED_STEMS: usize = 256;
/// Most segments one pointer event looks at; the rest of the layer offers nothing.
pub const MAX_SEGMENTS: usize = 20_000;
/// Most straight vertical (and horizontal) edges paired for centrelines.
pub const MAX_CENTRELINE_EDGES: usize = 512;
/// How near the pointer, in screen points, a segment or handle must end for its continuation to
/// be offered.
pub const LINE_REACH: f64 = 200.0;
/// An edge whose ends differ by at most this many units on one axis runs along the other.
const STRAIGHT: f64 = 0.5;
/// Points per segment piece in a drawn parallel outline.
const OUTLINE_STEPS: usize = 16;
/// Guides within this many degrees of a multiple of 90 are not diagonal.
const DIAGONAL_MARGIN: f64 = 0.5;

/// Which suggestions are on. All are off until switched on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Switches {
    pub handles: bool,
    pub lines: bool,
    pub distance: bool,
    pub stems: bool,
}

/// The command ids that flip the switches, in menu order.
pub const SWITCH_COMMANDS: [&str; 4] = ["view.suggestHandles", "view.suggestLines", "view.suggestDistance", "view.suggestStems"];

impl Switches {
    pub fn any(self) -> bool {
        self.handles || self.lines || self.distance || self.stems
    }

    /// The switch that the command `id` flips.
    pub fn get_mut(&mut self, id: &str) -> Option<&mut bool> {
        match id {
            "view.suggestHandles" => Some(&mut self.handles),
            "view.suggestLines" => Some(&mut self.lines),
            "view.suggestDistance" => Some(&mut self.distance),
            "view.suggestStems" => Some(&mut self.stems),
            _ => None,
        }
    }
}

/// One declared stem of the edited glyph's master.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeclaredStem {
    pub width: f64,
    /// Measured vertically (a horizontal stem, such as a crossbar).
    pub horizontal: bool,
}

/// What the suggestions read from the document for the edited layer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Suggest {
    pub on: Switches,
    /// The declared stems that apply to the edited glyph.
    pub stems: Vec<DeclaredStem>,
    /// The master's `suggestDistance`: the distance of a parallel outline from an edge that runs
    /// up and down (`x`) and from one that runs across (`y`).
    pub distance: Option<(f64, f64)>,
}

impl Suggest {
    /// Keeps the stems of a usable width, at most [`MAX_DECLARED_STEMS`], and distances that are
    /// usable lengths with at least one above zero.
    pub fn new(on: Switches, stems: impl IntoIterator<Item = DeclaredStem>, distance: Option<(f64, f64)>) -> Self {
        let length = |v: f64| (0.0..=MAX_COORDINATE).contains(&v);
        let stems = stems.into_iter().filter(|s| length(s.width) && s.width > 0.0).take(MAX_DECLARED_STEMS).collect();
        let distance = distance.filter(|&(x, y)| length(x) && length(y) && (x > 0.0 || y > 0.0));
        Self { on, stems, distance }
    }
}

/// The drag the suggestions refine.
#[derive(Clone, Copy, Debug)]
pub struct Dragged<'a> {
    pub selection: &'a Selection,
    pub moving: &'a Moving,
    /// Where the grabbed point was when the drag began.
    pub grab: Point,
    /// How far the scene already shows the dragged points moved: the shift of the drag's last
    /// preview, which the session applies before the next event arrives.
    pub shown: (f64, f64),
}

/// `snapped`, the plain snap of the grabbed point dragged to `target`, refined by the
/// suggestions that are on. `distance` is the snap distance in screen points.
pub fn refine(scene: &Scene, view: &View, dragged: Dragged<'_>, target: Point, snapped: Snap, distance: f64) -> Snap {
    let kept = matches!(snapped.kind, Some(SnapKind::Node | SnapKind::Handle | SnapKind::Guide | SnapKind::Metric));
    if kept || !scene.suggest.on.any() || !usable(target) || !usable(dragged.grab) {
        return snapped;
    }
    let tol = view.units(distance);
    if tol <= 0.0 {
        return snapped;
    }
    match suggest(scene, view, dragged, target, tol) {
        Some(f) => Snap { point: f.point, kind: Some(SnapKind::Align), node: None, marker: Some(f.point), lines: f.lines },
        None => snapped,
    }
}

/// The parallel outlines Suggest Distance offers near `near` (font units): both sides of every
/// segment that does not move and comes within `reach` units of it, as polylines. Empty while
/// the switch is off or the master sets no distance.
pub fn distance_outlines(scene: &Scene, moving: &Moving, near: Point, reach: f64) -> Vec<Vec<Point>> {
    let Some((dx, dy)) = scene.suggest.distance.filter(|_| scene.suggest.on.distance) else { return Vec::new() };
    if !finite(near) || !reach.is_finite() || reach < 0.0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for seg in still_segments(scene, moving) {
        for piece in &seg.pieces {
            if piece.nearest(near).is_none_or(|(d, _)| d > reach + dx.max(dy)) {
                continue;
            }
            for side in [1.0, -1.0] {
                out.push(offset_polyline(piece, dx, dy, side));
            }
        }
    }
    out
}

/// A suggestion found: where the point goes, how far it moved, and the lines to draw.
struct Found {
    point: Point,
    d: f64,
    lines: Vec<(Point, Point)>,
}

fn better(best: &mut Option<Found>, f: Found) {
    if usable(f.point) && f.d.is_finite() && best.as_ref().is_none_or(|b| f.d < b.d) {
        *best = Some(f);
    }
}

fn suggest(scene: &Scene, view: &View, dragged: Dragged<'_>, target: Point, tol: f64) -> Option<Found> {
    let on = scene.suggest.on;
    if let Some(handle) = single_handle(scene, dragged.selection) {
        return if on.handles { handle_suggestion(scene, handle, target, tol) } else { None };
    }
    if on.stems
        && let Some(f) = stem_suggestion(scene, dragged, target, tol)
    {
        return Some(f);
    }
    if dragged.selection.points.is_empty() {
        return None;
    }
    let mut best = None;
    if on.lines
        && let Some(f) = line_suggestion(scene, view, dragged.moving, target, tol)
    {
        better(&mut best, f);
    }
    if on.distance
        && let Some(f) = distance_suggestion(scene, dragged.moving, target, tol)
    {
        better(&mut best, f);
    }
    best
}

/// The handle being dragged, when the selection is one off-curve point.
fn single_handle(scene: &Scene, selection: &Selection) -> Option<(usize, usize)> {
    if selection.len() != 1 {
        return None;
    }
    let &(s, n) = selection.points.iter().next()?;
    scene.node(s, n).filter(|node| !node.on_curve()).map(|_| (s, n))
}

/// The index before (`forward == false`) or after `i` in a path of `count` nodes.
fn neighbour(count: usize, closed: bool, i: usize, forward: bool) -> Option<usize> {
    let j = if forward {
        if i + 1 < count {
            Some(i + 1)
        } else if closed {
            Some(0)
        } else {
            None
        }
    } else {
        i.checked_sub(1).or_else(|| if closed { count.checked_sub(1) } else { None })
    };
    j.filter(|&j| j != i)
}

/// The handle `(s, n)` held on the line through its node at a right angle, flat, parallel to the
/// segment's other handle, or at a right angle to that handle.
fn handle_suggestion(scene: &Scene, (s, n): (usize, usize), p: Point, tol: f64) -> Option<Found> {
    let path = scene.path(s)?;
    let count = path.nodes.len();
    let on_curve = |i: usize| path.nodes.get(i).is_some_and(Node::on_curve);
    let off_curve = |i: usize| path.nodes.get(i).is_some_and(|node| !node.on_curve());
    // The node the handle belongs to is its on-curve neighbour; the segment's other handle sits
    // on the far side, next to the segment's other node.
    let forward = [false, true].into_iter().find(|&f| neighbour(count, path.closed, n, f).is_some_and(on_curve))?;
    let node = neighbour(count, path.closed, n, forward)?;
    let anchor = path.nodes.get(node)?.point();
    let mut directions = vec![(1.0, 0.0), (0.0, 1.0)];
    let other = neighbour(count, path.closed, n, !forward).filter(|&o| off_curve(o) && o != node);
    if let Some(o) = other
        && let Some(oa) = neighbour(count, path.closed, o, !forward).filter(|&i| on_curve(i))
        && let (Some(h), Some(a)) = (path.nodes.get(o), path.nodes.get(oa))
    {
        let (vx, vy) = (h.x - a.x, h.y - a.y);
        let len = vx.hypot(vy);
        if len.is_finite() && len > 1e-9 {
            let u = (vx / len, vy / len);
            directions.push(u);
            directions.push((-u.1, u.0));
        }
    }
    if !finite(anchor) {
        return None;
    }
    let mut best = None;
    for (ux, uy) in directions {
        let along = (p.x - anchor.x) * ux + (p.y - anchor.y) * uy;
        let foot = Point::new(anchor.x + along * ux, anchor.y + along * uy);
        let d = dist(p, foot);
        if d <= tol {
            better(&mut best, Found { point: foot, d, lines: vec![(anchor, foot)] });
        }
    }
    best
}

/// The segments that no moving point belongs to, at most [`MAX_SEGMENTS`].
fn still_segments(scene: &Scene, moving: &Moving) -> Vec<Segment> {
    let mut out = Vec::new();
    for (shape, s) in scene.layer.shapes.iter().enumerate() {
        let fontcraft_model::Shape::Path(path) = s else { continue };
        if moving.shapes.contains(&shape) {
            continue;
        }
        for seg in path.segments() {
            if out.len() >= MAX_SEGMENTS {
                return out;
            }
            let moves = |i: &usize| moving.points.contains(&(shape, *i));
            if !(moves(&seg.start) || moves(&seg.end) || seg.handles.iter().any(moves)) {
                out.push(seg);
            }
        }
    }
    out
}

/// The two ends of a straight segment, when both are usable and apart.
fn straight_ends(seg: &Segment) -> Option<(Point, Point)> {
    if !seg.is_line() {
        return None;
    }
    let (a, b) = match seg.pieces.as_slice() {
        [Piece::Line(l)] => (l.p0, l.p1),
        _ => return None,
    };
    (usable(a) && usable(b) && dist(a, b) > 1e-9).then_some((a, b))
}

/// The edge `(a, b)` as `(position, low, high)` when it runs up and down (`vertical`) or across.
fn axis_edge((a, b): (Point, Point), vertical: bool) -> Option<(f64, f64, f64)> {
    let (pos_a, pos_b, run_a, run_b) = if vertical { (a.x, b.x, a.y, b.y) } else { (a.y, b.y, a.x, b.x) };
    ((pos_a - pos_b).abs() <= STRAIGHT && (run_a - run_b).abs() > STRAIGHT).then(|| ((pos_a + pos_b) / 2.0, run_a.min(run_b), run_a.max(run_b)))
}

/// An infinite line through `a` and `b`; drawn from `a` to the snapped point.
#[derive(Clone, Copy)]
struct Guideline {
    a: Point,
    b: Point,
}

impl Guideline {
    /// The distance from `p` to the line and the nearest point on it.
    fn foot(self, p: Point) -> Option<(f64, Point)> {
        let (vx, vy) = (self.b.x - self.a.x, self.b.y - self.a.y);
        let len2 = vx * vx + vy * vy;
        if !len2.is_finite() || len2 <= 1e-18 {
            return None;
        }
        let t = ((p.x - self.a.x) * vx + (p.y - self.a.y) * vy) / len2;
        let foot = Point::new(self.a.x + t * vx, self.a.y + t * vy);
        let d = dist(p, foot);
        d.is_finite().then_some((d, foot))
    }

    /// Where the two lines cross, unless they are parallel.
    fn crossing(self, other: Guideline) -> Option<Point> {
        let (r, s) = ((self.b.x - self.a.x, self.b.y - self.a.y), (other.b.x - other.a.x, other.b.y - other.a.y));
        let cross = r.0 * s.1 - r.1 * s.0;
        let scale = r.0.hypot(r.1) * s.0.hypot(s.1);
        if !(cross.is_finite() && scale.is_finite()) || cross.abs() <= 1e-9 * scale {
            return None;
        }
        let t = ((other.a.x - self.a.x) * s.1 - (other.a.y - self.a.y) * s.0) / cross;
        let x = Point::new(self.a.x + t * r.0, self.a.y + t * r.1);
        usable(x).then_some(x)
    }
}

/// The lines Suggest Lines offers near `p`.
fn guidelines(scene: &Scene, view: &View, moving: &Moving, p: Point) -> Vec<Guideline> {
    let reach = view.units(LINE_REACH);
    let near = |a: Point, b: Point| dist(p, a).min(dist(p, b)) <= reach;
    let mut out = Vec::new();
    let segments = still_segments(scene, moving);
    // Continuations of straight segments.
    for (a, b) in segments.iter().filter_map(straight_ends) {
        if near(a, b) {
            out.push(Guideline { a, b });
        }
    }
    // Handle directions: each handle that stays put, from the node it belongs to.
    for (shape, s) in scene.layer.shapes.iter().enumerate() {
        let fontcraft_model::Shape::Path(path) = s else { continue };
        if moving.shapes.contains(&shape) {
            continue;
        }
        let count = path.nodes.len().min(MAX_SEGMENTS);
        for i in 0..count {
            let Some(h) = path.nodes.get(i).filter(|node| !node.on_curve()) else { continue };
            for forward in [false, true] {
                let Some(k) = neighbour(count, path.closed, i, forward) else { continue };
                let Some(anchor) = path.nodes.get(k).filter(|node| node.on_curve()) else { continue };
                let still = !moving.points.contains(&(shape, i)) && !moving.points.contains(&(shape, k));
                let (a, b) = (anchor.point(), h.point());
                if still && usable(a) && usable(b) && dist(a, b) > 1e-9 && near(a, b) {
                    out.push(Guideline { a, b });
                }
            }
        }
    }
    // Perpendiculars of diagonal guides, through each guide's point.
    if scene.guides_shown {
        let guides = scene.local_guides().iter().chain(scene.shown_global_guides().map(|(_, g)| g));
        for g in guides {
            let a = Point::new(g.x, g.y);
            let off_axis = g.angle.rem_euclid(90.0);
            if !usable(a) || !g.angle.is_finite() || off_axis <= DIAGONAL_MARGIN || off_axis >= 90.0 - DIAGONAL_MARGIN {
                continue;
            }
            let (ux, uy) = crate::geometry::direction(g.angle + 90.0);
            out.push(Guideline { a, b: Point::new(a.x + ux, a.y + uy) });
        }
    }
    // Centrelines between neighbouring facing edges: the middles of stems and counters.
    for vertical in [true, false] {
        let edges: Vec<(f64, f64, f64)> =
            segments.iter().filter_map(straight_ends).filter_map(|e| axis_edge(e, vertical)).take(MAX_CENTRELINE_EDGES).collect();
        for &(pos, lo, hi) in &edges {
            // The nearest edge past this one whose run overlaps it.
            let partner = edges.iter().filter(|&&(q, l, h)| q > pos + STRAIGHT && l.max(lo) < h.min(hi)).min_by(|x, y| x.0.total_cmp(&y.0));
            let Some(&(q, l, h)) = partner else { continue };
            let (mid, from, to) = ((pos + q) / 2.0, l.max(lo), h.min(hi));
            let (a, b) = if vertical { (Point::new(mid, from), Point::new(mid, to)) } else { (Point::new(from, mid), Point::new(to, mid)) };
            let along = if vertical { p.y } else { p.x };
            if (from - reach..=to + reach).contains(&along) {
                out.push(Guideline { a, b });
            }
        }
    }
    out
}

fn line_suggestion(scene: &Scene, view: &View, moving: &Moving, p: Point, tol: f64) -> Option<Found> {
    let mut near: Vec<(f64, Point, Guideline)> =
        guidelines(scene, view, moving, p).into_iter().filter_map(|g| g.foot(p).filter(|(d, _)| *d <= tol).map(|(d, f)| (d, f, g))).collect();
    near.sort_by(|x, y| x.0.total_cmp(&y.0));
    let mut it = near.iter();
    let &(d, foot, first) = it.next()?;
    // A second line within reach that crosses the first near the pointer gives the crossing.
    for &(_, _, second) in it {
        if let Some(x) = first.crossing(second)
            && dist(p, x) <= 2.0 * tol
        {
            return Some(Found { point: x, d: dist(p, x), lines: vec![(first.a, x), (second.a, x)] });
        }
    }
    Some(Found { point: foot, d, lines: vec![(first.a, foot)] })
}

/// The unit normal of `piece` at `t`, turned left of its direction.
fn normal(piece: &Piece, t: f64) -> Option<(f64, f64)> {
    let h = 1e-4;
    let (t0, t1) = ((t - h).max(0.0), (t + h).min(1.0));
    let (a, b) = (piece.eval(t0), piece.eval(t1));
    let (vx, vy) = (b.x - a.x, b.y - a.y);
    let len = vx.hypot(vy);
    (len.is_finite() && len > 1e-12).then(|| (-vy / len, vx / len))
}

/// The distance of the parallel outline in the direction `(nx, ny)`: `dx` across an edge that
/// runs up and down, `dy` across one that runs left and right, and the blend of both between.
fn offset(dx: f64, dy: f64, (nx, ny): (f64, f64)) -> f64 {
    nx.abs() * dx + ny.abs() * dy
}

/// The parallel of `piece` on the side `side` (1: left of its direction, -1: right).
fn offset_polyline(piece: &Piece, dx: f64, dy: f64, side: f64) -> Vec<Point> {
    (0..=OUTLINE_STEPS)
        .filter_map(|i| {
            let t = i as f64 / OUTLINE_STEPS as f64;
            let q = piece.eval(t);
            let (nx, ny) = normal(piece, t)?;
            let (nx, ny) = (nx * side, ny * side);
            let d = offset(dx, dy, (nx, ny));
            let p = Point::new(q.x + nx * d, q.y + ny * d);
            usable(p).then_some(p)
        })
        .collect()
}

fn distance_suggestion(scene: &Scene, moving: &Moving, p: Point, tol: f64) -> Option<Found> {
    let (dx, dy) = scene.suggest.distance?;
    let mut best = None;
    for seg in still_segments(scene, moving) {
        for piece in &seg.pieces {
            let Some((d, t)) = piece.nearest(p) else { continue };
            if !d.is_finite() || d <= 1e-9 {
                continue;
            }
            let q = piece.eval(t);
            let n = ((p.x - q.x) / d, (p.y - q.y) / d);
            let want = offset(dx, dy, n);
            let miss = (d - want).abs();
            if want <= 0.0 || miss > tol {
                continue;
            }
            let side = match normal(piece, t) {
                Some((nx, ny)) if nx * n.0 + ny * n.1 < 0.0 => -1.0,
                _ => 1.0,
            };
            let lines = offset_polyline(piece, dx, dy, side).windows(2).filter_map(|w| Some((*w.first()?, *w.get(1)?))).collect();
            better(&mut best, Found { point: Point::new(q.x + n.0 * want, q.y + n.1 * want), d: miss, lines });
        }
    }
    best
}

/// A straight vertical or horizontal edge being dragged: where it lands one of the declared stems
/// away from a facing edge whose run overlaps it.
fn stem_suggestion(scene: &Scene, dragged: Dragged<'_>, target: Point, tol: f64) -> Option<Found> {
    if scene.suggest.stems.is_empty() {
        return None;
    }
    let ends: Vec<Point> = dragged
        .selection
        .points
        .iter()
        .filter_map(|&(s, n)| scene.node(s, n))
        .filter(|n| n.on_curve())
        .map(|n| Point::new(n.x - dragged.shown.0, n.y - dragged.shown.1))
        .take(MAX_SEGMENTS)
        .collect();
    if ends.len() < 2 {
        return None;
    }
    let (lo, hi) = bounds(ends.iter().copied())?;
    let (shift_x, shift_y) = (target.x - dragged.grab.x, target.y - dragged.grab.y);
    let segments = still_segments(scene, dragged.moving);
    let mut best = None;
    for vertical in [true, false] {
        let Some((pos, run_lo, run_hi)) = axis_edge((lo, hi), vertical) else { continue };
        let now = pos + if vertical { shift_x } else { shift_y };
        for (q, l, h) in segments.iter().filter_map(straight_ends).filter_map(|e| axis_edge(e, vertical)) {
            let (from, to) = (l.max(run_lo), h.min(run_hi));
            if to - from <= STRAIGHT {
                continue;
            }
            let middle = (from + to) / 2.0;
            // A horizontal stem is measured vertically, across an edge that runs left and right.
            for stem in scene.suggest.stems.iter().filter(|s| s.horizontal != vertical) {
                for at in [q - stem.width, q + stem.width] {
                    let miss = (at - now).abs();
                    if miss > tol {
                        continue;
                    }
                    let (point, line) = if vertical {
                        (Point::new(target.x + at - now, target.y), (Point::new(q, middle), Point::new(at, middle)))
                    } else {
                        (Point::new(target.x, target.y + at - now), (Point::new(middle, q), Point::new(middle, at)))
                    };
                    better(&mut best, Found { point, d: miss, lines: vec![line] });
                }
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use fontcraft_model::{Guide, Layer, Node, NodeKind, Path, Shape};
    use serde_json::Value;

    use super::*;
    use crate::event::PointerKind::{Down, Drag};
    use crate::event::{Modifiers, PointerEvent};
    use crate::output::Action;
    use crate::select::SelectTool;
    use crate::tool::{Context, Tool, ToolSettings};

    const NONE: Modifiers = Modifiers::NONE;

    fn on(x: f64, y: f64) -> Node {
        Node::new(x, y, NodeKind::Line)
    }

    fn off(x: f64, y: f64) -> Node {
        Node::new(x, y, NodeKind::OffCurve)
    }

    fn curve(x: f64, y: f64) -> Node {
        Node::new(x, y, NodeKind::Curve)
    }

    fn all_on() -> Switches {
        Switches { handles: true, lines: true, distance: true, stems: true }
    }

    /// Presses at `from`, drags to `to` with the Select tool and returns the last move preview's
    /// (dx, dy), with the overlays drawn at the end.
    /// Drags with the Select tool. Each preview is applied to the layer the next event sees, as
    /// the session does: the selected points move by the preview's shift.
    fn drag(layer: &Layer, suggest: &Suggest, from: (f64, f64), to: (f64, f64)) -> ((f64, f64), usize) {
        let mut shown = layer.clone();
        let mut selection = Selection::new();
        let mut tool = SelectTool::default();
        let mut last = None;
        let mut steps = vec![(Down, from)];
        let mid = ((from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0);
        steps.extend([(Drag, mid), (Drag, to)]);
        for (kind, (x, y)) in steps {
            let mut scene = Scene::new("A", "m01", &shown);
            scene.suggest = suggest.clone();
            let cx = Context { scene: &scene, view: View::default(), selection: &selection, settings: ToolSettings::default() };
            let out = tool.pointer(&cx, &PointerEvent::new(kind, x, y, NONE));
            let mut moved = None;
            for a in out.actions {
                match a {
                    Action::Select { selection: s } => selection = s,
                    Action::Preview { invocation } => {
                        let n = |k: &str| invocation.params.get(k).and_then(Value::as_f64).unwrap_or(0.0);
                        last = Some((n("dx"), n("dy")));
                        moved = last;
                    }
                    _ => {}
                }
            }
            if let Some((dx, dy)) = moved {
                shown = layer.clone();
                for &(s, n) in &selection.points {
                    if let Some(Shape::Path(path)) = shown.shapes.get_mut(s)
                        && let Some(node) = path.nodes.get_mut(n)
                    {
                        node.x += dx;
                        node.y += dy;
                    }
                }
            }
        }
        (last.expect("the drag previews a move"), tool.overlays().len())
    }

    fn close(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6
    }

    /// One open cubic: (0,0), handles (h1) and (300,200), to (400,0).
    fn arch(h1: (f64, f64)) -> Layer {
        let mut layer = Layer::new("m01", 600.0);
        layer.shapes.push(Shape::Path(Path::new(false, vec![on(0.0, 0.0), off(h1.0, h1.1), off(300.0, 200.0), curve(400.0, 0.0)])));
        layer
    }

    #[test]
    fn a_dragged_handle_snaps_flat_parallel_and_square_to_the_other_handle() {
        let s = Suggest::new(Switches { handles: true, ..Switches::default() }, [], None);
        // Flat: the handle at (100,50) dragged to (150,2) lands on y = 0.
        let (moved, overlays) = drag(&arch((100.0, 50.0)), &s, (100.0, 50.0), (150.0, 2.0));
        assert!(close(moved, (50.0, -50.0)), "{moved:?}");
        assert!(overlays >= 1, "the suggestion line is drawn");
        // Parallel: the other handle points from (400,0) to (300,200), direction (-1, 2). The
        // line through (0,0) that way passes (-50,100); dragged to (-52,101) the handle lands on it.
        let (moved, _) = drag(&arch((100.0, 50.0)), &s, (100.0, 50.0), (-52.0, 101.0));
        let landed = (100.0 + moved.0, 50.0 + moved.1);
        assert!((landed.0 * 2.0 + landed.1).abs() < 1e-6, "on the parallel: {landed:?}");
        // Square: (2, 1) is at a right angle to (-1, 2); (200,100) lies on it, (201,103) lands there.
        let (moved, _) = drag(&arch((100.0, 50.0)), &s, (100.0, 50.0), (201.0, 103.0));
        let landed = (100.0 + moved.0, 50.0 + moved.1);
        assert!((landed.0 - 2.0 * landed.1).abs() < 1e-6, "at a right angle: {landed:?}");
        // Off, the handle goes where it is dragged.
        let (moved, _) = drag(&arch((100.0, 50.0)), &Suggest::default(), (100.0, 50.0), (-52.0, 101.0));
        assert!(close(moved, (-152.0, 51.0)), "{moved:?}");
    }

    #[test]
    fn a_dragged_edge_snaps_to_a_declared_stem() {
        // A square 0..100; the right edge (nodes 1 and 2) dragged from x 100 to 82 lands at 80,
        // a vertical stem of 80 from the left edge.
        let mut layer = Layer::new("m01", 600.0);
        layer.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
        let stems = [DeclaredStem { width: 80.0, horizontal: false }, DeclaredStem { width: 30.0, horizontal: true }];
        let s = Suggest::new(Switches { stems: true, ..Switches::default() }, stems, None);
        let (moved, _) = drag(&layer, &s, (100.0, 50.0), (82.0, 50.0));
        assert!(close(moved, (-20.0, 0.0)), "{moved:?}");
        // The top edge dragged from y 100 to 33 lands at 30, a horizontal stem above the bottom.
        let (moved, _) = drag(&layer, &s, (50.0, 100.0), (50.0, 33.0));
        assert!(close(moved, (0.0, -70.0)), "{moved:?}");
        // Far from any stem width, nothing changes.
        let (moved, _) = drag(&layer, &s, (100.0, 50.0), (60.0, 50.0));
        assert!(close(moved, (-40.0, 0.0)), "{moved:?}");
    }

    #[test]
    fn a_dragged_node_follows_lines_and_parallel_outlines() {
        // A square 0..100 and a lone node at (300,300).
        let mut layer = Layer::new("m01", 600.0);
        layer.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
        layer.shapes.push(Shape::Path(Path::new(false, vec![on(300.0, 300.0), on(320.0, 340.0)])));
        layer.info.guides.push(Guide { x: 0.0, y: 0.0, angle: 45.0, ..Default::default() });
        let lines = Suggest::new(Switches { lines: true, ..Switches::default() }, [], None);
        // (102,180) lands on the continuation of the square's right edge, x = 100.
        let (moved, _) = drag(&layer, &lines, (300.0, 300.0), (102.0, 180.0));
        assert!(close(moved, (-200.0, -120.0)), "{moved:?}");
        // The perpendicular of the 45° guide through (0,0) is y = -x: (-150, 152) lands on it.
        let (moved, _) = drag(&layer, &lines, (300.0, 300.0), (-151.0, 152.0));
        let landed = (300.0 + moved.0, 300.0 + moved.1);
        assert!((landed.0 + landed.1).abs() < 1e-6, "{landed:?}");
        // Suggest Distance at 40 across vertical edges: (143, 50) lands 40 right of x = 100.
        let distance = Suggest::new(Switches { distance: true, ..Switches::default() }, [], Some((40.0, 25.0)));
        let (moved, _) = drag(&layer, &distance, (300.0, 300.0), (143.0, 50.0));
        assert!(close(moved, (-160.0, -250.0)), "{moved:?}");
        // And 25 above a horizontal edge: (50, 127) lands at y = 125.
        let (moved, _) = drag(&layer, &distance, (300.0, 300.0), (50.0, 127.0));
        assert!(close(moved, (-250.0, -175.0)), "{moved:?}");
        let mut scene = Scene::new("A", "m01", &layer);
        scene.suggest = distance;
        assert_eq!(distance_outlines(&scene, &Moving::default(), Point::new(150.0, 50.0), 20.0).len(), 2, "both sides of the right edge");
    }

    #[test]
    fn plain_snapping_wins_and_bad_values_are_dropped() {
        let mut layer = Layer::new("m01", 600.0);
        layer.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
        layer.shapes.push(Shape::Path(Path::new(false, vec![on(300.0, 300.0), on(320.0, 340.0)])));
        // Near the node (100,100), the node wins over every suggestion.
        let s = Suggest::new(all_on(), [DeclaredStem { width: 3.0, horizontal: false }], Some((2.0, 2.0)));
        let (moved, _) = drag(&layer, &s, (300.0, 300.0), (101.0, 102.0));
        assert!(close(moved, (-200.0, -200.0)), "{moved:?}");
        let bad = Suggest::new(
            all_on(),
            [DeclaredStem { width: f64::NAN, horizontal: false }, DeclaredStem { width: -4.0, horizontal: true }],
            Some((f64::INFINITY, 1.0)),
        );
        assert!(bad.stems.is_empty() && bad.distance.is_none());
    }
}
