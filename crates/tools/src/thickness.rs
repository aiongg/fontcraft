//! The Thickness tool: edits the width of a stroked path on canvas, for the whole path or node
//! by node (`thickness` row, `docs/spec/90-secondary-references.md`).
//!
//! Each on-curve node of a stroked path shows two control points, one on each edge of the
//! stroke, across the path at the node. Dragging one changes the widths:
//!
//! | Gesture | Command | Change |
//! |---|---|---|
//! | drag | `path.setNodeWidths` | the dragged side at the node |
//! | Alt+drag | `path.setNodeWidths` | both sides at the node |
//! | Shift+drag | `path.setStroke` (and `path.setNodeWidths`) | the path's width; nodes with their own widths change on both sides alike |
//! | Shift+Alt+drag | `path.setStroke` (and `path.setNodeWidths`) | the path's stroke position; nodes with their own widths move sideways, keeping their width |
//! | Backspace or Delete | `path.setNodeWidths {clear}` | the picked nodes take the path's width again |
//!
//! A click picks one control point, Shift+click adds or removes one, and a marquee from empty
//! canvas picks every control point inside it (Shift adds them). A drag on a picked control point
//! changes every picked one by the same amount. The drag is one interaction: each pointer move
//! previews the command on the state before the drag, so the values sent are absolute and taken
//! from the widths the drag started with.

use std::collections::BTreeMap;

use fontcraft_geom::{Point, Vec2};
use fontcraft_model::node_width::{self, MAX_NODE_WIDTH};
use fontcraft_model::{Node, Path};
use serde_json::{Map, Value, json};

use crate::MAX_SELECTION_LIST;
use crate::event::{Key, KeyEvent, PointerEvent, PointerKind};
use crate::geometry::{dist, finite};
use crate::output::{Action, Invocation, Output, Overlay, OverlayKind};
use crate::scene::Scene;
use crate::tool::{Context, Tool};

/// Narrowest stroke a Shift+drag leaves, in font units.
pub const MIN_PATH_WIDTH: f64 = 1.0;

/// Which edge of the stroke a control point sits on, looking along the path direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Side {
    Left,
    Right,
}

/// A control point: one side of one node's stroke.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ControlPoint {
    pub shape: usize,
    pub node: usize,
    pub side: Side,
}

/// A control point where it is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub cp: ControlPoint,
    /// The node the control point belongs to.
    pub node_at: Point,
    /// The control point itself, on the stroke's edge.
    pub at: Point,
    /// The unit direction from the node towards this side's edge.
    pub normal: Vec2,
    /// The width on this side: the distance from the node to the edge.
    pub width: f64,
    /// The node has its own widths (not the path's).
    pub own: bool,
}

/// The control points of every stroked, unlocked path of the scene that can be edited, at most
/// [`MAX_SELECTION_LIST`].
pub fn control_points(scene: &Scene<'_>) -> Vec<Placed> {
    let mut out = Vec::new();
    for (shape, path) in scene.paths() {
        if path.info.locked {
            continue;
        }
        let Some((width, position)) = node_width::path_stroke(path, scene.glyphs_version) else { continue };
        for (i, node) in path.nodes.iter().enumerate() {
            if !node.on_curve() || node.locked {
                continue;
            }
            let Some(t) = node_tangent(path, i) else { continue };
            let n = Vec2::new(-t.y, t.x);
            let [l, r] = node_width::resolved(node, width, position);
            let own = node_width::get(node).is_some();
            let at = node.point();
            for (side, normal, w) in [(Side::Left, n, l), (Side::Right, -n, r)] {
                if out.len() >= MAX_SELECTION_LIST {
                    return out;
                }
                out.push(Placed { cp: ControlPoint { shape, node: i, side }, node_at: at, at: at + normal * w, normal, width: w, own });
            }
        }
    }
    out
}

