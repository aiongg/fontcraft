//! The Select tool (`V`): click, `Shift`-click, marquee, double-click, and dragging the selection.
//!
//! Gestures (`[[tool]] select` and `docs/spec/03a` § 6, § 7, with the M3 answers):
//!
//! | Gesture | Result |
//! |---|---|
//! | click an object | select it; a segment selects its two end nodes |
//! | `Shift`+click | toggle it in the selection |
//! | drag on empty canvas | marquee: nodes, handles and anchors inside; `Shift` toggles them, `Alt` takes on-curve nodes only and components too, `Ctrl` turns the marquee |
//! | drag an object | `selection.move` in one interaction; snapping unless `Ctrl` is held |
//! | `Alt` held at mouse-down | the drag moves a copy (`duplicate: true`) |
//! | `Alt` held at mouse-down, with nothing to copy (lone nodes, no whole segment) | only the selected points move (`mode: pointsOnly`) |
//! | `Ctrl+Alt` at mouse-down, or `Alt` pressed during the drag | nudge (`mode: nudge`) |
//! | drag handles only, `Alt` held | each handle keeps its angle (`mode: pointsOnly`) |
//! | drag handles only, `Ctrl+Alt` held | the opposite handle of a smooth node mirrors the moved one (`mode: mirror`) |
//! | `Alt`+drag a curve segment, selected alone | `path.reshapeSegment`: the handle lengths change, the nodes stay |
//! | an open end dropped on another open end | the move and `path.connectNodes` as one step |
//! | `Alt`+click a line segment | `path.convertSegmentToCurve` |
//! | double-click a node | `path.toggleNodeType` (`Alt`: `path.toggleNodeTypeKeepingOutline`) |
//! | double-click a segment | select its whole path |
//! | double-click a component | `component.editBaseGlyph`: its base glyph joins the tab text, edited |
//! | drag a guide's knob, or the guide elsewhere; double-click it | see [`crate::guide_drag`] |
//! | drag a smart handle of a selected smart component | `smartComponent.setValue` in one interaction |
//! | click or drag a PostScript hint's badge or marker; `Tab`, `Delete` with hints selected | see [`crate::hint_drag`] |
//! | `Escape` during a gesture | cancel it |
//!
//! Presses on the bounding box's knobs and just outside it go to [`crate::bbox`] first, then
//! Tunni lines and `Shift`+`Alt` presses on nodes to [`crate::drag_aids`], which also gives
//! drags the lever (`Cmd`) and the points that follow a move.

use fontcraft_geom::Point;
use serde_json::{Value, json};

use crate::event::{Key, KeyEvent, Modifiers, PointerEvent, PointerKind};
use crate::geometry::{dist, finite, rect_corners};
use crate::guide_drag::GuideDrag;
use crate::hint_drag::HintDrag;
use crate::hit::{Hit, HitResult, hit_test};
use crate::output::{Action, Invocation, Output, Overlay, OverlayKind};
use crate::scene::Scene;
use crate::selection::{Selection, in_region, path_points};
use crate::snap::{Moving, Snap, SnapKind, snap};
use crate::tool::{Context, Tool};

/// How a drag moves points (`selection.move`'s `mode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveMode {
    Normal,
    PointsOnly,
    Nudge,
    Mirror,
}

impl MoveMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            MoveMode::Normal => "normal",
            MoveMode::PointsOnly => "pointsOnly",
            MoveMode::Nudge => "nudge",
            MoveMode::Mirror => "mirror",
        }
    }
}

/// The parameters of `selection.move` for `selection` in `scene`.
pub fn move_params(scene: &Scene, selection: &Selection, dx: f64, dy: f64, mode: MoveMode, duplicate: bool) -> Value {
    let mut m = selection.targets(scene);
    m.insert("dx".into(), json!(dx));
    m.insert("dy".into(), json!(dy));
    if mode != MoveMode::Normal {
        m.insert("mode".into(), json!(mode.as_str()));
    }
    if duplicate {
        m.insert("duplicate".into(), json!(true));
    }
    Value::Object(m)
}

