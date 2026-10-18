//! The Pixel tool (`X`, sharing its toolbar button with the Pencil): click or drag to paint
//! pixel components on the font's grid (`[[tool]] pixel`, `spec/inventory/09-reusing-shapes.toml`).
//!
//! The tool knows nothing of the grid: it sends the pointer positions of the gesture to
//! `pixel.setPixels` as `stroke` with mode `paint`, and the command finds the cells they cross.
//! That gives the four gestures one rule. A click toggles its cell; a drag that starts on an
//! empty cell adds a pixel in every cell it crosses, and one that starts on a pixel removes them.
//!
//! While the button is down the stroke so far is previewed on the interaction's snapshot, so the
//! pixels show as they are painted. On release the preview is dropped and the whole stroke runs
//! once as one command: one undo step, and the result the shell reads to offer Add Pixel Glyph
//! when the font has no glyph named `pixel` (`dialog.addPixelGlyph`).

use fontcraft_geom::Point;
use serde_json::{Value, json};

use crate::event::{Key, KeyEvent, PointerEvent, PointerKind};
use crate::geometry::finite;
use crate::output::{Action, Invocation, Output};
use crate::tool::{Context, Tool};

/// Most pointer positions one stroke keeps; later ones replace the last kept, which only
/// straightens the end of a very long drag.
pub const MAX_STROKE_POINTS: usize = 10_000;

/// The Pixel tool.
#[derive(Clone, Debug, Default)]
pub struct PixelTool {
    stroke: Option<Vec<Point>>,
}

impl PixelTool {
    /// The `pixel.setPixels` that paints `stroke` on the scene's layer.
    pub fn invocation(cx: &Context<'_>, stroke: &[Point]) -> Invocation {
        let mut params = cx.scene.target();
        params.insert("stroke".into(), Value::Array(stroke.iter().map(|p| json!([p.x, p.y])).collect()));
        params.insert("mode".into(), json!("paint"));
        Invocation::new("pixel.setPixels", Value::Object(params))
    }
}

impl Tool for PixelTool {
    fn id(&self) -> &'static str {
        "pixel"
    }

    fn pointer(&mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        if !finite(ev.pos) {
            return Output::default();
        }
        match ev.kind {
            PointerKind::Down => {
                if cx.scene.locked {
                    return Output::handled();
                }
                let stroke = vec![ev.pos];
                let mut out = Output::with(Action::Begin { label: "Pixel".into() });
                out.push(Action::Preview { invocation: Self::invocation(cx, &stroke) });
                self.stroke = Some(stroke);
                out
            }
            PointerKind::Drag => {
                let Some(stroke) = self.stroke.as_mut() else { return Output::default() };
                if stroke.last() == Some(&ev.pos) {
                    return Output::handled();
                }
                if stroke.len() >= MAX_STROKE_POINTS {
                    stroke.pop();
                }
                stroke.push(ev.pos);
                Output::with(Action::Preview { invocation: Self::invocation(cx, stroke) })
            }
            PointerKind::Up => {
                let Some(mut stroke) = self.stroke.take() else { return Output::default() };
                if stroke.last() != Some(&ev.pos) && stroke.len() < MAX_STROKE_POINTS {
                    stroke.push(ev.pos);
                }
                let mut out = Output::with(Action::Cancel);
                out.push(Action::Run { invocation: Self::invocation(cx, &stroke) });
                out
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
        match self.stroke.take() {
            Some(_) => Output::with(Action::Cancel),
            None => Output::default(),
        }
    }

    fn busy(&self) -> bool {
        self.stroke.is_some()
    }
}

#[cfg(test)]
mod tests {
    use fontcraft_model::Layer;

    use super::*;
    use crate::event::Modifiers;
    use crate::scene::Scene;
    use crate::selection::Selection;
    use crate::tool::ToolSettings;
    use crate::view::View;

    fn event(kind: PointerKind, x: f64, y: f64) -> PointerEvent {
        PointerEvent { kind, pos: Point::new(x, y), mods: Modifiers::NONE }
    }

    fn stroke_of(a: &Action) -> Value {
        match a {
            Action::Preview { invocation } | Action::Run { invocation } => {
                assert_eq!(invocation.id, "pixel.setPixels");
                assert_eq!((&invocation.params["glyph"], &invocation.params["mode"]), (&json!("A"), &json!("paint")));
                invocation.params["stroke"].clone()
            }
            other => panic!("not a command: {other:?}"),
        }
    }

    #[test]
    fn a_gesture_previews_its_stroke_and_runs_it_once_on_release() {
        let layer = Layer::new("m01", 600.0);
        let scene = Scene::new("A", "m01", &layer);
        let selection = Selection::new();
        let cx = Context { scene: &scene, view: View::default(), selection: &selection, settings: ToolSettings::default() };
        let mut tool = PixelTool::default();

        // A click: the cell under the pointer, previewed at once, then run as one step.
        let down = tool.pointer(&cx, &event(PointerKind::Down, 10.0, 20.0));
        assert!(matches!(down.actions.first(), Some(Action::Begin { .. })));
        assert_eq!(down.actions.get(1).map(stroke_of), Some(json!([[10.0, 20.0]])));
        assert!(tool.busy());
        let up = tool.pointer(&cx, &event(PointerKind::Up, 10.0, 20.0));
        assert_eq!(up.actions.first(), Some(&Action::Cancel), "the preview is dropped");
        assert_eq!(up.actions.get(1).map(stroke_of), Some(json!([[10.0, 20.0]])));
        assert!(!tool.busy());

        // A drag: every position the pointer passed, in order.
        tool.pointer(&cx, &event(PointerKind::Down, 0.0, 0.0));
        let drag = tool.pointer(&cx, &event(PointerKind::Drag, 120.0, 0.0));
        assert_eq!(drag.actions.first().map(stroke_of), Some(json!([[0.0, 0.0], [120.0, 0.0]])));
        let up = tool.pointer(&cx, &event(PointerKind::Up, 120.0, 80.0));
        assert_eq!(up.actions.get(1).map(stroke_of), Some(json!([[0.0, 0.0], [120.0, 0.0], [120.0, 80.0]])));

        // Escape ends a gesture without painting.
        tool.pointer(&cx, &event(PointerKind::Down, 0.0, 0.0));
        let esc = tool.key(&cx, &KeyEvent::press(Key::Escape, Modifiers::NONE));
        assert_eq!(esc.actions, vec![Action::Cancel]);
        assert!(!tool.busy());
        assert_eq!(tool.pointer(&cx, &event(PointerKind::Up, 0.0, 0.0)).actions, Vec::new());
    }
}
