//! Behaviour tests of the drag aids, through the Select tool: gestures in, actions out.

use std::collections::BTreeMap;

use fontcraft_geom::{CubicBez, ParamCurve, Point};
use fontcraft_model::plist::{Dict, Str, Value as PValue};
use fontcraft_model::{Layer, Node, NodeKind, Path, Shape, fc_keys};
use serde_json::Value;

use super::*;
use crate::event::PointerKind::{Down, Drag, Up};
use crate::select::SelectTool;
use crate::tool::Tool;
use crate::view::View;

const NONE: Modifiers = Modifiers::NONE;
const SHIFT_ALT: Modifiers = Modifiers { shift: true, alt: true, cmd: false, ctrl: false };
/// The lever, with snapping off.
const LEVER: Modifiers = Modifiers { shift: false, alt: false, cmd: true, ctrl: true };
const CTRL: Modifiers = Modifiers::CTRL;

fn node(x: f64, y: f64, kind: NodeKind) -> Node {
    Node::new(x, y, kind)
}

fn on(x: f64, y: f64) -> Node {
    node(x, y, NodeKind::Line)
}

fn off(x: f64, y: f64) -> Node {
    node(x, y, NodeKind::OffCurve)
}

fn curve(x: f64, y: f64) -> Node {
    node(x, y, NodeKind::Curve)
}

fn smooth(mut n: Node) -> Node {
    n.smooth = true;
    n
}

/// Shape 0: the open curve (0,0), handles (0,50) (50,100), to (100,100). Its Tunni point is
/// (0,100), 100 units from both nodes, so both tensions are 50 %. Shape 1: the same curve 300
/// units to the right.
fn tunni_layer() -> Layer {
    let mut layer = Layer::new("m01", 600.0);
    for dx in [0.0, 300.0] {
        layer.shapes.push(Shape::Path(Path::new(false, vec![on(dx, 0.0), off(dx, 50.0), off(dx + 50.0, 100.0), curve(dx + 100.0, 100.0)])));
    }
    layer
}

/// The cubic (0,0) (0,100) (100,200) (200,200).
fn whole() -> CubicBez {
    CubicBez::new(Point::new(0.0, 0.0), Point::new(0.0, 100.0), Point::new(100.0, 200.0), Point::new(200.0, 200.0))
}

/// Shape 0: [`whole`] cut in two at t = 0.5, an open path whose middle node (3) is smooth.
fn split_layer() -> Layer {
    let (a, b) = (whole().subsegment(0.0..0.5), whole().subsegment(0.5..1.0));
    let mut layer = Layer::new("m01", 600.0);
    let nodes = vec![
        on(a.p0.x, a.p0.y),
        off(a.p1.x, a.p1.y),
        off(a.p2.x, a.p2.y),
        smooth(curve(b.p0.x, b.p0.y)),
        off(b.p1.x, b.p1.y),
        off(b.p2.x, b.p2.y),
        curve(b.p3.x, b.p3.y),
    ];
    layer.shapes.push(Shape::Path(Path::new(false, nodes)));
    layer
}

/// Shape 0, an open path: a corner (0,0), handles (40,40) (80,80), a smooth node (120,120) whose
/// handles are diagonal (not an extreme), handles (160,160) (200,200), and the end (240,240).
fn nudge_layer() -> Layer {
    let mut layer = Layer::new("m01", 600.0);
    let nodes =
        vec![on(0.0, 0.0), off(40.0, 40.0), off(80.0, 80.0), smooth(curve(120.0, 120.0)), off(160.0, 160.0), off(200.0, 200.0), curve(240.0, 240.0)];
    layer.shapes.push(Shape::Path(Path::new(false, nodes)));
    layer
}

/// Stands in for the shell: owns the layer and the selection, applies `Select` actions and
/// records every action.
struct Harness {
    layer: Layer,
    selection: Selection,
    settings: ToolSettings,
    tool: SelectTool,
    actions: Vec<Action>,
}

impl Harness {
    fn new(layer: Layer) -> Self {
        let settings = ToolSettings { tunni_lines: true, ..ToolSettings::default() };
        Self { layer, selection: Selection::new(), settings, tool: SelectTool::default(), actions: Vec::new() }
    }

