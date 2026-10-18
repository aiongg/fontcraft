//! The Rapid tool: a contour drawn by clicking its on-curve nodes only; the tool places the
//! handles at one consistent curve tension (`[[tool]] rapid`, `docs/spec/90-secondary-references.md`).
//!
//! | Gesture | Result |
//! |---|---|
//! | click | adds a node of the latched sub-mode ([`RapidTool::set_mode`]); with none latched, a smooth curve node |
//! | Alt+click | adds a corner node, when no sub-mode is latched |
//! | Cmd+Alt+click | adds a tangent node, when no sub-mode is latched |
//! | drag a node, or drag right after adding one | moves it; the handles follow |
//! | drag a tension point | sets how tight that one segment is; a click on it returns it to the default |
//! | click the first node | closes the contour and adds it |
//! | Return, Escape, a double-click, or leaving the tool | adds the open contour |
//! | Backspace or Delete | removes the last node |
//!
//! **Handles.** A segment between two corner or tangent nodes is straight; every other segment
//! is a curve. A curve node's direction is that of the line from the node before it to the node
//! after it; a tangent node takes the direction of its straight side; a corner, and either end of
//! an open contour, aims its handle at the other end's handle. Each handle is then set to the
//! tension, in percent of the distance from its node to the segment's Tunni point (where the two
//! handle lines meet). A segment whose handle lines do not meet ahead of both nodes gets handles
//! of the tension times the chord over √2 (a quarter circle's proportion).
//!
//! **Tension.** The contour is added as one `command.batch`: `path.addPath` with the handles
//! drawn, then `path.setCurveTension` without a `tension` for every segment that has a Tunni
//! point and no tension of its own, so the engine sets them to the master's `curveTension`
//! parameter. The preview uses the same parameter, which the shell passes in
//! [`ToolSettings::curve_tension`](crate::tool::ToolSettings::curve_tension). A segment whose tension point was dragged keeps the dragged value.

use std::f64::consts::SQRT_2;

use fontcraft_geom::{BezPath, PathEl, Point, Vec2, flatten};
use fontcraft_model::{Node, NodeKind};
use serde_json::{Map, Value, json};

use crate::draw::params;
use crate::event::{Key, KeyEvent, Modifiers, PointerEvent, PointerKind};
use crate::geometry::{dist, finite};
use crate::output::{Action, Invocation, Output, Overlay, OverlayKind};
use crate::tool::{Context, Tool};

/// The tension used when nothing else gives one, in percent (the engine's default too).
pub const DEFAULT_TENSION: f64 = 55.0;
/// The range a tension is held to, in percent.
pub const MIN_TENSION: f64 = 1.0;
pub const MAX_TENSION: f64 = 100.0;
/// Most nodes in one contour drawn with the tool.
pub const MAX_RAPID_NODES: usize = 10_000;
/// A Tunni distance longer than this many chords counts as no Tunni point: the handle lines are
/// nearly parallel.
const MAX_TUNNI_CHORDS: f64 = 4.0;
/// How far the preview may stray from the curve, in font units.
pub(crate) const PREVIEW_TOLERANCE: f64 = 0.5;
/// Most lines one preview is drawn with.
const MAX_PREVIEW_LINES: usize = 20_000;

/// The kind of node a click adds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RapidMode {
    /// A smooth node between curves.
    Curve,
    /// A sharp node; segments between corners are straight.
    Corner,
    /// A smooth node between a straight segment and a curve.
    Tangent,
}

/// One segment of the contour being drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RapidSegment {
    pub from: Point,
    pub to: Point,
    /// The two handles; `None` for a straight segment.
    pub handles: Option<(Point, Point)>,
    /// Where the handle lines meet, when they meet ahead of both nodes.
    pub tunni: Option<Point>,
}

impl RapidSegment {
    /// The point dragged to change the segment's tension: halfway between the two handles.
    pub fn tension_point(&self) -> Option<Point> {
        self.tunni.and(self.handles).map(|(a, b)| a.midpoint(b))
    }

