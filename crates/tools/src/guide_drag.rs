//! The Select tool's guide gestures (`guides.knobAndSelection`, `guides.angleLock`,
//! `docs/spec/03a` § 20).
//!
//! | Gesture | Result |
//! |---|---|
//! | drag a guide's knob | `guide.setProperties` with the knob's new `x`, `y`, in one interaction; the knob snaps like a node unless `Ctrl` is held |
//! | drag the guide elsewhere | `guide.setProperties` with a new `angle`: the guide turns about its knob, following the pointer |
//! | drag an angle-locked guide elsewhere | it moves, as a knob drag does |
//! | double-click the knob | the guide turns by 90° (an angle-locked guide does not) |
//! | double-click the guide elsewhere | the guide's properties dialog ([`PROPERTIES_DIALOG`]) |
//!
//! A locked guide does none of these. Neither does a local guide of a locked glyph; a global
//! guide belongs to the master, so a glyph lock leaves it free.

use fontcraft_geom::Point;
use fontcraft_model::{Family, Guide, Layer};
use serde_json::{Map, Value, json};

use crate::geometry::{dist, finite};
use crate::output::Invocation;
use crate::scene::Scene;
use crate::selection::{GuideRef, GuideScope};
use crate::snap::{Moving, Snap, snap};
use crate::tool::Context;

/// The dialog a double-click on a guide's line opens; the shell registers it.
pub const PROPERTIES_DIALOG: &str = "dialog.guideProperties";

/// The `.glyphs` key that holds a guide's sidebearing reference, as `guide.setProperties`
/// writes it: absent for the left side, `right`, or `center` for both.
const RELATIVE_KEY: &str = "alignment";

/// Closer to the knob than this (screen points), the pointer gives no usable angle.
const MIN_TURN_RADIUS: f64 = 1.0;

/// The guide at `r` in `scene`.
pub fn guide<'s>(scene: &'s Scene<'_>, r: GuideRef) -> Option<&'s Guide> {
    let list: &'s [Guide] = match r.scope {
        GuideScope::Local => scene.local_guides(),
        GuideScope::Global => scene.global_guides,
    };
    list.get(r.index)
}

/// Whether a gesture may change the guide: it is not locked, and a local one's glyph is not
/// locked either.
pub fn editable(scene: &Scene, r: GuideRef, g: &Guide) -> bool {
    !g.locked && (r.scope == GuideScope::Global || !scene.locked)
}

fn knob(g: &Guide) -> Point {
    Point::new(g.x, g.y)
}

/// Whether `pos` (font units) is on the guide's knob.
pub fn on_knob(cx: &Context<'_>, g: &Guide, pos: Point) -> bool {
    dist(cx.view.to_screen(knob(g)), cx.view.to_screen(pos)) <= cx.settings.hit_tolerance
}

/// An angle in degrees, kept from 0 to 360 as the guide commands keep it.
fn norm_angle(a: f64) -> f64 {
    let r = a.rem_euclid(360.0);
    if r.is_finite() && r < 360.0 { r } else { 0.0 }
}

/// The angle in degrees of the line from `from` to `to`.
fn heading(from: Point, to: Point) -> f64 {
    (to.y - from.y).atan2(to.x - from.x).to_degrees()
}

/// `{glyph, layer, guide}`: the address `guide.setProperties` takes.
fn target(scene: &Scene, r: GuideRef) -> Map<String, Value> {
    let mut m = scene.target();
    m.insert("guide".into(), json!({"scope": r.scope.as_str(), "index": r.index}));
    m
}

/// The sidebearing reference as `guide.setProperties` takes it.
fn relative_to(g: &Guide) -> &'static str {
    match g.extra.get(Family::Glyphs, RELATIVE_KEY).and_then(fontcraft_model::plist::Value::as_str) {
        Some("right") => "right",
        Some("center") => "both",
        _ => "left",
    }
}

/// The parameters the properties dialog opens with: the guide's address and its present
/// properties, named as `guide.setProperties` names them.
pub fn properties(scene: &Scene, r: GuideRef, g: &Guide) -> Map<String, Value> {
    let mut m = target(scene, r);
    m.insert("x".into(), json!(g.x));
    m.insert("y".into(), json!(g.y));
    m.insert("angle".into(), json!(norm_angle(g.angle)));
    m.insert("angleLocked".into(), json!(g.angle_locked));
    m.insert("relativeTo".into(), json!(relative_to(g)));
    m.insert("measurement".into(), json!(g.show_measurement));
    m.insert("name".into(), json!(g.name.clone().unwrap_or_default()));
    m.insert("color".into(), json!(g.color.clone().unwrap_or_default()));
    m
}

/// What a double-click at `pos` on guide `r` runs: a quarter turn on the knob, the properties
/// dialog elsewhere. `None` when the guide may not change.
pub fn double_click(cx: &Context<'_>, r: GuideRef, pos: Point) -> Option<Invocation> {
    let g = guide(cx.scene, r)?;
    if !editable(cx.scene, r, g) {
        return None;
    }
    if !on_knob(cx, g, pos) {
        return Some(Invocation::new("dialog.open", json!({"dialog": PROPERTIES_DIALOG, "params": properties(cx.scene, r, g)})));
    }
    if g.angle_locked || !g.angle.is_finite() {
        return None;
    }
    let mut params = target(cx.scene, r);
    params.insert("angle".into(), json!(norm_angle(g.angle + 90.0)));
    Some(Invocation::new("guide.setProperties", Value::Object(params)))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Move,
    Turn,
}