/// The direction of travel through on-curve node `i`: the mean of the directions towards it and
/// away from it, from the nearest nodes that do not sit on it.
fn node_tangent(path: &Path, i: usize) -> Option<Vec2> {
    let here = path.nodes.get(i)?.point();
    let count = path.nodes.len();
    let look = |forward: bool| -> Option<Vec2> {
        for step in 1..count {
            if !path.closed && (if forward { i + step >= count } else { step > i }) {
                return None;
            }
            let j = if forward { (i + step) % count } else { (i + count - step) % count };
            let p = path.nodes.get(j)?.point();
            let v = if forward { p - here } else { here - p };
            let len = v.hypot();
            if len > 1e-9 && len.is_finite() {
                return Some(v / len);
            }
        }
        None
    };
    let (into, out) = (look(false), look(true));
    let t = match (into, out) {
        (Some(a), Some(b)) => {
            let m = a + b;
            if m.hypot() > 1e-6 { m / m.hypot() } else { b }
        }
        (Some(a), None) => a,
        (None, Some(b)) => b,
        (None, None) => return None,
    };
    t.is_finite().then_some(t)
}

fn hit(shown: &[Placed], cx: &Context<'_>, at: Point) -> Option<Placed> {
    let tolerance = cx.view.units(cx.settings.hit_tolerance);
    shown.iter().map(|p| (dist(p.at, at), p)).filter(|(d, _)| *d <= tolerance).min_by(|a, b| a.0.total_cmp(&b.0)).map(|(_, p)| *p)
}

/// What a drag changes, from its modifier keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    OneSide,
    BothSides,
    WholePath,
    Shift,
}

impl Mode {
    fn of(shift: bool, alt: bool) -> Self {
        match (shift, alt) {
            (false, false) => Mode::OneSide,
            (false, true) => Mode::BothSides,
            (true, false) => Mode::WholePath,
            (true, true) => Mode::Shift,
        }
    }
}

/// A path as the drag found it: its stroke and the widths of its on-curve nodes.
#[derive(Clone, Debug)]
struct PathBase {
    width: f64,
    position: f64,
    /// Node index → (widths drawn with, has its own).
    nodes: BTreeMap<usize, ([f64; 2], bool)>,
}

#[derive(Clone, Debug)]
struct Drag {
    grab: Placed,
    group: Vec<ControlPoint>,
    base: BTreeMap<usize, PathBase>,
    began: bool,
}

#[derive(Clone, Copy, Debug)]
struct Marquee {
    start: Point,
    current: Point,
    add: bool,
    dragged: bool,
}

#[derive(Clone, Debug, Default)]
enum State {
    #[default]
    Idle,
    Pressed {
        grab: Placed,
        start: Point,
    },
    Dragging(Drag),
    Marquee(Marquee),
}

/// The Thickness tool.
#[derive(Clone, Debug, Default)]
pub struct ThicknessTool {
    state: State,
    /// The picked control points.
    picked: Vec<ControlPoint>,
    /// The control points as the last event saw them, for the overlays.
    shown: Vec<Placed>,
    hover: Option<ControlPoint>,
}

impl ThicknessTool {
    /// The picked control points.
    pub fn picked(&self) -> &[ControlPoint] {
        &self.picked
    }

    fn refresh(&mut self, cx: &Context<'_>) {
        self.shown = control_points(cx.scene);
        let shown = &self.shown;
        self.picked.retain(|cp| shown.iter().any(|p| p.cp == *cp));
    }
}