#[derive(Clone, Debug)]
struct Press {
    down: Point,
    /// The point that snaps: the grabbed node, handle or anchor, else the pointer.
    grab: Point,
    hit: HitResult,
    mods: Modifiers,
    selection: Selection,
    /// A plain click on an object of a larger selection selects only that object, unless the
    /// press becomes a drag.
    reduce_to: Option<Selection>,
    /// What a drag does to the pressed guide, when it moves or turns that guide alone.
    guide: Option<GuideDrag>,
}

#[derive(Clone, Debug)]
struct Drag {
    down: Point,
    grab: Point,
    selection: Selection,
    moving: Moving,
    alt_at_start: bool,
    nudge_at_start: bool,
    duplicate: bool,
    /// `Alt` at mouse-down with nothing to copy: only the selected points move.
    points_only: bool,
    /// Only handles are dragged: `Alt` keeps their angles, `Ctrl+Alt` mirrors them.
    handles_only: bool,
    last_pos: Point,
    last_sent: Option<(f64, f64, MoveMode)>,
    snap: Option<Snap>,
    /// The open end being dragged, when the drag may close or join paths.
    end: Option<(usize, usize)>,
    lever: crate::drag_aids::Lever,
    followers: crate::drag_aids::Followers,
}

#[derive(Clone, Debug)]
struct Marquee {
    start: Point,
    current: Point,
    /// The marquee's turn, in radians.
    angle: f64,
    /// While `Ctrl` turns the marquee: the pointer's angle and the marquee's angle when `Ctrl`
    /// went down, and the marquee's extents then.
    turning: Option<(f64, f64, f64, f64)>,
    extend: bool,
    nodes_only: bool,
    base: Selection,
    last: Selection,
}

impl Marquee {
    fn extents(&self) -> (f64, f64) {
        if let Some((_, _, u, v)) = self.turning {
            return (u, v);
        }
        let (c, s) = (self.angle.cos(), self.angle.sin());
        let (dx, dy) = (self.current.x - self.start.x, self.current.y - self.start.y);
        (dx * c + dy * s, -dx * s + dy * c)
    }

    fn polygon(&self) -> [Point; 4] {
        let (u, v) = self.extents();
        rect_corners(self.start, self.angle, u, v)
    }

    fn update(&mut self, pos: Point, ctrl: bool) {
        let pointer_angle = |p: Point, start: Point| (p.y - start.y).atan2(p.x - start.x);
        if ctrl {
            if self.turning.is_none() {
                let (u, v) = self.extents();
                self.turning = Some((pointer_angle(self.current, self.start), self.angle, u, v));
            }
            if let Some((from, base, _, _)) = self.turning {
                let a = base + pointer_angle(pos, self.start) - from;
                if a.is_finite() {
                    self.angle = a;
                }
            }
        } else {
            self.turning = None;
            self.current = pos;
        }
    }

    fn selection(&self, scene: &Scene) -> Selection {
        let region = in_region(scene, &self.polygon(), self.nodes_only, self.nodes_only);
        if self.extend {
            let mut s = self.base.clone();
            for item in region.items() {
                s.toggle(item);
            }
            s
        } else {
            region
        }
    }
}

#[derive(Clone, Debug, Default)]
enum State {
    #[default]
    Idle,
    Pressed(Box<Press>),
    Dragging(Box<Drag>),
    Marquee(Box<Marquee>),
    Guide(Box<GuideDrag>),
    Smart(Box<SmartDrag>),
    Hint(Box<HintDrag>),
    Reshape(Box<Reshape>),
}

/// `t` is held this far inside a segment being reshaped (the command's range).
const RESHAPE_MIN_T: f64 = 0.01;

/// An `Alt`-drag on a curve segment: `path.reshapeSegment` previews in one interaction, the
/// segment's point at `t` following the pointer.
#[derive(Clone, Debug)]
struct Reshape {
    shape: usize,
    node: usize,
    t: f64,
    down: Point,
    /// The segment's point at `t` when the drag began.
    grab: Point,
    last: Option<Point>,
}

