//! The Magnet tool: links nodes to guides, making power guides, and nodes to PostScript hints
//! (`magnet` row, `docs/plans/M13.md` § 4 › M13.36).
//!
//! | Gesture | Command |
//! |---|---|
//! | drag a node onto a guide | `guide.makePowerGuide` with that guide and node |
//! | drag a linked node into empty space | `guide.unlinkPowerGuides` with that node |
//! | double-click a guide | `guide.makePowerGuide` (every node lying on it) |
//! | double-click a power guide | `guide.unlinkPowerGuides` with that guide |
//! | Shift+click a node | `hint.ps.addHorizontalHint` with that node alone: a ghost hint |
//! | Shift+drag from one node to another | `hint.ps.addHorizontalHint` or `hint.ps.addVerticalHint` with the two nodes, horizontal when they are further apart vertically |
//!
//! While a node is dragged, a line runs from it to the pointer and the guide under the pointer is
//! marked. Nothing changes until the release, and each gesture runs one command.

use fontcraft_geom::Point;
use fontcraft_model::guide_data;
use fontcraft_model::{Guide, LayerSide};
use serde_json::{Map, Value, json};

use crate::event::{Key, KeyEvent, PointerEvent, PointerKind};
use crate::geometry::{dist, finite, line_distance};
use crate::hit::{Hit, hit_test};
use crate::output::{Action, Invocation, Output, Overlay, OverlayKind};
use crate::scene::Scene;
use crate::selection::{GuideRef, GuideScope};
use crate::tool::{Context, Tool};

/// A guide under the pointer, and the nearest point of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuideHit {
    pub guide: GuideRef,
    pub at: Point,
}

/// The unlocked guide nearest to `pos` within the hit tolerance: local guides first on a tie,
/// global ones only where they show.
pub fn guide_at(cx: &Context<'_>, pos: Point) -> Option<GuideHit> {
    let scene = cx.scene;
    if !scene.guides_shown {
        return None;
    }
    let tolerance = cx.view.units(cx.settings.hit_tolerance);
    let local = scene.local_guides().iter().enumerate().map(|(index, g)| (GuideRef { scope: GuideScope::Local, index }, g));
    let global = scene.shown_global_guides().map(|(index, g)| (GuideRef { scope: GuideScope::Global, index }, g));
    let mut best: Option<(f64, GuideHit)> = None;
    for (r, g) in local.chain(global) {
        if g.locked {
            continue;
        }
        let Some((d, foot)) = line_distance(pos, scene.knob(g), g.angle) else { continue };
        if d <= tolerance && best.is_none_or(|(b, _)| d < b) {
            best = Some((d, GuideHit { guide: r, at: foot }));
        }
    }
    best.map(|(_, h)| h)
}

/// The guide at `r` in the scene.
fn guide<'s>(scene: &'s Scene<'_>, r: GuideRef) -> Option<&'s Guide> {
    match r.scope {
        GuideScope::Local => scene.local_guides().get(r.index),
        GuideScope::Global => scene.global_guides.get(r.index),
    }
}

/// Whether a power guide of the scene holds the node `(shape, node)` of the edited layer.
pub fn is_linked(scene: &Scene<'_>, shape: usize, node: usize) -> bool {
    let local = scene.local_guides().iter().flat_map(guide_data::links).any(|l| l.glyph.is_none() && (l.shape, l.node) == (shape, node));
    let master_layer = scene.side == LayerSide::Foreground && scene.layer.is_master_layer();
    local
        || (master_layer
            && scene
                .global_guides
                .iter()
                .flat_map(guide_data::links)
                .any(|l| l.glyph.as_deref() == Some(scene.glyph.as_str()) && (l.shape, l.node) == (shape, node)))
}

/// An on-curve or off-curve node under `pos` that can be linked: in an editable, unlocked shape.
fn node_at(cx: &Context<'_>, pos: Point) -> Option<(usize, usize, Point)> {
    let hit = hit_test(cx.scene, &cx.view, pos, cx.settings.hit_tolerance)?;
    match hit.hit {
        Hit::Node { shape, node } | Hit::Handle { shape, node } if !hit.locked && cx.scene.shape_editable(shape) => {
            let at = cx.scene.node(shape, node)?.point();
            Some((shape, node, at))
        }
        _ => None,
    }
}

fn guide_json(r: GuideRef) -> Value {
    json!({"scope": match r.scope { GuideScope::Local => "local", GuideScope::Global => "global" }, "index": r.index})
}

fn invocation(scene: &Scene<'_>, id: &str, extra: Map<String, Value>) -> Invocation {
    let mut params = scene.target();
    params.extend(extra);
    Invocation::new(id, Value::Object(params))
}

fn params(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs.iter().map(|(k, v)| ((*k).to_string(), v.clone())).collect()
}

#[derive(Clone, Copy, Debug)]
struct Press {
    shape: usize,
    node: usize,
    /// The node's position.
    at: Point,
    /// The pointer's position at the press.
    start: Point,
    shift: bool,
}

