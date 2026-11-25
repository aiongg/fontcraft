//! Behaviour tests: gestures in, actions out.

use fontcraft_geom::{BezPath, Point};
use fontcraft_model::{Alignment, Anchor, Component, Font, Guide, Layer, Node, NodeKind, Path, Shape};
use serde_json::{Value, json};

use crate::event::PointerKind::{DoubleClick, Down, Drag, Move, Up};
use crate::hand::HandTool;
use crate::lasso::LassoTool;
use crate::select::SelectTool;
use crate::selection::{in_region, select_all, step, tab_order};
use crate::zoom::{ZOOM_STEP, ZoomTool};
use crate::*;

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

fn guide(x: f64, y: f64, angle: f64) -> Guide {
    Guide { x, y, angle, ..Default::default() }
}

/// Shape 0: a closed square (0,0) (100,0) (100,100) (0,100). Shape 1: an open curve from (200,0)
/// through the handles (200,50) (250,100) to (300,100). Shape 2: an open line (400,0) to
/// (500,0). An anchor `top` at (50,150); a local guide along y = 300.
fn test_layer() -> Layer {
    let mut layer = Layer::new("m01", 600.0);
    layer.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
    layer.shapes.push(Shape::Path(Path::new(false, vec![on(200.0, 0.0), off(200.0, 50.0), off(250.0, 100.0), curve(300.0, 100.0)])));
    layer.shapes.push(Shape::Path(Path::new(false, vec![on(400.0, 0.0), on(500.0, 0.0)])));
    layer.anchors.push(Anchor::new("top", 50.0, 150.0));
    layer.info.guides.push(guide(0.0, 300.0, 0.0));
    layer
}

fn ev(kind: PointerKind, x: f64, y: f64, mods: Modifiers) -> PointerEvent {
    PointerEvent::new(kind, x, y, mods)
}

fn points(list: &[(usize, usize)]) -> Selection {
    Selection::from_items(list.iter().map(|&(s, n)| Item::Point(s, n)))
}

/// Stands in for the shell: owns the layer, the selection and the view, applies `Select`
/// actions, and records every action.
struct Harness {
    layer: Layer,
    selection: Selection,
    view: View,
    settings: ToolSettings,
    actions: Vec<Action>,
}

impl Harness {
    fn new(layer: Layer) -> Self {
        Self { layer, selection: Selection::new(), view: View::default(), settings: ToolSettings::default(), actions: Vec::new() }
    }