    fn pointer(&mut self, kind: PointerKind, x: f64, y: f64, mods: Modifiers) {
        let scene = Scene::new("A", "m01", &self.layer);
        let cx = Context { scene: &scene, view: View::default(), selection: &self.selection, settings: self.settings };
        let out = self.tool.pointer(&cx, &PointerEvent::new(kind, x, y, mods));
        for a in &out.actions {
            if let Action::Select { selection } = a {
                self.selection = selection.clone();
            }
        }
        self.actions.extend(out.actions);
    }

    /// Press at `from`, drag through the middle to `to`, release there.
    fn drag(&mut self, from: (f64, f64), to: (f64, f64), mods: Modifiers) {
        self.pointer(Down, from.0, from.1, mods);
        self.pointer(Drag, (from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0, mods);
        self.pointer(Drag, to.0, to.1, mods);
        self.pointer(Up, to.0, to.1, mods);
    }

    fn begun(&self) -> Vec<&str> {
        self.actions.iter().filter_map(|a| if let Action::Begin { label } = a { Some(label.as_str()) } else { None }).collect()
    }

    /// The last preview's offset of each point it moves.
    fn moved(&self) -> BTreeMap<(usize, usize), (f64, f64)> {
        let inv = self
            .actions
            .iter()
            .rev()
            .find_map(|a| if let Action::Preview { invocation } = a { Some(invocation.clone()) } else { None })
            .expect("a preview");
        offsets(&inv)
    }
}

/// What a preview gives each point, from its `selection.move` steps.
fn offsets(inv: &Invocation) -> BTreeMap<(usize, usize), (f64, f64)> {
    let steps: Vec<Value> = if inv.id == "command.batch" {
        inv.params["steps"].as_array().unwrap().clone()
    } else {
        vec![serde_json::json!({ "command": inv.id, "params": inv.params })]
    };
    let mut out = BTreeMap::new();
    for step in steps {
        assert_eq!(step["command"], "selection.move");
        let p = &step["params"];
        let d = (p["dx"].as_f64().unwrap(), p["dy"].as_f64().unwrap());
        for point in p["points"].as_array().unwrap() {
            out.insert((point[0].as_u64().unwrap() as usize, point[1].as_u64().unwrap() as usize), d);
        }
    }
    out
}

fn near(a: (f64, f64), b: (f64, f64)) -> bool {
    (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6
}

type Moved = ((usize, usize), (f64, f64));

fn assert_moves(h: &Harness, expected: &[Moved]) {
    let got = h.moved();
    let got: Vec<_> = got.into_iter().filter(|(_, d)| !near(*d, (0.0, 0.0))).collect();
    assert_eq!(got.len(), expected.len(), "{got:?}");
    for ((at, d), (want_at, want)) in got.iter().zip(expected) {
        assert!(at == want_at && near(*d, *want), "got {at:?} {d:?}, expected {want_at:?} {want:?}");
    }
}

#[test]
fn dragging_a_tunni_line_changes_both_handles_alike() {
    let mut h = Harness::new(tunni_layer());
    // The line runs from (0,50) to (50,100); dragged by (-10,10) it moves a fifth of the way to
    // the Tunni point, so both tensions go from 50 % to 70 %.
    h.drag((25.0, 75.0), (15.0, 85.0), NONE);
    assert_eq!(h.begun(), ["Tunni Line"]);
    assert_moves(&h, &[((0, 1), (0.0, 20.0)), ((0, 2), (-20.0, 0.0))]);
    assert!(matches!(h.actions.last(), Some(Action::Commit)));

    // Shift-dragging the Tunni point does the same.
    let mut h = Harness::new(tunni_layer());
    h.drag((0.0, 100.0), (-10.0, 110.0), Modifiers::SHIFT);
    assert_moves(&h, &[((0, 1), (0.0, 20.0)), ((0, 2), (-20.0, 0.0))]);
}

#[test]
fn selected_tunni_lines_change_together() {
    let mut h = Harness::new(tunni_layer());
    h.selection = Selection::from_items([Item::Point(1, 1), Item::Point(1, 2)]);
    h.drag((25.0, 75.0), (15.0, 85.0), NONE);
    assert_moves(&h, &[((0, 1), (0.0, 20.0)), ((0, 2), (-20.0, 0.0)), ((1, 1), (0.0, 20.0)), ((1, 2), (-20.0, 0.0))]);
}

#[test]
fn alt_dragging_a_tunni_line_or_dragging_its_point_moves_one_handle() {
    // Alt on the line near the first handle: only it changes, along its own direction.
    let mut h = Harness::new(tunni_layer());
    h.drag((10.0, 60.0), (10.0, 75.0), Modifiers::ALT);
    assert_moves(&h, &[((0, 1), (0.0, 15.0))]);

    // The point dragged without Shift: the handle whose direction the drag follows.
    let mut h = Harness::new(tunni_layer());
    h.drag((0.0, 100.0), (-20.0, 100.0), NONE);
    assert_moves(&h, &[((0, 2), (-20.0, 0.0))]);

    // A handle never gets shorter than the minimum.
    let mut h = Harness::new(tunni_layer());
    h.drag((10.0, 60.0), (10.0, -200.0), Modifiers::ALT);
    assert_moves(&h, &[((0, 1), (0.0, -49.0))]);
}

#[test]
fn clicking_a_tunni_line_selects_its_handles_and_hidden_lines_are_left_alone() {
    let mut h = Harness::new(tunni_layer());
    h.pointer(Down, 25.0, 75.0, NONE);
    h.pointer(Up, 25.0, 75.0, NONE);
    assert_eq!(h.selection, Selection::from_items([Item::Point(0, 1), Item::Point(0, 2)]));
    h.pointer(Down, 25.0, 75.0, Modifiers::SHIFT);
    h.pointer(Up, 25.0, 75.0, Modifiers::SHIFT);
    assert!(h.selection.is_empty(), "Shift-click toggles them");
    assert!(h.begun().is_empty());

    let mut h = Harness::new(tunni_layer());
    h.settings.tunni_lines = false;
    h.drag((25.0, 75.0), (15.0, 85.0), NONE);
    assert!(h.begun().is_empty(), "without Tunni lines the press starts a marquee: {:?}", h.actions);
}

#[test]
fn the_lever_slows_a_move_and_never_jumps() {
    // Ratio 6 at actual size: 12 units of pointer movement move the node 2.
    let mut h = Harness::new(tunni_layer());
    h.drag((100.0, 100.0), (112.0, 100.0), LEVER);
    assert_moves(&h, &[((0, 3), (2.0, 0.0))]);

    // Beyond the ramp (50 screen points) the node follows the pointer one to one.
    let mut h = Harness::new(tunni_layer());
    h.drag((100.0, 100.0), (200.0, 100.0), LEVER);
    let d = h.moved()[&(0, 3)];
    assert!(near(d, (50.0 / 6.0 + 50.0, 0.0)), "{d:?}");

    // Letting go of the lever continues from where the node stands.
    let mut h = Harness::new(tunni_layer());
    h.pointer(Down, 100.0, 100.0, LEVER);
    h.pointer(Drag, 112.0, 100.0, LEVER);
    h.pointer(Drag, 122.0, 100.0, CTRL);
    assert_moves(&h, &[((0, 3), (12.0, 0.0))]);

    // Ratio 1 turns it off.
    let mut h = Harness::new(tunni_layer());
    h.settings.lever_ratio = 1.0;
    h.drag((100.0, 100.0), (112.0, 100.0), LEVER);
    assert_moves(&h, &[((0, 3), (12.0, 0.0))]);
}

/// The points of shape 0 after the last preview.
fn moved_points(h: &Harness) -> Vec<Point> {
    let moves = h.moved();
    let Some(Shape::Path(path)) = h.layer.shapes.first() else { panic!("no path") };
    path.nodes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let (dx, dy) = moves.get(&(0, i)).copied().unwrap_or((0.0, 0.0));
            Point::new(n.x + dx, n.y + dy)
        })
        .collect()
}

