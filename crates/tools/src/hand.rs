//! The Hand tool (`H`, or hold `Space`): drag to scroll the canvas.

use fontcraft_geom::Point;

use crate::event::{PointerEvent, PointerKind};
use crate::geometry::finite;
use crate::output::{Action, Output, ViewChange};
use crate::tool::{Context, Tool};

/// The Hand tool.
#[derive(Clone, Debug, Default)]
pub struct HandTool {
    /// The pointer's last screen position during a drag.
    last: Option<Point>,
}

impl Tool for HandTool {
    fn id(&self) -> &'static str {
        "hand"
    }

    fn pointer(&mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        if !finite(ev.pos) {
            return Output::default();
        }
        // The event's font position was measured in the view the shell has now, after the pans
        // it already applied, so this is where the pointer is on screen.
        let screen = cx.view.to_screen(ev.pos);
        match ev.kind {
            PointerKind::Down => {
                self.last = Some(screen);
                Output::handled()
            }
            PointerKind::Drag | PointerKind::Up => {
                let Some(last) = self.last else { return Output::default() };
                self.last = if ev.kind == PointerKind::Up { None } else { Some(screen) };
                let (dx, dy) = (screen.x - last.x, screen.y - last.y);
                if !(dx.is_finite() && dy.is_finite()) || dx.abs().max(dy.abs()) <= 0.0 {
                    return Output::handled();
                }
                Output::with(Action::View { change: ViewChange::Pan { dx, dy } })
            }
            PointerKind::Move | PointerKind::DoubleClick => Output::default(),
        }
    }

    fn cancel(&mut self) -> Output {
        self.last = None;
        Output::default()
    }

    fn busy(&self) -> bool {
        self.last.is_some()
    }
}