#[derive(Clone, Copy, Debug, Default)]
enum State {
    #[default]
    Idle,
    Pressed(Press),
    Dragging {
        press: Press,
        current: Point,
    },
}

/// The Magnet tool.
#[derive(Clone, Debug, Default)]
pub struct MagnetTool {
    state: State,
    /// The guide the dragged node would be linked to, for the overlay.
    target: Option<GuideHit>,
}

impl Tool for MagnetTool {
    fn id(&self) -> &'static str {
        "magnet"
    }

    fn pointer(&mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        if !finite(ev.pos) {
            return Output::default();
        }
        match ev.kind {
            PointerKind::Move => Output::default(),
            PointerKind::Down => {
                self.target = None;
                match node_at(cx, ev.pos) {
                    Some((shape, node, at)) if !cx.scene.locked => {
                        self.state = State::Pressed(Press { shape, node, at, start: ev.pos, shift: ev.mods.shift });
                        Output::handled()
                    }
                    _ => {
                        self.state = State::Idle;
                        Output::default()
                    }
                }
            }
            PointerKind::Drag => {
                let press = match self.state {
                    State::Idle => return Output::default(),
                    State::Pressed(press) => {
                        if dist(cx.view.to_screen(press.start), cx.view.to_screen(ev.pos)) < cx.settings.drag_threshold {
                            return Output::handled();
                        }
                        press
                    }
                    State::Dragging { press, .. } => press,
                };
                self.state = State::Dragging { press, current: ev.pos };
                self.target = if press.shift { None } else { guide_at(cx, ev.pos) };
                Output::handled()
            }
            PointerKind::Up => {
                let state = std::mem::take(&mut self.state);
                self.target = None;
                match state {
                    State::Idle => Output::default(),
                    State::Pressed(press) => {
                        let mut out = Output::handled();
                        if press.shift {
                            let nodes = json!([[press.shape, press.node]]);
                            out.push(Action::Run { invocation: invocation(cx.scene, "hint.ps.addHorizontalHint", params(&[("nodes", nodes)])) });
                        }
                        out
                    }
                    State::Dragging { press, .. } => {
                        let mut out = Output::handled();
                        if let Some(invocation) = release(cx, press, ev.pos) {
                            out.push(Action::Run { invocation });
                        }
                        out
                    }
                }
            }
            PointerKind::DoubleClick => {
                if node_at(cx, ev.pos).is_some() {
                    return Output::default();
                }
                let Some(hit) = guide_at(cx, ev.pos) else { return Output::default() };
                let power = guide(cx.scene, hit.guide).is_some_and(guide_data::is_power_guide);
                let id = if power { "guide.unlinkPowerGuides" } else { "guide.makePowerGuide" };
                let invocation = invocation(cx.scene, id, params(&[("guides", json!([guide_json(hit.guide)]))]));
                Output::with(Action::Run { invocation })
            }
        }
    }

    fn key(&mut self, _cx: &Context<'_>, ev: &KeyEvent) -> Output {
        if ev.pressed && ev.key == Key::Escape && self.busy() {
            self.state = State::Idle;
            self.target = None;
            return Output::handled();
        }
        Output::default()
    }

    fn cancel(&mut self) -> Output {
        self.state = State::Idle;
        self.target = None;
        Output::default()
    }

    fn busy(&self) -> bool {
        !matches!(self.state, State::Idle)
    }

    fn overlays(&self) -> Vec<Overlay> {
        let mut out = Vec::new();
        if let State::Dragging { press, current } = self.state {
            let to = self.target.map_or(current, |t| t.at);
            out.push(Overlay::Line { kind: OverlayKind::DrawPreview, from: press.at, to });
            out.push(Overlay::Marker { kind: OverlayKind::SnapTarget, at: press.at });
            if let Some(t) = self.target {
                out.push(Overlay::Marker { kind: OverlayKind::SnapTarget, at: t.at });
            }
        }
        out
    }
}

