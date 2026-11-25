//! The Measurement tool, the annotation sub-tools and snapping to guides and the background:
//! gestures in, actions out.

use fontcraft_geom::Point;
use fontcraft_model::{Annotation, Background, Extra, Guide, Layer, Path, Shape};
use serde_json::{Value, json};

use crate::annotate::{AnnotateTool, handle};
use crate::event::PointerKind::{Down, Drag, Up};
use crate::measure::MeasureTool;
use crate::snap::{Moving, SnapKind, snap};
use crate::*;

const NONE: Modifiers = Modifiers::NONE;

fn note(kind: &str, x: f64, y: f64, angle: f64, width: f64) -> Annotation {
    Annotation { kind: kind.into(), x, y, angle, text: None, width, extra: Extra::new() }
}

/// A square (0,0)–(100,100), and three annotations: an arrow with its tip at (300,300) at 45°,
/// a circle 100 across centred on (500,500), a text note at (600,100) 200 wide.
fn layer() -> Layer {
    let mut layer = Layer::new("m01", 600.0);
    layer.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
    layer.info.annotations.push(note("Arrow", 300.0, 300.0, 45.0, 0.0));
    layer.info.annotations.push(note("Circle", 500.0, 500.0, 0.0, 100.0));
    layer.info.annotations.push(note("Text", 600.0, 100.0, 0.0, 200.0));
    layer
}

/// Stands in for the shell: applies `Select` actions and records every action.
struct Shell {
    layer: Layer,
    selection: Selection,
    actions: Vec<Action>,
}

impl Shell {
    fn new(layer: Layer) -> Self {
        Self { layer, selection: Selection::new(), actions: Vec::new() }
    }

