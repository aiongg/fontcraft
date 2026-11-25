//! The Draw tool (`P`): paths built node by node (`[[tool]] draw`, `docs/spec/03a` § 5, § 12,
//! § 15).
//!
//! | Gesture | Result |
//! |---|---|
//! | click | a node; the next clicks add nodes to the same path, joined by straight segments |
//! | drag | a node with two handles; the segment before it is a curve |
//! | `Alt` during the drag | only the outgoing handle follows the pointer; the incoming one stays (a corner with handles) |
//! | `Cmd` during the drag | only the outgoing handle's length changes; both handles keep their angle |
//! | `Space` during the drag | the node moves, with its handles |
//! | `Shift` | the new node keeps to 45° steps from the previous one, a dragged handle to 45° steps from its node |
//! | click the other end of the path being drawn | close the path |
//! | click an open end of another path | while drawing: join the two paths; otherwise: continue that path from that end |
//! | click a node, not drawing | open a closed path there (two coincident ends); split an open path there |
//! | click a segment | a node there, as on empty canvas (the clicked path is left alone) |
//! | `Shift`+click a segment | `path.insertNode` at the extremum or inflection nearest to the click |
//! | `Backspace` (the end node selected, or nothing) | `edit.delete` of the last node; the drawing goes on from the one before |
//! | `Escape`, another tool | end the path |
//!
//! Each node placed is one interaction, so one undo step, of existing commands: `path.addPath`
//! for a path's first node; for the others a `command.batch` that adds the new segment as a path
//! of its own and joins it on with `path.connectNodes` (then `path.closeOpenPaths` to close). The
//! handle that leaves the last node, from the drag that placed it, stays with the tool until the
//! next node uses it. A segment with a handle at one end only gets the other one a third of the
//! way from its node towards the first.
//!
//! In a glyph (else a font) with quadratic curves the drawn curves are quadratic: the dragged
//! handles are the segment's off-curve points, two of them or the only one there is.
//!
//! Undo and redo keep the drawing: the tool follows its path by its end nodes, and goes on from
//! the end the path has. It ends when the path is gone or closed.

use std::collections::BTreeMap;
use std::f64::consts::FRAC_PI_4;

use fontcraft_geom::{CubicBez, ParamCurve, Point, QuadBez, Vec2};
use fontcraft_model::{Node, NodeKind};
use serde_json::{Map, Value, json};

use crate::event::{Key, KeyEvent, Modifiers, PointerEvent, PointerKind};
use crate::geometry::{dist, finite};
use crate::hit::{Hit, HitResult, hit_test};
use crate::output::{Action, Invocation, Output, Overlay, OverlayKind};
use crate::scene::Scene;
use crate::segment::segments;
use crate::selection::{Item, Selection};
use crate::snap::{Moving, snap};
use crate::tool::{Context, Tool};
use crate::view::View;

/// Straight pieces the preview of a curved segment is drawn with.
const PREVIEW_STEPS: usize = 16;
/// Handles shorter than this, in font units, count as none.
const MIN_HANDLE: f64 = 1e-6;
/// How far, in font units, an end node may be from where the tool put it: placed nodes are rounded.
const SAME_PLACE: f64 = 1.0;
/// Segment parameters this close to an end are the end node, not a place to insert.
const END_T: f64 = 1e-6;

/// `p` moved onto the nearest 45° direction from `origin`.
pub(crate) fn constrain(origin: Point, p: Point) -> Point {
    let v = p - origin;
    let len = v.hypot();
    if !len.is_finite() || len < MIN_HANDLE {
        return p;
    }
    let a = (v.y.atan2(v.x) / FRAC_PI_4).round() * FRAC_PI_4;
    let u = Vec2::new(a.cos(), a.sin());
    origin + u * v.dot(u)
}

fn unit(v: Vec2) -> Option<Vec2> {
    let len = v.hypot();
    (len.is_finite() && len >= MIN_HANDLE).then(|| v / len)
}

/// `h` when it is a usable handle of `node`.
fn handle(node: Point, h: Point) -> Option<Point> {
    (finite(h) && dist(h, node) >= MIN_HANDLE).then_some(h)
}

/// The two handles of the segment from `a` to `b`, given the one at each end that is known.
fn handles_between(a: Point, ha: Option<Point>, hb: Option<Point>, b: Point) -> Option<(Point, Point)> {
    let third = |from: Point, to: Point| from + (to - from) / 3.0;
    match (ha, hb) {
        (None, None) => None,
        (Some(h1), Some(h2)) => Some((h1, h2)),
        (Some(h1), None) => Some((h1, third(b, h1))),
        (None, Some(h2)) => Some((third(a, h2), h2)),
    }
}