impl Reshape {
    /// The reshape for a press on `hit`, when it is an unlocked cubic segment.
    fn press(scene: &Scene, hit: &HitResult, down: Point) -> Option<Self> {
        let Hit::Segment { shape, node, t, line: false, .. } = hit.hit else { return None };
        if hit.locked || scene.locked {
            return None;
        }
        let path = scene.path(shape)?;
        let seg = path.segment_ending_at(node)?;
        let cubic = seg.handles.len() == 2 && path.nodes.get(node).is_some_and(|n| n.kind == fontcraft_model::NodeKind::Curve);
        if !cubic || !t.is_finite() {
            return None;
        }
        let t = t.clamp(RESHAPE_MIN_T, 1.0 - RESHAPE_MIN_T);
        let grab = seg.eval(t).filter(|p| finite(*p)).unwrap_or(hit.at);
        Some(Self { shape, node, t, down, grab, last: None })
    }

    fn update(&mut self, cx: &Context<'_>, pos: Point) -> Option<Invocation> {
        let to = Point::new(self.grab.x + pos.x - self.down.x, self.grab.y + pos.y - self.down.y);
        if !finite(to) || self.last == Some(to) {
            return None;
        }
        self.last = Some(to);
        let mut params = Selection::new().targets(cx.scene);
        params.insert("segment".into(), json!([self.shape, self.node]));
        params.insert("t".into(), json!(self.t));
        params.insert("to".into(), json!([to.x, to.y]));
        Some(Invocation::new("path.reshapeSegment", Value::Object(params)))
    }
}

/// A smart handle being dragged: `smartComponent.setValue` previews in one interaction.
#[derive(Clone, Debug)]
struct SmartDrag {
    handle: crate::scene::SmartHandle,
    begun: bool,
    last: Option<f64>,
}

impl SmartDrag {
    fn update(&mut self, cx: &Context<'_>, pos: Point) -> Output {
        let mut out = Output::handled();
        if !self.begun {
            self.begun = true;
            out.push(Action::Begin { label: "Smart Handle".into() });
        }
        if let Some(value) = self.handle.value_at(pos)
            && self.last != Some(value)
        {
            self.last = Some(value);
            let params = json!({
                "glyph": cx.scene.glyph, "layer": cx.scene.layer.id, "component": self.handle.shape,
                "property": self.handle.property, "value": value,
            });
            out.push(Action::Preview { invocation: Invocation::new("smartComponent.setValue", params) });
        }
        out
    }
}

/// The Select tool.
#[derive(Clone, Debug, Default)]
pub struct SelectTool {
    state: State,
    hover: Option<HitResult>,
    /// A press or drag on the bounding box's knobs ([`crate::bbox`]).
    knobs: Option<crate::bbox::BoxDrag>,
    /// A Tunni line drag or a slide ([`crate::drag_aids`]).
    aid: Option<crate::drag_aids::AidDrag>,
}

