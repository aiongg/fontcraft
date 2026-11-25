//! The Higher-Order Interpolation tool: the paths nodes follow between the edited layer and its
//! neighbours along one axis (`higherOrderInterpolation`, `hoi.knobOnDottedLine`).
//!
//! The caller resolves what the tool sees into the scene ([`HoiMarks`]): for every node of the
//! edited layer its span to the next layer down and to the next layer up, with the knobs of the
//! curve it follows there.
//!
//! | Gesture | Command |
//! |---|---|
//! | drag a knob | `hoi.setControlPoints`: the knob of a straight path becomes an intermediate point |
//! | click a knob; Shift+click | picks it; adds or removes it |
//! | marquee from empty canvas | picks the knobs inside it (Shift adds them) |
//! | Return | `hoi.convertToHandles` on the picked intermediate points |
//! | Delete or Backspace | `hoi.removeCurve` on the picked curves |
//! | Escape | drops the picks |
//!
//! Picks on both sides of the layer take one command each, since a command names one span. The
//! tool takes Return and Delete whatever is picked, so neither acts on the outline while it is
//! the tool.

use std::collections::{BTreeMap, BTreeSet};

use fontcraft_geom::{Point, Vec2};
use serde::Serialize;
use serde_json::{Value, json};

use crate::event::{Key, KeyEvent, PointerEvent, PointerKind};
use crate::geometry::{dist, finite, usable};
use crate::output::{Action, Invocation, Output, Overlay, OverlayKind, badge_number};
use crate::scene::Scene;
use crate::tool::{Context, Tool};

/// The tool's id.
pub const ID: &str = "higherOrderInterpolation";
/// Most nodes of the edited layer that get a span on each side, and most marker positions per
/// layer.
pub const MAX_NODES: usize = 4_000;

/// Which neighbour a span goes to: the next layer up the axis or the next one down.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Toward {
    Up,
    Down,
}

impl Toward {
    /// The commands' `toward` value.
    pub fn as_str(self) -> &'static str {
        match self {
            Toward::Up => "up",
            Toward::Down => "down",
        }
    }
}

/// How a node moves along a span (the commands' `type`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    #[default]
    Linear,
    IntermediatePoint,
    Handles,
    Rotation,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Linear, Kind::IntermediatePoint, Kind::Handles, Kind::Rotation];

    /// The commands' `type` value.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Linear => "linear",
            Kind::IntermediatePoint => "intermediatePoint",
            Kind::Handles => "handles",
            Kind::Rotation => "rotation",
        }
    }
}

/// One node's span to a neighbouring layer.
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    /// The node, as `[shape, node]` on the edited layer.
    pub shape: usize,
    pub node: usize,
    pub toward: Toward,
    /// The node on the span's lower and upper layer.
    pub lower: Point,
    pub upper: Point,
    pub kind: Kind,
    /// The middle of a straight path; the intermediate point; the lower and the upper handle; the
    /// rotation's centre.
    pub knobs: Vec<Point>,
    /// The curve from the lower to the upper node, sampled; empty for a straight path.
    pub curve: Vec<Point>,
}

/// What the tool shows and edits on one layer along one axis, resolved by the caller.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HoiMarks {
    /// The axis tag.
    pub axis: String,
    /// The nodes of the edited layer, the next layer down the axis and the next layer up.
    pub here: Vec<Point>,
    pub below: Vec<Point>,
    pub above: Vec<Point>,
    pub spans: Vec<Span>,
}

/// A node's span, as the commands address it: shape, node and side.
pub type SpanKey = (usize, usize, Toward);

/// A knob: one of a span's knobs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Knob {
    pub shape: usize,
    pub node: usize,
    pub toward: Toward,
    /// Which knob of the span: 0, or 1 for the upper handle.
    pub index: usize,
}

impl Knob {
    pub fn key(&self) -> SpanKey {
        (self.shape, self.node, self.toward)
    }
}

impl HoiMarks {
    /// The spans by address.
    pub fn by_key(&self) -> BTreeMap<SpanKey, &Span> {
        self.spans.iter().map(|s| ((s.shape, s.node, s.toward), s)).collect()
    }