fn kind_name(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Line => "line",
        NodeKind::Curve => "curve",
        NodeKind::QCurve => "qcurve",
        NodeKind::OffCurve => "offcurve",
    }
}

fn off(p: Point) -> Value {
    json!({"x": p.x, "y": p.y, "type": "offcurve"})
}

/// The off-curve points of the segment from `a` to `b`, given the handle at each end that is
/// known; none for a straight segment. A quadratic segment takes the handles there are.
fn off_curves(a: Point, ha: Option<Point>, hb: Option<Point>, b: Point, quadratic: bool) -> Vec<Point> {
    if quadratic {
        return [ha, hb].into_iter().flatten().collect();
    }
    handles_between(a, ha, hb, b).map_or_else(Vec::new, |(h1, h2)| vec![h1, h2])
}

/// The nodes of a path from `a` to `b` (`path.addPath`), and how many there are.
fn piece(a: Point, hs: &[Point], b: Point, b_smooth: bool, quadratic: bool) -> (Value, usize) {
    let mut nodes = vec![json!({"x": a.x, "y": a.y})];
    nodes.extend(hs.iter().map(|&h| off(h)));
    nodes.push(if hs.is_empty() {
        json!({"x": b.x, "y": b.y, "smooth": b_smooth})
    } else {
        json!({"x": b.x, "y": b.y, "type": if quadratic { "qcurve" } else { "curve" }, "smooth": b_smooth})
    });
    let len = nodes.len();
    (Value::Array(nodes), len)
}

/// Points along the segment from `a` to `b` with the off-curve points `hs`, without `a`.
fn samples(a: Point, hs: &[Point], b: Point, quadratic: bool) -> Vec<Point> {
    let at = |k: usize| k as f64 / PREVIEW_STEPS as f64;
    match hs {
        [] => vec![b],
        &[h1, h2] if !quadratic => {
            let c = CubicBez::new(a, h1, h2, b);
            (1..=PREVIEW_STEPS).map(|k| c.eval(at(k))).collect()
        }
        _ => {
            let mut out = Vec::new();
            let mut from = a;
            for (i, &h) in hs.iter().enumerate() {
                let to = hs.get(i + 1).map_or(b, |next| h.midpoint(*next));
                let q = QuadBez::new(from, h, to);
                out.extend((1..=PREVIEW_STEPS).map(|k| q.eval(at(k))));
                from = to;
            }
            out
        }
    }
}

/// `base` (glyph and layer) with the entries of `extra`.
pub(crate) fn params(base: &Map<String, Value>, extra: Value) -> Value {
    let mut m = base.clone();
    if let Value::Object(e) = extra {
        m.extend(e);
    }
    Value::Object(m)
}

fn step(command: &str, base: &Map<String, Value>, extra: Value) -> Value {
    json!({"command": command, "params": params(base, extra)})
}

fn batch(label: &str, steps: Vec<Value>) -> Invocation {
    Invocation::new("command.batch", json!({"label": label, "steps": steps}))
}

fn select_point(shape: usize, node: usize) -> Action {
    Action::Select { selection: Selection::from_items([Item::Point(shape, node)]) }
}

/// The open path being drawn: new nodes go after its last node, or before its first.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Drawing {
    shape: usize,
    /// New nodes go before the first node (the path was continued at its start).
    at_start: bool,
    /// The path's node count when the tool last saw it.
    count: usize,
    /// The handle leaving the end node towards the next node, from the drag that placed it.
    pending: Option<Point>,
}

impl Drawing {
    fn end_index(&self) -> usize {
        if self.at_start { 0 } else { self.count.saturating_sub(1) }
    }

    fn other_index(&self) -> usize {
        if self.at_start { self.count.saturating_sub(1) } else { 0 }
    }

    /// The path `shape` as one this drawing could go on with: its node count, where its end node
    /// is, and where the node at its other end is. `None` for a closed path.
    fn ends(&self, scene: &Scene, shape: usize) -> Option<(usize, Point, Point)> {
        let path = scene.path(shape)?;
        if path.closed || !scene.shape_editable(shape) {
            return None;
        }
        let (first, last) = (path.nodes.first()?, path.nodes.last()?);
        let (end, other) = if self.at_start { (first, last) } else { (last, first) };
        (end.on_curve() && other.on_curve()).then(|| (path.nodes.len(), end.point(), other.point()))
    }
}

