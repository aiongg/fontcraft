//! The Lasso tool (`V`, `Shift+V` from Select): drag a free-form outline; the nodes, handles and
//! anchors inside it become the selection. `Shift` toggles them in the selection, `Alt` takes
//! on-curve nodes only. A click without a drag selects like the Select tool's click.

use fontcraft_geom::Point;

use crate::MAX_LASSO_POINTS;
use crate::event::{Key, KeyEvent, PointerEvent, PointerKind};
use crate::geometry::{dist, finite};
use crate::hit::hit_test;
use crate::output::{Action, Output, Overlay, OverlayKind};
use crate::selection::{Selection, in_region};
use crate::tool::{Context, Tool};

/// Screen distance below which a new lasso point is dropped as a repeat of the last one.
const MIN_STEP: f64 = 1.0;

#[derive(Clone, Debug)]
struct Drawing {
    points: Vec<Point>,
    last_screen: Point,
    extend: bool,
    nodes_only: bool,
    dragged: bool,
}

/// The Lasso tool.
#[derive(Clone, Debug, Default)]
pub struct LassoTool {
    drawing: Option<Drawing>,
}

impl LassoTool {
    fn finish(&mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        let mut out = Output::handled();
        let Some(d) = self.drawing.take() else {
            out.handled = false;
            return out;
        };
        let mut selection = if d.extend { cx.selection.clone() } else { Selection::new() };
        if d.dragged && d.points.len() >= 3 {
            for item in in_region(cx.scene, &d.points, d.nodes_only).items() {
                if d.extend {
                    selection.toggle(item);
                } else {
                    selection.insert(item);
                }
            }
        } else if let Some(hit) = hit_test(cx.scene, &cx.view, ev.pos, cx.settings.hit_tolerance) {
            for item in hit.items() {
                if d.extend {
                    selection.toggle(item);
                } else {
                    selection.insert(item);
                }
            }
        }
        if &selection != cx.selection {
            out.push(Action::Select { selection });
        }
        out
    }
}

impl Tool for LassoTool {
    fn id(&self) -> &'static str {
        "lasso"
    }

    fn pointer(&mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        if !finite(ev.pos) {
            return Output::default();
        }
        match ev.kind {
            PointerKind::Down => {
                self.drawing = Some(Drawing {
                    points: vec![ev.pos],
                    last_screen: cx.view.to_screen(ev.pos),
                    extend: ev.mods.shift,
                    nodes_only: ev.mods.alt,
                    dragged: false,
                });
                Output::handled()
            }
            PointerKind::Drag => {
                let Some(d) = self.drawing.as_mut() else { return Output::default() };
                let screen = cx.view.to_screen(ev.pos);
                let step = if d.dragged { MIN_STEP } else { cx.settings.drag_threshold.max(MIN_STEP) };
                if dist(screen, d.last_screen) >= step {
                    d.dragged = true;
                    d.last_screen = screen;
                    if d.points.len() < MAX_LASSO_POINTS {
                        d.points.push(ev.pos);
                    }
                }
                Output::handled()
            }
            PointerKind::Up => {
                if let Some(d) = self.drawing.as_mut()
                    && d.dragged
                    && d.points.len() < MAX_LASSO_POINTS
                {
                    d.points.push(ev.pos);
                }
                self.finish(cx, ev)
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
        self.drawing = None;
        Output::default()
    }

    fn busy(&self) -> bool {
        self.drawing.is_some()
    }

    fn overlays(&self) -> Vec<Overlay> {
        match &self.drawing {
            Some(d) if d.dragged => vec![Overlay::Polygon { kind: OverlayKind::Lasso, points: d.points.clone() }],
            _ => Vec::new(),
        }
    }
}
