//! The Primitives tool (`F`): rectangles and ellipses (`[[tool]] rectangle`, `[[tool]] ellipse`,
//! `docs/spec/03a` § 5). One toolbar button with two modes, each its own tool id; `Shift+F` or
//! `tool.drawRectangle` / `tool.drawCircle` switch between them.
//!
//! | Gesture | Result |
//! |---|---|
//! | drag | `path.addRectangle` / `path.addEllipse` between the press and the pointer, in one interaction |
//! | `Shift` during the drag | a square / a circle |
//! | `Alt` during the drag | the press is the centre |
//! | click | asks for `dialog.primitiveMeasurements` with the click as `x`, `y` |
//!
//! The press and the pointer snap like dragged points (`Ctrl` suspends it).

use fontcraft_geom::Point;
use serde_json::{Map, Value, json};

use crate::draw::params;
use crate::event::{Key, KeyEvent, Modifiers, PointerEvent, PointerKind};
use crate::geometry::{dist, finite};
use crate::output::{Action, Invocation, Output, Overlay, badge_number};
use crate::selection::Selection;
use crate::snap::{Moving, snap};
use crate::tool::{Context, Tool};

/// Nodes in the largest primitive (the ellipse): what a snap must not hit of the shape itself.
const PRIMITIVE_NODES: usize = 12;

#[derive(Clone, Debug)]
struct Press {
    start: Point,
    down: Point,
    last: Point,
    dragging: bool,
    base: Map<String, Value>,
    /// The index the new shape gets; its nodes are not snap targets.
    shape: usize,
    sent: Option<Value>,
}

/// The Primitives tool in rectangle or ellipse mode (the default is rectangle mode).
#[derive(Clone, Debug, Default)]
pub struct PrimitiveTool {
    ellipse: bool,
    press: Option<Press>,
}

/// The parameters of `path.addRectangle` / `path.addEllipse` for a drag from `start` to `to`;
/// `None` while the box has no width or no height.
pub fn shape_params(start: Point, to: Point, mods: Modifiers) -> Option<Map<String, Value>> {
    let (mut w, mut h) = (to.x - start.x, to.y - start.y);
    if mods.shift {
        let side = w.abs().max(h.abs());
        (w, h) = (side.copysign(if w == 0.0 { 1.0 } else { w }), side.copysign(if h == 0.0 { 1.0 } else { h }));
    }
    if mods.alt {
        (w, h) = (2.0 * w, 2.0 * h);
    }
    if !(w.is_finite() && h.is_finite()) || w == 0.0 || h == 0.0 {
        return None;
    }
    let mut m = Map::new();
    m.insert("x".into(), json!(start.x));
    m.insert("y".into(), json!(start.y));
    m.insert("width".into(), json!(w));
    m.insert("height".into(), json!(h));
    if mods.alt {
        m.insert("fromCenter".into(), json!(true));
    }
    Some(m)
}

impl PrimitiveTool {
    pub fn rectangle() -> Self {
        Self { ellipse: false, press: None }
    }

    pub fn ellipse() -> Self {
        Self { ellipse: true, press: None }
    }

    fn command(&self) -> &'static str {
        if self.ellipse { "path.addEllipse" } else { "path.addRectangle" }
    }

    fn snapped(cx: &Context<'_>, p: Point, mods: Modifiers, shape: usize) -> Point {
        if mods.ctrl {
            return p;
        }
        let moving = Moving { points: (0..PRIMITIVE_NODES).map(|n| (shape, n)).collect(), shapes: [shape].into_iter().collect() };
        snap(cx.scene, &cx.view, p, &moving, cx.settings.snap_distance).point
    }

    /// The preview for the pointer at `pos`, when it differs from the last one sent.
    fn preview(&mut self, cx: &Context<'_>, pos: Point, mods: Modifiers) -> Option<Action> {
        let command = self.command();
        let press = self.press.as_mut()?;
        press.last = pos;
        let to = Self::snapped(cx, pos, mods, press.shape);
        let extra = shape_params(press.start, to, mods)?;
        let p = params(&press.base, Value::Object(extra));
        if press.sent.as_ref() == Some(&p) {
            return None;
        }
        press.sent = Some(p.clone());
        Some(Action::Preview { invocation: Invocation::new(command, p) })
    }

    fn label(&self) -> &'static str {
        if self.ellipse { "Add Ellipse" } else { "Add Rectangle" }
    }
}

impl Tool for PrimitiveTool {
    fn id(&self) -> &'static str {
        if self.ellipse { "ellipse" } else { "rectangle" }
    }

    fn pointer(&mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        if !finite(ev.pos) {
            return Output::default();
        }
        match ev.kind {
            PointerKind::Down => {
                let mut out = self.cancel();
                out.handled = true;
                let shape = cx.scene.layer.shapes.len();
                let start = Self::snapped(cx, ev.pos, ev.mods, shape);
                let base = Selection::new().targets(cx.scene);
                self.press = Some(Press { start, down: ev.pos, last: ev.pos, dragging: false, base, shape, sent: None });
                out
            }
            PointerKind::Drag => {
                let Some(press) = &mut self.press else { return Output::default() };
                let mut out = Output::handled();
                if !press.dragging {
                    if dist(cx.view.to_screen(press.down), cx.view.to_screen(ev.pos)) < cx.settings.drag_threshold {
                        return out;
                    }
                    press.dragging = true;
                    out.push(Action::Begin { label: self.label().into() });
                }
                if let Some(a) = self.preview(cx, ev.pos, ev.mods) {
                    out.push(a);
                }
                out
            }
            PointerKind::Up => {
                let Some(press) = &self.press else { return Output::default() };
                let mut out = Output::handled();
                if press.dragging {
                    if let Some(a) = self.preview(cx, ev.pos, ev.mods) {
                        out.push(a);
                    }
                    out.push(Action::Commit);
                } else {
                    let shape = if self.ellipse { "ellipse" } else { "rectangle" };
                    let p = params(&press.base, json!({"x": press.start.x, "y": press.start.y, "shape": shape}));
                    out.push(Action::Run { invocation: Invocation::new("dialog.primitiveMeasurements", p) });
                }
                self.press = None;
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

    fn modifiers(&mut self, cx: &Context<'_>, mods: Modifiers) -> Output {
        let mut out = Output::default();
        let Some(last) = self.press.as_ref().filter(|p| p.dragging).map(|p| p.last) else { return out };
        if let Some(a) = self.preview(cx, last, mods) {
            out.push(a);
        }
        out
    }

    fn cancel(&mut self) -> Output {
        let mut out = Output::default();
        if self.press.take().is_some_and(|p| p.dragging) {
            out.push(Action::Cancel);
        }
        out
    }

    fn busy(&self) -> bool {
        self.press.is_some()
    }

    /// While a box is dragged out: its width and height beside the pointer (`w×h`).
    fn overlays(&self) -> Vec<Overlay> {
        let Some(press) = self.press.as_ref().filter(|p| p.dragging) else { return Vec::new() };
        let Some(sent) = &press.sent else { return Vec::new() };
        let size = |key: &str| sent.get(key).and_then(Value::as_f64).map(f64::abs);
        let (Some(w), Some(h)) = (size("width"), size("height")) else { return Vec::new() };
        vec![Overlay::Badge { at: press.last, text: format!("{}×{}", badge_number(w), badge_number(h)) }]
    }
}