/// The node a drag places and the handles it pulls out of it.
#[derive(Clone, Copy, Debug)]
struct Handles {
    node: Point,
    down: Point,
    last: Point,
    mods: Modifiers,
    /// The pointer has moved past the drag threshold.
    dragging: bool,
    /// The handle towards the next segment.
    out: Option<Point>,
    /// The handle of the segment that ends at the node.
    inn: Option<Point>,
    smooth: bool,
    /// While `Alt` is held: the incoming handle as it was when `Alt` went down.
    alt_in: Option<Option<Point>>,
    /// While `Cmd` is held: the direction both handles keep.
    cmd_dir: Option<Vec2>,
    /// While `Space` is held: the pointer, the node and its handles when `Space` went down.
    space: Option<(Point, Point, Option<Point>, Option<Point>)>,
}

impl Handles {
    fn new(node: Point, down: Point, mods: Modifiers, inn: Option<Point>) -> Self {
        Self { node, down, last: down, mods, dragging: false, out: None, inn, smooth: false, alt_in: None, cmd_dir: None, space: None }
    }

    fn update(&mut self, view: &View, threshold: f64, pos: Point, mods: Modifiers, space: bool) {
        if !finite(pos) {
            return;
        }
        self.last = pos;
        self.mods = mods;
        if space {
            let (from, node, out, inn) = *self.space.get_or_insert((pos, self.node, self.out, self.inn));
            let d = pos - from;
            self.node = node + d;
            self.out = out.map(|p| p + d);
            self.inn = inn.map(|p| p + d);
            return;
        }
        self.space = None;
        if !self.dragging {
            if dist(view.to_screen(self.down), view.to_screen(pos)) < threshold {
                return;
            }
            self.dragging = true;
        }
        let node = self.node;
        let d = if mods.shift { constrain(node, pos) } else { pos };
        if mods.alt {
            let kept = *self.alt_in.get_or_insert(self.inn);
            self.cmd_dir = None;
            self.out = handle(node, d);
            self.inn = kept;
            self.smooth = false;
            return;
        }
        self.alt_in = None;
        if mods.cmd {
            let dir = self.cmd_dir.or_else(|| self.out.and_then(|o| unit(o - node))).or_else(|| unit(d - node));
            if let Some(u) = dir {
                self.cmd_dir = Some(u);
                self.out = handle(node, node + u * (d - node).dot(u).max(0.0));
                return;
            }
        }
        self.cmd_dir = None;
        self.out = handle(node, d);
        self.inn = self.out.map(|o| node + (node - o));
        self.smooth = self.out.is_some();
    }

    fn overlays(&self, both: bool) -> Vec<Overlay> {
        if !self.dragging {
            return Vec::new();
        }
        let mut v = Vec::new();
        for h in [self.out, if both { self.inn } else { None }].into_iter().flatten() {
            v.push(Overlay::Line { kind: OverlayKind::DrawHandle, from: self.node, to: h });
            v.push(Overlay::Marker { kind: OverlayKind::DrawHandle, at: h });
        }
        v
    }
}

/// An open end a gesture finishes on: its address, position and type, and the handle next to
/// it in its path, if there is one.
#[derive(Clone, Copy, Debug)]
struct End {
    shape: usize,
    node: usize,
    at: Point,
    smooth: bool,
    adjacent: Option<(usize, Point)>,
}

impl End {
    fn of(scene: &Scene, shape: usize, node: usize) -> Option<Self> {
        let path = scene.path(shape)?;
        let n = path.nodes.get(node)?;
        let next = if node == 0 { 1 } else { node.checked_sub(1)? };
        let adjacent = path.nodes.get(next).filter(|h| h.kind == NodeKind::OffCurve).map(|h| (next, h.point()));
        Some(Self { shape, node, at: n.point(), smooth: n.smooth, adjacent })
    }

    /// The handle on the far side of the end, mirrored through it, for a smooth end.
    fn mirrored(&self) -> Option<Point> {
        let (_, h) = self.adjacent.filter(|_| self.smooth)?;
        handle(self.at, self.at + (self.at - h))
    }
}

#[derive(Clone, Debug)]
enum Kind {
    /// The first node of a new path, at shape index `shape`.
    Start { shape: usize },
    /// A node added at the end of the path being drawn.
    Append { d: Drawing, end: Point },
    /// The other end of the path being drawn: close it.
    Close { d: Drawing, end: Point, other: End },
    /// An open end of another path: join the paths.
    Join { d: Drawing, end: Point, other: End },
}

/// A press that edits through an interaction.
#[derive(Clone, Debug)]
struct Placing {
    kind: Kind,
    h: Handles,
    /// Glyph and layer, for every command's parameters.
    base: Map<String, Value>,
    /// The layer's shape count at the press: the index a path added now gets.
    shapes: usize,
    /// The drawn curves are quadratic.
    quadratic: bool,
    sent: Option<Value>,
}

