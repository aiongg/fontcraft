//! The drawing tools without an engine: tool letters, the primitives' parameters, and what a
//! fitted stroke turns into.

use fontcraft_geom::fit::fit_polyline;
use fontcraft_geom::{BezPath, Point};
use fontcraft_model::{Layer, NodeKind};
use serde_json::json;

use crate::cycling::tool_for_letter;
use crate::draw::constrain;
use crate::pencil::nodes_of;
use crate::primitives::shape_params;
use crate::*;

/// The tool a letter key asks for in `toolbox`, or `None` when the key is not a tool letter.
fn letter(toolbox: &mut Toolbox, c: char, shift: bool) -> Option<String> {
    let layer = Layer::new("m01", 600.0);
    let scene = Scene::new("A", "m01", &layer);
    let selection = Selection::new();
    let cx = Context { scene: &scene, view: View::default(), selection: &selection, settings: ToolSettings::default() };
    let mods = if shift { Modifiers::SHIFT } else { Modifiers::NONE };
    let out = toolbox.key(&cx, &KeyEvent::press(Key::Char(c), mods));
    match out.actions.as_slice() {
        [Action::Run { invocation }] if invocation.id == "tool.select" => {
            let id = invocation.params["tool"].as_str().unwrap().to_string();
            toolbox.select(&id).unwrap();
            Some(id)
        }
        _ => None,
    }
}

#[test]
fn letters_choose_tools_and_shift_steps_through_a_shared_button() {
    let mut t = Toolbox::new();
    assert_eq!(letter(&mut t, 'p', false).as_deref(), Some("draw"));
    assert_eq!(letter(&mut t, 'b', false).as_deref(), Some("pencil"));
    assert_eq!(letter(&mut t, 'b', true), None, "one tool has B: Shift+B is not a tool key");
    assert_eq!(letter(&mut t, 'f', false).as_deref(), Some("rectangle"), "the first mode before any was used");
    assert_eq!(letter(&mut t, 'f', true).as_deref(), Some("ellipse"));
    assert_eq!(letter(&mut t, 'f', false).as_deref(), Some("ellipse"), "the letter keeps the active mode");
    assert_eq!(letter(&mut t, 'p', false).as_deref(), Some("draw"));
    assert_eq!(letter(&mut t, 'f', false).as_deref(), Some("ellipse"), "the letter returns to the mode used last");
    assert_eq!(letter(&mut t, 'v', true).as_deref(), Some("lasso"));
    assert_eq!(letter(&mut t, 'f', true).as_deref(), Some("rectangle"), "Shift steps on from the mode used last");
    assert_eq!(letter(&mut t, 'v', false).as_deref(), Some("lasso"));
    assert_eq!(tool_for_letter('q', false, "draw", &[]), None);
}

#[test]
fn primitive_drags_follow_shift_and_alt() {
    let s = Point::new(100.0, 100.0);
    let p = |to: (f64, f64), mods: Modifiers| shape_params(s, Point::new(to.0, to.1), mods).map(serde_json::Value::Object);
    assert_eq!(p((300.0, 250.0), Modifiers::NONE), Some(json!({"x": 100.0, "y": 100.0, "width": 200.0, "height": 150.0})));
    assert_eq!(p((300.0, 50.0), Modifiers::SHIFT), Some(json!({"x": 100.0, "y": 100.0, "width": 200.0, "height": -200.0})));
    assert_eq!(p((150.0, 130.0), Modifiers::ALT), Some(json!({"x": 100.0, "y": 100.0, "width": 100.0, "height": 60.0, "fromCenter": true})));
    assert_eq!(p((300.0, 100.0), Modifiers::NONE), None, "no height, no shape");
    assert_eq!(p((f64::NAN, 100.0), Modifiers::SHIFT), None);
}

#[test]
fn shift_keeps_to_45_degree_steps() {
    let o = Point::new(0.0, 0.0);
    assert_eq!(constrain(o, Point::new(100.0, 10.0)), Point::new(100.0, 0.0));
    let d = constrain(o, Point::new(100.0, 90.0));
    assert!((d.x - d.y).abs() < 1e-9 && (d.x - 95.0).abs() < 1e-9, "{d:?}");
    assert_eq!(constrain(o, o), o);
}

#[test]
fn a_fitted_stroke_becomes_nodes_with_smooth_joins() {
    let pts: Vec<Point> = (0..400).map(|i| Point::new(i as f64, 100.0 * (i as f64 / 60.0).sin())).collect();
    let nodes = nodes_of(&fit_polyline(&pts, 0.5, false), false);
    assert!(nodes.len() >= 4);
    assert_eq!(nodes[0].kind, NodeKind::Line);
    assert_eq!((nodes[0].x, nodes[0].y), (0.0, 0.0));
    let last = nodes.last().unwrap();
    assert!(last.on_curve() && (last.x - 399.0).abs() < 1e-6);
    let inner: Vec<_> = nodes.iter().skip(1).take(nodes.len() - 2).filter(|n| n.on_curve()).collect();
    assert!(!inner.is_empty() && inner.iter().all(|n| n.smooth), "joins inside a fitted stroke are smooth");
    // A closed square drawn as a polygon keeps its corners and drops the repeated start.
    let mut square = BezPath::new();
    square.move_to((0.0, 0.0));
    square.line_to((100.0, 0.0));
    square.line_to((100.0, 100.0));
    square.line_to((0.0, 0.0));
    square.close_path();
    let n = nodes_of(&square, true);
    assert_eq!(n.len(), 3);
    assert!(n.iter().all(|n| !n.smooth));
}
