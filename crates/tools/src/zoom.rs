//! The Zoom tool (`Z`, or hold `Cmd+Space`): click to zoom in at the pointer, `Alt`+click to zoom
//! out, drag a rectangle to fill the window with it. One step is a factor of √2 (`docs/spec/03b`
//! question 8).

use fontcraft_geom::Point;

use crate::event::{Key, KeyEvent, Modifiers, PointerEvent, PointerKind};
use crate::geometry::{dist, finite};
use crate::output::{Action, Output, Overlay, OverlayKind, ViewChange};
use crate::tool::{Context, Tool};

/// The zoom factor of one step.
pub const ZOOM_STEP: f64 = std::f64::consts::SQRT_2;

#[derive(Clone, Copy, Debug)]
struct Press {
    start: Point,
    current: Point,
    mods: Modifiers,
    dragged: bool,
}

/// The Zoom tool.
#[derive(Clone, Debug, Default)]
pub struct ZoomTool {
    press: Option<Press>,
}

impl Tool for ZoomTool {
    fn id(&self) -> &'static str {
        "zoom"
    }

    fn pointer(&mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        if !finite(ev.pos) {
            return Output::default();
        }
        match ev.kind {
            PointerKind::Down => {
                self.press = Some(Press { start: ev.pos, current: ev.pos, mods: ev.mods, dragged: false });
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
                let change = if p.dragged {
                    let (x0, x1) = (p.start.x.min(ev.pos.x), p.start.x.max(ev.pos.x));
                    let (y0, y1) = (p.start.y.min(ev.pos.y), p.start.y.max(ev.pos.y));
                    let (width, height) = (x1 - x0, y1 - y0);
                    if !(width > 0.0 && height > 0.0 && width.is_finite() && height.is_finite()) {
                        return Output::handled();
                    }
                    ViewChange::ZoomToRect { x: x0, y: y0, width, height }
                } else {
                    let factor = if p.mods.alt { 1.0 / ZOOM_STEP } else { ZOOM_STEP };
                    ViewChange::Zoom { factor, at: p.start }
                };
                Output::with(Action::View { change })
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
            Some(p) if p.dragged => {
                let (a, b) = (p.start, p.current);
                let points = vec![a, Point::new(b.x, a.y), b, Point::new(a.x, b.y)];
                vec![Overlay::Polygon { kind: OverlayKind::ZoomRect, points }]
            }
            _ => Vec::new(),
        }
    }
}