impl Tool for ThicknessTool {
    fn id(&self) -> &'static str {
        "thickness"
    }

    fn pointer(&mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        if !finite(ev.pos) {
            return Output::default();
        }
        self.refresh(cx);
        match ev.kind {
            PointerKind::Move => {
                self.hover = hit(&self.shown, cx, ev.pos).map(|p| p.cp);
                Output::default()
            }
            PointerKind::Down => {
                self.state = match hit(&self.shown, cx, ev.pos) {
                    Some(grab) => State::Pressed { grab, start: ev.pos },
                    None => State::Marquee(Marquee { start: ev.pos, current: ev.pos, add: ev.mods.shift, dragged: false }),
                };
                Output::handled()
            }
            PointerKind::Drag => self.drag(cx, ev),
            PointerKind::Up => self.release(cx, ev),
            PointerKind::DoubleClick => Output::default(),
        }
    }

    fn key(&mut self, cx: &Context<'_>, ev: &KeyEvent) -> Output {
        if !ev.pressed {
            return Output::default();
        }
        match ev.key {
            Key::Escape if self.busy() => self.cancel_with_handled(),
            Key::Escape if !self.picked.is_empty() => {
                self.picked.clear();
                Output::handled()
            }
            Key::Backspace | Key::Delete if !self.busy() && !self.picked.is_empty() => {
                self.refresh(cx);
                match clear_invocation(cx.scene, &self.shown, &self.picked) {
                    Some(invocation) => Output::with(Action::Run { invocation }),
                    None => Output::handled(),
                }
            }
            _ => Output::default(),
        }
    }

    fn cancel(&mut self) -> Output {
        let state = std::mem::take(&mut self.state);
        self.picked.clear();
        self.hover = None;
        match state {
            State::Dragging(d) if d.began => Output::with(Action::Cancel),
            _ => Output::default(),
        }
    }

    fn busy(&self) -> bool {
        !matches!(self.state, State::Idle)
    }

    fn overlays(&self) -> Vec<Overlay> {
        let mut out = Vec::new();
        for p in &self.shown {
            out.push(Overlay::Line { kind: OverlayKind::DrawHandle, from: p.node_at, to: p.at });
            let kind = if self.picked.contains(&p.cp) {
                OverlayKind::SnapTarget
            } else if self.hover == Some(p.cp) {
                OverlayKind::Hover
            } else {
                OverlayKind::DrawHandle
            };
            out.push(Overlay::Marker { kind, at: p.at });
        }
        if let State::Marquee(m) = self.state
            && m.dragged
        {
            let (a, b) = (m.start, m.current);
            out.push(Overlay::Polygon { kind: OverlayKind::Marquee, points: vec![a, Point::new(b.x, a.y), b, Point::new(a.x, b.y)] });
        }
        out
    }
}

impl ThicknessTool {
    fn cancel_with_handled(&mut self) -> Output {
        let state = std::mem::take(&mut self.state);
        let mut out = Output::handled();
        if let State::Dragging(d) = state
            && d.began
        {
            out.push(Action::Cancel);
        }
        out
    }

    fn drag(&mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        let threshold = |start: Point| dist(cx.view.to_screen(start), cx.view.to_screen(ev.pos)) >= cx.settings.drag_threshold;
        match &mut self.state {
            State::Idle => Output::default(),
            State::Marquee(m) => {
                m.current = ev.pos;
                if threshold(m.start) {
                    m.dragged = true;
                }
                Output::handled()
            }
            State::Pressed { grab, start } => {
                let (grab, start) = (*grab, *start);
                if !threshold(start) || cx.scene.locked {
                    return Output::handled();
                }
                if !self.picked.contains(&grab.cp) {
                    self.picked = vec![grab.cp];
                }
                let group = self.picked.clone();
                let base = capture(cx.scene, &group);
                let mut drag = Drag { grab, group, base, began: false };
                let mut out = Output::handled();
                if let Some(invocation) = drag_invocation(cx.scene, &drag, ev) {
                    out.push(Action::Begin { label: "Thickness".into() });
                    out.push(Action::Preview { invocation });
                    drag.began = true;
                }
                self.state = State::Dragging(drag);
                out
            }
            State::Dragging(drag) => {
                let mut out = Output::handled();
                if let Some(invocation) = drag_invocation(cx.scene, drag, ev) {
                    if !drag.began {
                        out.push(Action::Begin { label: "Thickness".into() });
                        drag.began = true;
                    }
                    out.push(Action::Preview { invocation });
                }
                out
            }
        }
    }

