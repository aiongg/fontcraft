//! The Knife tool (`E`, sharing its toolbar button with Erase): drag a straight line across
//! outlines to cut them along it (`docs/spec/03a-edit-view-drawing.md` § 12). While the drag
//! goes on the canvas draws the line; on release the tool runs `path.cutWithLine` once, which is
//! one undo step. A press that does not move past the drag threshold cuts nothing.
//!
//! Locked paths, and paths outside the path focus, are left out of the cut: the pointer tools
//! leave them alone.

use fontcraft_geom::Point;
use fontcraft_model::Shape;
use serde_json::json;

use crate::event::{Key, KeyEvent, PointerEvent, PointerKind};
use crate::geometry::{dist, finite};
use crate::output::{Action, Invocation, Output, Overlay, OverlayKind};
use crate::tool::{Context, Tool};

#[derive(Clone, Copy, Debug)]
struct Press {
    start: Point,
    current: Point,
    dragged: bool,
}

/// The Knife tool.
#[derive(Clone, Debug, Default)]
pub struct KnifeTool {
    press: Option<Press>,
}

impl Tool for KnifeTool {
    fn id(&self) -> &'static str {
        "knife"
    }

    fn pointer(&mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        if !finite(ev.pos) {
            return Output::default();
        }
        match ev.kind {
            PointerKind::Down => {
                self.press = Some(Press { start: ev.pos, current: ev.pos, dragged: false });
                Output::handled()
            }
            PointerKind::Drag => {
                let Some(p) = self.press.as_mut() else { return Output::default() };
                p.current = ev.pos;
                if dist(cx.view.to_screen(p.start), cx.view.to_screen(ev.pos)) >= cx.settings.drag_threshold {
                    p.dragged = true;
                }
                Output::handled()
            }
            PointerKind::Up => {
                let Some(p) = self.press.take() else { return Output::default() };
                if !p.dragged || p.start == ev.pos || cx.scene.locked {
                    return Output::handled();
                }
                match cut(cx, p.start, ev.pos) {
                    Some(invocation) => Output::with(Action::Run { invocation }),
                    None => Output::handled(),
                }
            }
            PointerKind::Move | PointerKind::DoubleClick => Output::default(),
        }
    }

    fn key(&mut self, _cx: &Context<'_>, ev: &KeyEvent) -> Output {
        if ev.pressed && ev.key == Key::Escape && self.busy() {
            let mut out = self.cancel();
            out.handled = true;
            return out;
        }
        Output::default()
    }

    fn cancel(&mut self) -> Output {
        self.press = None;
        Output::default()
    }

    fn busy(&self) -> bool {
        self.press.is_some()
    }

    fn overlays(&self) -> Vec<Overlay> {
        match self.press {
            Some(p) if p.dragged => vec![Overlay::Line { kind: OverlayKind::KnifeLine, from: p.start, to: p.current }],
            _ => Vec::new(),
        }
    }
}

/// `path.cutWithLine` from `from` to `to` on the scene's layer. When some paths are locked or
/// outside the focus, `shapes` names the others; `None` when no path may be cut.
fn cut(cx: &Context<'_>, from: Point, to: Point) -> Option<Invocation> {
    let scene = cx.scene;
    let paths: Vec<(usize, bool)> = scene
        .layer
        .shapes
        .iter()
        .enumerate()
        .filter_map(|(i, shape)| match shape {
            Shape::Path(path) => Some((i, !path.info.locked && scene.focus.as_ref().is_none_or(|f| f.contains(&i)))),
            _ => None,
        })
        .collect();
    let usable: Vec<usize> = paths.iter().filter(|(_, ok)| *ok).map(|(i, _)| *i).collect();
    if usable.is_empty() {
        return None;
    }
    let mut params = json!({
        "glyph": scene.glyph,
        "layer": scene.layer_id,
        "from": [from.x, from.y],
        "to": [to.x, to.y],
    });
    if usable.len() < paths.len()
        && let Some(o) = params.as_object_mut()
    {
        o.insert("shapes".into(), json!(usable));
    }
    Some(Invocation::new("path.cutWithLine", params))
}

#[cfg(test)]
mod tests {
    use fontcraft_model::{Layer, Path};
    use serde_json::json;

    use super::*;
    use crate::{Modifiers, Scene, Selection, ToolSettings, View};

    fn pointer(knife: &mut KnifeTool, layer: &Layer, kind: PointerKind, x: f64, y: f64) -> Output {
        let scene = Scene::new("A", "m01", layer);
        let selection = Selection::new();
        let cx = Context { scene: &scene, view: View::default(), selection: &selection, settings: ToolSettings::default() };
        knife.pointer(&cx, &PointerEvent::new(kind, x, y, Modifiers::NONE))
    }

    fn layer() -> Layer {
        let mut layer = Layer::new("m01", 600.0);
        layer.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
        let mut locked = Path::rect(200.0, 0.0, 300.0, 100.0);
        locked.info.locked = true;
        layer.shapes.push(Shape::Path(locked));
        layer
    }

    #[test]
    fn a_drag_shows_the_line_and_cuts_once_on_release() {
        let layer = layer();
        let mut knife = KnifeTool::default();
        assert!(pointer(&mut knife, &layer, PointerKind::Down, 50.0, -20.0).actions.is_empty());
        assert!(pointer(&mut knife, &layer, PointerKind::Drag, 50.0, 40.0).actions.is_empty());
        assert_eq!(knife.overlays(), vec![Overlay::Line { kind: OverlayKind::KnifeLine, from: Point::new(50.0, -20.0), to: Point::new(50.0, 40.0) }]);
        let out = pointer(&mut knife, &layer, PointerKind::Up, 50.0, 120.0);
        let expected = json!({"glyph": "A", "layer": "m01", "from": [50.0, -20.0], "to": [50.0, 120.0], "shapes": [0]});
        assert_eq!(out.actions, vec![Action::Run { invocation: Invocation::new("path.cutWithLine", expected) }], "the locked path is left out");
        assert!(!knife.busy() && knife.overlays().is_empty());
    }

    #[test]
    fn a_click_or_escape_cuts_nothing() {
        let layer = layer();
        let mut knife = KnifeTool::default();
        pointer(&mut knife, &layer, PointerKind::Down, 50.0, -20.0);
        assert_eq!(pointer(&mut knife, &layer, PointerKind::Up, 50.5, -20.0).actions, vec![]);

        pointer(&mut knife, &layer, PointerKind::Down, 50.0, -20.0);
        pointer(&mut knife, &layer, PointerKind::Drag, 50.0, 120.0);
        let scene = Scene::new("A", "m01", &layer);
        let selection = Selection::new();
        let cx = Context { scene: &scene, view: View::default(), selection: &selection, settings: ToolSettings::default() };
        assert!(knife.key(&cx, &KeyEvent::press(Key::Escape, Modifiers::NONE)).handled);
        assert!(!knife.busy());
        assert!(pointer(&mut knife, &layer, PointerKind::Up, 50.0, 120.0).actions.is_empty());
    }
}