fn close(a: Point, b: Point) -> bool {
    (a - b).hypot() < 1e-3
}

#[test]
fn sliding_a_node_keeps_the_curve() {
    let mut h = Harness::new(split_layer());
    let (from, to) = (whole().eval(0.5), whole().eval(0.75));
    h.drag((from.x, from.y), (to.x, to.y), SHIFT_ALT);
    assert_eq!(h.begun(), ["Slide"]);
    let p = moved_points(&h);
    assert!(close(p[3], to), "the node lands on the curve: {:?}", p[3]);
    let first = CubicBez::new(p[0], p[1], p[2], p[3]);
    let second = CubicBez::new(p[3], p[4], p[5], p[6]);
    for (s, t) in [(0.25, 0.1875), (0.5, 0.375), (0.75, 0.5625)] {
        assert!(close(first.eval(s), whole().eval(t)), "first segment at {s}");
    }
    assert!(close(second.eval(0.5), whole().eval(0.875)), "second segment");

    // A press without a drag toggles the node, as Shift-click does.
    let mut h = Harness::new(split_layer());
    h.pointer(Down, from.x, from.y, SHIFT_ALT);
    h.pointer(Up, from.x, from.y, SHIFT_ALT);
    assert_eq!(h.selection, Selection::from_items([Item::Point(0, 3)]));
}