    fn release(&mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        match std::mem::take(&mut self.state) {
            State::Idle => Output::default(),
            State::Pressed { grab, .. } => {
                if ev.mods.shift {
                    match self.picked.iter().position(|cp| *cp == grab.cp) {
                        Some(i) => {
                            self.picked.remove(i);
                        }
                        None => self.picked.push(grab.cp),
                    }
                } else {
                    self.picked = vec![grab.cp];
                }
                Output::handled()
            }
            State::Dragging(drag) => {
                let mut out = Output::handled();
                if drag.began {
                    if let Some(invocation) = drag_invocation(cx.scene, &drag, ev) {
                        out.push(Action::Preview { invocation });
                    }
                    out.push(Action::Commit);
                }
                out
            }
            State::Marquee(m) => {
                if !m.dragged {
                    if !m.add {
                        self.picked.clear();
                    }
                    return Output::handled();
                }
                let (lo, hi) =
                    (Point::new(m.start.x.min(ev.pos.x), m.start.y.min(ev.pos.y)), Point::new(m.start.x.max(ev.pos.x), m.start.y.max(ev.pos.y)));
                let inside = self.shown.iter().filter(|p| p.at.x >= lo.x && p.at.x <= hi.x && p.at.y >= lo.y && p.at.y <= hi.y).map(|p| p.cp);
                if !m.add {
                    self.picked.clear();
                }
                for cp in inside {
                    if !self.picked.contains(&cp) && self.picked.len() < MAX_SELECTION_LIST {
                        self.picked.push(cp);
                    }
                }
                Output::handled()
            }
        }
    }
}

/// The paths of `group` as they are now: stroke and every on-curve node's widths.
fn capture(scene: &Scene<'_>, group: &[ControlPoint]) -> BTreeMap<usize, PathBase> {
    let mut out = BTreeMap::new();
    for cp in group {
        if out.contains_key(&cp.shape) {
            continue;
        }
        let Some(path) = scene.path(cp.shape) else { continue };
        let Some((width, position)) = node_width::path_stroke(path, scene.glyphs_version) else { continue };
        let nodes = path
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.on_curve())
            .map(|(i, n): (usize, &Node)| (i, (node_width::resolved(n, width, position), node_width::get(n).is_some())))
            .collect();
        out.insert(cp.shape, PathBase { width, position, nodes });
    }
    out
}

/// A width rounded to a hundredth of a unit and kept within what a node may hold.
fn tidy(w: f64) -> f64 {
    ((w * 100.0).round() / 100.0).clamp(0.0, MAX_NODE_WIDTH)
}

/// The command a drag to `ev.pos` runs, from the widths the drag started with.
fn drag_invocation(scene: &Scene<'_>, drag: &Drag, ev: &PointerEvent) -> Option<Invocation> {
    let g = drag.grab;
    let reach = (ev.pos - g.node_at).dot(g.normal);
    if !reach.is_finite() {
        return None;
    }
    let delta = reach.max(0.0) - g.width;
    let mode = Mode::of(ev.mods.shift, ev.mods.alt);
    // Node → new [left, right].
    let mut nodes: BTreeMap<(usize, usize), [f64; 2]> = BTreeMap::new();
    let mut strokes: Vec<(usize, Map<String, Value>)> = Vec::new();
    match mode {
        Mode::OneSide | Mode::BothSides => {
            for cp in &drag.group {
                let Some(([l, r], _)) = drag.base.get(&cp.shape).and_then(|b| b.nodes.get(&cp.node)).copied() else { continue };
                let key = (cp.shape, cp.node);
                let [cl, cr] = nodes.get(&key).copied().unwrap_or([l, r]);
                let new = match (mode, cp.side) {
                    (Mode::BothSides, _) => [tidy(l + delta), tidy(r + delta)],
                    (_, Side::Left) => [tidy(l + delta), cr],
                    (_, Side::Right) => [cl, tidy(r + delta)],
                };
                nodes.insert(key, new);
            }
        }
        Mode::WholePath | Mode::Shift => {
            // Shift+Alt moves the stroke towards the grabbed side by `delta`.
            let toward_left = if g.cp.side == Side::Left { delta } else { -delta };
            for (&shape, base) in &drag.base {
                let mut stroke = Map::new();
                if mode == Mode::WholePath {
                    stroke.insert(
                        "width".into(),
                        json!(((base.width + 2.0 * delta) * 100.0).round().clamp(MIN_PATH_WIDTH * 100.0, MAX_NODE_WIDTH * 100.0) / 100.0),
                    );
                } else if base.width > 0.0 {
                    let position = (base.position - 2.0 * toward_left / base.width).clamp(-1.0, 1.0);
                    stroke.insert("position".into(), json!((position * 10_000.0).round() / 100.0));
                }
                strokes.push((shape, stroke));
                for (&node, &([l, r], own)) in &base.nodes {
                    if !own {
                        continue;
                    }
                    let new = if mode == Mode::WholePath {
                        [tidy(l + delta), tidy(r + delta)]
                    } else {
                        let s = toward_left.clamp(-l, r);
                        [tidy(l + s), tidy(r - s)]
                    };
                    nodes.insert((shape, node), new);
                }
            }
        }
    }
    let mut steps = Vec::new();
    for (shape, stroke) in strokes {
        let mut params = scene.target();
        params.insert("shapes".into(), json!([shape]));
        params.extend(stroke);
        steps.push(Invocation::new("path.setStroke", Value::Object(params)));
    }
    if !nodes.is_empty() {
        let mut params = scene.target();
        params.insert("points".into(), json!(nodes.keys().map(|(s, n)| [*s, *n]).collect::<Vec<_>>()));
        params.insert("left".into(), json!(nodes.values().map(|[l, _]| *l).collect::<Vec<_>>()));
        params.insert("right".into(), json!(nodes.values().map(|[_, r]| *r).collect::<Vec<_>>()));
        steps.push(Invocation::new("path.setNodeWidths", Value::Object(params)));
    }
    one_step(steps)
}