/// The command a drag from `press` released at `pos` runs.
fn release(cx: &Context<'_>, press: Press, pos: Point) -> Option<Invocation> {
    let node = json!([press.shape, press.node]);
    if press.shift {
        let (shape, other, at) = node_at(cx, pos)?;
        if (shape, other) == (press.shape, press.node) {
            return None;
        }
        let horizontal = (at.y - press.at.y).abs() >= (at.x - press.at.x).abs();
        let id = if horizontal { "hint.ps.addHorizontalHint" } else { "hint.ps.addVerticalHint" };
        return Some(invocation(cx.scene, id, params(&[("nodes", json!([[node, [shape, other]]]))])));
    }
    match guide_at(cx, pos) {
        Some(hit) => {
            Some(invocation(cx.scene, "guide.makePowerGuide", params(&[("guides", json!([guide_json(hit.guide)])), ("nodes", json!([node]))])))
        }
        None if is_linked(cx.scene, press.shape, press.node) => {
            Some(invocation(cx.scene, "guide.unlinkPowerGuides", params(&[("nodes", json!([node]))])))
        }
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use fontcraft_model::guide_data::Link;
    use fontcraft_model::{Layer, Node, NodeKind, Path, Shape};

    use super::*;
    use crate::{Modifiers, Selection, ToolSettings, View};

    /// A bar from x 100 to 300 with a vertical guide at x 400.
    fn layer(linked: bool) -> Layer {
        let points = [(100.0, 0.0), (100.0, 100.0), (300.0, 100.0), (300.0, 0.0)];
        let path = Path::new(true, points.iter().map(|(x, y)| Node::new(*x, *y, NodeKind::Line)).collect());
        let mut layer = Layer::new("m01", 600.0);
        layer.shapes.push(Shape::Path(path));
        let mut guide = Guide { x: 400.0, angle: 90.0, ..Guide::default() };
        if linked {
            guide_data::set_links(&mut guide, &[Link::local(0, 2)]).unwrap();
        }
        layer.info.guides.push(guide);
        layer
    }

    fn event(tool: &mut MagnetTool, layer: &Layer, kind: PointerKind, x: f64, y: f64, mods: Modifiers) -> Output {
        let scene = Scene::new("A", "m01", layer);
        let selection = Selection::new();
        let cx = Context { scene: &scene, view: View::default(), selection: &selection, settings: ToolSettings::default() };
        tool.pointer(&cx, &PointerEvent::new(kind, x, y, mods))
    }

    fn drag(tool: &mut MagnetTool, layer: &Layer, from: (f64, f64), to: (f64, f64), mods: Modifiers) -> Option<Invocation> {
        event(tool, layer, PointerKind::Down, from.0, from.1, mods);
        event(tool, layer, PointerKind::Drag, to.0, to.1, mods);
        assert!(tool.busy());
        let out = event(tool, layer, PointerKind::Up, to.0, to.1, mods);
        match out.actions.as_slice() {
            [Action::Run { invocation }] => Some(invocation.clone()),
            [] => None,
            other => panic!("unexpected actions: {other:?}"),
        }
    }

    #[test]
    fn dragging_a_node_onto_a_guide_links_it_and_off_a_guide_unlinks_it() {
        let mut tool = MagnetTool::default();
        let inv = drag(&mut tool, &layer(false), (300.0, 100.0), (401.0, 150.0), Modifiers::NONE).unwrap();
        assert_eq!(inv.id, "guide.makePowerGuide");
        assert_eq!(inv.params["guides"], json!([{"scope": "local", "index": 0}]));
        assert_eq!(inv.params["nodes"], json!([[0, 2]]));
        assert_eq!(inv.params["glyph"], json!("A"));
        // Into empty space: a linked node loses its link, an unlinked one does nothing.
        let inv = drag(&mut tool, &layer(true), (300.0, 100.0), (250.0, 300.0), Modifiers::NONE).unwrap();
        assert_eq!((inv.id.as_str(), &inv.params["nodes"]), ("guide.unlinkPowerGuides", &json!([[0, 2]])));
        assert_eq!(drag(&mut tool, &layer(false), (300.0, 100.0), (250.0, 300.0), Modifiers::NONE), None);
    }

    #[test]
    fn double_click_makes_or_unlinks_a_power_guide() {
        let mut tool = MagnetTool::default();
        for (linked, id) in [(false, "guide.makePowerGuide"), (true, "guide.unlinkPowerGuides")] {
            let out = event(&mut tool, &layer(linked), PointerKind::DoubleClick, 400.0, 500.0, Modifiers::NONE);
            match out.actions.as_slice() {
                [Action::Run { invocation }] => {
                    assert_eq!(invocation.id, id);
                    assert_eq!(invocation.params["guides"], json!([{"scope": "local", "index": 0}]));
                }
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn shift_gestures_add_ghost_and_stem_hints() {
        let mut tool = MagnetTool::default();
        let layer = layer(false);
        event(&mut tool, &layer, PointerKind::Down, 100.0, 0.0, Modifiers::SHIFT);
        let out = event(&mut tool, &layer, PointerKind::Up, 100.0, 0.0, Modifiers::SHIFT);
        match out.actions.as_slice() {
            [Action::Run { invocation }] => {
                assert_eq!((invocation.id.as_str(), &invocation.params["nodes"]), ("hint.ps.addHorizontalHint", &json!([[0, 0]])))
            }
            other => panic!("{other:?}"),
        }
        // From the bottom-left node to the top-left one: a horizontal stem hint.
        let inv = drag(&mut tool, &layer, (100.0, 0.0), (100.0, 100.0), Modifiers::SHIFT).unwrap();
        assert_eq!(inv.id, "hint.ps.addHorizontalHint");
        assert_eq!(inv.params["nodes"], json!([[[0, 0], [0, 1]]]));
        // From the bottom-left node to the bottom-right one: a vertical stem hint.
        let inv = drag(&mut tool, &layer, (100.0, 0.0), (300.0, 0.0), Modifiers::SHIFT).unwrap();
        assert_eq!(inv.id, "hint.ps.addVerticalHint");
    }
}
