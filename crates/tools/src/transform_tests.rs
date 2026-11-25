//! Behaviour tests for the bounding-box knobs and the Rotate and Scale tools: gestures in,
//! interactions and transform commands out.

use fontcraft_geom::{BezPath, Point, Rect};
use fontcraft_model::{Anchor, Component, Layer, Path, Shape};
use serde_json::{Value, json};

use crate::bbox::{Grab, Knob, selection_box, shown_box};
use crate::event::PointerKind::{Down, Drag, Up};
use crate::rotate::RotateTool;
use crate::scale::ScaleTool;
use crate::select::SelectTool;
use crate::*;

const NONE: Modifiers = Modifiers::NONE;

/// Shape 0: the closed square (0,0) (100,0) (100,100) (0,100). An anchor `top` at (50,150).
fn square_layer() -> Layer {
    let mut layer = Layer::new("m01", 600.0);
    layer.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
    layer.anchors.push(Anchor::new("top", 50.0, 150.0));
    layer
}

fn square() -> Selection {
    Selection::from_items((0..4).map(|n| Item::Point(0, n)))
}

/// Stands in for the shell, like the harness of `tests.rs`: owns the layer, selection, view and
/// settings, applies `Select` actions and records every action. View: one screen point per unit.
struct Shell {
    layer: Layer,
    selection: Selection,
    settings: ToolSettings,
    actions: Vec<Action>,
}

impl Shell {
    fn new(selection: Selection) -> Self {
        let settings = ToolSettings { bounding_box: true, ..ToolSettings::default() };
        Self { layer: square_layer(), selection, settings, actions: Vec::new() }
    }

