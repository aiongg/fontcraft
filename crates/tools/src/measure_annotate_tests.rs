//! The Measurement tool and snapping to guides and the background: gestures in, actions out.

use fontcraft_geom::Point;
use fontcraft_model::{Background, Guide, Layer, Path, Shape};
use serde_json::json;

use crate::event::PointerKind::{Down, Drag, Up};
use crate::measure::MeasureTool;
use crate::snap::{Moving, SnapKind, snap};
use crate::*;

const NONE: Modifiers = Modifiers::NONE;

/// A square (0,0)–(100,100).
fn layer() -> Layer {
    let mut layer = Layer::new("m01", 600.0);
    layer.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
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
}