    pub fn span(&self, key: SpanKey) -> Option<&Span> {
        self.spans.iter().find(|s| (s.shape, s.node, s.toward) == key)
    }

    /// Every knob with its position.
    pub fn knobs(&self) -> impl Iterator<Item = (Knob, Point)> + '_ {
        self.spans
            .iter()
            .flat_map(|s| s.knobs.iter().enumerate().map(|(index, at)| (Knob { shape: s.shape, node: s.node, toward: s.toward, index }, *at)))
    }

    /// The knob nearest `pos`, within `tolerance` font units.
    pub fn hit(&self, pos: Point, tolerance: f64) -> Option<Knob> {
        self.knobs().map(|(k, at)| (dist(at, pos), k)).filter(|(d, _)| *d <= tolerance).min_by(|a, b| a.0.total_cmp(&b.0)).map(|(_, k)| k)
    }
}

/// The parameters of a `hoi.*` command on `nodes` of the scene's layer, on one side of it.
fn params(scene: &Scene<'_>, marks: &HoiMarks, toward: Toward, nodes: &[(usize, usize)], extra: &[(&str, Value)]) -> Value {
    let mut m = scene.target();
    m.insert("points".into(), json!(nodes.iter().map(|(s, n)| [s, n]).collect::<Vec<_>>()));
    m.insert("axis".into(), json!(marks.axis));
    m.insert("toward".into(), json!(toward.as_str()));
    for (key, value) in extra {
        m.insert((*key).into(), value.clone());
    }
    Value::Object(m)
}

/// Command `id` for the picked spans that `wanted` keeps, one invocation per side of the layer.
pub fn invocations(scene: &Scene<'_>, picks: &[Knob], id: &str, extra: &[(&str, Value)], wanted: impl Fn(Kind) -> bool) -> Vec<Invocation> {
    let Some(marks) = &scene.hoi else { return Vec::new() };
    let spans = marks.by_key();
    let mut sides: BTreeMap<Toward, BTreeSet<(usize, usize)>> = BTreeMap::new();
    for pick in picks {
        if spans.get(&pick.key()).is_some_and(|s| wanted(s.kind)) {
            sides.entry(pick.toward).or_default().insert((pick.shape, pick.node));
        }
    }
    sides
        .into_iter()
        .map(|(toward, nodes)| {
            let nodes: Vec<(usize, usize)> = nodes.into_iter().collect();
            Invocation::new(id, params(scene, marks, toward, &nodes, extra))
        })
        .collect()
}

#[derive(Clone, Debug)]
struct Drag {
    knob: Knob,
    start: Point,
    /// The span's knobs when the drag began.
    base: Vec<Point>,
    began: bool,
    /// Where the dragged knob is, for the badge.
    at: Point,
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
    Dragging(Drag),
    Marquee(Marquee),
}

/// The Higher-Order Interpolation tool.
#[derive(Clone, Debug, Default)]
pub struct HoiTool {
    state: State,
    picked: Vec<Knob>,
    hover: Option<Knob>,
    /// The knobs to ring, as the last event saw them.
    rings: Vec<(OverlayKind, Point)>,
}

impl HoiTool {
    /// Drops the picks whose knob is gone and finds the ringed knobs.
    fn refresh(&mut self, marks: &HoiMarks) {
        let spans = marks.by_key();
        let place = |k: &Knob| spans.get(&k.key()).and_then(|s| s.knobs.get(k.index)).copied();
        self.picked.retain(|k| place(k).is_some());
        self.hover = self.hover.filter(|k| place(k).is_some());
        self.rings = self.picked.iter().filter_map(place).map(|at| (OverlayKind::SnapTarget, at)).collect();
        if let Some(at) = self.hover.filter(|k| !self.picked.contains(k)).as_ref().and_then(place) {
            self.rings.push((OverlayKind::Hover, at));
        }
    }