    /// The tension, in percent, that puts the tension point nearest `p`.
    fn tension_towards(&self, p: Point) -> Option<f64> {
        let tunni = self.tunni?;
        let mid = self.from.midpoint(self.to);
        let axis = tunni - mid;
        let len2 = axis.hypot2();
        if !len2.is_finite() || len2 <= 1e-12 {
            return None;
        }
        let t = 100.0 * (p - mid).dot(axis) / len2;
        t.is_finite().then(|| ((t * 10.0).round() / 10.0).clamp(MIN_TENSION, MAX_TENSION))
    }
}

/// The node kind each node acts as: the ends of an open contour act as corners.
fn effective(modes: &[RapidMode], i: usize, closed: bool) -> RapidMode {
    if !closed && (i == 0 || i + 1 == modes.len()) {
        return RapidMode::Corner;
    }
    modes.get(i).copied().unwrap_or(RapidMode::Corner)
}

fn unit(v: Vec2) -> Option<Vec2> {
    let len = v.hypot();
    (len.is_finite() && len > 1e-9).then_some(v / len)
}

/// The segments through `nodes`, each curve at the tension `tension(segment)` (percent).
pub fn segments(nodes: &[(Point, RapidMode)], closed: bool, tension: impl Fn(usize) -> f64) -> Vec<RapidSegment> {
    let n = nodes.len();
    let count = if closed && n >= 2 { n } else { n.saturating_sub(1) };
    if count == 0 {
        return Vec::new();
    }
    let modes: Vec<RapidMode> = nodes.iter().map(|(_, m)| *m).collect();
    let at = |i: usize| nodes.get(i % n).map_or(Point::ZERO, |(p, _)| *p);
    let kind = |i: usize| effective(&modes, i % n, closed);
    let straight = |i: usize| kind(i) != RapidMode::Curve && kind(i + 1) != RapidMode::Curve;
    // The direction of travel at node `i`, when it has one of its own.
    let direction = |i: usize| -> Option<Vec2> {
        let before = if closed { Some((i + n - 1) % n) } else { i.checked_sub(1) };
        let after = (closed || i + 1 < n).then_some(i);
        let (Some(b), Some(a)) = (before, after) else { return None };
        let catmull = || unit(at(i + 1) - at(i + n - 1)).or_else(|| unit(at(i + 1) - at(i)));
        match kind(i) {
            RapidMode::Corner => None,
            RapidMode::Curve => catmull(),
            RapidMode::Tangent => match (straight(b), straight(a)) {
                (true, false) => unit(at(i) - at(i + n - 1)),
                (false, true) => unit(at(i + 1) - at(i)),
                (true, true) => None,
                (false, false) => catmull(),
            },
        }
    };
    (0..count)
        .map(|i| {
            let (from, to) = (at(i), at(i + 1));
            if straight(i) {
                return RapidSegment { from, to, handles: None, tunni: None };
            }
            let chord = to - from;
            let len = chord.hypot();
            let (d0, d1) = match (direction(i), direction(i + 1)) {
                (Some(a), Some(b)) => (a, b),
                (Some(a), None) => (a, unit(to - (from + a * (len / 3.0))).unwrap_or(a)),
                (None, Some(b)) => (unit((to - b * (len / 3.0)) - from).unwrap_or(b), b),
                (None, None) => {
                    let c = unit(chord).unwrap_or(Vec2::new(1.0, 0.0));
                    (c, c)
                }
            };
            let t = tension(i).clamp(MIN_TENSION, MAX_TENSION) / 100.0;
            // from + a·d0 = to − b·d1
            let det = d0.cross(d1);
            let (a, b) = if det.abs() > 1e-9 { (chord.cross(d1) / det, d0.cross(chord) / det) } else { (-1.0, -1.0) };
            let meets = a > 1e-9 && b > 1e-9 && a < MAX_TUNNI_CHORDS * len && b < MAX_TUNNI_CHORDS * len;
            if meets {
                let tunni = from + d0 * a;
                RapidSegment { from, to, handles: Some((from + d0 * (a * t), to - d1 * (b * t))), tunni: Some(tunni) }
            } else {
                let h = len * t / SQRT_2;
                RapidSegment { from, to, handles: Some((from + d0 * h, to - d1 * h)), tunni: None }
            }
        })
        .collect()
}