impl Placing {
    fn invocation(&self) -> Invocation {
        let h = &self.h;
        let m = self.shapes;
        let quadratic = self.quadratic;
        match &self.kind {
            Kind::Start { .. } => {
                let node = json!({"x": h.node.x, "y": h.node.y, "smooth": h.smooth});
                Invocation::new("path.addPath", params(&self.base, json!({"closed": false, "nodes": [node]})))
            }
            Kind::Append { d, end } => {
                let hs = off_curves(*end, d.pending, h.inn, h.node, quadratic);
                let (nodes, _) = piece(*end, &hs, h.node, h.smooth, quadratic);
                let mut steps = vec![
                    step("path.addPath", &self.base, json!({"closed": false, "nodes": nodes})),
                    step("path.connectNodes", &self.base, json!({"points": [[d.shape, d.end_index()], [m, 0]]})),
                ];
                if d.at_start {
                    // Joining at the first node turns the path round; this turns it back.
                    steps.push(step("path.reverseContours", &self.base, json!({"shapes": [d.shape]})));
                }
                batch("Draw", steps)
            }
            Kind::Close { d, end, other } => {
                // The closing segment runs from the last node to the first.
                let (a, ha, b, hb) = if d.at_start { (other.at, h.inn, *end, d.pending) } else { (*end, d.pending, other.at, h.inn) };
                let (nodes, _) = piece(a, &off_curves(a, ha, hb, b, quadratic), b, false, quadratic);
                let mut steps = self.far_handle(other);
                steps.push(step("path.addPath", &self.base, json!({"closed": false, "nodes": nodes})));
                steps.push(step("path.connectNodes", &self.base, json!({"points": [[d.shape, d.count.saturating_sub(1)], [m, 0]]})));
                steps.push(step("path.closeOpenPaths", &self.base, json!({"shapes": [d.shape]})));
                batch("Close Path", steps)
            }
            Kind::Join { d, end, other } => {
                let smooth = if self.shapes_far_handle(other) { h.smooth } else { other.smooth };
                let hs = off_curves(*end, d.pending, h.inn, other.at, quadratic);
                let (nodes, len) = piece(*end, &hs, other.at, smooth, quadratic);
                let mut steps = self.far_handle(other);
                steps.push(step("path.addPath", &self.base, json!({"closed": false, "nodes": nodes})));
                steps.push(step("path.connectNodes", &self.base, json!({"points": [[d.shape, d.end_index()], [m, 0]]})));
                // The joined segment ends the drawn path now, whichever end it was drawn from.
                let joined = d.count + len - 2;
                steps.push(step("path.connectNodes", &self.base, json!({"points": [[d.shape, joined], [other.shape, other.node]]})));
                batch("Connect Nodes", steps)
            }
        }
    }

    /// Whether the drag shapes the handle on the far side of the end the gesture finishes on.
    fn shapes_far_handle(&self, other: &End) -> bool {
        self.h.dragging && self.h.out.is_some() && other.adjacent.is_some()
    }

    /// The steps that move the handle on the far side of `other` to the dragged handle, and give
    /// the node the type the drag gives it. Run before anything changes the indices.
    fn far_handle(&self, other: &End) -> Vec<Value> {
        let (Some((index, at)), Some(out), true) = (other.adjacent, self.h.out, self.h.dragging) else { return Vec::new() };
        let mut steps = Vec::new();
        if other.smooth != self.h.smooth {
            let to = if self.h.smooth { "smooth" } else { "corner" };
            steps.push(step("path.toggleNodeType", &self.base, json!({"points": [[other.shape, other.node]], "to": to, "keepOutline": true})));
        }
        steps.push(step("selection.move", &self.base, json!({"points": [[other.shape, index]], "dx": out.x - at.x, "dy": out.y - at.y})));
        steps
    }

    fn preview(&mut self) -> Option<Action> {
        let invocation = self.invocation();
        if self.sent.as_ref() == Some(&invocation.params) {
            return None;
        }
        self.sent = Some(invocation.params.clone());
        Some(Action::Preview { invocation })
    }

    /// The drawing after the interaction is committed, and the point to select.
    fn finish(&self) -> (Option<Drawing>, Option<(usize, usize)>) {
        let h = &self.h;
        match &self.kind {
            Kind::Start { shape } => (Some(Drawing { shape: *shape, at_start: false, count: 1, pending: h.out }), Some((*shape, 0))),
            Kind::Append { d, end } => {
                let added = off_curves(*end, d.pending, h.inn, h.node, self.quadratic).len() + 1;
                let next = Drawing { count: d.count + added, pending: h.out, ..*d };
                (Some(next), Some((next.shape, next.end_index())))
            }
            Kind::Close { d, .. } => (None, Some((d.shape, d.other_index()))),
            Kind::Join { .. } => (None, None),
        }
    }
}