    fn press(&mut self, cx: &Context<'_>, marks: &HoiMarks, ev: &PointerEvent) {
        let Some(knob) = marks.hit(ev.pos, cx.view.units(cx.settings.hit_tolerance)) else {
            self.state = State::Marquee(Marquee { start: ev.pos, current: ev.pos, add: ev.mods.shift, dragged: false });
            return;
        };
        if ev.mods.shift {
            match self.picked.iter().position(|k| *k == knob) {
                Some(at) => {
                    self.picked.remove(at);
                }
                None => self.picked.push(knob),
            }
            self.state = State::Idle;
            return;
        }
        if !self.picked.contains(&knob) {
            self.picked = vec![knob];
        }
        let base = marks.span(knob.key()).map(|s| s.knobs.clone()).unwrap_or_default();
        let at = base.get(knob.index).copied().unwrap_or(ev.pos);
        self.state = State::Dragging(Drag { knob, start: ev.pos, base, began: false, at });
    }

    fn drag(&mut self, cx: &Context<'_>, marks: &HoiMarks, ev: &PointerEvent) -> Output {
        let moved = |start: Point| dist(cx.view.to_screen(start), cx.view.to_screen(ev.pos)) >= cx.settings.drag_threshold;
        match &mut self.state {
            State::Idle => Output::default(),
            State::Marquee(m) => {
                m.current = ev.pos;
                m.dragged |= moved(m.start);
                Output::handled()
            }
            State::Dragging(d) => {
                if cx.scene.locked || !d.began && !moved(d.start) {
                    return Output::handled();
                }
                let delta: Vec2 = ev.pos - d.start;
                let mut positions = d.base.clone();
                let Some(slot) = positions.get_mut(d.knob.index) else { return Output::handled() };
                let to = *slot + delta;
                *slot = Point::new(to.x.round(), to.y.round());
                d.at = *slot;
                if !positions.iter().all(|p| usable(*p)) {
                    return Output::handled();
                }
                let mut out = Output::handled();
                if !d.began {
                    d.began = true;
                    out.push(Action::Begin { label: "Move Interpolation Knob".into() });
                }
                let list = json!(positions.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>());
                let node = (d.knob.shape, d.knob.node);
                let invocation = Invocation::new("hoi.setControlPoints", params(cx.scene, marks, d.knob.toward, &[node], &[("positions", list)]));
                out.push(Action::Preview { invocation });
                out
            }
        }
    }

    fn release(&mut self, marks: &HoiMarks) -> Output {
        match std::mem::take(&mut self.state) {
            State::Idle => Output::default(),
            State::Dragging(d) => {
                if d.began {
                    return Output::with(Action::Commit);
                }
                // A click on one of several picks leaves that one.
                self.picked = vec![d.knob];
                Output::handled()
            }
            State::Marquee(m) => {
                if !m.add {
                    self.picked.clear();
                }
                if m.dragged {
                    let (lo, hi) = (
                        Point::new(m.start.x.min(m.current.x), m.start.y.min(m.current.y)),
                        Point::new(m.start.x.max(m.current.x), m.start.y.max(m.current.y)),
                    );
                    let inside = |p: Point| (lo.x..=hi.x).contains(&p.x) && (lo.y..=hi.y).contains(&p.y);
                    let had: BTreeSet<Knob> = self.picked.iter().copied().collect();
                    self.picked.extend(marks.knobs().filter(|(k, at)| inside(*at) && !had.contains(k)).map(|(k, _)| k));
                }
                Output::handled()
            }
        }
    }
}