/// One invocation for `steps`: the step itself when there is one, else a batch.
fn one_step(mut steps: Vec<Invocation>) -> Option<Invocation> {
    if steps.len() <= 1 {
        return steps.pop();
    }
    let steps: Vec<Value> = steps.into_iter().map(|i| json!({"command": i.id, "params": i.params})).collect();
    Some(Invocation::new("command.batch", json!({"label": "Thickness", "steps": steps})))
}

/// `path.setNodeWidths {clear}` for the picked nodes that have their own widths, or `None` when
/// none has.
fn clear_invocation(scene: &Scene<'_>, shown: &[Placed], picked: &[ControlPoint]) -> Option<Invocation> {
    let mut nodes: Vec<[usize; 2]> = shown.iter().filter(|p| p.own && picked.contains(&p.cp)).map(|p| [p.cp.shape, p.cp.node]).collect();
    nodes.sort_unstable();
    nodes.dedup();
    if nodes.is_empty() || scene.locked {
        return None;
    }
    let mut params = scene.target();
    params.insert("points".into(), json!(nodes));
    params.insert("clear".into(), json!(true));
    Some(Invocation::new("path.setNodeWidths", Value::Object(params)))
}

#[cfg(test)]
mod tests {
    use fontcraft_model::{Dimension, Layer, NodeKind, Shape};
    use serde_json::json;

    use super::*;
    use crate::{Modifiers, Selection, ToolSettings, View};

    /// A layer with one open stroked path from (0, 0) to (100, 0), 20 units wide, centred. Left
    /// of the path direction is up, so node 0's control points sit at (0, 10) and (0, −10).
    fn layer(own: Option<[f64; 2]>) -> Layer {
        let mut path = Path::new(false, vec![Node::new(0.0, 0.0, NodeKind::Line), Node::new(100.0, 0.0, NodeKind::Line)]);
        path.info.attrs.stroke_width = Some(Dimension::Number(20.0));
        if let Some(w) = own {
            node_width::set(&mut path.nodes[1], Some(w)).unwrap();
        }
        let mut layer = Layer::new("m01", 600.0);
        layer.shapes.push(Shape::Path(path));
        layer
    }

    fn event(tool: &mut ThicknessTool, layer: &Layer, kind: PointerKind, x: f64, y: f64, mods: Modifiers) -> Output {
        let scene = Scene::new("A", "m01", layer);
        let selection = Selection::new();
        let cx = Context { scene: &scene, view: View::default(), selection: &selection, settings: ToolSettings::default() };
        tool.pointer(&cx, &PointerEvent::new(kind, x, y, mods))
    }

    fn key(tool: &mut ThicknessTool, layer: &Layer, k: Key) -> Output {
        let scene = Scene::new("A", "m01", layer);
        let selection = Selection::new();
        let cx = Context { scene: &scene, view: View::default(), selection: &selection, settings: ToolSettings::default() };
        tool.key(&cx, &KeyEvent::press(k, Modifiers::NONE))
    }