    fn run(&mut self, f: impl FnOnce(&Context<'_>) -> Output) -> Output {
        let scene = Scene::new("A", "m01", &self.layer);
        let cx = Context { scene: &scene, view: View::default(), selection: &self.selection, settings: self.settings };
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
        self.run(|cx| tool.pointer(cx, &PointerEvent::new(kind, x, y, mods)))
    }

    fn drag(&mut self, tool: &mut dyn Tool, from: (f64, f64), to: (f64, f64), mods: Modifiers) {
        self.pointer(tool, Down, from.0, from.1, mods);
        self.pointer(tool, Drag, to.0, to.1, mods);
        self.pointer(tool, Up, to.0, to.1, mods);
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

    fn last_preview(&self) -> Invocation {
        self.previews().last().map(|i| (*i).clone()).unwrap()
    }

    fn begins(&self) -> Vec<String> {
        self.actions
            .iter()
            .filter_map(|a| match a {
                Action::Begin { label } => Some(label.clone()),
                _ => None,
            })
            .collect()
    }
}

fn square_targets(mut extra: Value) -> Value {
    let m = extra.as_object_mut().unwrap();
    m.insert("glyph".into(), json!("A"));
    m.insert("layer".into(), json!("m01"));
    m.insert("points".into(), json!([[0, 0], [0, 1], [0, 2], [0, 3]]));
    extra
}

#[test]
fn the_box_follows_the_selection_and_needs_more_than_one_item() {
    let layer = square_layer();
    let scene = Scene::new("A", "m01", &layer);
    assert_eq!(selection_box(&scene, &square()), Some(Rect::new(0.0, 0.0, 100.0, 100.0)));
    let with_anchor = Selection::from_items([Item::Point(0, 0), Item::Anchor("top".into())]);
    assert_eq!(selection_box(&scene, &with_anchor), Some(Rect::new(0.0, 0.0, 50.0, 150.0)));
    assert_eq!(selection_box(&scene, &Selection::new()), Some(Rect::new(0.0, 0.0, 100.0, 100.0)), "nothing selected: the layer's shapes");
    assert_eq!(shown_box(&scene, &Selection::from_items([Item::Point(0, 1)])), None, "one node has no box");
    assert!(shown_box(&scene, &Selection::from_items([Item::Shape(0)])).is_some(), "a whole path has one");

    let mut layer = square_layer();
    layer.shapes.push(Shape::Component(Component::new("B", 0.0, 0.0)));
    let mut scene = Scene::new("A", "m01", &layer);
    let mut outline = BezPath::new();
    outline.move_to((200.0, 0.0));
    outline.line_to((300.0, 0.0));
    outline.line_to((300.0, 80.0));
    outline.close_path();
    scene.add_component(1, &outline, false);
    assert_eq!(selection_box(&scene, &Selection::from_items([Item::Shape(1)])), Some(Rect::new(200.0, 0.0, 300.0, 80.0)));
}

#[test]
fn a_corner_knob_scales_about_the_opposite_corner_in_one_interaction() {
    let mut shell = Shell::new(square());
    let mut tool = SelectTool::default();
    // The box is drawn 6 points outside the selection: the top-right knob is at (106, 106).
    shell.drag(&mut tool, (106.0, 106.0), (156.0, 106.0), NONE);
    assert_eq!(shell.begins(), vec!["Scale".to_string()]);
    let p = shell.last_preview();
    assert_eq!(p.id, "transform.scale");
    assert_eq!(p.params, square_targets(json!({"x": 150.0, "y": 100.0, "origin": [0.0, 0.0]})));
    assert_eq!(shell.actions.last(), Some(&Action::Commit));
    assert_eq!(shell.selection, square(), "the selection stays");
    assert!(!tool.busy());
}

#[test]
fn shift_keeps_proportions_and_alt_scales_about_the_centre() {
    let mut shell = Shell::new(square());
    let mut tool = SelectTool::default();
    shell.pointer(&mut tool, Down, 106.0, 106.0, NONE);
    shell.pointer(&mut tool, Drag, 156.0, 126.0, Modifiers::SHIFT);
    assert_eq!(shell.last_preview().params, square_targets(json!({"x": 150.0, "y": 150.0, "origin": [0.0, 0.0]})));
    shell.run(|cx| tool.modifiers(cx, Modifiers::ALT));
    assert_eq!(shell.last_preview().params, square_targets(json!({"x": 200.0, "y": 140.0, "origin": [50.0, 50.0]})));
    shell.pointer(&mut tool, Up, 156.0, 126.0, Modifiers::ALT);
    assert_eq!(shell.actions.last(), Some(&Action::Commit));
    assert_eq!(shell.begins().len(), 1, "one interaction for the whole drag");
}

#[test]
fn a_side_knob_scales_one_axis_and_escape_cancels() {
    let mut shell = Shell::new(square());
    let mut tool = SelectTool::default();
    shell.pointer(&mut tool, Down, 106.0, 50.0, NONE);
    shell.pointer(&mut tool, Drag, 56.0, 80.0, NONE);
    assert_eq!(shell.last_preview().params, square_targets(json!({"x": 50.0, "y": 100.0, "origin": [0.0, 50.0]})));
    let out = shell.run(|cx| tool.key(cx, &KeyEvent::press(Key::Escape, NONE)));
    assert!(out.handled);
    assert_eq!(shell.actions.last(), Some(&Action::Cancel));
    assert!(!tool.busy());
}

#[test]
fn a_drag_just_outside_the_box_rotates_about_the_opposite_knob() {
    let mut shell = Shell::new(square());
    let mut tool = SelectTool::default();
    shell.drag(&mut tool, (118.0, 118.0), (-118.0, 118.0), NONE);
    assert_eq!(shell.begins(), vec!["Rotate".to_string()]);
    let p = shell.last_preview();
    assert_eq!(p.id, "transform.rotateCounterclockwise");
    assert_eq!(p.params, square_targets(json!({"angle": 90.0, "origin": [0.0, 0.0]})));

    let mut shell = Shell::new(square());
    shell.drag(&mut tool, (118.0, 50.0), (50.0, 118.0), Modifiers::ALT);
    assert_eq!(shell.last_preview().params, square_targets(json!({"angle": 90.0, "origin": [50.0, 50.0]})), "Alt: about the centre");

    // A click there is a click on empty canvas.
    let mut shell = Shell::new(square());
    shell.pointer(&mut tool, Down, 118.0, 118.0, NONE);
    shell.pointer(&mut tool, Up, 118.0, 118.0, NONE);
    assert!(shell.selection.is_empty());
    assert!(shell.begins().is_empty());
}

#[test]
fn without_the_box_a_press_on_a_knob_is_an_ordinary_press() {
    let mut shell = Shell::new(square());
    shell.settings.bounding_box = false;
    let mut tool = SelectTool::default();
    shell.drag(&mut tool, (106.0, 106.0), (156.0, 106.0), NONE);
    assert!(shell.begins().is_empty(), "a marquee, not a transform");
    assert!(shell.previews().is_empty());

    let layer = square_layer();
    let scene = Scene::new("A", "m01", &layer);
    let one = Selection::from_items([Item::Point(0, 0)]);
    let cx =
        Context { scene: &scene, view: View::default(), selection: &one, settings: ToolSettings { bounding_box: true, ..ToolSettings::default() } };
    assert_eq!(bbox::grab_at(&cx, Point::new(6.0, 6.0)), None, "one node: no box");
    let all = square();
    let cx = Context { selection: &all, ..cx };
    assert_eq!(bbox::grab_at(&cx, Point::new(-6.0, 50.0)).map(|g| g.1), Some(Grab::Knob(Knob::Left)));
    assert_eq!(bbox::grab_at(&cx, Point::new(50.0, 50.0)), None, "inside the box");
    assert_eq!(bbox::grab_at(&cx, Point::new(200.0, 50.0)), None, "far outside");
}

#[test]
fn the_rotate_tool_turns_about_the_box_centre_or_the_placed_point() {
    let mut shell = Shell::new(square());
    let mut tool = RotateTool::default();
    shell.drag(&mut tool, (100.0, 50.0), (50.0, 100.0), NONE);
    assert_eq!(shell.begins(), vec!["Rotate".to_string()]);
    assert_eq!(shell.last_preview().params, square_targets(json!({"angle": 90.0, "origin": [50.0, 50.0]})));
    assert_eq!(shell.actions.last(), Some(&Action::Commit));

    let mut shell = Shell::new(square());
    shell.pointer(&mut tool, Down, 10.0, 20.0, NONE);
    let out = shell.pointer(&mut tool, Up, 10.0, 20.0, NONE);
    assert_eq!(out.actions, vec![Action::Run { invocation: Invocation::new("transform.setOrigin", json!({"point": [10.0, 20.0]})) }]);
    shell.settings.reference = Some(Point::new(10.0, 20.0));
    shell.drag(&mut tool, (110.0, 20.0), (10.0, 120.0), NONE);
    assert_eq!(shell.last_preview().params, square_targets(json!({"angle": 90.0, "origin": [10.0, 20.0]})));

    // Shift: quarter turns. Once the shell has no point, the box's centre is the origin again.
    let mut shell = Shell::new(square());
    shell.drag(&mut tool, (100.0, 50.0), (60.0, 100.0), Modifiers::SHIFT);
    assert_eq!(shell.last_preview().params, square_targets(json!({"angle": 90.0, "origin": [50.0, 50.0]})));
}

#[test]
fn the_scale_tool_scales_about_the_origin_and_shift_keeps_proportions() {
    let mut shell = Shell::new(square());
    let mut tool = ScaleTool::default();
    shell.drag(&mut tool, (100.0, 50.0), (150.0, 50.0), NONE);
    assert_eq!(shell.begins(), vec!["Scale".to_string()]);
    assert_eq!(
        shell.last_preview().params,
        square_targets(json!({"x": 200.0, "y": 100.0, "origin": [50.0, 50.0]})),
        "the press is level with the origin: y keeps"
    );
    let mut shell = Shell::new(square());
    shell.drag(&mut tool, (100.0, 50.0), (150.0, 50.0), Modifiers::SHIFT);
    assert_eq!(shell.last_preview().params, square_targets(json!({"x": 200.0, "y": 200.0, "origin": [50.0, 50.0]})));

    // Nothing selected: the whole layer, about its box centre.
    let mut shell = Shell::new(Selection::new());
    shell.drag(&mut tool, (100.0, 100.0), (150.0, 150.0), NONE);
    assert_eq!(shell.last_preview().params, json!({"glyph": "A", "layer": "m01", "x": 200.0, "y": 200.0, "origin": [50.0, 50.0]}));
}

#[test]
fn hostile_drags_send_bounded_values_and_never_panic() {
    let mut shell = Shell::new(square());
    let mut tool = ScaleTool::default();
    shell.drag(&mut tool, (100.0, 100.0), (1e300, -1e300), NONE);
    let p = shell.last_preview();
    assert_eq!(p.params["x"], json!(1_000_000.0));
    assert_eq!(p.params["y"], json!(-1_000_000.0));
    for (x, y) in [(f64::NAN, 0.0), (f64::INFINITY, 1.0), (f64::MAX, f64::MAX), (50.0, 50.0)] {
        let mut select = SelectTool::default();
        let mut rotate = RotateTool::default();
        shell.drag(&mut select, (106.0, 106.0), (x, y), Modifiers::SHIFT.with_alt());
        shell.drag(&mut rotate, (100.0, 50.0), (x, y), Modifiers::SHIFT);
        shell.drag(&mut tool, (x, y), (100.0, 50.0), NONE);
    }
    for inv in shell.previews() {
        for key in ["x", "y", "angle"] {
            if let Some(v) = inv.params.get(key) {
                assert!(v.as_f64().is_some_and(|v| v.is_finite() && v.abs() <= 1_000_000.0), "{inv:?}");
            }
        }
    }
}

#[test]
fn r_and_s_choose_the_tools() {
    let mut shell = Shell::new(square());
    let mut tools = Toolbox::new();
    for (c, id) in [('r', "rotate"), ('s', "scale")] {
        let out = shell.run(|cx| tools.key(cx, &KeyEvent::press(Key::Char(c), NONE)));
        assert_eq!(out.actions, vec![Action::Run { invocation: Invocation::new("tool.select", json!({"tool": id})) }]);
        assert!(tools.select(id).is_ok());
        assert_eq!(tools.active_id(), id);
    }
    assert!(TOOLS.iter().any(|t| t.id == "rotate" && t.shortcut == "R"));
}