/// The segments as a path, for the preview.
pub(crate) fn bez_of(segments: &[RapidSegment], closed: bool) -> BezPath {
    let mut out = BezPath::new();
    let Some(first) = segments.first() else { return out };
    out.move_to(first.from);
    for s in segments {
        match s.handles {
            Some((a, b)) => out.curve_to(a, b, s.to),
            None => out.line_to(s.to),
        }
    }
    if closed {
        out.close_path();
    }
    out
}

/// `path` as preview lines.
pub(crate) fn preview_lines(path: &BezPath) -> Vec<Overlay> {
    let mut out = Vec::new();
    let (mut last, mut start) = (Point::ZERO, Point::ZERO);
    flatten(path.iter(), PREVIEW_TOLERANCE, |el| {
        let to = match el {
            PathEl::MoveTo(p) => {
                (last, start) = (p, p);
                return;
            }
            PathEl::LineTo(p) => p,
            PathEl::ClosePath => start,
            PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => p,
        };
        if out.len() < MAX_PREVIEW_LINES {
            out.push(Overlay::Line { kind: OverlayKind::DrawPreview, from: last, to });
        }
        last = to;
    });
    out
}

/// A node as `path.addPath` takes it.
pub(crate) fn node_value(n: &Node) -> Value {
    let kind = match n.kind {
        NodeKind::Line => "line",
        NodeKind::Curve => "curve",
        NodeKind::QCurve => "qcurve",
        NodeKind::OffCurve => "offcurve",
    };
    let mut v = json!({"x": n.x, "y": n.y, "type": kind});
    if n.smooth
        && let Some(o) = v.as_object_mut()
    {
        o.insert("smooth".into(), json!(true));
    }
    v
}

/// A press: on a node (to move it, or to close the contour on the first one) or on a tension
/// point.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Press {
    Node { index: usize, close: bool },
    Tension { segment: usize },
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Held {
    press: Press,
    start: Point,
    moved: bool,
}

/// The Rapid tool.
#[derive(Clone, Debug)]
pub struct RapidTool {
    nodes: Vec<(Point, RapidMode)>,
    /// The tension of each segment set by dragging its tension point, by its first node.
    tensions: Vec<Option<f64>>,
    latched: Option<RapidMode>,
    default_tension: f64,
    held: Option<Held>,
    /// The command target and the number of shapes of the layer, from the last event.
    target: Map<String, Value>,
    shapes: usize,
}

impl Default for RapidTool {
    fn default() -> Self {
        Self { nodes: Vec::new(), tensions: Vec::new(), latched: None, default_tension: DEFAULT_TENSION, held: None, target: Map::new(), shapes: 0 }
    }
}

impl RapidTool {
    /// Latches the kind of node a plain click adds; `None` returns to modifier keys.
    pub fn set_mode(&mut self, mode: Option<RapidMode>) {
        self.latched = mode;
    }

    pub fn mode(&self) -> Option<RapidMode> {
        self.latched
    }

    /// The tension the preview draws with: the master's `curveTension` parameter. Values outside
    /// 1 to 100 percent use [`DEFAULT_TENSION`].
    pub fn set_default_tension(&mut self, tension: f64) {
        self.default_tension = if (MIN_TENSION..=MAX_TENSION).contains(&tension) { tension } else { DEFAULT_TENSION };
    }

    /// The nodes placed so far.
    pub fn nodes(&self) -> &[(Point, RapidMode)] {
        &self.nodes
    }

    /// The segments of the contour as drawn now (open).
    pub fn current_segments(&self) -> Vec<RapidSegment> {
        self.segments(false)
    }

    fn segments(&self, closed: bool) -> Vec<RapidSegment> {
        segments(&self.nodes, closed, |i| self.tensions.get(i).copied().flatten().unwrap_or(self.default_tension))
    }

    fn mode_for(&self, m: Modifiers) -> RapidMode {
        if let Some(mode) = self.latched {
            return mode;
        }
        match (m.cmd, m.alt) {
            (true, true) => RapidMode::Tangent,
            (false, true) => RapidMode::Corner,
            _ => RapidMode::Curve,
        }
    }