    fn run(&mut self, f: impl FnOnce(&Context<'_>) -> Output) -> Output {
        let scene = Scene::new("A", "m01", &self.layer);
        let cx = Context { scene: &scene, view: self.view, selection: &self.selection, settings: self.settings };
        let out = f(&cx);
        for a in &out.actions {
            if let Action::Select { selection } = a {
                self.selection = selection.clone();
            }
        }
        self.actions.extend(out.actions.iter().cloned());
        out
    }

    fn pointer(&mut self, tool: &mut dyn Tool, kind: PointerKind, x: f64, y: f64, mods: Modifiers) -> Output {
        self.run(|cx| tool.pointer(cx, &ev(kind, x, y, mods)))
    }

    fn click(&mut self, tool: &mut dyn Tool, x: f64, y: f64, mods: Modifiers) {
        self.pointer(tool, Down, x, y, mods);
        self.pointer(tool, Up, x, y, mods);
    }

    fn key(&mut self, tools: &mut Toolbox, key: Key, mods: Modifiers) -> Output {
        self.run(|cx| tools.key(cx, &KeyEvent::press(key, mods)))
    }

    fn previews(&self) -> Vec<&Invocation> {
        self.actions
            .iter()
            .filter_map(|a| match a {
                Action::Preview { invocation } => Some(invocation),
                _ => None,
            })
            .collect()
    }

    fn count(&self, pred: impl Fn(&Action) -> bool) -> usize {
        self.actions.iter().filter(|a| pred(a)).count()
    }
}

fn runs(out: &Output) -> Vec<&Invocation> {
    out.actions
        .iter()
        .filter_map(|a| match a {
            Action::Run { invocation } => Some(invocation),
            _ => None,
        })
        .collect()
}

#[test]
fn click_selects_the_nearest_node_within_tolerance() {
    let mut h = Harness::new(test_layer());
    let mut t = SelectTool::default();
    h.click(&mut t, 97.0, 2.0, NONE);
    assert_eq!(h.selection, points(&[(0, 1)]));

    // The tolerance is in screen points: at 4× zoom the same spot is out of reach.
    let layer = test_layer();
    let scene = Scene::new("A", "m01", &layer);
    assert!(hit_test(&scene, &View::new(1.0, Point::ZERO), Point::new(97.0, 2.0), 6.0).is_some());
    assert!(hit_test(&scene, &View::new(4.0, Point::ZERO), Point::new(97.0, 2.0), 6.0).is_none());

    // A click on empty canvas clears the selection.
    h.click(&mut t, 150.0, 50.0, NONE);
    assert!(h.selection.is_empty());
}

#[test]
fn a_node_beats_a_segment_and_a_segment_click_selects_its_ends() {
    let mut h = Harness::new(test_layer());
    let mut t = SelectTool::default();
    h.click(&mut t, 50.0, 1.0, NONE);
    assert_eq!(h.selection, points(&[(0, 0), (0, 1)]));
    h.click(&mut t, 3.0, 1.0, NONE);
    assert_eq!(h.selection, points(&[(0, 0)]));
}

#[test]
fn shift_click_toggles() {
    let mut h = Harness::new(test_layer());
    let mut t = SelectTool::default();
    h.selection = points(&[(0, 0)]);
    h.click(&mut t, 100.0, 0.0, Modifiers::SHIFT);
    assert_eq!(h.selection, points(&[(0, 0), (0, 1)]));
    h.click(&mut t, 0.0, 0.0, Modifiers::SHIFT);
    assert_eq!(h.selection, points(&[(0, 1)]));
}

#[test]
fn clicking_one_node_of_a_selection_selects_only_it() {
    let mut h = Harness::new(test_layer());
    let mut t = SelectTool::default();
    h.selection = points(&[(0, 0), (0, 1), (0, 2)]);
    h.click(&mut t, 100.0, 0.0, NONE);
    assert_eq!(h.selection, points(&[(0, 1)]));
}

#[test]
fn lasso_selects_the_enclosed_nodes() {
    let mut h = Harness::new(test_layer());
    let mut t = LassoTool::default();
    h.pointer(&mut t, Down, -10.0, -10.0, NONE);
    for (x, y) in [(50.0, -10.0), (110.0, -10.0), (110.0, 10.0), (50.0, 10.0), (-10.0, 10.0)] {
        h.pointer(&mut t, Drag, x, y, NONE);
    }
    assert!(!t.overlays().is_empty(), "the lasso outline is drawn while dragging");
    h.pointer(&mut t, Up, -10.0, 0.0, NONE);
    assert_eq!(h.selection, points(&[(0, 0), (0, 1)]));
    assert!(t.overlays().is_empty());

    // Alt: on-curve nodes only.
    let mut h = Harness::new(test_layer());
    h.pointer(&mut t, Down, 190.0, -10.0, Modifiers::ALT);
    for (x, y) in [(260.0, -10.0), (260.0, 110.0), (190.0, 110.0)] {
        h.pointer(&mut t, Drag, x, y, Modifiers::ALT);
    }
    h.pointer(&mut t, Up, 190.0, 0.0, Modifiers::ALT);
    assert_eq!(h.selection, points(&[(1, 0)]));
}

#[test]
fn marquee_selects_nodes_and_handles_and_alt_takes_nodes_only() {
    let mut h = Harness::new(test_layer());
    let mut t = SelectTool::default();
    h.pointer(&mut t, Down, 190.0, -10.0, NONE);
    h.pointer(&mut t, Drag, 260.0, 110.0, NONE);
    assert!(matches!(t.overlays().first(), Some(Overlay::Polygon { kind: OverlayKind::Marquee, .. })));
    h.pointer(&mut t, Up, 260.0, 110.0, NONE);
    assert_eq!(h.selection, points(&[(1, 0), (1, 1), (1, 2)]));

    let mut h = Harness::new(test_layer());
    h.pointer(&mut t, Down, 190.0, -10.0, Modifiers::ALT);
    h.pointer(&mut t, Up, 260.0, 110.0, Modifiers::ALT);
    assert_eq!(h.selection, points(&[(1, 0)]));
}

#[test]
fn ctrl_turns_the_marquee() {
    let mut h = Harness::new(test_layer());
    let mut t = SelectTool::default();
    h.pointer(&mut t, Down, -20.0, -20.0, NONE);
    h.pointer(&mut t, Drag, 20.0, 20.0, NONE);
    assert_eq!(h.selection, points(&[(0, 0)]));
    // Turning the 40 × 40 marquee by 45° about its start puts its far corner straight up, away
    // from (0, 0).
    let start_angle = (40.0f64).atan2(40.0);
    let r = 60.0;
    h.pointer(
        &mut t,
        Drag,
        -20.0 + r * (start_angle + std::f64::consts::FRAC_PI_4).cos(),
        -20.0 + r * (start_angle + std::f64::consts::FRAC_PI_4).sin(),
        Modifiers::CTRL,
    );
    h.pointer(&mut t, Drag, -20.0, 40.0, Modifiers::CTRL);
    let Some(Overlay::Polygon { points: corners, .. }) = t.overlays().first().cloned() else { panic!("no marquee") };
    let far = corners[2];
    assert!((far.x - -20.0).abs() < 1e-6 && (far.y - (40.0f64 * 2.0f64.sqrt() - 20.0)).abs() < 1e-6, "{far:?}");
}

#[test]
fn a_drag_is_one_interaction_with_one_move_command() {
    let mut h = Harness::new(test_layer());
    let mut t = SelectTool::default();
    h.pointer(&mut t, Down, 100.0, 0.0, NONE);
    h.pointer(&mut t, Drag, 110.0, 20.0, NONE);
    h.pointer(&mut t, Drag, 130.0, 40.0, NONE);
    h.pointer(&mut t, Up, 130.0, 40.0, NONE);
    assert_eq!(h.count(|a| matches!(a, Action::Begin { .. })), 1);
    assert_eq!(h.count(|a| matches!(a, Action::Commit)), 1);
    assert!(matches!(h.actions.last(), Some(Action::Commit)));
    let last = h.previews().last().cloned().cloned().unwrap();
    assert_eq!(last.id, "selection.move");
    assert_eq!(last.params, json!({"glyph": "A", "layer": "m01", "points": [[0, 1]], "dx": 30.0, "dy": 40.0}));
    assert!(!t.busy());
}

#[test]
fn a_press_that_does_not_move_past_the_threshold_is_a_click() {
    let mut h = Harness::new(test_layer());
    let mut t = SelectTool::default();
    h.pointer(&mut t, Down, 100.0, 0.0, NONE);
    h.pointer(&mut t, Drag, 101.0, 1.0, NONE);
    h.pointer(&mut t, Up, 101.0, 1.0, NONE);
    assert_eq!(h.count(|a| matches!(a, Action::Begin { .. } | Action::Preview { .. } | Action::Commit)), 0);
}

#[test]
fn option_drag_moves_a_duplicate() {
    let mut h = Harness::new(test_layer());
    let mut t = SelectTool::default();
    h.selection = points(&[(0, 0), (0, 1), (0, 2), (0, 3)]);
    h.pointer(&mut t, Down, 0.0, 100.0, Modifiers::ALT);
    h.pointer(&mut t, Drag, 50.0, 250.0, Modifiers::ALT);
    h.pointer(&mut t, Up, 50.0, 250.0, Modifiers::ALT);
    assert!(matches!(h.actions.first(), Some(Action::Begin { label }) if label == "Duplicate"));
    let last = h.previews().last().cloned().cloned().unwrap();
    assert_eq!(last.params["duplicate"], json!(true));
    assert_eq!(last.params["points"], json!([[0, 0], [0, 1], [0, 2], [0, 3]]));
    assert_eq!((last.params["dx"].as_f64(), last.params["dy"].as_f64()), (Some(50.0), Some(150.0)));
    assert!(last.params.get("mode").is_none());
}

#[test]
fn alt_pressed_during_a_drag_nudges() {
    let mut h = Harness::new(test_layer());
    let mut t = SelectTool::default();
    h.pointer(&mut t, Down, 100.0, 0.0, NONE);
    h.pointer(&mut t, Drag, 120.0, 30.0, NONE);
    h.pointer(&mut t, Drag, 130.0, 40.0, Modifiers::ALT);
    let last = h.previews().last().cloned().cloned().unwrap();
    assert_eq!(last.params["mode"], json!("nudge"));
    assert!(last.params.get("duplicate").is_none());
}

fn last_offset(h: &Harness) -> (f64, f64) {
    let p = &h.previews().last().unwrap().params;
    (p["dx"].as_f64().unwrap(), p["dy"].as_f64().unwrap())
}

#[test]
fn shift_holds_a_drag_to_the_nearer_axis_and_snaps_along_it_only() {
    let mut h = Harness::new(test_layer());
    let mut t = SelectTool::default();
    h.pointer(&mut t, Down, 100.0, 0.0, NONE);
    h.pointer(&mut t, Drag, 160.0, 37.0, NONE);
    assert_eq!(last_offset(&h), (60.0, 37.0));
    // Shift pressed and released during the drag, without moving the pointer.
    h.run(|cx| t.modifiers(cx, Modifiers::SHIFT));
    assert_eq!(last_offset(&h), (60.0, 0.0));
    h.run(|cx| t.modifiers(cx, NONE));
    assert_eq!(last_offset(&h), (60.0, 37.0));
    // A node on the held line is a target, and so is the guide that crosses it.
    h.pointer(&mut t, Drag, 198.0, 9.0, Modifiers::SHIFT);
    assert_eq!(last_offset(&h), (100.0, 0.0));
    h.pointer(&mut t, Drag, 109.0, 298.0, Modifiers::SHIFT);
    assert_eq!(last_offset(&h), (0.0, 300.0));
    h.pointer(&mut t, Up, 109.0, 298.0, Modifiers::SHIFT);

    // A lone handle goes onto the horizontal or the vertical through its node at (200,0).
    h.pointer(&mut t, Down, 200.0, 50.0, NONE);
    h.pointer(&mut t, Drag, 260.0, 30.0, Modifiers::SHIFT);
    assert_eq!(last_offset(&h), (60.0, -50.0));
    h.pointer(&mut t, Drag, 230.0, 70.0, Modifiers::SHIFT);
    assert_eq!(last_offset(&h), (0.0, 20.0));
    h.pointer(&mut t, Up, 230.0, 70.0, Modifiers::SHIFT);

    // An anchor and a guide, pressed with Shift held.
    h.selection.clear();
    h.pointer(&mut t, Down, 50.0, 150.0, Modifiers::SHIFT);
    h.pointer(&mut t, Drag, 63.0, 190.0, Modifiers::SHIFT);
    assert_eq!(last_offset(&h), (0.0, 40.0));
    h.pointer(&mut t, Up, 63.0, 190.0, Modifiers::SHIFT);
    h.selection.clear();
    h.pointer(&mut t, Down, 0.0, 300.0, Modifiers::SHIFT);
    h.pointer(&mut t, Drag, 40.0, 317.0, Modifiers::SHIFT);
    let guide = h.previews().last().cloned().cloned().unwrap();
    assert_eq!(guide.id, "guide.setProperties");
    assert_eq!((guide.params["x"].as_f64(), guide.params["y"].as_f64()), (Some(40.0), Some(300.0)));
    h.pointer(&mut t, Up, 40.0, 317.0, Modifiers::SHIFT);
}

#[test]
fn a_shift_press_adds_to_the_selection_and_drags_it_and_drops_a_selected_object_at_the_release() {
    let mut h = Harness::new(test_layer());
    let mut t = SelectTool::default();
    h.selection = points(&[(0, 0)]);
    h.pointer(&mut t, Down, 100.0, 0.0, Modifiers::SHIFT);
    assert_eq!(h.selection, points(&[(0, 0), (0, 1)]), "added at the press");
    h.pointer(&mut t, Drag, 109.0, 60.0, Modifiers::SHIFT);
    h.pointer(&mut t, Up, 109.0, 60.0, Modifiers::SHIFT);
    let moved = h.previews().last().cloned().cloned().unwrap();
    assert_eq!(moved.params["points"], json!([[0, 0], [0, 1]]));
    assert_eq!(last_offset(&h), (0.0, 60.0));
    assert!(matches!(h.actions.last(), Some(Action::Commit)));
    assert_eq!(h.selection, points(&[(0, 0), (0, 1)]), "a drag keeps the selection");

    // A selected object leaves at the release, and not when the press becomes a drag.
    h.pointer(&mut t, Down, 0.0, 0.0, Modifiers::SHIFT);
    assert_eq!(h.selection, points(&[(0, 0), (0, 1)]));
    h.pointer(&mut t, Drag, 9.0, 60.0, Modifiers::SHIFT);
    h.pointer(&mut t, Up, 9.0, 60.0, Modifiers::SHIFT);
    assert_eq!(h.selection, points(&[(0, 0), (0, 1)]));
    h.click(&mut t, 0.0, 0.0, Modifiers::SHIFT);
    assert_eq!(h.selection, points(&[(0, 1)]));
}

#[test]
fn escape_during_a_drag_cancels_it() {
    let mut h = Harness::new(test_layer());
    let mut tools = Toolbox::new();
    h.run(|cx| tools.pointer(cx, &ev(Down, 100.0, 0.0, NONE)));
    h.run(|cx| tools.pointer(cx, &ev(Drag, 140.0, 30.0, NONE)));
    assert!(tools.busy());
    let out = h.key(&mut tools, Key::Escape, NONE);
    assert!(out.handled);
    assert!(matches!(out.actions.as_slice(), [Action::Cancel]));
    assert!(!tools.busy());
    // The release that follows does nothing.
    let out = h.run(|cx| tools.pointer(cx, &ev(Up, 140.0, 30.0, NONE)));
    assert!(out.actions.is_empty());
}

#[test]
fn snapping_prefers_nodes_over_guides() {
    let mut layer = test_layer();
    layer.info.guides.push(guide(0.0, 100.0, 0.0));
    let scene = Scene::new("A", "m01", &layer);
    let view = View::default();
    let none = Moving::default();
    let s = snap(&scene, &view, Point::new(101.0, 102.0), &none, 4.0);
    assert_eq!((s.kind, s.point, s.node), (Some(SnapKind::Node), Point::new(100.0, 100.0), Some((0, 2))));
    let s = snap(&scene, &view, Point::new(150.0, 101.0), &none, 4.0);
    assert_eq!((s.kind, s.point), (Some(SnapKind::Guide), Point::new(150.0, 100.0)));
    let s = snap(&scene, &view, Point::new(201.0, 52.0), &none, 4.0);
    assert_eq!((s.kind, s.point), (Some(SnapKind::Handle), Point::new(200.0, 50.0)));
    // With nothing else near, lining up with a node on one axis.
    let s = snap(&scene, &view, Point::new(398.0, 60.0), &none, 4.0);
    assert_eq!((s.kind, s.point), (Some(SnapKind::Align), Point::new(400.0, 60.0)));
    assert!(!s.lines.is_empty());
    // Points that move with the drag are not targets.
    let moving = Moving::from_selection(&scene, &points(&[(0, 2)]));
    let s = snap(&scene, &view, Point::new(101.0, 102.0), &moving, 4.0);
    assert_eq!(s.kind, Some(SnapKind::Guide));
}

#[test]
fn ctrl_suspends_snapping_during_a_drag() {
    let mut h = Harness::new(test_layer());
    let mut t = SelectTool::default();
    h.pointer(&mut t, Down, 0.0, 0.0, NONE);
    h.pointer(&mut t, Drag, 99.0, 98.0, NONE);
    let snapped = h.previews().last().cloned().cloned().unwrap();
    assert_eq!((snapped.params["dx"].as_f64(), snapped.params["dy"].as_f64()), (Some(100.0), Some(100.0)));
    h.pointer(&mut t, Drag, 99.0, 98.0, Modifiers::CTRL);
    let free = h.previews().last().cloned().cloned().unwrap();
    assert_eq!((free.params["dx"].as_f64(), free.params["dy"].as_f64()), (Some(99.0), Some(98.0)));
}

#[test]
fn dropping_an_open_end_on_another_joins_them_in_one_step() {
    let mut h = Harness::new(test_layer());
    let mut t = SelectTool::default();
    h.pointer(&mut t, Down, 400.0, 0.0, NONE);
    h.pointer(&mut t, Drag, 350.0, 50.0, NONE);
    h.pointer(&mut t, Drag, 301.0, 101.0, NONE);
    h.pointer(&mut t, Up, 301.0, 101.0, NONE);
    assert_eq!(h.count(|a| matches!(a, Action::Begin { .. })), 1);
    assert!(matches!(h.actions.last(), Some(Action::Commit)));
    let last = h.previews().last().cloned().cloned().unwrap();
    assert_eq!(last.id, "command.batch");
    let steps = last.params["steps"].as_array().unwrap();
    assert_eq!(steps[0]["command"], json!("selection.move"));
    assert_eq!((steps[0]["params"]["dx"].as_f64(), steps[0]["params"]["dy"].as_f64()), (Some(-100.0), Some(100.0)));
    assert_eq!(steps[1]["command"], json!("path.connectNodes"));
    assert_eq!(steps[1]["params"]["points"], json!([[2, 0], [1, 3]]));
}

#[test]
fn alt_click_on_a_line_segment_makes_it_a_curve() {
    let mut h = Harness::new(test_layer());
    let mut t = SelectTool::default();
    h.click(&mut t, 50.0, 1.0, Modifiers::ALT);
    let run: Vec<&Invocation> = h
        .actions
        .iter()
        .filter_map(|a| match a {
            Action::Run { invocation } => Some(invocation),
            _ => None,
        })
        .collect();
    assert_eq!(run.len(), 1);
    assert_eq!(run[0].id, "path.convertSegmentToCurve");
    assert_eq!(run[0].params["segments"], json!([[0, 1]]));
    assert!(run[0].params.get("orthogonal").is_none());
    h.actions.clear();
    h.click(&mut t, 50.0, 1.0, Modifiers::SHIFT.with_alt());
    let Some(Action::Run { invocation }) = h.actions.last() else { panic!("{:?}", h.actions) };
    assert_eq!((invocation.id.as_str(), &invocation.params["orthogonal"]), ("path.convertSegmentToCurve", &json!(true)));
}

#[test]
fn double_clicks_toggle_a_node_or_select_a_path() {
    let mut h = Harness::new(test_layer());
    let mut t = SelectTool::default();
    h.pointer(&mut t, DoubleClick, 50.0, 1.0, NONE);
    assert_eq!(h.selection, points(&[(0, 0), (0, 1), (0, 2), (0, 3)]));
    let out = h.pointer(&mut t, DoubleClick, 0.0, 0.0, NONE);
    let run = runs(&out);
    assert_eq!(run.len(), 1);
    assert_eq!(run[0].id, "path.toggleNodeType");
    assert_eq!(run[0].params["points"], json!([[0, 0]]));
    let out = h.pointer(&mut t, DoubleClick, 0.0, 0.0, Modifiers::ALT);
    assert_eq!(runs(&out)[0].id, "path.toggleNodeTypeKeepingOutline");
}

#[test]
fn locked_points_select_but_do_not_move() {
    let mut layer = test_layer();
    if let Some(Shape::Path(p)) = layer.shapes.get_mut(0) {
        p.info.locked = true;
    }
    let mut h = Harness::new(layer);
    let mut t = SelectTool::default();
    h.pointer(&mut t, Move, 100.0, 0.0, NONE);
    assert!(matches!(t.overlays().first(), Some(Overlay::Marker { kind: OverlayKind::HoverLocked, .. })));
    h.pointer(&mut t, Down, 100.0, 0.0, NONE);
    h.pointer(&mut t, Drag, 150.0, 50.0, NONE);
    h.pointer(&mut t, Up, 150.0, 50.0, NONE);
    assert_eq!(h.selection, points(&[(0, 1)]));
    assert_eq!(h.count(|a| matches!(a, Action::Begin { .. } | Action::Preview { .. })), 0);
}

#[test]
fn arrow_keys_move_the_selection_by_the_increments() {
    let mut h = Harness::new(test_layer());
    let mut tools = Toolbox::new();
    assert!(!h.key(&mut tools, Key::Right, NONE).handled, "nothing selected");
    h.selection = points(&[(0, 0)]);
    let cases = [
        (Key::Right, NONE, json!(1.0), json!(0.0), None),
        (Key::Left, Modifiers::SHIFT, json!(-10.0), json!(0.0), None),
        (Key::Up, Modifiers::CMD, json!(0.0), json!(100.0), None),
        (Key::Down, Modifiers::ALT, json!(0.0), json!(-1.0), Some("pointsOnly")),
        (Key::Down, Modifiers::ALT.with_ctrl(), json!(0.0), json!(-1.0), Some("nudge")),
    ];
    for (key, mods, dx, dy, mode) in cases {
        let out = h.key(&mut tools, key, mods);
        let run = runs(&out);
        assert_eq!(run.len(), 1, "{key:?} {mods:?}");
        assert_eq!(run[0].id, "selection.move");
        assert_eq!((&run[0].params["dx"], &run[0].params["dy"]), (&dx, &dy), "{key:?} {mods:?}");
        assert_eq!(run[0].params.get("mode").and_then(Value::as_str), mode);
    }
}

#[test]
fn edit_view_keys_run_their_commands() {
    let mut h = Harness::new(test_layer());
    let mut tools = Toolbox::new();
    h.selection = points(&[(1, 0), (1, 1)]);
    let out = h.key(&mut tools, Key::Return, NONE);
    assert_eq!(runs(&out)[0].id, "path.toggleNodeType");
    assert_eq!(runs(&out)[0].params["points"], json!([[1, 0]]), "only on-curve nodes");
    let out = h.key(&mut tools, Key::Delete, NONE);
    assert_eq!(runs(&out)[0].id, "edit.delete");
    assert_eq!(runs(&out)[0].params["points"], json!([[1, 0], [1, 1]]));
    let out = h.key(&mut tools, Key::Delete, Modifiers::SHIFT);
    assert_eq!(runs(&out)[0].id, "path.deleteAndBreak");
    let out = h.key(&mut tools, Key::Delete, Modifiers { shift: true, ..Modifiers::ALT });
    assert_eq!(runs(&out)[0].id, "path.deleteAndBreakInAllMasters");
    let out = h.key(&mut tools, Key::Tab, NONE);
    assert_eq!(runs(&out)[0].id, "selection.selectNext");
    let out = h.key(&mut tools, Key::Tab, Modifiers::SHIFT);
    assert_eq!(runs(&out)[0].id, "selection.selectPrevious");
    assert!(!h.key(&mut tools, Key::Char('a'), Modifiers::CMD).handled, "global shortcuts pass through");
}

#[test]
fn tool_letters_and_space_choose_tools() {
    let mut h = Harness::new(test_layer());
    let mut tools = Toolbox::new();
    let out = h.key(&mut tools, Key::Char('h'), NONE);
    assert_eq!(runs(&out)[0].params, json!({"tool": "hand"}));
    let out = h.key(&mut tools, Key::Char('v'), Modifiers::SHIFT);
    assert_eq!(runs(&out)[0].params, json!({"tool": "lasso"}));
    let out = h.key(&mut tools, Key::Char('e'), NONE);
    assert_eq!(runs(&out)[0].params, json!({"tool": "knife"}), "every tool's letter comes from the tool list");

    let switch = tools.select("lasso").unwrap();
    assert_eq!((switch.tool, switch.previous), ("lasso", "select"));
    assert_eq!(tools.select("zoom").unwrap().previous, "lasso");
    let out = h.key(&mut tools, Key::Char('v'), NONE);
    assert_eq!(runs(&out)[0].params, json!({"tool": "lasso"}), "V returns to the group's last tool");
    assert_eq!(tools.last_in_group("select"), Some("lasso"), "the toolbar's shared button shows it too");
    assert_eq!(tools.last_in_group("annotation"), None, "no annotation tool chosen yet");
    assert!(tools.select("no such tool").is_err());
    assert_eq!(tools.base_id(), "zoom");

    tools.select("select").unwrap();
    assert!(h.key(&mut tools, Key::Space, NONE).handled);
    assert_eq!(tools.active_id(), "hand");
    h.run(|cx| tools.key(cx, &KeyEvent::release(Key::Space, NONE)));
    assert_eq!(tools.active_id(), "select");
    h.key(&mut tools, Key::Space, Modifiers::CMD);
    assert_eq!(tools.active_id(), "zoom");

    let list = tools.list();
    assert_eq!(list["active"], json!("zoom"));
    assert_eq!(list["tools"].as_array().map(Vec::len), Some(TOOLS.len()));
}

#[test]
fn a_drag_in_progress_is_cancelled_by_a_tool_switch() {
    let mut h = Harness::new(test_layer());
    let mut tools = Toolbox::new();
    h.run(|cx| tools.pointer(cx, &ev(Down, 100.0, 0.0, NONE)));
    h.run(|cx| tools.pointer(cx, &ev(Drag, 140.0, 30.0, NONE)));
    let switch = tools.select("hand").unwrap();
    assert!(matches!(switch.output.actions.as_slice(), [Action::Cancel]));
}

#[test]
fn hand_pans_and_zoom_zooms() {
    let mut h = Harness::new(test_layer());
    h.view = View::new(2.0, Point::new(10.0, 500.0));
    let mut hand = HandTool::default();
    h.pointer(&mut hand, Down, 0.0, 0.0, NONE);
    let out = h.pointer(&mut hand, Drag, 10.0, 5.0, NONE);
    assert_eq!(out.actions, vec![Action::View { change: ViewChange::Pan { dx: 20.0, dy: -10.0 } }]);
    h.pointer(&mut hand, Up, 10.0, 5.0, NONE);
    assert!(!hand.busy());

    let mut zoom = ZoomTool::default();
    h.pointer(&mut zoom, Down, 10.0, 10.0, NONE);
    let out = h.pointer(&mut zoom, Up, 10.0, 10.0, NONE);
    assert_eq!(out.actions, vec![Action::View { change: ViewChange::Zoom { factor: ZOOM_STEP, at: Point::new(10.0, 10.0) } }]);
    h.pointer(&mut zoom, Down, 10.0, 10.0, Modifiers::ALT);
    let out = h.pointer(&mut zoom, Up, 10.0, 10.0, Modifiers::ALT);
    assert_eq!(out.actions, vec![Action::View { change: ViewChange::Zoom { factor: 1.0 / ZOOM_STEP, at: Point::new(10.0, 10.0) } }]);
    h.pointer(&mut zoom, Down, 100.0, 50.0, NONE);
    h.pointer(&mut zoom, Drag, 0.0, 0.0, NONE);
    let out = h.pointer(&mut zoom, Up, 0.0, 0.0, NONE);
    assert_eq!(out.actions, vec![Action::View { change: ViewChange::ZoomToRect { x: 0.0, y: 0.0, width: 100.0, height: 50.0 } }]);
}

#[test]
fn components_are_hit_inside_their_outline_and_move_as_shapes() {
    let mut font = Font::new("Test");
    font.glyph_mut("B").unwrap().layer_mut("m01").unwrap().shapes.push(Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
    let mut c = Component::new("B", 200.0, 0.0);
    // Alone on the layer, the component would be aligned automatically and could not be dragged.
    c.info.alignment = Alignment::Disabled;
    font.glyph_mut("A").unwrap().layer_mut("m01").unwrap().shapes.push(Shape::Component(c));
    let scene = Scene::from_font(&font, "A", None).unwrap();
    assert_eq!(scene.components().len(), 1);
    let view = View::default();
    let hit = hit_test(&scene, &view, Point::new(250.0, 50.0), 6.0).unwrap();
    assert_eq!(hit.hit, Hit::Component { shape: 0 });
    assert!(hit_test(&scene, &view, Point::new(150.0, 50.0), 6.0).is_none());
    // The master's metric lines come with the scene: x-height is 500.
    let s = snap(&scene, &view, Point::new(50.0, 498.0), &Moving::default(), 4.0);
    assert_eq!((s.kind, s.point), (Some(SnapKind::Metric), Point::new(50.0, 500.0)));
    // A component's corners are node targets.
    let s = snap(&scene, &view, Point::new(302.0, 101.0), &Moving::default(), 4.0);
    assert_eq!((s.kind, s.point), (Some(SnapKind::Node), Point::new(300.0, 100.0)));

    let selection = Selection::from_items([Item::Shape(0)]);
    let cx = Context { scene: &scene, view, selection: &selection, settings: ToolSettings::default() };
    let mut t = SelectTool::default();
    let mut actions = Vec::new();
    for e in [ev(Down, 250.0, 50.0, NONE), ev(Drag, 260.0, 70.0, Modifiers::CTRL), ev(Up, 260.0, 70.0, Modifiers::CTRL)] {
        actions.extend(t.pointer(&cx, &e).actions);
    }
    let last = actions
        .iter()
        .rev()
        .find_map(|a| match a {
            Action::Preview { invocation } => Some(invocation),
            _ => None,
        })
        .unwrap();
    assert_eq!(last.params, json!({"glyph": "A", "layer": "m01", "shapes": [0], "dx": 10.0, "dy": 20.0}));
}

/// While a path node is dragged near a component, the component's nearby nodes show as small markers; far from it,
/// or with snapping suspended, none do.
#[test]
fn a_drag_near_a_component_shows_its_nodes_as_markers() {
    let mut font = Font::new("Test");
    font.glyph_mut("B").unwrap().layer_mut("m01").unwrap().shapes.push(Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
    let mut c = Component::new("B", 200.0, 0.0);
    c.info.alignment = Alignment::Disabled;
    {
        let mut glyph = font.glyph_mut("A").unwrap();
        let layer = glyph.layer_mut("m01").unwrap();
        layer.shapes.push(Shape::Component(c));
        layer.shapes.push(Shape::Path(Path::rect(-300.0, 0.0, -200.0, 100.0)));
    }
    let scene = Scene::from_font(&font, "A", None).unwrap();
    let selection = points(&[(1, 2)]);
    let cx = Context { scene: &scene, view: View::default(), selection: &selection, settings: ToolSettings::default() };
    let markers = |t: &SelectTool| t.overlays().into_iter().filter(|o| matches!(o, Overlay::Marker { kind: OverlayKind::ComponentNode, .. })).count();
    let corner = match scene.node(1, 2) {
        Some(n) => n.point(),
        None => Point::new(-200.0, 100.0),
    };
    let mut t = SelectTool::default();
    let _ = t.pointer(&cx, &ev(Down, corner.x, corner.y, NONE));
    // From (-200, 100) to 10 units off the component's corner at (200, 100).
    let _ = t.pointer(&cx, &ev(Drag, 190.0, 100.0, NONE));
    assert!(markers(&t) >= 1, "the corner near the dragged node shows: {:?}", t.overlays());
    let _ = t.pointer(&cx, &ev(Drag, 190.0, 100.0, Modifiers::CTRL));
    assert_eq!(markers(&t), 0, "Ctrl suspends snapping and the markers");
    let _ = t.pointer(&cx, &ev(Drag, -100.0, 400.0, NONE));
    assert_eq!(markers(&t), 0, "far from the component, none show");
    let _ = t.pointer(&cx, &ev(Up, -100.0, 400.0, NONE));
    assert!(component_markers(&scene, &View::default(), Point::new(1.0e9, f64::NAN), &Moving::default(), 4.0).is_empty());
}

/// Drags a component of glyph `A` from inside its outline and returns the actions.
fn drag_component(scene: &Scene) -> Vec<Action> {
    let selection = Selection::from_items([Item::Shape(0)]);
    let cx = Context { scene, view: View::default(), selection: &selection, settings: ToolSettings::default() };
    let mut t = SelectTool::default();
    let mut actions = Vec::new();
    for e in [ev(Down, 250.0, 50.0, NONE), ev(Drag, 260.0, 70.0, Modifiers::CTRL), ev(Up, 260.0, 70.0, Modifiers::CTRL)] {
        actions.extend(t.pointer(&cx, &e).actions);
    }
    actions
}

#[test]
fn locked_and_fully_aligned_components_are_not_dragged() {
    // A font whose glyph `A` holds one component of `B` (a 100-unit square) at x 200.
    let font_with = |alignment: Alignment, locked: bool| {
        let mut font = Font::new("Test");
        font.glyph_mut("B").unwrap().layer_mut("m01").unwrap().shapes.push(Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
        let mut c = Component::new("B", 200.0, 0.0);
        c.info.alignment = alignment;
        c.info.locked = locked;
        font.glyph_mut("A").unwrap().layer_mut("m01").unwrap().shapes.push(Shape::Component(c));
        font
    };
    let moves = |a: &Action| matches!(a, Action::Begin { .. } | Action::Preview { .. } | Action::Run { .. });

    // Alone on the layer, with the default alignment: aligned in both directions.
    let font = font_with(Alignment::Default, false);
    let scene = Scene::from_font(&font, "A", None).unwrap();
    assert!(hit_test(&scene, &View::default(), Point::new(250.0, 50.0), 6.0).unwrap().locked);
    assert!(!drag_component(&scene).iter().any(moves), "an aligned component stays put");

    let font = font_with(Alignment::Disabled, true);
    let scene = Scene::from_font(&font, "A", None).unwrap();
    assert!(!drag_component(&scene).iter().any(moves), "a locked component stays put");

    // Aligned horizontally only: the drag goes ahead, and `selection.move` keeps its x.
    let font = font_with(Alignment::Horizontal, false);
    let scene = Scene::from_font(&font, "A", None).unwrap();
    assert!(drag_component(&scene).iter().any(|a| matches!(a, Action::Preview { invocation } if invocation.id == "selection.move")));
}

#[test]
fn selection_helpers_follow_the_rows() {
    let layer = test_layer();
    let scene = Scene::new("A", "m01", &layer);
    // Select All: points first, then anchors and components too.
    let first = select_all(&scene, &Selection::new());
    assert_eq!(first.points.len(), 10);
    assert!(first.anchors.is_empty());
    let second = select_all(&scene, &first);
    assert!(second.anchors.contains("top"));
    // Tab walks points in path order, then anchors, then guides, and wraps.
    let order = tab_order(&scene);
    assert_eq!(order.len(), 12);
    assert_eq!(step(&scene, &points(&[(0, 3)]), true), Some(points(&[(1, 0)])));
    let last = Selection::from_items([Item::Guide(GuideRef { scope: GuideScope::Local, index: 0 })]);
    assert_eq!(step(&scene, &last, true), Some(points(&[(0, 0)])));
    assert_eq!(step(&scene, &points(&[(0, 0)]), false), Some(last));
    assert_eq!(step(&scene, &points(&[(0, 0), (0, 1)]), true), None);
    // The target object round-trips through JSON.
    let mut s = points(&[(0, 1)]);
    s.anchors.insert("top".into());
    s.guides.insert(GuideRef { scope: GuideScope::Local, index: 0 });
    let t = Value::Object(s.targets(&scene));
    assert_eq!(t["guides"], json!([{"scope": "local", "index": 0}]));
    assert_eq!(Selection::from_value(&t).unwrap(), s);
}

#[test]
fn hostile_input_never_panics() {
    let nan = f64::NAN;
    let huge = 1e300;
    let mut bad = Layer::new("m01", nan);
    bad.shapes.push(Shape::Path(Path::new(true, vec![on(nan, 0.0), on(huge, -huge), curve(5.0, 5.0)])));
    bad.shapes.push(Shape::Path(Path::new(false, vec![off(1.0, 1.0), on(2.0, 2.0), Node::new(3.0, 3.0, NodeKind::QCurve)])));
    bad.shapes.push(Shape::Path(Path::new(true, vec![off(0.0, 0.0), off(10.0, 0.0), off(10.0, 10.0)])));
    bad.shapes.push(Shape::Path(Path::new(true, Vec::new())));
    let mut q = vec![on(0.0, 0.0)];
    q.extend((0..12_000).map(|i| off(f64::from(i), 1.0)));
    q.push(Node::new(0.0, 2.0, NodeKind::QCurve));
    bad.shapes.push(Shape::Path(Path::new(true, q)));
    bad.shapes.push(Shape::Component(Component::new("missing", nan, 0.0)));
    bad.anchors.push(Anchor::new("x", nan, huge));
    bad.info.guides.push(guide(nan, 0.0, nan));
    bad.info.guides.push(guide(0.0, 0.0, huge));
    let empty = Layer::new("m01", 0.0);

    for layer in [bad, empty, test_layer()] {
        let views = [
            View::default(),
            View::new(nan, Point::new(nan, huge)),
            View::new(0.0, Point::ZERO),
            View::new(-3.0, Point::ZERO),
            View::new(f64::INFINITY, Point::ZERO),
        ];
        for view in views {
            let mut h = Harness::new(layer.clone());
            h.view = view;
            h.selection = points(&[(99, 99), (0, 0), (4, 7)]);
            h.selection.anchors.insert("gone".into());
            h.selection.guides.insert(GuideRef { scope: GuideScope::Global, index: 9 });
            {
                let mut scene = Scene::new("A", "m01", &h.layer);
                let mut outline = BezPath::new();
                outline.move_to((nan, 0.0));
                outline.line_to((huge, huge));
                outline.close_path();
                scene.add_component(5, &outline, false);
                for p in [Point::new(nan, nan), Point::new(huge, -huge), Point::new(0.0, 0.0), Point::new(5.0, 1.0)] {
                    let _ = hit_test(&scene, &view, p, 6.0);
                    let _ = hit_test(&scene, &view, p, nan);
                    let _ = snap(&scene, &view, p, &Moving::from_selection(&scene, &h.selection), 4.0);
                    let _ = snap(&scene, &view, p, &Moving::default(), huge);
                }
                let _ = in_region(&scene, &[Point::new(nan, 0.0), Point::new(huge, huge), Point::new(-huge, 0.0)], false, true);
                let _ = select_all(&scene, &h.selection);
                let _ = step(&scene, &h.selection, true);
                let _ = h.selection.targets(&scene);
                let mut s = h.selection.clone();
                s.retain_valid(&scene);
                assert!(s.points.iter().all(|&(sh, n)| scene.node(sh, n).is_some()));
            }
            let mut tools: Vec<Box<dyn Tool>> = TOOLS.iter().filter_map(|t| make_tool(t.id)).collect();
            for tool in &mut tools {
                for mods in [NONE, Modifiers::ALT.with_ctrl()] {
                    for (x, y) in [(nan, nan), (huge, -huge), (0.0, 0.0), (-huge, huge)] {
                        for kind in [Move, Down, Drag, Drag, Up, DoubleClick, Up] {
                            h.pointer(tool.as_mut(), kind, x, y, mods);
                        }
                    }
                }
                let _ = h.run(|cx| tool.key(cx, &KeyEvent::press(Key::Escape, NONE)));
                let _ = tool.cancel();
            }
            let mut toolbox = Toolbox::new();
            for key in [Key::Left, Key::Up, Key::Return, Key::Delete, Key::Backspace, Key::Tab, Key::Space, Key::Char('v'), Key::Escape] {
                for mods in [NONE, Modifiers::SHIFT, Modifiers::ALT, Modifiers::CMD, Modifiers::ALT.with_ctrl()] {
                    h.key(&mut toolbox, key, mods);
                }
            }
            h.settings = ToolSettings {
                hit_tolerance: nan,
                snap_distance: huge,
                drag_threshold: -1.0,
                increments: [nan, huge, 0.0],
                ..ToolSettings::default()
            };
            let mut t = SelectTool::default();
            for kind in [Down, Drag, Drag, Up] {
                h.pointer(&mut t, kind, 1.0, 1.0, NONE);
            }
        }
    }

    // A lasso drawn with very many points keeps at most the cap.
    let mut h = Harness::new(test_layer());
    let mut lasso = LassoTool::default();
    h.pointer(&mut lasso, Down, 0.0, 0.0, NONE);
    for i in 0..(MAX_LASSO_POINTS + 5_000) {
        let a = i as f64 * 0.01;
        h.pointer(&mut lasso, Drag, 1000.0 * a.cos(), 1000.0 * a.sin() + i as f64, NONE);
    }
    match lasso.overlays().first() {
        Some(Overlay::Polygon { points, .. }) => assert!(points.len() <= MAX_LASSO_POINTS),
        other => panic!("expected the lasso outline, got {other:?}"),
    }
    h.pointer(&mut lasso, Up, 0.0, 0.0, NONE);

    // A selection read from outside is capped and checked.
    let list: Vec<Value> = (0..=MAX_SELECTION_LIST).map(|_| json!([0, 0])).collect();
    let too_many = json!({ "points": Value::Array(list) });
    assert!(Selection::from_value(&too_many).is_err());
    assert!(Selection::from_value(&json!({"points": [["x", -1]]})).is_err());
    assert!(Selection::from_value(&json!({"guides": [{"scope": "sideways", "index": 0}]})).is_err());
    assert_eq!(Selection::from_value(&json!({})).unwrap(), Selection::new());
}

#[test]
fn key_names_and_modifier_names_parse() {
    assert_eq!(Key::parse("ArrowLeft"), Some(Key::Left));
    assert_eq!(Key::parse("Enter"), Some(Key::Return));
    assert_eq!(Key::parse("V"), Some(Key::Char('v')));
    assert_eq!(Key::parse(" "), Some(Key::Space));
    assert_eq!(Key::parse("Hyper"), None);
    assert_eq!(Modifiers::from_names(["shift", "Option"]), Ok(Modifiers::SHIFT.with_alt()));
    assert!(Modifiers::from_names(["fn"]).is_err());
}

#[test]
fn the_font_sets_the_arrow_increments() {
    let mut font = Font::new("Test");
    std::sync::Arc::make_mut(&mut font.info).settings.keyboard_increment_big = Some(20.0);
    std::sync::Arc::make_mut(&mut font.info).settings.keyboard_increment_huge = Some(f64::NAN);
    let s = ToolSettings::default().with_font(&font);
    assert_eq!(s.increments, [1.0, 20.0, 100.0]);
}

/// One open path: a corner (0,0), handles (0,100) (100,200), a smooth node (200,200), handles
/// (300,200) (400,100), and a corner (400,0).
fn smooth_layer() -> Layer {
    let mut layer = Layer::new("m01", 600.0);
    let mut node = curve(200.0, 200.0);
    node.smooth = true;
    let nodes = vec![on(0.0, 0.0), off(0.0, 100.0), off(100.0, 200.0), node, off(300.0, 200.0), off(400.0, 100.0), curve(400.0, 0.0)];
    layer.shapes.push(Shape::Path(Path::new(false, nodes)));
    layer
}

/// Press at `from` and drag to `to` with `mods`, then release; the last preview.
fn modifier_drag(h: &mut Harness, from: (f64, f64), to: (f64, f64), mods: Modifiers) -> Invocation {
    let mut t = SelectTool::default();
    h.actions.clear();
    h.pointer(&mut t, Down, from.0, from.1, mods);
    h.pointer(&mut t, Drag, (from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0, mods);
    h.pointer(&mut t, Drag, to.0, to.1, mods);
    h.pointer(&mut t, Up, to.0, to.1, mods);
    assert_eq!(h.count(|a| matches!(a, Action::Commit)), 1, "one interaction");
    h.previews().last().cloned().cloned().unwrap()
}

#[test]
fn alt_on_a_handle_keeps_its_angle_and_ctrl_alt_mirrors_it() {
    let mut h = Harness::new(smooth_layer());
    let kept = modifier_drag(&mut h, (300.0, 200.0), (337.0, 261.0), Modifiers::ALT);
    assert_eq!(kept.id, "selection.move");
    assert_eq!(kept.params["points"], json!([[0, 4]]));
    assert_eq!(kept.params["mode"], json!("pointsOnly"));
    assert!(kept.params.get("duplicate").is_none(), "a handle alone has nothing to copy");
    assert!(matches!(h.actions.iter().find(|a| matches!(a, Action::Begin { .. })), Some(Action::Begin { label }) if label == "Move"));

    let mirrored = modifier_drag(&mut h, (300.0, 200.0), (337.0, 261.0), Modifiers::ALT.with_ctrl());
    assert_eq!(mirrored.params["mode"], json!("mirror"));
    assert_eq!((mirrored.params["dx"].as_f64(), mirrored.params["dy"].as_f64()), (Some(37.0), Some(61.0)), "Ctrl suspends snapping");

    // Alt added during a handle drag keeps the angle; it does not nudge.
    let mut t = SelectTool::default();
    h.actions.clear();
    h.pointer(&mut t, Down, 300.0, 200.0, NONE);
    h.pointer(&mut t, Drag, 320.0, 230.0, NONE);
    h.pointer(&mut t, Drag, 337.0, 261.0, Modifiers::ALT);
    assert_eq!(h.previews().last().unwrap().params["mode"], json!("pointsOnly"));
}

#[test]
fn alt_on_lone_nodes_moves_only_them_and_a_selected_segment_still_duplicates() {
    let mut h = Harness::new(test_layer());
    let lone = modifier_drag(&mut h, (300.0, 100.0), (337.0, 161.0), Modifiers::ALT);
    assert_eq!(lone.params["points"], json!([[1, 3]]));
    assert_eq!(lone.params["mode"], json!("pointsOnly"));
    assert!(lone.params.get("duplicate").is_none());
    assert!(matches!(h.actions.iter().find(|a| matches!(a, Action::Begin { .. })), Some(Action::Begin { label }) if label == "Move"));

    // The two ends of the line: a whole segment to copy.
    h.selection = points(&[(2, 0), (2, 1)]);
    let copy = modifier_drag(&mut h, (400.0, 0.0), (437.0, 61.0), Modifiers::ALT);
    assert_eq!(copy.params["duplicate"], json!(true));
    assert!(copy.params.get("mode").is_none());
}

#[test]
fn alt_on_a_curve_segment_reshapes_it() {
    let mut h = Harness::new(test_layer());
    // (231.25, 68.75) is the curve's point at t = 0.5.
    let inv = modifier_drag(&mut h, (231.25, 68.75), (241.25, 88.75), Modifiers::ALT);
    assert!(matches!(h.actions.iter().find(|a| matches!(a, Action::Begin { .. })), Some(Action::Begin { label }) if label == "Reshape Segment"));
    assert_eq!(inv.id, "path.reshapeSegment");
    assert_eq!(inv.params["segment"], json!([1, 3]));
    let t = inv.params["t"].as_f64().unwrap();
    assert!((t - 0.5).abs() < 0.01, "t = {t}");
    let to = (inv.params["to"][0].as_f64().unwrap(), inv.params["to"][1].as_f64().unwrap());
    assert!((to.0 - 241.25).abs() < 0.5 && (to.1 - 88.75).abs() < 0.5, "to = {to:?}");
    assert!(h.previews().iter().all(|i| i.id == "path.reshapeSegment"), "no move is sent");
    assert_eq!(h.selection, points(&[(1, 0), (1, 3)]), "the press selects the segment's ends");

    // A line segment has no handles: Alt-dragging it copies it.
    h.selection = Selection::new();
    let line = modifier_drag(&mut h, (450.0, 0.0), (487.0, 61.0), Modifiers::ALT);
    assert_eq!(line.id, "selection.move");
    assert_eq!(line.params["duplicate"], json!(true));
}