impl SelectTool {
    fn down(&mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        let mut out = self.cancel();
        out.handled = true;
        let scene = cx.scene;
        let near = |p: Point| dist(cx.view.to_screen(p), cx.view.to_screen(ev.pos)) <= cx.settings.hit_tolerance;
        if !scene.locked
            && let Some(handle) = scene.smart_handles.iter().find(|h| near(h.at))
        {
            self.state = State::Smart(Box::new(SmartDrag { handle: handle.clone(), begun: false, last: None }));
            return out;
        }
        if let Some((pressed, drag)) = crate::hint_drag::press(cx, ev) {
            out.extend(pressed);
            if let Some(drag) = drag {
                self.state = State::Hint(Box::new(drag));
            }
            return out;
        }
        let Some(hit) = hit_test(scene, &cx.view, ev.pos, cx.settings.hit_tolerance) else {
            let base = if ev.mods.shift { cx.selection.clone() } else { Selection::new() };
            if !ev.mods.shift && !cx.selection.is_empty() {
                out.push(Action::Select { selection: Selection::new() });
            }
            self.state = State::Marquee(Box::new(Marquee {
                start: ev.pos,
                current: ev.pos,
                angle: 0.0,
                turning: None,
                extend: ev.mods.shift,
                nodes_only: ev.mods.alt,
                last: base.clone(),
                base,
            }));
            return out;
        };
        let items = hit.items();
        let mut selection = cx.selection.clone();
        if ev.mods.shift {
            for item in items {
                selection.toggle(item);
            }
            out.push(Action::Select { selection });
            return out;
        }
        let mut reduce_to = None;
        if hit.selected_in(cx.selection) {
            if selection.len() > items.len() {
                reduce_to = Some(Selection::from_items(items));
            }
        } else {
            selection = Selection::from_items(items);
            out.push(Action::Select { selection: selection.clone() });
        }
        let grab = match hit.hit {
            Hit::Node { .. } | Hit::Handle { .. } | Hit::Anchor { .. } => hit.at,
            _ => ev.pos,
        };
        // A knob dragged with other objects selected moves them all; Alt duplicates.
        let guide = match hit.hit {
            Hit::Guide { guide } if !ev.mods.alt => {
                let knob = crate::guide_drag::guide(scene, guide).is_some_and(|g| crate::guide_drag::on_knob(cx, g, ev.pos));
                if knob && selection.len() > 1 { None } else { GuideDrag::press(cx, guide, ev.pos, hit.at) }
            }
            _ => None,
        };
        self.state = State::Pressed(Box::new(Press { down: ev.pos, grab, hit, mods: ev.mods, selection, reduce_to, guide }));
        out
    }

    fn drag(&mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        let mut out = Output::handled();
        match &mut self.state {
            State::Idle => out.handled = false,
            State::Pressed(press) => {
                let moved = dist(cx.view.to_screen(press.down), cx.view.to_screen(ev.pos));
                if moved < cx.settings.drag_threshold {
                    return out;
                }
                if let Some(mut guide) = press.guide.take() {
                    out.push(Action::Begin { label: guide.label().into() });
                    if let Some(inv) = guide.update(cx, ev.pos, ev.mods.ctrl) {
                        out.push(Action::Preview { invocation: inv });
                    }
                    self.state = State::Guide(Box::new(guide));
                    return out;
                }
                if press.hit.locked || cx.scene.locked {
                    return out;
                }
                let mods = press.mods;
                let alone = press.selection == Selection::from_items(press.hit.items());
                if mods.alt
                    && !mods.ctrl
                    && alone
                    && let Some(mut reshape) = Reshape::press(cx.scene, &press.hit, press.down)
                {
                    out.push(Action::Begin { label: "Reshape Segment".into() });
                    if let Some(inv) = reshape.update(cx, ev.pos) {
                        out.push(Action::Preview { invocation: inv });
                    }
                    self.state = State::Reshape(Box::new(reshape));
                    return out;
                }
                let copyable = press.selection.has_copyable_part(cx.scene);
                let duplicate = mods.alt && !mods.ctrl && copyable;
                let nudge_at_start = mods.alt && mods.ctrl;
                let handles_only = press.selection.is_handles_only(cx.scene);
                let points_only = mods.alt && !mods.ctrl && !copyable && !handles_only;
                let only = press.selection.points.iter().next().copied().filter(|_| press.selection.len() == 1);
                let end = only.filter(|&(s, n)| !duplicate && cx.scene.is_open_end(s, n));
                let followers = if duplicate {
                    Default::default()
                } else {
                    crate::drag_aids::Followers::new(cx.scene, &press.selection, cx.settings.power_nudge)
                };
                let drag = Drag {
                    down: press.down,
                    grab: press.grab,
                    moving: Moving::from_selection(cx.scene, &press.selection),
                    selection: std::mem::take(&mut press.selection),
                    alt_at_start: mods.alt,
                    nudge_at_start,
                    duplicate,
                    points_only,
                    handles_only,
                    last_pos: ev.pos,
                    last_sent: None,
                    snap: None,
                    end,
                    lever: crate::drag_aids::Lever::new(press.down),
                    followers,
                };
                out.push(Action::Begin { label: if duplicate { "Duplicate".into() } else { "Move".into() } });
                let mut drag = Box::new(drag);
                if let Some(inv) = drag.update(cx, ev.pos, ev.mods) {
                    out.push(Action::Preview { invocation: inv });
                }
                self.state = State::Dragging(drag);
            }
            State::Dragging(drag) => {
                if let Some(inv) = drag.update(cx, ev.pos, ev.mods) {
                    out.push(Action::Preview { invocation: inv });
                }
            }
            State::Marquee(m) => {
                m.update(ev.pos, ev.mods.ctrl);
                let s = m.selection(cx.scene);
                if s != m.last {
                    m.last = s.clone();
                    out.push(Action::Select { selection: s });
                }
            }
            State::Guide(guide) => {
                if let Some(inv) = guide.update(cx, ev.pos, ev.mods.ctrl) {
                    out.push(Action::Preview { invocation: inv });
                }
            }
            State::Smart(smart) => out.actions.extend(smart.update(cx, ev.pos).actions),
            State::Hint(hint) => out.extend(hint.drag(cx, ev)),
            State::Reshape(reshape) => {
                if let Some(inv) = reshape.update(cx, ev.pos) {
                    out.push(Action::Preview { invocation: inv });
                }
            }
        }
        out
    }