#[test]
fn sliding_an_open_end_past_the_end_extends_the_curve() {
    let mut layer = Layer::new("m01", 600.0);
    let c = whole();
    layer.shapes.push(Shape::Path(Path::new(false, vec![on(0.0, 0.0), off(c.p1.x, c.p1.y), off(c.p2.x, c.p2.y), curve(c.p3.x, c.p3.y)])));
    let mut h = Harness::new(layer);
    let to = c.eval(-0.25);
    h.drag((0.0, 0.0), (to.x, to.y), SHIFT_ALT);
    let p = moved_points(&h);
    assert!(close(p[0], to), "{:?}", p[0]);
    let extended = CubicBez::new(p[0], p[1], p[2], p[3]);
    assert!(close(extended.eval(0.2), Point::new(0.0, 0.0)), "the old end lies on the longer curve");
    assert!(close(extended.eval(0.6), c.eval(0.5)));
}

#[test]
fn power_nudge_and_servants_follow_a_move() {
    // Without Power Nudge the move is a plain move.
    let mut h = Harness::new(nudge_layer());
    h.selection = Selection::from_items([Item::Point(0, 6)]);
    h.drag((240.0, 240.0), (264.0, 240.0), CTRL);
    assert_moves(&h, &[((0, 6), (24.0, 0.0))]);

    // With it, the smooth node halfway to the fixed corner follows by half, its handles with it.
    let mut h = Harness::new(nudge_layer());
    h.settings.power_nudge = true;
    h.selection = Selection::from_items([Item::Point(0, 6)]);
    h.drag((240.0, 240.0), (264.0, 240.0), CTRL);
    assert_moves(&h, &[((0, 2), (12.0, 0.0)), ((0, 3), (12.0, 0.0)), ((0, 4), (12.0, 0.0)), ((0, 6), (24.0, 0.0))]);
    assert_eq!(h.begun(), ["Move"]);

    // A declared y servant follows on y only, with Power Nudge off.
    let mut layer = nudge_layer();
    if let Some(Shape::Path(path)) = layer.shapes.first_mut() {
        let mut link = Dict::default();
        link.insert("axis", PValue::String(Str::from("y")));
        path.nodes[3].info.user_data.0.insert(fc_keys::SERVANT, PValue::Dict(link));
    }
    let mut h = Harness::new(layer);
    h.selection = Selection::from_items([Item::Point(0, 6)]);
    h.drag((240.0, 240.0), (264.0, 264.0), CTRL);
    assert_moves(&h, &[((0, 2), (0.0, 12.0)), ((0, 3), (0.0, 12.0)), ((0, 4), (0.0, 12.0)), ((0, 6), (24.0, 24.0))]);

    // An extreme stays fixed: with handles made horizontal, node 3 is an extreme.
    let mut layer = nudge_layer();
    if let Some(Shape::Path(path)) = layer.shapes.first_mut() {
        path.nodes[2] = off(80.0, 120.0);
        path.nodes[4] = off(160.0, 120.0);
    }
    let scene = Scene::new("A", "m01", &layer);
    let one = Selection::from_items([Item::Point(0, 6)]);
    assert!(Followers::new(&scene, &one, true).is_empty());
}