impl Tool for HoiTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn pointer(&mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        if !finite(ev.pos) {
            return Output::default();
        }
        let Some(marks) = &cx.scene.hoi else {
            self.picked.clear();
            self.rings.clear();
            return Output::default();
        };
        let out = match ev.kind {
            PointerKind::Move => {
                self.hover = marks.hit(ev.pos, cx.view.units(cx.settings.hit_tolerance));
                Output::default()
            }
            PointerKind::Down => {
                self.press(cx, marks, ev);
                Output::handled()
            }
            PointerKind::Drag => self.drag(cx, marks, ev),
            PointerKind::Up => self.release(marks),
            PointerKind::DoubleClick => Output::default(),
        };
        self.refresh(marks);
        out
    }

    fn key(&mut self, cx: &Context<'_>, ev: &KeyEvent) -> Output {
        if !ev.pressed {
            return Output::default();
        }
        if let Some(marks) = &cx.scene.hoi {
            self.refresh(marks);
        }
        let commands = match ev.key {
            Key::Escape if self.busy() => {
                let state = std::mem::take(&mut self.state);
                let mut out = Output::handled();
                if let State::Dragging(d) = state
                    && d.began
                {
                    out.push(Action::Cancel);
                }
                return out;
            }
            Key::Escape if !self.picked.is_empty() => {
                self.picked.clear();
                self.rings.clear();
                return Output::handled();
            }
            Key::Return | Key::Delete | Key::Backspace if self.busy() || cx.scene.locked => return Output::handled(),
            Key::Return => invocations(cx.scene, &self.picked, "hoi.convertToHandles", &[], |k| k == Kind::IntermediatePoint),
            Key::Delete | Key::Backspace => invocations(cx.scene, &self.picked, "hoi.removeCurve", &[], |k| k != Kind::Linear),
            _ => return Output::default(),
        };
        let mut out = Output::handled();
        for invocation in commands {
            out.push(Action::Run { invocation });
        }
        out
    }

    fn cancel(&mut self) -> Output {
        let state = std::mem::take(&mut self.state);
        self.picked.clear();
        self.rings.clear();
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
        let mut out: Vec<Overlay> = self.rings.iter().map(|(kind, at)| Overlay::Marker { kind: *kind, at: *at }).collect();
        match &self.state {
            State::Marquee(m) if m.dragged => {
                let (a, b) = (m.start, m.current);
                out.push(Overlay::Polygon { kind: OverlayKind::Marquee, points: vec![a, Point::new(b.x, a.y), b, Point::new(a.x, b.y)] });
            }
            State::Dragging(d) if d.began => {
                out.push(Overlay::Badge { at: d.at, text: format!("x:{} y:{}", badge_number(d.at.x), badge_number(d.at.y)) });
            }
            _ => {}
        }
        out
    }

    fn hoi_picks(&self) -> &[Knob] {
        &self.picked
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use fontcraft_model::{Layer, Path, Shape};

    use super::*;
    use crate::{Modifiers, Selection, ToolSettings, View};

    fn span(node: usize, toward: Toward, kind: Kind, lower: Point, upper: Point, knobs: &[(f64, f64)]) -> Span {
        Span { shape: 0, node, toward, lower, upper, kind, knobs: knobs.iter().map(|&(x, y)| Point::new(x, y)).collect(), curve: Vec::new() }
    }

    /// Node 0: straight to the layer above, its knob at (50, 100). Node 1: an intermediate point at (200, 100) up and
    /// handles at (200, -100) and (240, -100) down.
    fn marks() -> HoiMarks {
        let p = Point::new;
        HoiMarks {
            axis: "wght".into(),
            spans: vec![
                span(0, Toward::Up, Kind::Linear, p(0.0, 100.0), p(100.0, 100.0), &[(50.0, 100.0)]),
                span(1, Toward::Up, Kind::IntermediatePoint, p(150.0, 100.0), p(250.0, 100.0), &[(200.0, 100.0)]),
                span(1, Toward::Down, Kind::Handles, p(150.0, -100.0), p(250.0, -100.0), &[(200.0, -100.0), (240.0, -100.0)]),
            ],
            ..HoiMarks::default()
        }
    }

    struct Harness {
        layer: Layer,
        tool: HoiTool,
    }

    impl Harness {
        fn new() -> Self {
            let mut layer = Layer::new("m01", 600.0);
            layer.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
            Harness { layer, tool: HoiTool::default() }
        }

        fn run(&mut self, f: impl FnOnce(&mut HoiTool, &Context<'_>) -> Output) -> Output {
            let mut scene = Scene::new("A", "m01", &self.layer);
            scene.hoi = Some(marks());
            let selection = Selection::new();
            let cx = Context { scene: &scene, view: View::default(), selection: &selection, settings: ToolSettings::default() };
            f(&mut self.tool, &cx)
        }

        fn pointer(&mut self, kind: PointerKind, x: f64, y: f64, mods: Modifiers) -> Output {
            self.run(|tool, cx| tool.pointer(cx, &PointerEvent::new(kind, x, y, mods)))
        }

        fn click(&mut self, x: f64, y: f64, mods: Modifiers) {
            self.pointer(PointerKind::Down, x, y, mods);
            self.pointer(PointerKind::Up, x, y, mods);
        }

        fn key(&mut self, key: Key) -> Output {
            self.run(|tool, cx| tool.key(cx, &KeyEvent::press(key, Modifiers::NONE)))
        }
    }

    fn command(toward: &str, node: usize, extra: Value) -> Value {
        let mut m = json!({"glyph": "A", "layer": "m01", "points": [[0, node]], "axis": "wght", "toward": toward});
        m.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        m
    }

    #[test]
    fn a_knob_drag_previews_the_control_points_and_is_one_interaction() {
        let mut h = Harness::new();
        h.pointer(PointerKind::Down, 51.0, 100.0, Modifiers::NONE);
        assert!(h.pointer(PointerKind::Drag, 51.5, 100.0, Modifiers::NONE).actions.is_empty(), "not a drag yet");
        let out = h.pointer(PointerKind::Drag, 61.2, 140.0, Modifiers::NONE);
        let set = Invocation::new("hoi.setControlPoints", command("up", 0, json!({"positions": [[60.0, 140.0]]})));
        assert_eq!(out.actions, [Action::Begin { label: "Move Interpolation Knob".into() }, Action::Preview { invocation: set }]);
        assert_eq!(h.pointer(PointerKind::Up, 61.2, 140.0, Modifiers::NONE).actions, [Action::Commit]);

        // A handle goes with its other handle; Escape puts the curve back.
        h.pointer(PointerKind::Down, 240.0, -100.0, Modifiers::NONE);
        let out = h.pointer(PointerKind::Drag, 240.0, -60.0, Modifiers::NONE);
        let set = Invocation::new("hoi.setControlPoints", command("down", 1, json!({"positions": [[200.0, -100.0], [240.0, -60.0]]})));
        assert_eq!(out.actions.last(), Some(&Action::Preview { invocation: set }));
        assert_eq!(h.key(Key::Escape).actions, [Action::Cancel]);
        assert!(!h.tool.busy());
    }

    #[test]
    fn return_and_delete_run_the_commands_on_the_picked_knobs() {
        let mut h = Harness::new();
        assert_eq!(h.key(Key::Delete), Output::handled(), "nothing picked: the outline keeps its nodes");
        h.click(200.0, 100.0, Modifiers::NONE);
        let convert = Invocation::new("hoi.convertToHandles", command("up", 1, json!({})));
        assert_eq!(h.key(Key::Return).actions, [Action::Run { invocation: convert }]);

        // A marquee around every knob: the straight path has no curve to remove, and each side takes its command.
        h.pointer(PointerKind::Down, -50.0, -200.0, Modifiers::NONE);
        h.pointer(PointerKind::Drag, 400.0, 300.0, Modifiers::NONE);
        h.pointer(PointerKind::Up, 400.0, 300.0, Modifiers::NONE);
        assert_eq!(h.tool.hoi_picks().len(), 4);
        let remove = |toward: &str| Action::Run { invocation: Invocation::new("hoi.removeCurve", command(toward, 1, json!({}))) };
        assert_eq!(h.key(Key::Backspace).actions, [remove("up"), remove("down")]);

        h.click(50.0, 100.0, Modifiers::SHIFT);
        assert_eq!(h.tool.hoi_picks().len(), 3, "Shift+click takes a pick away");
        assert!(h.key(Key::Escape).handled && h.tool.hoi_picks().is_empty());
        assert!(!h.key(Key::Escape).handled, "Escape goes on once nothing is picked");
    }
}