    fn press_at(&self, cx: &Context<'_>, p: Point) -> Option<Press> {
        let tolerance = cx.view.units(cx.settings.hit_tolerance);
        let near = |q: Point| dist(q, p) <= tolerance;
        if self.nodes.len() >= 3 && self.nodes.first().is_some_and(|(q, _)| near(*q)) {
            return Some(Press::Node { index: 0, close: true });
        }
        if let Some(segment) = self.segments(false).iter().position(|s| s.tension_point().is_some_and(near)) {
            return Some(Press::Tension { segment });
        }
        let index = self.nodes.iter().position(|(q, _)| near(*q))?;
        Some(Press::Node { index, close: false })
    }

    /// Adds the contour (closed or open) and starts a new one. Nothing is added when there are
    /// too few nodes.
    fn finish(&mut self, closed: bool) -> Output {
        let nodes = std::mem::take(&mut self.nodes);
        let tensions = std::mem::take(&mut self.tensions);
        self.held = None;
        let enough = if closed { nodes.len() >= 3 } else { nodes.len() >= 2 };
        if !enough {
            return Output::handled();
        }
        let tension_of = |i: usize| tensions.get(i).copied().flatten();
        let segs = segments(&nodes, closed, |i| tension_of(i).unwrap_or(self.default_tension));
        match commit(&self.target, self.shapes, &segs, closed, |i| tension_of(i).is_none()) {
            Some(invocation) => Output::with(Action::Run { invocation }),
            None => Output::handled(),
        }
    }
}

/// The invocation that adds the contour of `segs` as shape `shape`: `path.addPath`, then
/// `path.setCurveTension` at the master's tension for the curve segments with a Tunni point for
/// which `follow_master` holds.
fn commit(
    target: &Map<String, Value>,
    shape: usize,
    segs: &[RapidSegment],
    closed: bool,
    follow_master: impl Fn(usize) -> bool,
) -> Option<Invocation> {
    let mut nodes: Vec<Node> = Vec::new();
    let count = segs.len();
    for (i, s) in segs.iter().enumerate() {
        // The node that starts segment i; a closed contour's first node ends the last segment.
        let before = if i == 0 { if closed { segs.last() } else { None } } else { segs.get(i - 1) };
        let kind = if before.is_some_and(|b| b.handles.is_some()) { NodeKind::Curve } else { NodeKind::Line };
        nodes.push(Node::new(s.from.x, s.from.y, kind));
        if let Some((a, b)) = s.handles {
            nodes.push(Node::new(a.x, a.y, NodeKind::OffCurve));
            nodes.push(Node::new(b.x, b.y, NodeKind::OffCurve));
        }
        if !closed && i + 1 == count {
            nodes.push(Node::new(s.to.x, s.to.y, if s.handles.is_some() { NodeKind::Curve } else { NodeKind::Line }));
        }
    }
    // The index of each segment's end node, now that every node has its place.
    let on_curve: Vec<usize> = nodes.iter().enumerate().filter(|(_, n)| n.on_curve()).map(|(i, _)| i).collect();
    let ends: Vec<usize> = (0..count)
        .map(|i| if closed && i + 1 == count { on_curve.first() } else { on_curve.get(i + 1) })
        .map(|end| end.copied().unwrap_or(0))
        .collect();
    mark_smooth(&mut nodes, closed);
    let list: Vec<Value> = nodes.iter().map(node_value).collect();
    let add = Invocation::new("path.addPath", params(target, json!({"closed": closed, "nodes": list})));
    let master: Vec<[usize; 2]> =
        segs.iter().zip(&ends).enumerate().filter(|(i, (s, _))| s.tunni.is_some() && follow_master(*i)).map(|(_, (_, e))| [shape, *e]).collect();
    if master.is_empty() {
        return Some(add);
    }
    let tension = Invocation::new("path.setCurveTension", params(target, json!({"segments": master})));
    let steps: Vec<Value> = [add, tension].into_iter().map(|i| json!({"command": i.id, "params": i.params})).collect();
    Some(Invocation::new("command.batch", json!({"label": "Rapid", "steps": steps})))
}