/// A guide drag, from the press to the release.
#[derive(Clone, Debug)]
pub struct GuideDrag {
    guide: GuideRef,
    mode: Mode,
    down: Point,
    /// The point that snaps while moving: the knob, or the pressed point of the line.
    grab: Point,
    /// The knob and the angle at the press.
    knob: Point,
    angle: f64,
    pub last_pos: Point,
    last_sent: Option<(f64, f64, f64)>,
    pub snap: Option<Snap>,
}

impl GuideDrag {
    /// The drag a press at `pos` on guide `r` starts; `at` is the nearest point of the guide's
    /// line. `None` when the guide may not change.
    pub fn press(cx: &Context<'_>, r: GuideRef, pos: Point, at: Point) -> Option<Self> {
        let g = guide(cx.scene, r)?;
        let k = knob(g);
        if !editable(cx.scene, r, g) || !finite(k) || !finite(pos) || !g.angle.is_finite() {
            return None;
        }
        let (mode, grab) = if on_knob(cx, g, pos) {
            (Mode::Move, k)
        } else if g.angle_locked {
            (Mode::Move, if finite(at) { at } else { pos })
        } else {
            (Mode::Turn, pos)
        };
        Some(Self { guide: r, mode, down: pos, grab, knob: k, angle: g.angle, last_pos: pos, last_sent: None, snap: None })
    }

    /// The undo step's label.
    pub fn label(&self) -> &'static str {
        match self.mode {
            Mode::Move => "Move Guide",
            Mode::Turn => "Rotate Guide",
        }
    }

    /// Takes the drag to `pos` and returns the preview to run, when it differs from the last
    /// one. `ctrl` suspends snapping.
    pub fn update(&mut self, cx: &Context<'_>, pos: Point, ctrl: bool) -> Option<Invocation> {
        if !finite(pos) {
            return None;
        }
        self.last_pos = pos;
        let mut params = target(cx.scene, self.guide);
        let sent = match self.mode {
            Mode::Move => {
                let to = Point::new(self.grab.x + pos.x - self.down.x, self.grab.y + pos.y - self.down.y);
                self.snap = if ctrl { None } else { Some(snap_apart(cx, self.guide, to)) };
                let to = self.snap.as_ref().map_or(to, |s| s.point);
                let (x, y) = (self.knob.x + to.x - self.grab.x, self.knob.y + to.y - self.grab.y);
                if !(x.is_finite() && y.is_finite()) {
                    return None;
                }
                params.insert("x".into(), json!(x));
                params.insert("y".into(), json!(y));
                (x, y, self.angle)
            }
            Mode::Turn => {
                if dist(cx.view.to_screen(self.knob), cx.view.to_screen(pos)) < MIN_TURN_RADIUS {
                    return None;
                }
                // The pressed side of the knob follows the pointer, so the guide does not jump
                // when the drag starts.
                let a = norm_angle(self.angle + heading(self.knob, pos) - heading(self.knob, self.down));
                params.insert("angle".into(), json!(a));
                (self.knob.x, self.knob.y, a)
            }
        };
        if self.last_sent == Some(sent) {
            return None;
        }
        self.last_sent = Some(sent);
        Some(Invocation::new("guide.setProperties", Value::Object(params)))
    }
}

/// Snaps `p` as a node would snap, except to the guide being dragged.
fn snap_apart(cx: &Context<'_>, r: GuideRef, p: Point) -> Snap {
    let (moving, distance) = (Moving::default(), cx.settings.snap_distance);
    if !cx.scene.guides_shown {
        return snap(cx.scene, &cx.view, p, &moving, distance);
    }
    match r.scope {
        GuideScope::Local => {
            let mut layer: Layer = (*cx.scene.layer).clone();
            if r.index < layer.info.guides.len() {
                layer.info.guides.remove(r.index);
            }
            snap(&with_parts(cx.scene, &layer, cx.scene.global_guides), &cx.view, p, &moving, distance)
        }
        GuideScope::Global => {
            let mut globals = cx.scene.global_guides.to_vec();
            if r.index < globals.len() {
                globals.remove(r.index);
            }
            let mut without = with_parts(cx.scene, &cx.scene.layer, &globals);
            // The indices after the dragged guide move down by one.
            without.hidden_global_guides =
                cx.scene.hidden_global_guides.iter().filter(|&&i| i != r.index).map(|&i| if i > r.index { i - 1 } else { i }).collect();
            snap(&without, &cx.view, p, &moving, distance)
        }
    }
}

/// `scene` with another layer and other global guides.
fn with_parts<'b>(scene: &Scene<'b>, layer: &'b Layer, globals: &'b [Guide]) -> Scene<'b> {
    let mut s = scene.clone();
    s.layer = std::borrow::Cow::Borrowed(layer);
    s.global_guides = globals;
    s
}
