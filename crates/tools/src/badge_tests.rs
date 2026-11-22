//! Drag badges: the numbers each drag shows beside the pointer while it runs.

use fontcraft_geom::Point;
use fontcraft_model::{Layer, Path, Shape};

use crate::event::PointerKind::{Down, Drag};
use crate::output::{badge_number, badge_percent};
use crate::primitives::PrimitiveTool;
use crate::rotate::RotateTool;
use crate::scale::ScaleTool;
use crate::select::SelectTool;
use crate::*;

const NONE: Modifiers = Modifiers::NONE;

/// Shape 0: the closed square (0,0) (100,0) (100,100) (0,100).
fn square_layer() -> Layer {
    let mut layer = Layer::new("m01", 600.0);
    layer.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
    layer
}

fn square() -> Selection {
    Selection::from_items((0..4).map(|n| Item::Point(0, n)))
}

/// Presses at `from` and drags to `to` with `tool` (no release), applying selections, and returns
/// the badges the tool shows then. The view has one screen point per unit; the box is on.
fn badges_during(tool: &mut dyn Tool, selection: Selection, from: (f64, f64), to: (f64, f64), mods: Modifiers) -> Vec<(Point, String)> {
    let layer = square_layer();
    let scene = Scene::new("A", "m01", &layer);
    let settings = ToolSettings { bounding_box: true, ..ToolSettings::default() };
    let mut selection = selection;
    for (kind, (x, y)) in [(Down, from), (Drag, to)] {
        let out = {
            let cx = Context { scene: &scene, view: View::default(), selection: &selection, settings };
            tool.pointer(&cx, &PointerEvent::new(kind, x, y, mods))
        };
        for a in out.actions {
            if let Action::Select { selection: s } = a {
                selection = s;
            }
        }
    }
    tool.overlays()
        .into_iter()
        .filter_map(|o| match o {
            Overlay::Badge { at, text } => Some((at, text)),
            _ => None,
        })
        .collect()
}

fn texts(badges: &[(Point, String)]) -> Vec<&str> {
    badges.iter().map(|(_, t)| t.as_str()).collect()
}

/// Numbers are whole, percentages have one decimal, and zero carries no sign.
#[test]
fn badge_numbers_are_rounded() {
    assert_eq!((badge_number(119.6), badge_number(-0.3), badge_number(-35.5)), ("120".into(), "0".into(), "-36".into()));
    assert_eq!((badge_percent(75.84), badge_percent(-0.01), badge_percent(150.0)), ("75.8%".into(), "0.0%".into(), "150.0%".into()));
    assert_eq!(badge_number(f64::NAN), "–");
}

/// Primitives show the box's size, the Select tool the offset (for a move and for an Alt-drag
/// copy alike), a knob the percentages and the new size, a turn outside the box,
/// and the Rotate tool, the angle, and the Scale tool the percentages. Each badge sits at the
/// pointer.
#[test]
fn each_drag_shows_its_badge_at_the_pointer() {
    let rect = badges_during(&mut PrimitiveTool::rectangle(), Selection::new(), (200.0, 0.0), (320.0, -45.0), NONE);
    assert_eq!(rect, vec![(Point::new(320.0, -45.0), "120×45".to_string())]);
    let ellipse = badges_during(&mut PrimitiveTool::ellipse(), Selection::new(), (200.0, 0.0), (250.0, 80.0), NONE);
    assert_eq!(texts(&ellipse), vec!["50×80"]);

    // The node at (100, 100) dragged by (30, -20), with snapping off.
    let one = Selection::from_items([Item::Point(0, 2)]);
    let moved = badges_during(&mut SelectTool::default(), one.clone(), (100.0, 100.0), (130.0, 80.0), Modifiers { ctrl: true, ..NONE });
    assert_eq!(moved, vec![(Point::new(130.0, 80.0), "x:30 y:-20".to_string())]);
    // An Alt-drag copy of the square, grabbed on its bottom segment away from the box's knobs.
    let copy = badges_during(&mut SelectTool::default(), square(), (25.0, 0.0), (55.0, -20.0), Modifiers { alt: true, ..NONE });
    assert_eq!(texts(&copy), vec!["x:30 y:-20"]);

    // The top-right knob, drawn 6 points outside the box, pulled 50 right: 150 % by 100 %.
    let knob = badges_during(&mut SelectTool::default(), square(), (106.0, 106.0), (156.0, 106.0), NONE);
    assert_eq!(knob, vec![(Point::new(156.0, 106.0), "150.0%/100.0% 150×100".to_string())]);
    let turned = badges_during(&mut SelectTool::default(), square(), (118.0, 118.0), (-118.0, 118.0), NONE);
    assert_eq!(texts(&turned), vec!["90°"]);

    let rotate = badges_during(&mut RotateTool::default(), square(), (100.0, 50.0), (50.0, 100.0), NONE);
    assert_eq!(texts(&rotate), vec!["90°"]);
    let scale = badges_during(&mut ScaleTool::default(), square(), (100.0, 50.0), (150.0, 50.0), NONE);
    assert_eq!(texts(&scale), vec!["200.0%/100.0%"]);
}