/// Marks smooth the on-curve nodes whose handles (or straight sides) leave in one direction.
fn mark_smooth(nodes: &mut [Node], closed: bool) {
    let n = nodes.len();
    let flags: Vec<bool> = (0..n)
        .map(|i| {
            let Some(node) = nodes.get(i) else { return false };
            if !node.on_curve() || (!closed && (i == 0 || i + 1 == n)) {
                return false;
            }
            let (Some(prev), Some(next)) = (nodes.get((i + n - 1) % n), nodes.get((i + 1) % n)) else { return false };
            let (a, b) = (node.point() - prev.point(), next.point() - node.point());
            let scale = a.hypot() * b.hypot();
            scale > 0.0 && a.dot(b) > 0.0 && a.cross(b).abs() <= 1e-6 * scale
        })
        .collect();
    for (node, smooth) in nodes.iter_mut().zip(flags) {
        node.smooth = smooth;
    }
}

impl Tool for RapidTool {
    fn id(&self) -> &'static str {
        "rapid"
    }

    fn pointer(&mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        if !finite(ev.pos) {
            return Output::default();
        }
        self.target = cx.scene.target();
        self.shapes = cx.scene.layer.shapes.len();
        self.set_default_tension(cx.settings.curve_tension);
        match ev.kind {
            PointerKind::Down => {
                if cx.scene.locked {
                    return Output::handled();
                }
                let press = match self.press_at(cx, ev.pos) {
                    Some(press) => press,
                    None if self.nodes.len() < MAX_RAPID_NODES => {
                        self.nodes.push((ev.pos, self.mode_for(ev.mods)));
                        if self.nodes.len() >= 2 {
                            self.tensions.push(None);
                        }
                        Press::Node { index: self.nodes.len() - 1, close: false }
                    }
                    None => return Output::handled(),
                };
                self.held = Some(Held { press, start: ev.pos, moved: false });
                Output::handled()
            }
            PointerKind::Drag => {
                let Some(mut held) = self.held else { return Output::default() };
                if !held.moved && dist(cx.view.to_screen(held.start), cx.view.to_screen(ev.pos)) < cx.settings.drag_threshold {
                    return Output::handled();
                }
                held.moved = true;
                match held.press {
                    Press::Node { index, .. } => {
                        if let Some(node) = self.nodes.get_mut(index) {
                            node.0 = ev.pos;
                        }
                    }
                    Press::Tension { segment } => {
                        let t = self.segments(false).get(segment).and_then(|s| s.tension_towards(ev.pos));
                        if let (Some(t), Some(slot)) = (t, self.tensions.get_mut(segment)) {
                            *slot = Some(t);
                        }
                    }
                }
                self.held = Some(held);
                Output::handled()
            }
            PointerKind::Up => {
                let Some(held) = self.held.take() else { return Output::default() };
                match held.press {
                    Press::Node { close: true, .. } if !held.moved => self.finish(true),
                    Press::Tension { segment } if !held.moved => {
                        if let Some(slot) = self.tensions.get_mut(segment) {
                            *slot = None;
                        }
                        Output::handled()
                    }
                    _ => Output::handled(),
                }
            }
            PointerKind::DoubleClick if !self.nodes.is_empty() => self.finish(false),
            PointerKind::Move | PointerKind::DoubleClick => Output::default(),
        }
    }

    fn key(&mut self, _cx: &Context<'_>, ev: &KeyEvent) -> Output {
        if !ev.pressed || self.nodes.is_empty() {
            return Output::default();
        }
        match ev.key {
            Key::Escape | Key::Return => self.finish(false),
            Key::Backspace | Key::Delete if self.held.is_none() => {
                self.nodes.pop();
                self.tensions.truncate(self.nodes.len().saturating_sub(1));
                Output::handled()
            }
            _ => Output::default(),
        }
    }

    /// Leaving the tool adds the contour drawn so far.
    fn cancel(&mut self) -> Output {
        let mut out = self.finish(false);
        out.handled = false;
        out
    }

    fn busy(&self) -> bool {
        self.held.is_some()
    }

    fn overlays(&self) -> Vec<Overlay> {
        let segs = self.segments(false);
        let mut out = preview_lines(&bez_of(&segs, false));
        for s in &segs {
            if let Some(p) = s.tension_point() {
                out.push(Overlay::Marker { kind: OverlayKind::SnapTarget, at: p });
            }
        }
        for (i, (p, _)) in self.nodes.iter().enumerate() {
            let held = matches!(self.held, Some(Held { press: Press::Node { index, .. }, .. }) if index == i);
            out.push(Overlay::Marker { kind: if held { OverlayKind::Hover } else { OverlayKind::DrawHandle }, at: *p });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use fontcraft_model::{Layer, Path, Shape};

    use super::*;
    use crate::scene::Scene;
    use crate::{Selection, ToolSettings, View};

    fn layer() -> Layer {
        let mut layer = Layer::new("m01", 600.0);
        layer.shapes.push(Shape::Path(Path::new(
            true,
            vec![Node::new(0.0, 0.0, NodeKind::Line), Node::new(10.0, 0.0, NodeKind::Line), Node::new(0.0, 10.0, NodeKind::Line)],
        )));
        layer
    }

    fn event(tool: &mut RapidTool, layer: &Layer, kind: PointerKind, x: f64, y: f64, mods: Modifiers) -> Output {
        let scene = Scene::new("A", "m01", layer);
        let selection = Selection::new();
        let cx = Context { scene: &scene, view: View::default(), selection: &selection, settings: ToolSettings::default() };
        tool.pointer(&cx, &PointerEvent::new(kind, x, y, mods))
    }

    fn key(tool: &mut RapidTool, layer: &Layer, k: Key) -> Output {
        let scene = Scene::new("A", "m01", layer);
        let selection = Selection::new();
        let cx = Context { scene: &scene, view: View::default(), selection: &selection, settings: ToolSettings::default() };
        tool.key(&cx, &KeyEvent::press(k, Modifiers::NONE))
    }

    fn click(tool: &mut RapidTool, layer: &Layer, x: f64, y: f64, mods: Modifiers) -> Output {
        event(tool, layer, PointerKind::Down, x, y, mods);
        event(tool, layer, PointerKind::Up, x, y, mods)
    }

    fn run(out: &Output) -> Invocation {
        match out.actions.as_slice() {
            [Action::Run { invocation }] => invocation.clone(),
            other => panic!("expected one run, got {other:?}"),
        }
    }

    /// The tension of the cubic `p0 p1 p2 p3` at its first handle, in percent.
    fn tension_of(p: [Point; 4]) -> f64 {
        let (d0, d1) = (p[1] - p[0], p[3] - p[2]);
        let a = (p[3] - p[0]).cross(d1) / d0.cross(d1);
        100.0 / a
    }

    #[test]
    fn clicks_give_a_closed_contour_at_the_masters_curve_tension() {
        let layer = layer();
        let mut tool = RapidTool::default();
        for (x, y) in [(100.0, 0.0), (0.0, 100.0), (-100.0, 0.0), (0.0, -100.0)] {
            assert!(click(&mut tool, &layer, x, y, Modifiers::NONE).actions.is_empty());
        }
        // Clicking the first node closes the contour and adds it.
        let inv = run(&click(&mut tool, &layer, 100.0, 0.0, Modifiers::NONE));
        assert!(tool.nodes().is_empty());
        assert_eq!(inv.id, "command.batch");
        let add = &inv.params["steps"][0];
        assert_eq!(add["command"], "path.addPath");
        assert_eq!(add["params"]["closed"], true);
        let nodes = add["params"]["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), 12);
        // Node 0 ends the closing curve; every on-curve node is smooth.
        assert_eq!(nodes[0], json!({"x": 100.0, "y": 0.0, "type": "curve", "smooth": true}));
        assert_eq!(nodes[1]["type"], "offcurve");
        let pt = |i: usize| Point::new(nodes[i % 12]["x"].as_f64().unwrap(), nodes[i % 12]["y"].as_f64().unwrap());
        // Drawn at the default tension: a circle's handles.
        assert!((pt(1) - Point::new(100.0, 55.0)).hypot() < 1e-9, "{:?}", pt(1));
        assert!((tension_of([pt(0), pt(1), pt(2), pt(3)]) - DEFAULT_TENSION).abs() < 1e-9);
        // Then every segment is set to the master's tension: no explicit value is sent.
        let tension = &inv.params["steps"][1];
        assert_eq!(tension["command"], "path.setCurveTension");
        assert_eq!(tension["params"], json!({"glyph": "A", "layer": "m01", "segments": [[1, 3], [1, 6], [1, 9], [1, 0]]}));
    }

    #[test]
    fn corners_give_lines_and_tangent_nodes_join_lines_smoothly() {
        let layer = layer();
        let mut tool = RapidTool::default();
        click(&mut tool, &layer, 0.0, 0.0, Modifiers::ALT);
        click(&mut tool, &layer, 200.0, 0.0, Modifiers::CMD.with_alt());
        click(&mut tool, &layer, 300.0, 100.0, Modifiers::NONE);
        click(&mut tool, &layer, 200.0, 200.0, Modifiers::ALT);
        let inv = run(&key(&mut tool, &layer, Key::Escape));
        let add = &inv.params["steps"][0]["params"];
        assert_eq!(add["closed"], false);
        let types: Vec<&str> = add["nodes"].as_array().unwrap().iter().map(|n| n["type"].as_str().unwrap()).collect();
        assert_eq!(types, ["line", "line", "offcurve", "offcurve", "curve", "offcurve", "offcurve", "curve"]);
        // The tangent node continues the straight segment: its handle points along +x.
        let n = &add["nodes"].as_array().unwrap()[2];
        assert!(n["y"].as_f64().unwrap().abs() < 1e-9 && n["x"].as_f64().unwrap() > 200.0, "{n}");
        assert_eq!(add["nodes"][1]["smooth"], true);
    }

    #[test]
    fn a_dragged_tension_point_keeps_its_segment_tension() {
        let layer = layer();
        let mut tool = RapidTool::default();
        for (x, y) in [(100.0, 0.0), (0.0, 100.0), (-100.0, 0.0)] {
            click(&mut tool, &layer, x, y, Modifiers::NONE);
        }
        // Open: the ends aim at their neighbour's handle, so segment 0 has a Tunni point.
        let seg = tool.current_segments()[0];
        let tp = seg.tension_point().unwrap();
        let tunni = seg.tunni.unwrap();
        let mid = seg.from.midpoint(seg.to);
        let target = mid + (tunni - mid) * 0.8;
        event(&mut tool, &layer, PointerKind::Down, tp.x, tp.y, Modifiers::NONE);
        event(&mut tool, &layer, PointerKind::Drag, target.x, target.y, Modifiers::NONE);
        event(&mut tool, &layer, PointerKind::Up, target.x, target.y, Modifiers::NONE);
        assert_eq!(tool.nodes().len(), 3, "dragging a tension point adds no node");
        let seg = tool.current_segments()[0];
        let (h1, _) = seg.handles.unwrap();
        assert!(((h1 - seg.from).hypot() / (tunni - seg.from).hypot() - 0.8).abs() < 1e-3);
        let inv = run(&tool.cancel());
        // Only segment 1 follows the master; segment 0 keeps its 80 %.
        assert_eq!(inv.params["steps"][1]["params"]["segments"], json!([[1, 6]]));
    }

    #[test]
    fn backspace_removes_the_last_node_and_bad_input_adds_nothing() {
        let layer = layer();
        let mut tool = RapidTool::default();
        click(&mut tool, &layer, 0.0, 0.0, Modifiers::NONE);
        click(&mut tool, &layer, 100.0, 0.0, Modifiers::NONE);
        assert!(key(&mut tool, &layer, Key::Backspace).handled);
        assert_eq!(tool.nodes().len(), 1);
        for kind in [PointerKind::Down, PointerKind::Drag, PointerKind::Up] {
            assert!(event(&mut tool, &layer, kind, f64::NAN, 0.0, Modifiers::NONE).actions.is_empty());
        }
        // One node is not a contour: leaving the tool adds nothing.
        assert!(tool.cancel().actions.is_empty());
        assert!(tool.nodes().is_empty());
    }
}