    /// Presses at `from`, drags to `to` and releases there; the actions of the release.
    fn drag(tool: &mut ThicknessTool, layer: &Layer, from: (f64, f64), to: (f64, f64), mods: Modifiers) -> Vec<Action> {
        event(tool, layer, PointerKind::Down, from.0, from.1, mods);
        let moved = event(tool, layer, PointerKind::Drag, to.0, to.1, mods);
        assert_eq!(moved.actions.first(), Some(&Action::Begin { label: "Thickness".into() }));
        event(tool, layer, PointerKind::Up, to.0, to.1, mods).actions
    }

    fn committed(actions: &[Action]) -> Invocation {
        assert_eq!(actions.last(), Some(&Action::Commit));
        match actions.first() {
            Some(Action::Preview { invocation }) => invocation.clone(),
            other => panic!("no preview: {other:?}"),
        }
    }

    #[test]
    fn control_points_sit_on_the_stroke_edges() {
        let layer = layer(Some([30.0, 5.0]));
        let scene = Scene::new("A", "m01", &layer);
        let at: Vec<(usize, Side, Point)> = control_points(&scene).iter().map(|p| (p.cp.node, p.cp.side, p.at)).collect();
        assert_eq!(
            at,
            vec![
                (0, Side::Left, Point::new(0.0, 10.0)),
                (0, Side::Right, Point::new(0.0, -10.0)),
                (1, Side::Left, Point::new(100.0, 30.0)),
                (1, Side::Right, Point::new(100.0, -5.0)),
            ]
        );
    }

    #[test]
    fn shift_drag_changes_the_whole_path_width() {
        let layer = layer(None);
        let mut tool = ThicknessTool::default();
        let inv = committed(&drag(&mut tool, &layer, (0.0, 10.0), (0.0, 25.0), Modifiers::SHIFT));
        assert_eq!(inv, Invocation::new("path.setStroke", json!({"glyph": "A", "layer": "m01", "shapes": [0], "width": 50.0})));
    }

    #[test]
    fn shift_drag_keeps_the_modulation_of_nodes_with_their_own_widths() {
        let layer = layer(Some([30.0, 5.0]));
        let mut tool = ThicknessTool::default();
        let inv = committed(&drag(&mut tool, &layer, (0.0, 10.0), (0.0, 15.0), Modifiers::SHIFT));
        assert_eq!(inv.id, "command.batch");
        assert_eq!(inv.params["steps"][0]["params"]["width"], json!(30.0));
        assert_eq!(inv.params["steps"][1]["command"], json!("path.setNodeWidths"));
        assert_eq!(
            (inv.params["steps"][1]["params"]["left"].clone(), inv.params["steps"][1]["params"]["right"].clone()),
            (json!([35.0]), json!([10.0]))
        );
    }

    #[test]
    fn alt_drag_changes_one_node_on_both_sides() {
        let layer = layer(None);
        let mut tool = ThicknessTool::default();
        let inv = committed(&drag(&mut tool, &layer, (100.0, -10.0), (100.0, -16.0), Modifiers::ALT));
        assert_eq!(
            inv,
            Invocation::new("path.setNodeWidths", json!({"glyph": "A", "layer": "m01", "points": [[0, 1]], "left": [16.0], "right": [16.0]}))
        );
    }

    #[test]
    fn a_plain_drag_changes_the_dragged_side_only() {
        let layer = layer(None);
        let mut tool = ThicknessTool::default();
        let inv = committed(&drag(&mut tool, &layer, (100.0, 10.0), (100.0, 40.0), Modifiers::NONE));
        assert_eq!(inv.params["points"], json!([[0, 1]]));
        assert_eq!((inv.params["left"].clone(), inv.params["right"].clone()), (json!([40.0]), json!([10.0])));
    }