    fn up(&mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        let mut out = Output::handled();
        match std::mem::take(&mut self.state) {
            State::Idle => out.handled = false,
            State::Pressed(press) => {
                let press = *press;
                if let Hit::Segment { shape, node, line: true, .. } = press.hit.hit
                    && press.mods.alt
                    && !press.hit.locked
                {
                    let mut params = Selection::new().targets(cx.scene);
                    params.insert("segments".into(), json!([[shape, node]]));
                    out.push(Action::Run { invocation: Invocation::new("path.convertSegmentToCurve", Value::Object(params)) });
                } else if let Some(selection) = press.reduce_to {
                    out.push(Action::Select { selection });
                }
            }
            State::Dragging(mut drag) => {
                let preview = drag.update(cx, ev.pos, ev.mods);
                let target = drag.snap.as_ref().filter(|s| s.kind == Some(SnapKind::Node)).and_then(|s| s.node);
                match (drag.end, target) {
                    (Some(a), Some(b)) if a != b && cx.scene.is_open_end(b.0, b.1) => {
                        let (dx, dy, mode) = drag.last_sent.unwrap_or((0.0, 0.0, MoveMode::Normal));
                        let moved = move_params(cx.scene, &drag.selection, dx, dy, mode, false);
                        let mut connect = Selection::new().targets(cx.scene);
                        connect.insert("points".into(), json!([[a.0, a.1], [b.0, b.1]]));
                        let batch = json!({
                            "label": "Connect Nodes",
                            "steps": [
                                {"command": "selection.move", "params": moved},
                                {"command": "path.connectNodes", "params": Value::Object(connect)},
                            ],
                        });
                        out.push(Action::Preview { invocation: Invocation::new("command.batch", batch) });
                    }
                    _ => {
                        if let Some(inv) = preview {
                            out.push(Action::Preview { invocation: inv });
                        }
                    }
                }
                out.push(Action::Commit);
            }
            State::Marquee(mut m) => {
                m.update(ev.pos, ev.mods.ctrl);
                let s = m.selection(cx.scene);
                if s != m.last {
                    out.push(Action::Select { selection: s });
                }
            }
            State::Guide(mut guide) => {
                if let Some(inv) = guide.update(cx, ev.pos, ev.mods.ctrl) {
                    out.push(Action::Preview { invocation: inv });
                }
                out.push(Action::Commit);
            }
            State::Smart(mut smart) => {
                if smart.begun {
                    out.actions.extend(smart.update(cx, ev.pos).actions);
                    out.push(Action::Commit);
                }
            }
            State::Hint(hint) => out.extend(hint.release(cx, ev)),
            State::Reshape(mut reshape) => {
                if let Some(inv) = reshape.update(cx, ev.pos) {
                    out.push(Action::Preview { invocation: inv });
                }
                out.push(Action::Commit);
            }
        }
        out
    }