#[derive(Clone, Debug, Default)]
enum Gesture {
    #[default]
    Idle,
    Placing(Box<Placing>),
    /// The end of the path being drawn (or an open end to continue from) pressed: a drag pulls
    /// out the handle towards the next node.
    Pull {
        h: Box<Handles>,
        d: Drawing,
    },
    /// A press whose command runs on release.
    Click(Invocation),
}

/// The Draw tool.
#[derive(Clone, Debug, Default)]
pub struct DrawTool {
    gesture: Gesture,
    drawing: Option<Drawing>,
    /// Where the end of the path being drawn is.
    end_pos: Option<Point>,
    /// Where the node at its other end is.
    other_pos: Option<Point>,
    /// The handle that left the end node when the path had each node count: what an undo or a
    /// redo to that count gives back.
    trail: BTreeMap<usize, Option<Point>>,
    /// The drawn curves are quadratic.
    quadratic: bool,
    hover: Option<Point>,
    /// `Space` is held during the press.
    space: bool,
}

impl DrawTool {
    /// Brings the drawing in line with the scene: another edit, an undo or a redo may have
    /// changed its path. Ends it when the path is gone.
    fn refresh(&mut self, scene: &Scene) {
        match self.drawing.and_then(|d| self.follow(scene, d)) {
            Some((d, end, other)) => self.resume(d, Some(end), Some(other)),
            None => self.stop(),
        }
        if self.drawing.is_some() {
            self.quadratic = scene.draws_quadratic();
        }
    }

    fn resume(&mut self, d: Drawing, end: Option<Point>, other: Option<Point>) {
        self.drawing = Some(d);
        self.end_pos = end;
        self.other_pos = other;
        self.trail.insert(d.count, d.pending);
    }

    fn stop(&mut self) {
        self.drawing = None;
        self.end_pos = None;
        self.other_pos = None;
        self.trail.clear();
    }

    /// The drawing `d` in `scene`, and where its two ends are: its path by the place of either
    /// end node; a path of another index that has the other end node and a node count the drawing
    /// had; else the path at its index with its node count (moved as a whole).
    fn follow(&self, scene: &Scene, d: Drawing) -> Option<(Drawing, Point, Point)> {
        let is = |known: Option<Point>, p: Point| known.is_some_and(|q| dist(p, q) <= SAME_PLACE);
        let found = |shape: usize, (count, end, other): (usize, Point, Point)| {
            let pending = if count == d.count || is(self.end_pos, end) { d.pending } else { self.trail.get(&count).copied().flatten() };
            (Drawing { shape, count, pending, ..d }, end, other)
        };
        let here = d.ends(scene, d.shape);
        if let Some(path) = here.filter(|&(_, end, other)| is(self.end_pos, end) || is(self.other_pos, other)) {
            return Some(found(d.shape, path));
        }
        let elsewhere = (0..scene.layer.shapes.len())
            .filter(|&shape| shape != d.shape)
            .filter_map(|shape| Some((shape, d.ends(scene, shape)?)))
            .find(|&(_, (count, _, other))| self.trail.contains_key(&count) && is(self.other_pos, other));
        match elsewhere {
            Some((shape, path)) => Some(found(shape, path)),
            None => here.filter(|&(count, _, _)| count == d.count).map(|path| found(d.shape, path)),
        }
    }

    /// `Backspace` while drawing: the last node goes, and the drawing goes on from the one before.
    /// `None` when something else is selected.
    fn remove_last(&mut self, cx: &Context<'_>) -> Option<Output> {
        self.refresh(cx.scene);
        let d = self.drawing?;
        let own = Selection::from_items([Item::Point(d.shape, d.end_index())]);
        if !(cx.selection.is_empty() || *cx.selection == own) {
            return None;
        }
        let path = cx.scene.path(d.shape)?;
        let mut left: Vec<(usize, &Node)> = path.nodes.iter().enumerate().filter(|(_, n)| n.on_curve()).collect();
        if d.at_start {
            left.reverse();
        }
        left.pop()?;
        let base = Selection::new().targets(cx.scene);
        let mut out = Output::handled();
        let Some(&(before, node)) = left.last() else {
            self.stop();
            out.push(Action::Run { invocation: Invocation::new("edit.delete", params(&base, json!({"shapes": [d.shape]}))) });
            out.push(Action::Select { selection: Selection::new() });
            return Some(out);
        };
        let (next, invocation) = if left.len() == 1 {
            // A path of one node is not what `edit.delete` leaves: the node starts a path anew.
            let start = json!({"x": node.x, "y": node.y, "smooth": node.smooth});
            let steps = vec![
                step("edit.delete", &base, json!({"shapes": [d.shape]})),
                step("path.addPath", &base, json!({"closed": false, "nodes": [start]})),
            ];
            let next = Drawing { shape: cx.scene.layer.shapes.len().saturating_sub(1), count: 1, ..d };
            (next, batch("Delete", steps))
        } else {
            let count = if d.at_start { path.nodes.len().saturating_sub(before) } else { before + 1 };
            let delete = params(&base, json!({"points": [[d.shape, d.end_index()]]}));
            (Drawing { count, ..d }, Invocation::new("edit.delete", delete))
        };
        let next = Drawing { pending: self.trail.get(&next.count).copied().flatten(), ..next };
        let other = if next.count == 1 { Some(node.point()) } else { self.other_pos };
        self.resume(next, Some(node.point()), other);
        out.push(Action::Run { invocation });
        out.push(select_point(next.shape, next.end_index()));
        Some(out)
    }