    fn run(&mut self, f: impl FnOnce(&Context<'_>) -> Output) -> Output {
        let scene = Scene::new("A", "m01", &self.layer);
        let cx = Context { scene: &scene, view: View::default(), selection: &self.selection, settings: ToolSettings::default() };
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

    fn click(&mut self, tool: &mut dyn Tool, x: f64, y: f64, mods: Modifiers) {
        self.pointer(tool, Down, x, y, mods);
        self.pointer(tool, Up, x, y, mods);
    }

    fn drag(&mut self, tool: &mut dyn Tool, from: (f64, f64), to: (f64, f64), mods: Modifiers) {
        self.pointer(tool, Down, from.0, from.1, mods);
        self.pointer(tool, Drag, (from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0, mods);
        self.pointer(tool, Drag, to.0, to.1, mods);
        self.pointer(tool, Up, to.0, to.1, mods);
    }

    fn runs(&self) -> Vec<&Invocation> {
        self.actions
            .iter()
            .filter_map(|a| match a {
                Action::Run { invocation } => Some(invocation),
                _ => None,
            })
            .collect()
    }

    fn last_preview(&self) -> Option<&Invocation> {
        self.actions.iter().rev().find_map(|a| match a {
            Action::Preview { invocation } => Some(invocation),
            _ => None,
        })
    }
}

fn ruler(tool: &MeasureTool) -> Option<(Point, Point)> {
    tool.overlays().iter().find_map(|o| match o {
        Overlay::Line { kind: OverlayKind::Ruler, from, to } => Some((*from, *to)),
        _ => None,
    })
}

#[test]
fn the_ruler_snaps_constrains_stays_and_never_edits() {
    let mut shell = Shell::new(layer());
    let mut tool = MeasureTool::default();
    // The press lands on the square's corner (snapping); Shift keeps the ruler horizontal.
    shell.drag(&mut tool, (102.0, 101.0), (250.0, 120.0), Modifiers::SHIFT);
    assert_eq!(ruler(&tool), Some((Point::new(100.0, 100.0), Point::new(250.0, 100.0))));
    assert!(!tool.busy(), "the ruler stays after the button comes up");
    assert!(shell.actions.is_empty(), "measuring changes nothing: {:?}", shell.actions);
    // Ctrl suspends snapping.
    shell.drag(&mut tool, (102.0, 101.0), (102.0, 300.0), Modifiers::CTRL);
    assert_eq!(ruler(&tool), Some((Point::new(102.0, 101.0), Point::new(102.0, 300.0))));
    // A click removes it; so does Escape.
    shell.click(&mut tool, 400.0, 400.0, NONE);
    assert_eq!(ruler(&tool), None);
    shell.drag(&mut tool, (300.0, 0.0), (400.0, 0.0), NONE);
    assert!(ruler(&tool).is_some());
    let out = shell.run(|cx| tool.key(cx, &KeyEvent::press(Key::Escape, NONE)));
    assert!(out.handled && ruler(&tool).is_none());
}

#[test]
fn g_during_a_ruler_drag_adds_a_measurement_guide_in_one_step() {
    let mut l = layer();
    l.info.guides.push(Guide { x: 0.0, y: 700.0, ..Default::default() });
    let mut shell = Shell::new(l);
    let mut tool = MeasureTool::default();
    shell.pointer(&mut tool, Down, 200.0, 50.0, NONE);
    shell.pointer(&mut tool, Drag, 200.0, 250.0, NONE);
    let out = shell.run(|cx| tool.key(cx, &KeyEvent::press(Key::Char('g'), Modifiers::CMD.with_alt().with_ctrl())));
    assert!(out.handled);
    let runs = shell.runs();
    let [batch] = runs.as_slice() else { panic!("one command: {runs:?}") };
    assert_eq!(batch.id, "command.batch");
    assert_eq!(
        batch.params["steps"],
        json!([
            {"command": "guide.add", "params": {"glyph": "A", "layer": "m01", "x": 200.0, "y": 50.0, "angle": 90.0}},
            {"command": "guide.toggleMeasurement", "params": {"glyph": "A", "layer": "m01", "guide": {"scope": "local", "index": 1}, "on": true}},
        ])
    );
}

#[test]
fn snapping_reaches_the_background_and_guide_crossings() {
    let mut l = Layer::new("m01", 600.0);
    let bg = Background { shapes: vec![Shape::Path(Path::rect(200.0, 200.0, 300.0, 300.0))], ..Default::default() };
    l.info.background = Some(bg);
    l.info.guides.push(Guide { x: 50.0, y: 0.0, angle: 90.0, ..Default::default() });
    let mut scene = Scene::new("A", "m01", &l);
    scene.metric_lines = vec![0.0, 500.0];
    let view = View::default();
    let at = |x: f64, y: f64| snap(&scene, &view, Point::new(x, y), &Moving::default(), 4.0);

    let s = at(202.0, 298.0);
    assert_eq!((s.point, s.kind, s.node), (Point::new(200.0, 300.0), Some(SnapKind::Node), None), "a background node");
    let s = at(52.0, 2.0);
    assert_eq!((s.point, s.kind), (Point::new(50.0, 0.0), Some(SnapKind::Guide)), "where the guide crosses the baseline");
    let s = at(52.0, 250.0);
    assert_eq!(s.kind, Some(SnapKind::Guide), "the guide alone");
    assert!((s.point.x - 50.0).abs() < 1e-9 && (s.point.y - 250.0).abs() < 1e-9, "{:?}", s.point);

    // Hidden background and hidden guides are not snapped to.
    scene.background_shown = false;
    scene.guides_shown = false;
    let at = |x: f64, y: f64| snap(&scene, &view, Point::new(x, y), &Moving::default(), 4.0);
    assert_eq!(at(202.0, 298.0).kind, None, "the hidden background");
    assert_eq!(at(52.0, 250.0).kind, None, "a hidden guide");
}

#[test]
fn each_sub_tool_click_adds_its_type_with_the_command_defaults() {
    for (name, kind) in [
        ("annotationText", "text"),
        ("annotationArrow", "arrow"),
        ("annotationCircle", "circle"),
        ("annotationPlus", "plus"),
        ("annotationMinus", "minus"),
    ] {
        let mut shell = Shell::new(layer());
        let mut tool = AnnotateTool::new(name).unwrap();
        shell.click(&mut tool, 50.0, 700.0, NONE);
        let runs = shell.runs();
        let [add] = runs.as_slice() else { panic!("{name}: {runs:?}") };
        assert_eq!(add.id, "annotation.add");
        assert_eq!(add.params, json!({"glyph": "A", "layer": "m01", "type": kind, "position": [50.0, 700.0]}), "{name}");
    }
    // The Selection sub-tool adds nothing; `annotation` names it.
    let mut shell = Shell::new(layer());
    let mut tool = make_tool("annotation").unwrap();
    assert_eq!(tool.id(), "annotationSelect");
    shell.click(tool.as_mut(), 50.0, 700.0, NONE);
    assert!(shell.runs().is_empty());
}

#[test]
fn dragging_an_annotation_moves_the_selection_in_one_interaction() {
    let mut shell = Shell::new(layer());
    let mut tool = AnnotateTool::new("annotationArrow").unwrap();
    // A press on an existing arrow selects it instead of adding one.
    shell.drag(&mut tool, (301.0, 301.0), (331.0, 321.0), NONE);
    assert_eq!(shell.selection, Selection::from_items([Item::Annotation(0)]));
    assert!(matches!(shell.actions.iter().find(|a| matches!(a, Action::Begin { .. })), Some(Action::Begin { label }) if label == "Move Annotations"));
    let preview = shell.last_preview().unwrap();
    assert_eq!(
        (preview.id.as_str(), &preview.params),
        ("annotation.move", &json!({"glyph": "A", "layer": "m01", "annotations": [0], "dx": 30.0, "dy": 20.0}))
    );
    assert_eq!(shell.actions.last(), Some(&Action::Commit));
    assert!(shell.runs().is_empty());
}

#[test]
fn a_shift_drag_adds_the_pressed_annotation_and_stays_on_an_axis() {
    let mut shell = Shell::new(layer());
    let mut tool = AnnotateTool::new("annotationSelect").unwrap();
    shell.selection = Selection::from_items([Item::Annotation(1)]);
    shell.drag(&mut tool, (301.0, 301.0), (341.0, 311.0), Modifiers::SHIFT);
    let preview = shell.last_preview().unwrap();
    assert_eq!(preview.params, json!({"glyph": "A", "layer": "m01", "annotations": [0, 1], "dx": 40.0, "dy": 0.0}));
    // A Shift click on a selected one drops it.
    shell.click(&mut tool, 301.0, 301.0, Modifiers::SHIFT);
    assert_eq!(shell.selection, Selection::from_items([Item::Annotation(1)]));
}

#[test]
fn handle_drags_update_width_angle_and_diameter() {
    let cases: [(usize, (f64, f64), Value); 3] =
        [(1, (500.0, 420.0), json!({"diameter": 160.0})), (2, (900.0, 100.0), json!({"width": 300.0})), (0, (300.0, 400.0), json!({"angle": 90.0}))];
    for (index, to, values) in cases {
        let mut shell = Shell::new(layer());
        shell.selection = Selection::from_items([Item::Annotation(index)]);
        let start = handle(&shell.layer.info.annotations[index], &View::default()).unwrap();
        let mut tool = AnnotateTool::new("annotationSelect").unwrap();
        shell.drag(&mut tool, (start.x, start.y), to, NONE);
        let preview = shell.last_preview().unwrap();
        let mut want = json!({"glyph": "A", "layer": "m01", "annotation": index});
        want.as_object_mut().unwrap().extend(values.as_object().unwrap().clone());
        assert_eq!((preview.id.as_str(), &preview.params), ("annotation.update", &want), "annotation {index}");
        assert_eq!(shell.actions.last(), Some(&Action::Commit));
    }
}

#[test]
fn annotation_keys_select_all_delete_and_move() {
    let mut shell = Shell::new(layer());
    let mut tools = Toolbox::new();
    let key = |shell: &mut Shell, tools: &mut Toolbox, key: Key, mods: Modifiers| shell.run(|cx| tools.key(cx, &KeyEvent::press(key, mods)));
    let out = key(&mut shell, &mut tools, Key::Char('a'), NONE);
    assert_eq!(runs_of(&out), vec![("tool.select", json!({"tool": "annotationSelect"}))]);
    tools.select("annotationCircle").unwrap();
    let out = key(&mut shell, &mut tools, Key::Char('a'), NONE);
    assert_eq!(runs_of(&out), vec![("tool.select", json!({"tool": "annotationCircle"}))], "A keeps the sub-tool in use");
    let out = key(&mut shell, &mut tools, Key::Char('a'), Modifiers::CMD);
    assert_eq!(runs_of(&out), vec![("annotation.selectAll", json!({"glyph": "A", "layer": "m01"}))]);
    shell.selection = Selection::from_items([Item::Annotation(0), Item::Annotation(2)]);
    let out = key(&mut shell, &mut tools, Key::Right, Modifiers::SHIFT);
    assert_eq!(runs_of(&out), vec![("annotation.move", json!({"glyph": "A", "layer": "m01", "annotations": [0, 2], "dx": 10.0, "dy": 0.0}))]);
    let out = key(&mut shell, &mut tools, Key::Delete, NONE);
    assert_eq!(runs_of(&out), vec![("annotation.delete", json!({"glyph": "A", "layer": "m01", "annotations": [0, 2]}))]);
    let out = key(&mut shell, &mut tools, Key::Char('l'), NONE);
    assert_eq!(runs_of(&out), vec![("tool.select", json!({"tool": "measurement"}))]);
}

fn runs_of(out: &Output) -> Vec<(&str, Value)> {
    out.actions
        .iter()
        .filter_map(|a| match a {
            Action::Run { invocation } => Some((invocation.id.as_str(), invocation.params.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn modifiers_hold_the_measurement_and_select_tools_until_released() {
    let mut shell = Shell::new(layer());
    let mut tools = Toolbox::new();
    let hold = Modifiers::CMD.with_alt().with_ctrl();
    tools.select("draw").unwrap();
    shell.run(|cx| tools.modifiers(cx, hold));
    assert_eq!((tools.active_id(), tools.base_id()), ("measurement", "draw"));
    // The held keys do not reach the ruler as Ctrl, so its press still snaps to the corner.
    shell.run(|cx| tools.pointer(cx, &PointerEvent::new(Down, 102.0, 101.0, hold)));
    shell.run(|cx| tools.pointer(cx, &PointerEvent::new(Drag, 250.0, 160.0, hold)));
    // Let go during the drag: the tool stays until the button comes up.
    shell.run(|cx| tools.modifiers(cx, NONE));
    assert_eq!(tools.active_id(), "measurement");
    let from = tools.overlays().iter().find_map(|o| match o {
        Overlay::Line { kind: OverlayKind::Ruler, from, .. } => Some(*from),
        _ => None,
    });
    assert_eq!(from, Some(Point::new(100.0, 100.0)));
    shell.run(|cx| tools.pointer(cx, &PointerEvent::new(Up, 250.0, 160.0, NONE)));
    assert_eq!(tools.active_id(), "draw", "the release returns to the chosen tool");
    assert!(tools.overlays().is_empty(), "the ruler goes with the held tool");
    assert!(shell.actions.is_empty(), "measuring changes nothing: {:?}", shell.actions);

    // A press with the keys already down takes up the tool at once.
    shell.run(|cx| tools.pointer(cx, &PointerEvent::new(Down, 300.0, 0.0, hold)));
    assert_eq!(tools.active_id(), "measurement");
    shell.run(|cx| tools.pointer(cx, &PointerEvent::new(Up, 300.0, 0.0, NONE)));
    assert_eq!(tools.active_id(), "draw");

    // Cmd in the Measurement tool holds the Select tool; Cmd+Alt+Ctrl there changes nothing.
    tools.select("measurement").unwrap();
    shell.run(|cx| tools.modifiers(cx, Modifiers::CMD));
    assert_eq!(tools.active_id(), "select");
    shell.run(|cx| tools.modifiers(cx, NONE));
    assert_eq!(tools.active_id(), "measurement");
    shell.run(|cx| tools.modifiers(cx, hold));
    assert_eq!(tools.active_id(), "measurement");
    // In the Text tool the keys move the measurement line instead.
    tools.select("text").unwrap();
    shell.run(|cx| tools.modifiers(cx, hold));
    assert_eq!(tools.active_id(), "text");
    // Choosing a tool drops a held one.
    tools.select("draw").unwrap();
    shell.run(|cx| tools.modifiers(cx, hold));
    tools.select("select").unwrap();
    assert_eq!(tools.active_id(), "select");
}

#[test]
fn off_the_mac_ctrl_alt_holds_nothing_and_ctrl_alt_shift_holds_the_measurement_tool() {
    use crate::toolbox::Keyboard;
    let mut shell = Shell::new(layer());
    let mut tools = Toolbox::new();
    tools.set_keyboard(Keyboard::Other);
    // Off the Mac the Control key reports both `cmd` and `ctrl`; AltGr reports Ctrl+Alt.
    let ctrl = Modifiers::CMD.with_ctrl();
    let ctrl_alt = ctrl.with_alt();
    shell.run(|cx| tools.modifiers(cx, ctrl_alt));
    assert_eq!(tools.active_id(), "select", "Ctrl+Alt (or AltGr) stays with the Select tool");
    shell.run(|cx| tools.modifiers(cx, ctrl_alt.with_shift()));
    assert_eq!(tools.active_id(), "measurement");
    shell.run(|cx| tools.modifiers(cx, ctrl_alt));
    assert_eq!(tools.active_id(), "select", "letting go of Shift gives way");

    tools.select("measurement").unwrap();
    shell.run(|cx| tools.modifiers(cx, ctrl));
    assert_eq!(tools.active_id(), "select", "Ctrl holds the Select tool");
    shell.run(|cx| tools.modifiers(cx, ctrl_alt));
    assert_eq!(tools.active_id(), "measurement", "Ctrl+Alt may be AltGr: nothing is held");

    // On the Mac, Cmd+Alt in the Measurement tool keeps the Select tool held.
    tools.set_keyboard(Keyboard::Mac);
    shell.run(|cx| tools.modifiers(cx, Modifiers::CMD));
    shell.run(|cx| tools.modifiers(cx, Modifiers::CMD.with_alt()));
    assert_eq!(tools.active_id(), "select");
    shell.run(|cx| tools.modifiers(cx, NONE));
    assert_eq!(tools.active_id(), "measurement");
}