    fn double_click(&mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        let mut out = self.cancel();
        out.handled = true;
        let Some(hit) = hit_test(cx.scene, &cx.view, ev.pos, cx.settings.hit_tolerance) else { return out };
        match hit.hit {
            Hit::Node { shape, node } if !hit.locked && !ev.mods.shift => {
                let id = if ev.mods.alt { "path.toggleNodeTypeKeepingOutline" } else { "path.toggleNodeType" };
                let mut params = Selection::new().targets(cx.scene);
                params.insert("points".into(), json!([[shape, node]]));
                out.push(Action::Run { invocation: Invocation::new(id, Value::Object(params)) });
            }
            Hit::Segment { shape, .. } => out.push(Action::Select { selection: path_points(cx.scene, shape) }),
            Hit::Component { shape } if !ev.mods.shift => {
                out.push(Action::Run { invocation: Invocation::new("component.editBaseGlyph", json!({"component": shape})) });
            }
            Hit::Guide { guide } => {
                if let Some(invocation) = crate::guide_drag::double_click(cx, guide, ev.pos) {
                    out.push(Action::Run { invocation });
                }
            }
            _ => {}
        }
        out
    }
}

impl Drag {
    /// Moves the drag to `pos` and returns the preview to run, if it differs from the last one.
    fn update(&mut self, cx: &Context<'_>, pos: Point, mods: Modifiers) -> Option<Invocation> {
        if !finite(pos) {
            return None;
        }
        self.last_pos = pos;
        let pos = self.lever.apply(cx, pos, mods);
        let target = Point::new(self.grab.x + pos.x - self.down.x, self.grab.y + pos.y - self.down.y);
        self.snap = if mods.ctrl {
            None
        } else {
            let plain = snap(cx.scene, &cx.view, target, &self.moving, cx.settings.snap_distance);
            // A copy leaves the dragged points where they were.
            let shown = match self.last_sent {
                Some((dx, dy, _)) if !self.duplicate => (dx, dy),
                _ => (0.0, 0.0),
            };
            let dragged = crate::suggestions::Dragged { selection: &self.selection, moving: &self.moving, grab: self.grab, shown };
            Some(crate::suggestions::refine(cx.scene, &cx.view, dragged, target, plain, cx.settings.snap_distance))
        };
        let to = self.snap.as_ref().map_or(target, |s| s.point);
        let (dx, dy) = (to.x - self.grab.x, to.y - self.grab.y);
        if !(dx.is_finite() && dy.is_finite()) {
            return None;
        }
        let mode = if self.handles_only {
            match (mods.alt, mods.ctrl) {
                (true, true) => MoveMode::Mirror,
                (true, false) => MoveMode::PointsOnly,
                _ => MoveMode::Normal,
            }
        } else if self.points_only {
            MoveMode::PointsOnly
        } else if self.nudge_at_start || (!self.alt_at_start && mods.alt) {
            MoveMode::Nudge
        } else {
            MoveMode::Normal
        };
        let sent = (dx, dy, mode);
        if self.last_sent == Some(sent) {
            return None;
        }
        self.last_sent = Some(sent);
        let inv = Invocation::new("selection.move", move_params(cx.scene, &self.selection, dx, dy, mode, self.duplicate));
        Some(if mode == MoveMode::Normal { self.followers.attach(cx.scene, inv, dx, dy) } else { inv })
    }
}