    /// Where a click puts a node: on the node or segment it hit, else snapped (unless `Ctrl`),
    /// then with `Shift` on a 45° line from `from`.
    fn place(cx: &Context<'_>, ev: &PointerEvent, hit: Option<&HitResult>, from: Option<Point>) -> Point {
        let p = match hit {
            Some(h) if matches!(h.hit, Hit::Node { .. } | Hit::Segment { .. }) => h.at,
            _ if ev.mods.ctrl => ev.pos,
            _ => snap(cx.scene, &cx.view, ev.pos, &Moving::default(), cx.settings.snap_distance).point,
        };
        match from {
            Some(e) if ev.mods.shift => constrain(e, p),
            _ => p,
        }
    }

    fn down(&mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        let mut out = self.end_gesture();
        out.handled = true;
        self.space = false;
        let scene = cx.scene;
        self.refresh(scene);
        let quadratic = scene.draws_quadratic();
        let base = Selection::new().targets(scene);
        let shapes = scene.layer.shapes.len();
        let hit = hit_test(scene, &cx.view, ev.pos, cx.settings.hit_tolerance).filter(|h| !h.locked);

        if ev.mods.shift
            && let Some(HitResult { hit: Hit::Segment { shape, node, .. }, .. }) = &hit
            && let Some(invocation) = insert_at_extreme(scene, &base, *shape, *node, ev.pos)
        {
            self.gesture = Gesture::Click(invocation);
            return out;
        }
        let node_hit = match hit.as_ref().map(|h| &h.hit) {
            Some(Hit::Node { shape, node }) => Some((*shape, *node)),
            _ => None,
        };
        let new = |kind: Kind, node: Point, inn: Option<Point>| Placing {
            kind,
            h: Handles::new(node, ev.pos, ev.mods, inn),
            base: base.clone(),
            shapes,
            quadratic,
            sent: None,
        };
        let placing = match (self.drawing, self.end_pos) {
            (Some(d), Some(end)) => match node_hit {
                Some((s, n)) if s == d.shape && n == d.end_index() => {
                    self.gesture = Gesture::Pull { h: Box::new(Handles::new(end, ev.pos, ev.mods, None)), d };
                    return out;
                }
                Some((s, n)) if s == d.shape && n == d.other_index() && d.count >= 2 => match End::of(scene, s, n) {
                    Some(other) => new(Kind::Close { d, end, other }, other.at, other.mirrored()),
                    None => return out,
                },
                Some((s, n)) if s != d.shape && scene.is_open_end(s, n) => match End::of(scene, s, n) {
                    Some(other) => new(Kind::Join { d, end, other }, other.at, other.mirrored()),
                    None => return out,
                },
                _ => {
                    let at = Self::place(cx, ev, hit.as_ref(), Some(end));
                    new(Kind::Append { d, end }, at, None)
                }
            },
            _ => {
                if let Some((s, n)) = node_hit {
                    if scene.is_open_end(s, n) {
                        let count = scene.path(s).map_or(0, |p| p.nodes.len());
                        let d = Drawing { shape: s, at_start: n == 0 && count > 1, count, pending: None };
                        let at = scene.node(s, n).map_or(ev.pos, |node| node.point());
                        self.gesture = Gesture::Pull { h: Box::new(Handles::new(at, ev.pos, ev.mods, None)), d };
                    } else if let Some(invocation) = open_at(scene, &base, s, n, shapes) {
                        self.gesture = Gesture::Click(invocation);
                    }
                    return out;
                }
                let at = Self::place(cx, ev, hit.as_ref(), None);
                new(Kind::Start { shape: shapes }, at, None)
            }
        };
        let label = match placing.kind {
            Kind::Close { .. } => "Close Path",
            Kind::Join { .. } => "Connect Nodes",
            _ => "Draw",
        };
        let mut placing = Box::new(placing);
        out.push(Action::Begin { label: label.into() });
        if let Some(a) = placing.preview() {
            out.push(a);
        }
        self.gesture = Gesture::Placing(placing);
        out
    }

