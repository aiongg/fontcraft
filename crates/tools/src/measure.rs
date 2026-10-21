//! The Measurement tool (`L`): read-only coordinates and a ruler (`[[tool]] measurement`,
//! `docs/spec/03b` questions 9 and 10).
//!
//! | Gesture | Result |
//! |---|---|
//! | select the tool | the canvas labels every point and the deltas between on-curve nodes (`measure.coordinates`) |
//! | drag | a ruler from the press to the pointer; its ends snap unless `Ctrl` is held; the canvas labels its crossings (`measure.ruler`) |
//! | `Shift`+drag | the ruler stays horizontal or vertical, whichever is nearer |
//! | `G` while dragging | a measurement guide along the ruler (`guide.add` and `guide.toggleMeasurement` as one step) |
//! | `Escape` | removes the ruler |
//! | `Cmd` held | the Select tool until it comes up ([`crate::toolbox`]) |
//!
//! `Cmd+Alt+Ctrl` held in another tool gives this tool until the keys come up.
//!
//! The ruler stays after the button comes up, until the next press or a tool change. The tool
//! changes nothing but the guide `G` asks for.

use fontcraft_geom::Point;
use serde_json::{Value, json};

use crate::event::{Key, KeyEvent, PointerEvent, PointerKind};
use crate::geometry::{dist, finite};
use crate::output::{Action, Invocation, Output, Overlay, OverlayKind};
use crate::snap::{Moving, snap};
use crate::tool::{Context, Tool};

#[derive(Clone, Copy, Debug, PartialEq)]
struct Ruler {
    from: Point,
    to: Point,
    /// The pointer moved past the drag threshold.
    dragged: bool,
}

/// The Measurement tool.
#[derive(Clone, Debug, Default)]
pub struct MeasureTool {
    ruler: Option<Ruler>,
    /// The button is down.
    dragging: bool,
}

/// Where a ruler end lands: on the nearer axis with `Shift`, else snapped unless `Ctrl` is held.
fn land(cx: &Context<'_>, from: Option<Point>, p: Point, shift: bool, ctrl: bool) -> Point {
    if let (Some(a), true) = (from, shift) {
        return if (p.x - a.x).abs() >= (p.y - a.y).abs() { Point::new(p.x, a.y) } else { Point::new(a.x, p.y) };
    }
    if ctrl {
        return p;
    }
    snap(cx.scene, &cx.view, p, &Moving::default(), cx.settings.snap_distance).point
}

/// The ruler's angle as a guide angle: degrees counter-clockwise from the x axis, 0 to 360.
fn guide_angle(from: Point, to: Point) -> f64 {
    (to.y - from.y).atan2(to.x - from.x).to_degrees().rem_euclid(360.0)
}

/// `command.batch` parameters that add a local guide along the ruler and make it a measurement
/// guide. The new guide's index is the number of local guides now, as `guide.add` appends.
pub fn guide_params(cx: &Context<'_>, from: Point, to: Point) -> Value {
    let index = cx.scene.local_guides().len();
    let mut add = cx.scene.target();
    add.insert("x".into(), json!(from.x));
    add.insert("y".into(), json!(from.y));
    add.insert("angle".into(), json!(guide_angle(from, to)));
    let mut toggle = cx.scene.target();
    toggle.insert("guide".into(), json!({"scope": "local", "index": index}));
    toggle.insert("on".into(), json!(true));
    json!({
        "label": "Add Measurement Guide",
        "steps": [
            {"command": "guide.add", "params": add},
            {"command": "guide.toggleMeasurement", "params": toggle},
        ],
    })
}

impl Tool for MeasureTool {
    fn id(&self) -> &'static str {
        "measurement"
    }

    fn pointer(&mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        if !finite(ev.pos) {
            return Output::default();
        }
        match ev.kind {
            PointerKind::Down => {
                let at = land(cx, None, ev.pos, false, ev.mods.ctrl);
                self.ruler = Some(Ruler { from: at, to: at, dragged: false });
                self.dragging = true;
                Output::handled()
            }
            PointerKind::Drag | PointerKind::Up => {
                let Some(r) = self.ruler.as_mut().filter(|_| self.dragging) else { return Output::default() };
                r.to = land(cx, Some(r.from), ev.pos, ev.mods.shift, ev.mods.ctrl);
                if dist(cx.view.to_screen(r.from), cx.view.to_screen(ev.pos)) >= cx.settings.drag_threshold {
                    r.dragged = true;
                }
                if ev.kind == PointerKind::Up {
                    self.dragging = false;
                    if !r.dragged {
                        self.ruler = None;
                    }
                }
                Output::handled()
            }
            PointerKind::Move | PointerKind::DoubleClick => Output::default(),
        }
    }

    fn key(&mut self, cx: &Context<'_>, ev: &KeyEvent) -> Output {
        if !ev.pressed {
            return Output::default();
        }
        match ev.key {
            Key::Escape if self.ruler.is_some() => {
                self.ruler = None;
                self.dragging = false;
                Output::handled()
            }
            Key::Char('g') if self.dragging => match self.ruler {
                Some(r) if r.dragged && dist(r.from, r.to) > 0.0 => {
                    Output::with(Action::Run { invocation: Invocation::new("command.batch", guide_params(cx, r.from, r.to)) })
                }
                _ => Output::handled(),
            },
            _ => Output::default(),
        }
    }

    fn cancel(&mut self) -> Output {
        self.ruler = None;
        self.dragging = false;
        Output::default()
    }

    fn busy(&self) -> bool {
        self.dragging
    }

    fn overlays(&self) -> Vec<Overlay> {
        match self.ruler {
            Some(r) if r.dragged => vec![Overlay::Line { kind: OverlayKind::Ruler, from: r.from, to: r.to }],
            _ => Vec::new(),
        }
    }
}