impl Tool for SelectTool {
    fn id(&self) -> &'static str {
        "select"
    }

    fn pointer(&mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        if !finite(ev.pos) {
            return Output::default();
        }
        if let Some(out) = crate::bbox::route(&mut self.knobs, cx, ev) {
            return out;
        }
        if let Some(out) = crate::drag_aids::route(&mut self.aid, cx, ev) {
            return out;
        }
        match ev.kind {
            PointerKind::Move => {
                if matches!(self.state, State::Idle) {
                    self.hover = hit_test(cx.scene, &cx.view, ev.pos, cx.settings.hit_tolerance);
                }
                Output::default()
            }
            PointerKind::Down => self.down(cx, ev),
            PointerKind::Drag => self.drag(cx, ev),
            PointerKind::Up => self.up(cx, ev),
            PointerKind::DoubleClick => self.double_click(cx, ev),
        }
    }

    fn key(&mut self, cx: &Context<'_>, ev: &KeyEvent) -> Output {
        if ev.pressed && ev.key == Key::Escape && self.busy() {
            let mut out = self.cancel();
            out.handled = true;
            return out;
        }
        if !self.busy()
            && let Some(out) = crate::hint_drag::key(cx, ev)
        {
            return out;
        }
        Output::default()
    }

    fn modifiers(&mut self, cx: &Context<'_>, mods: Modifiers) -> Output {
        if let Some(knobs) = &mut self.knobs {
            return knobs.modifiers(cx, mods);
        }
        let mut out = Output::default();
        match &mut self.state {
            State::Dragging(drag) => {
                let pos = drag.last_pos;
                if let Some(inv) = drag.update(cx, pos, mods) {
                    out.push(Action::Preview { invocation: inv });
                }
            }
            State::Marquee(m) => {
                let pos = m.current;
                if !mods.ctrl {
                    m.update(pos, false);
                }
            }
            State::Guide(guide) => {
                let pos = guide.last_pos;
                if let Some(inv) = guide.update(cx, pos, mods.ctrl) {
                    out.push(Action::Preview { invocation: inv });
                }
            }
            _ => {}
        }
        out
    }

    fn cancel(&mut self) -> Output {
        let mut out = crate::bbox::cancel(&mut self.knobs);
        out.extend(crate::drag_aids::cancel(&mut self.aid));
        self.hover = None;
        match std::mem::take(&mut self.state) {
            State::Dragging(_) | State::Guide(_) | State::Reshape(_) => out.push(Action::Cancel),
            State::Smart(smart) if smart.begun => out.push(Action::Cancel),
            State::Hint(hint) => out.extend(hint.cancel()),
            State::Marquee(m) => {
                let m = *m;
                if m.last != m.base {
                    out.push(Action::Select { selection: m.base });
                }
            }
            State::Idle | State::Pressed(_) | State::Smart(_) => {}
        }
        out
    }

    fn busy(&self) -> bool {
        self.knobs.is_some() || self.aid.is_some() || !matches!(self.state, State::Idle)
    }

    fn overlays(&self) -> Vec<Overlay> {
        match &self.state {
            State::Marquee(m) => vec![Overlay::Polygon { kind: OverlayKind::Marquee, points: m.polygon().to_vec() }],
            State::Dragging(drag) => snap_overlays(drag.snap.as_ref()),
            State::Guide(guide) => snap_overlays(guide.snap.as_ref()),
            State::Idle => match &self.hover {
                Some(h) => vec![Overlay::Marker { kind: if h.locked { OverlayKind::HoverLocked } else { OverlayKind::Hover }, at: h.at }],
                None => Vec::new(),
            },
            State::Hint(hint) => hint.overlays(),
            State::Pressed(_) | State::Smart(_) | State::Reshape(_) => Vec::new(),
        }
    }
}

/// The lines and the target marker of a snap.
fn snap_overlays(snap: Option<&Snap>) -> Vec<Overlay> {
    let Some(s) = snap else { return Vec::new() };
    let mut v: Vec<Overlay> = s.lines.iter().map(|&(from, to)| Overlay::Line { kind: OverlayKind::SnapLine, from, to }).collect();
    if let Some(at) = s.marker {
        v.push(Overlay::Marker { kind: OverlayKind::SnapTarget, at });
    }
    v
}