    fn drag(&mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        let mut out = Output::handled();
        let (threshold, space) = (cx.settings.drag_threshold, self.space);
        match &mut self.gesture {
            Gesture::Placing(p) => {
                p.h.update(&cx.view, threshold, ev.pos, ev.mods, space);
                if let Some(a) = p.preview() {
                    out.push(a);
                }
            }
            Gesture::Pull { h, .. } => h.update(&cx.view, threshold, ev.pos, ev.mods, false),
            Gesture::Click(_) => {}
            Gesture::Idle => out.handled = false,
        }
        out
    }

    fn up(&mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        let mut out = Output::handled();
        let (threshold, space) = (cx.settings.drag_threshold, self.space);
        self.space = false;
        match std::mem::take(&mut self.gesture) {
            Gesture::Idle => out.handled = false,
            Gesture::Click(invocation) => out.push(Action::Run { invocation }),
            Gesture::Pull { mut h, mut d } => {
                h.update(&cx.view, threshold, ev.pos, ev.mods, false);
                if h.dragging {
                    d.pending = h.out;
                }
                if self.drawing.is_none_or(|old| old.shape != d.shape) {
                    self.stop();
                }
                self.resume(d, Some(h.node), self.other_pos);
                self.refresh(cx.scene);
                out.push(select_point(d.shape, d.end_index()));
            }
            Gesture::Placing(mut p) => {
                p.h.update(&cx.view, threshold, ev.pos, ev.mods, space);
                if let Some(a) = p.preview() {
                    out.push(a);
                }
                out.push(Action::Commit);
                let (drawing, select) = p.finish();
                match (drawing, &p.kind) {
                    (Some(d), Kind::Start { .. }) => {
                        self.stop();
                        self.resume(d, Some(p.h.node), Some(p.h.node));
                    }
                    (Some(d), _) => {
                        // What was undone beyond this node cannot be redone.
                        self.trail.retain(|&count, _| count < d.count);
                        self.resume(d, Some(p.h.node), self.other_pos);
                    }
                    (None, _) => self.stop(),
                }
                out.push(match select {
                    Some((s, n)) => select_point(s, n),
                    None => Action::Select { selection: Selection::new() },
                });
            }
        }
        out
    }

    /// Ends the press in progress; an interaction it began is cancelled.
    fn end_gesture(&mut self) -> Output {
        let mut out = Output::default();
        if let Gesture::Placing(_) = std::mem::take(&mut self.gesture) {
            out.push(Action::Cancel);
        }
        out
    }

    /// The segment the next click would add, from the end of the path being drawn to the
    /// pointer, and the handle waiting at the end.
    fn rubber_band(&self) -> Vec<Overlay> {
        let (Some(d), Some(end)) = (self.drawing, self.end_pos) else { return Vec::new() };
        let mut v = Vec::new();
        if let Some(p) = d.pending {
            v.push(Overlay::Line { kind: OverlayKind::DrawHandle, from: end, to: p });
            v.push(Overlay::Marker { kind: OverlayKind::DrawHandle, at: p });
        }
        let Some(to) = self.hover.filter(|p| finite(*p)) else { return v };
        let mut from = end;
        for p in samples(end, &off_curves(end, d.pending, None, to, self.quadratic), to, self.quadratic) {
            v.push(Overlay::Line { kind: OverlayKind::DrawPreview, from, to: p });
            from = p;
        }
        v
    }
}

/// `path.insertNode` at the extremum or inflection of the segment `[shape, node]` nearest to
/// `pos`; `None` when the segment has neither inside it.
fn insert_at_extreme(scene: &Scene, base: &Map<String, Value>, shape: usize, node: usize, pos: Point) -> Option<Invocation> {
    let seg = scene.path(shape)?.segment_ending_at(node)?;
    let (_, t) = seg
        .extremes()
        .into_iter()
        .chain(seg.inflections())
        .filter(|t| *t > END_T && *t < 1.0 - END_T)
        .filter_map(|t| Some((seg.eval(t)?.distance(pos), t)))
        .min_by(|a, b| a.0.total_cmp(&b.0))?;
    Some(Invocation::new("path.insertNode", params(base, json!({"segment": [shape, node], "t": t}))))
}