    #[test]
    fn shift_alt_drag_moves_the_stroke_sideways_without_changing_its_width() {
        let layer = layer(Some([30.0, 5.0]));
        let mut tool = ThicknessTool::default();
        // The left control point of node 0 pulled up by 5: the stroke moves 5 units to the left.
        let inv = committed(&drag(&mut tool, &layer, (0.0, 10.0), (0.0, 15.0), Modifiers::SHIFT.with_alt()));
        assert_eq!(inv.id, "command.batch");
        // The command takes the position in percent: −50 is halfway to the left.
        assert_eq!(inv.params["steps"][0]["params"]["position"], json!(-50.0));
        assert!(inv.params["steps"][0]["params"].get("width").is_none());
        // Node 1 keeps its 35 units: 5 more on the left can only take 5 from the right.
        assert_eq!(
            (inv.params["steps"][1]["params"]["left"].clone(), inv.params["steps"][1]["params"]["right"].clone()),
            (json!([35.0]), json!([0.0]))
        );
    }

    #[test]
    fn backspace_clears_the_picked_nodes_own_widths() {
        let layer = layer(Some([30.0, 5.0]));
        let mut tool = ThicknessTool::default();
        event(&mut tool, &layer, PointerKind::Down, 100.0, 30.0, Modifiers::NONE);
        event(&mut tool, &layer, PointerKind::Up, 100.0, 30.0, Modifiers::NONE);
        assert_eq!(tool.picked(), &[ControlPoint { shape: 0, node: 1, side: Side::Left }]);
        let out = key(&mut tool, &layer, Key::Backspace);
        let expected = Invocation::new("path.setNodeWidths", json!({"glyph": "A", "layer": "m01", "points": [[0, 1]], "clear": true}));
        assert_eq!(out.actions, vec![Action::Run { invocation: expected }]);

        // A node with the path's widths has nothing to clear, and the key is still used.
        let plain = self::layer(None);
        let mut tool = ThicknessTool::default();
        event(&mut tool, &plain, PointerKind::Down, 0.0, 10.0, Modifiers::NONE);
        event(&mut tool, &plain, PointerKind::Up, 0.0, 10.0, Modifiers::NONE);
        let out = key(&mut tool, &plain, Key::Backspace);
        assert!(out.handled && out.actions.is_empty());
    }

    #[test]
    fn a_marquee_picks_control_points_and_a_drag_edits_them_together() {
        let layer = layer(None);
        let mut tool = ThicknessTool::default();
        // Around both left control points (y = 10), not the right ones (y = −10).
        event(&mut tool, &layer, PointerKind::Down, -20.0, 0.0, Modifiers::NONE);
        event(&mut tool, &layer, PointerKind::Drag, 120.0, 20.0, Modifiers::NONE);
        assert!(tool.overlays().iter().any(|o| matches!(o, Overlay::Polygon { kind: OverlayKind::Marquee, .. })));
        event(&mut tool, &layer, PointerKind::Up, 120.0, 20.0, Modifiers::NONE);
        assert_eq!(tool.picked().len(), 2);
        assert!(tool.picked().iter().all(|cp| cp.side == Side::Left));

        let inv = committed(&drag(&mut tool, &layer, (100.0, 10.0), (100.0, 14.0), Modifiers::NONE));
        assert_eq!(inv.params["points"], json!([[0, 0], [0, 1]]));
        assert_eq!((inv.params["left"].clone(), inv.params["right"].clone()), (json!([14.0, 14.0]), json!([10.0, 10.0])));
    }

    #[test]
    fn escape_cancels_a_drag_and_bad_input_does_nothing() {
        let layer = layer(None);
        let mut tool = ThicknessTool::default();
        event(&mut tool, &layer, PointerKind::Down, 0.0, 10.0, Modifiers::NONE);
        event(&mut tool, &layer, PointerKind::Drag, 0.0, 30.0, Modifiers::NONE);
        let out = key(&mut tool, &layer, Key::Escape);
        assert!(out.handled && out.actions.contains(&Action::Cancel));
        assert!(!tool.busy());
        for (x, y) in [(f64::NAN, 0.0), (f64::INFINITY, 1.0), (1e300, -1e300)] {
            for kind in [PointerKind::Down, PointerKind::Drag, PointerKind::Up] {
                let out = event(&mut tool, &layer, kind, x, y, Modifiers::SHIFT.with_alt());
                assert!(out.actions.iter().all(|a| !matches!(a, Action::Run { .. })));
            }
        }
    }
}