/// Opens the path `shape` at its on-curve node `node`: a closed path becomes an open one that
/// starts and ends there; an open path is split there into two. `path.deleteAndBreak` removes
/// the segment that ends at the node, and the same segment is added back as a path of its own
/// and joined on, so the outline does not change. `shapes` is the layer's shape count.
fn open_at(scene: &Scene, base: &Map<String, Value>, shape: usize, node: usize, shapes: usize) -> Option<Invocation> {
    let path = scene.path(shape)?;
    if path.closed && path.nodes.iter().filter(|n| n.on_curve()).count() < 2 {
        return None;
    }
    let seg = segments(shape, path).into_iter().find(|s| s.end == node)?;
    let start = path.nodes.get(seg.start)?;
    let end = path.nodes.get(node)?;
    let mut nodes = vec![json!({"x": start.x, "y": start.y})];
    for &i in &seg.handles {
        nodes.push(off(path.nodes.get(i)?.point()));
    }
    nodes.push(json!({"x": end.x, "y": end.y, "type": kind_name(end.kind), "smooth": end.smooth}));
    let add = step("path.addPath", base, json!({"closed": false, "nodes": nodes}));
    let mut steps = vec![step("path.deleteAndBreak", base, json!({"segments": [[shape, node]]}))];
    if path.closed {
        // What is left runs from the node round to the segment's start, which is its last node.
        let last = path.nodes.len().checked_sub(seg.handles.len() + 1)?;
        steps.push(add);
        steps.push(step("path.connectNodes", base, json!({"points": [[shape, last], [shapes, 0]]})));
    } else if seg.start > 0 {
        // Two pieces are left, at `shape` and `shape + 1`; the segment goes back on the first.
        steps.push(add);
        steps.push(step("path.connectNodes", base, json!({"points": [[shape, seg.start], [shapes + 1, 0]]})));
    } else {
        // The segment was the first one: it stays a path of its own.
        steps.push(add);
    }
    Some(batch("Open Path", steps))
}

impl Tool for DrawTool {
    fn id(&self) -> &'static str {
        "draw"
    }

    fn pointer(&mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        if !finite(ev.pos) {
            return Output::default();
        }
        match ev.kind {
            PointerKind::Move => {
                self.hover = Some(ev.pos);
                if matches!(self.gesture, Gesture::Idle) {
                    self.refresh(cx.scene);
                }
                Output::default()
            }
            PointerKind::Down => self.down(cx, ev),
            PointerKind::Drag => self.drag(cx, ev),
            PointerKind::Up => self.up(cx, ev),
            PointerKind::DoubleClick => Output::handled(),
        }
    }

    fn key(&mut self, cx: &Context<'_>, ev: &KeyEvent) -> Output {
        match ev.key {
            Key::Space => {
                let mut out = Output::handled();
                if self.space == ev.pressed {
                    return out;
                }
                self.space = ev.pressed;
                if let Gesture::Placing(p) = &mut self.gesture {
                    let (last, mods) = (p.h.last, p.h.mods);
                    p.h.update(&cx.view, cx.settings.drag_threshold, last, mods, ev.pressed);
                    if let Some(a) = p.preview() {
                        out.push(a);
                    }
                }
                out
            }
            Key::Escape if ev.pressed => {
                if self.busy() {
                    let mut out = self.end_gesture();
                    out.handled = true;
                    return out;
                }
                let drawing = self.drawing.is_some();
                self.stop();
                if drawing { Output::handled() } else { Output::default() }
            }
            Key::Backspace if ev.pressed && !self.busy() && ev.mods == Modifiers::NONE => self.remove_last(cx).unwrap_or_default(),
            _ => Output::default(),
        }
    }

    fn modifiers(&mut self, cx: &Context<'_>, mods: Modifiers) -> Output {
        let mut out = Output::default();
        let (threshold, space) = (cx.settings.drag_threshold, self.space);
        match &mut self.gesture {
            Gesture::Placing(p) => {
                let last = p.h.last;
                p.h.update(&cx.view, threshold, last, mods, space);
                if let Some(a) = p.preview() {
                    out.push(a);
                }
            }
            Gesture::Pull { h, .. } => {
                let last = h.last;
                h.update(&cx.view, threshold, last, mods, false);
            }
            Gesture::Click(_) | Gesture::Idle => {}
        }
        out
    }

    fn cancel(&mut self) -> Output {
        let out = self.end_gesture();
        self.stop();
        self.hover = None;
        self.space = false;
        out
    }

    fn busy(&self) -> bool {
        !matches!(self.gesture, Gesture::Idle)
    }

    fn overlays(&self) -> Vec<Overlay> {
        match &self.gesture {
            Gesture::Placing(p) => p.h.overlays(true),
            Gesture::Pull { h, .. } => h.overlays(false),
            Gesture::Click(_) => Vec::new(),
            Gesture::Idle => self.rubber_band(),
        }
    }
}
