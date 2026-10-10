//! The selection's bounding box, its knobs, and the freehand transform drags the knobs share with
//! the Rotate and Scale tools.
//!
//! Gestures on the box, with the Select tool and `view.showBoundingBox` on (`boundingBox.scaleKnobs`,
//! `boundingBox.rotateOutside`):
//!
//! | Gesture | Result |
//! |---|---|
//! | drag a knob | `transform.scale` about the opposite knob; a side knob scales one axis |
//! | `Shift` while dragging a knob | the same factor on both axes |
//! | `Alt` while dragging a knob | about the box centre |
//! | drag from just outside the box | `transform.rotateCounterclockwise` about the knob opposite the grab position |
//! | `Alt` while rotating | about the box centre |
//! | click just outside the box | what a click on empty canvas does: clear the selection |
//!
//! An object under the pointer in the band outside the box belongs to the Select tool.
//!
//! Every drag is one interaction. Its previews pass the fixed point as `origin: [x, y]` and the
//! factors or the angle measured from the press, so each preview replaces the last one.
//!
//! The box is the one the engine measures for a box origin (`docs/spec/04-palette.md`): selected
//! nodes and handles, the curves of whole paths, the outlines of components, then anchors, guides
//! and annotations; for an empty selection (the whole layer), its shapes, or its anchors when it
//! has none. A component's outline is measured from its nodes and handles, which can give a
//! slightly larger box than the curves.

use fontcraft_geom::{Point, Rect, Shape as _};
use fontcraft_model::{Node, Shape};
use serde_json::{Value, json};

use crate::event::{Modifiers, PointerEvent, PointerKind};
use crate::geometry::{dist, finite, near_box, usable};
use crate::hit::hit_test;
use crate::output::{Action, Invocation, Output, Overlay, OverlayKind};
use crate::scene::Scene;
use crate::selection::Selection;
use crate::tool::Context;

/// How far outside the selection the box and its knobs are drawn, in screen points, so the knobs
/// do not cover the selected nodes.
pub const KNOB_OFFSET: f64 = 6.0;
/// How far outside the drawn box a drag still rotates, in screen points.
pub const ROTATE_BAND: f64 = 16.0;
/// Largest scale a drag sends, in percent (the engine's limit).
const MAX_PERCENT: f64 = 1_000_000.0;
/// Factors and angles are rounded to this many parts per unit, so float noise does not reach the
/// font (a quarter turn is sent as exactly 90).
const PRECISION: f64 = 1e6;

/// One of the eight knobs, named by its place on the box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Knob {
    TopLeft,
    Top,
    TopRight,
    Left,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Knob {
    pub const ALL: [Knob; 8] = [Knob::TopLeft, Knob::Top, Knob::TopRight, Knob::Left, Knob::Right, Knob::BottomLeft, Knob::Bottom, Knob::BottomRight];

    /// The knob's direction from the box centre: −1, 0 or 1 on each axis (y grows upwards).
    pub fn side(self) -> (i8, i8) {
        match self {
            Knob::TopLeft => (-1, 1),
            Knob::Top => (0, 1),
            Knob::TopRight => (1, 1),
            Knob::Left => (-1, 0),
            Knob::Right => (1, 0),
            Knob::BottomLeft => (-1, -1),
            Knob::Bottom => (0, -1),
            Knob::BottomRight => (1, -1),
        }
    }

    /// The knob in direction `side`; `None` for the centre.
    pub fn from_side(side: (i8, i8)) -> Option<Knob> {
        Knob::ALL.into_iter().find(|k| k.side() == (side.0.signum(), side.1.signum()))
    }

    pub fn opposite(self) -> Knob {
        let (x, y) = self.side();
        Knob::from_side((-x, -y)).unwrap_or(self)
    }

    /// The knob's point on `r`. Centres are the mean of the two edges, as the engine computes a
    /// box origin.
    pub fn point_in(self, r: Rect) -> Point {
        let (x, y) = self.side();
        Point::new(coordinate(r.x0, r.x1, x), coordinate(r.y0, r.y1, y))
    }
}

fn coordinate(lo: f64, hi: f64, side: i8) -> f64 {
    match side {
        s if s < 0 => lo,
        0 => (lo + hi) / 2.0,
        _ => hi,
    }
}

/// The centre of `r`, as the engine's box origin `center`.
pub fn center(r: Rect) -> Point {
    Point::new((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0)
}

/// The box of `selection` in `scene`; for an empty selection, the box of the whole layer.
pub fn selection_box(scene: &Scene, selection: &Selection) -> Option<Rect> {
    let layer = &scene.layer;
    let anchors = |names: Option<&Selection>| {
        fontcraft_geom::bounds(layer.anchors.iter().filter(|a| names.is_none_or(|s| s.anchors.contains(&a.name))).map(|a| Point::new(a.x, a.y)))
    };
    if selection.is_empty() {
        let shapes = (0..layer.shapes.len()).filter_map(|i| shape_box(scene, i)).reduce(|a, b| a.union(b));
        return shapes.or_else(|| anchors(None));
    }
    // The points of a whole shape count through the shape's box.
    let points = fontcraft_geom::bounds(
        selection.points.iter().filter(|(s, _)| !selection.shapes.contains(s)).filter_map(|&(s, n)| scene.node(s, n)).map(Node::point),
    );
    let shapes = selection.shapes.iter().filter_map(|&i| shape_box(scene, i)).reduce(|a, b| a.union(b));
    let guides = fontcraft_geom::bounds(selection.guides.iter().filter_map(|g| {
        let list = match g.scope {
            crate::selection::GuideScope::Local => scene.local_guides(),
            crate::selection::GuideScope::Global => scene.global_guides,
        };
        list.get(g.index).map(|g| Point::new(g.x, g.y))
    }));
    let notes = fontcraft_geom::bounds(selection.annotations.iter().filter_map(|&i| layer.info.annotations.get(i)).map(|a| Point::new(a.x, a.y)));
    [points, shapes, anchors(Some(selection)), guides, notes].into_iter().flatten().reduce(|a, b| a.union(b))
}

/// The box of the shape at `index`: a path's curves, a component's outline, an image's placement.
fn shape_box(scene: &Scene, index: usize) -> Option<Rect> {
    let dot = |p: Point| finite(p).then(|| Rect::from_points(p, p));
    match scene.layer.shapes.get(index)? {
        Shape::Path(p) => match p.to_bez() {
            Ok(b) if !b.elements().is_empty() => Some(b.bounding_box()).filter(|r| rect_finite(*r)),
            _ => fontcraft_geom::bounds(p.nodes.iter().map(Node::point)),
        },
        Shape::Component(c) => match scene.components().iter().find(|g| g.shape == index) {
            Some(g) if !g.nodes.is_empty() => fontcraft_geom::bounds(g.nodes.iter().chain(&g.handles).copied()),
            _ => dot(c.affine() * Point::ZERO),
        },
        Shape::Image(img) => dot(img.transform.affine() * Point::ZERO),
        Shape::Raw(_) => None,
    }
}

fn rect_finite(r: Rect) -> bool {
    [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite())
}

/// The box drawn with knobs for `selection`: shown when it holds more than one item or a whole
/// shape, and the box has some extent.
pub fn shown_box(scene: &Scene, selection: &Selection) -> Option<Rect> {
    if selection.len() < 2 && selection.shapes.is_empty() {
        return None;
    }
    let r = selection_box(scene, selection)?;
    (rect_finite(r) && (r.width() > 0.0 || r.height() > 0.0)).then_some(r)
}

/// `r` grown by [`KNOB_OFFSET`] screen points on each side: where the box and its knobs are drawn.
pub fn drawn_box(r: Rect, units_per_point: f64) -> Rect {
    let d = KNOB_OFFSET * units_per_point;
    Rect::new(r.x0 - d, r.y0 - d, r.x1 + d, r.y1 + d)
}

/// Where each knob is drawn, in font units.
pub fn knob_points(r: Rect, units_per_point: f64) -> [(Knob, Point); 8] {
    let outer = drawn_box(r, units_per_point);
    Knob::ALL.map(|k| (k, k.point_in(outer)))
}

/// A percentage from a factor, rounded, within the engine's limit; 100 when it is not a number.
fn percent(factor: f64) -> f64 {
    let p = round(factor * 100.0);
    if p.is_finite() { p.clamp(-MAX_PERCENT, MAX_PERCENT) } else { 100.0 }
}

fn round(v: f64) -> f64 {
    (v * PRECISION).round() / PRECISION + 0.0
}

/// The factors that take `from` to `to` about `fixed`, on the axes `axes` turns on (others keep
/// 1). An axis where `from` lies within `min` of `fixed` keeps 1. `proportional`: both axes take
/// the factor that differs more from 1.
pub fn scale_factors(fixed: Point, from: Point, to: Point, axes: (bool, bool), proportional: bool, min: f64) -> (f64, f64) {
    let axis = |f: f64, a: f64, b: f64, on: bool| {
        let span = a - f;
        (on && span != 0.0 && span.abs() > min).then(|| (b - f) / span).filter(|v| v.is_finite())
    };
    let fx = axis(fixed.x, from.x, to.x, axes.0);
    let fy = axis(fixed.y, from.y, to.y, axes.1);
    if proportional {
        let s = match (fx, fy) {
            (Some(x), Some(y)) => {
                if (x - 1.0).abs() >= (y - 1.0).abs() {
                    x
                } else {
                    y
                }
            }
            (Some(v), None) | (None, Some(v)) => v,
            (None, None) => 1.0,
        };
        (s, s)
    } else {
        (fx.unwrap_or(1.0), fy.unwrap_or(1.0))
    }
}

/// The counter-clockwise angle in degrees, in (−180, 180], that turns `from` to `to` about
/// `pivot`. 0 when either lies on the pivot.
pub fn rotation_angle(pivot: Point, from: Point, to: Point) -> f64 {
    let (ax, ay) = (from.x - pivot.x, from.y - pivot.y);
    let (bx, by) = (to.x - pivot.x, to.y - pivot.y);
    let a = round((ax * by - ay * bx).atan2(ax * bx + ay * by).to_degrees());
    if a.is_finite() { a } else { 0.0 }
}

/// `angle` to the nearest multiple of `step` degrees.
pub fn snap_angle(angle: f64, step: f64) -> f64 {
    if step > 0.0 { (angle / step).round() * step + 0.0 } else { angle }
}

/// `transform.scale` of `selection` by the factors about `origin`.
pub fn scale_invocation(scene: &Scene, selection: &Selection, (fx, fy): (f64, f64), origin: Point) -> Invocation {
    let mut m = selection.targets(scene);
    m.insert("x".into(), json!(percent(fx)));
    m.insert("y".into(), json!(percent(fy)));
    m.insert("origin".into(), json!([origin.x, origin.y]));
    Invocation::new("transform.scale", Value::Object(m))
}

/// `transform.rotateCounterclockwise` of `selection` by `degrees` about `origin`.
pub fn rotate_invocation(scene: &Scene, selection: &Selection, degrees: f64, origin: Point) -> Invocation {
    let mut m = selection.targets(scene);
    m.insert("angle".into(), json!(degrees));
    m.insert("origin".into(), json!([origin.x, origin.y]));
    Invocation::new("transform.rotateCounterclockwise", Value::Object(m))
}

/// What a press on the box took hold of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grab {
    /// A knob: the drag scales.
    Knob(Knob),
    /// The band just outside the box, nearest this knob: the drag rotates about the opposite one.
    Outside(Knob),
}

/// What lies under `pos` on the box of the Select tool's selection: `None` when the box is not
/// showing or the pointer is elsewhere.
pub fn grab_at(cx: &Context<'_>, pos: Point) -> Option<(Rect, Grab)> {
    if !cx.settings.bounding_box || cx.scene.locked || !finite(pos) {
        return None;
    }
    let r = shown_box(cx.scene, cx.selection)?;
    let units = cx.view.units(1.0);
    let tolerance = cx.view.units(cx.settings.hit_tolerance);
    let nearest = knob_points(r, units)
        .into_iter()
        .map(|(k, at)| (k, dist(at, pos)))
        .filter(|(_, d)| d.is_finite() && *d <= tolerance)
        .min_by(|a, b| a.1.total_cmp(&b.1));
    if let Some((k, _)) = nearest {
        return Some((r, Grab::Knob(k)));
    }
    let outer = drawn_box(r, units);
    let inside = (outer.x0..=outer.x1).contains(&pos.x) && (outer.y0..=outer.y1).contains(&pos.y);
    if inside || !near_box(pos, (Point::new(outer.x0, outer.y0), Point::new(outer.x1, outer.y1)), ROTATE_BAND * units) {
        return None;
    }
    // An object in the band is the Select tool's to click or drag.
    if hit_test(cx.scene, &cx.view, pos, cx.settings.hit_tolerance).is_some() {
        return None;
    }
    let side = |v: f64, lo: f64, hi: f64| {
        if v < lo {
            -1
        } else if v > hi {
            1
        } else {
            0
        }
    };
    let k = Knob::from_side((side(pos.x, outer.x0, outer.x1), side(pos.y, outer.y0, outer.y1)))?;
    Some((r, Grab::Outside(k)))
}

/// A press or drag on the bounding box, handed over by the Select tool.
#[derive(Clone, Debug)]
pub struct BoxDrag {
    grab: Grab,
    /// The selection's box at the press; the previews change the font, not this.
    rect: Rect,
    down: Point,
    selection: Selection,
    started: bool,
    last_pos: Point,
    last_sent: Option<Invocation>,
}

impl BoxDrag {
    /// A press at `ev` that takes hold of the box, or `None`.
    pub fn press(cx: &Context<'_>, ev: &PointerEvent) -> Option<Self> {
        let (rect, grab) = grab_at(cx, ev.pos)?;
        Some(Self { grab, rect, down: ev.pos, selection: cx.selection.clone(), started: false, last_pos: ev.pos, last_sent: None })
    }

    pub fn grab(&self) -> Grab {
        self.grab
    }

    /// The fixed point for the modifiers `mods`.
    fn fixed(&self, mods: Modifiers) -> Point {
        let knob = match self.grab {
            Grab::Knob(k) | Grab::Outside(k) => k,
        };
        if mods.alt { center(self.rect) } else { knob.opposite().point_in(self.rect) }
    }

    fn invocation(&self, cx: &Context<'_>, pos: Point, mods: Modifiers) -> Invocation {
        let fixed = self.fixed(mods);
        match self.grab {
            Grab::Knob(k) => {
                let from = k.point_in(self.rect);
                let to = Point::new(from.x + pos.x - self.down.x, from.y + pos.y - self.down.y);
                let (sx, sy) = k.side();
                let factors = scale_factors(fixed, from, to, (sx != 0, sy != 0), mods.shift, 0.0);
                scale_invocation(cx.scene, &self.selection, factors, fixed)
            }
            Grab::Outside(_) => rotate_invocation(cx.scene, &self.selection, rotation_angle(fixed, self.down, pos), fixed),
        }
    }

    /// The preview for the pointer at `pos`, when it differs from the last one sent.
    fn update(&mut self, cx: &Context<'_>, pos: Point, mods: Modifiers) -> Option<Invocation> {
        if !finite(pos) {
            return None;
        }
        self.last_pos = pos;
        let inv = self.invocation(cx, pos, mods);
        if self.last_sent.as_ref() == Some(&inv) {
            return None;
        }
        self.last_sent = Some(inv.clone());
        Some(inv)
    }

    fn label(&self) -> &'static str {
        match self.grab {
            Grab::Knob(_) => "Scale",
            Grab::Outside(_) => "Rotate",
        }
    }

    fn drag(&mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        let mut out = Output::handled();
        if !self.started {
            if dist(cx.view.to_screen(self.down), cx.view.to_screen(ev.pos)) < cx.settings.drag_threshold {
                return out;
            }
            self.started = true;
            out.push(Action::Begin { label: self.label().into() });
        }
        if let Some(inv) = self.update(cx, ev.pos, ev.mods) {
            out.push(Action::Preview { invocation: inv });
        }
        out
    }

    fn up(mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        let mut out = Output::handled();
        if self.started {
            if let Some(inv) = self.update(cx, ev.pos, ev.mods) {
                out.push(Action::Preview { invocation: inv });
            }
            out.push(Action::Commit);
        } else if matches!(self.grab, Grab::Outside(_)) && !ev.mods.shift && !cx.selection.is_empty() {
            out.push(Action::Select { selection: Selection::new() });
        }
        out
    }

    /// The modifiers changed while the pointer stood still.
    pub fn modifiers(&mut self, cx: &Context<'_>, mods: Modifiers) -> Output {
        let mut out = Output::handled();
        if self.started
            && let Some(inv) = self.update(cx, self.last_pos, mods)
        {
            out.push(Action::Preview { invocation: inv });
        }
        out
    }

    fn cancel(self) -> Output {
        if self.started { Output::with(Action::Cancel) } else { Output::default() }
    }
}

/// The Select tool's hook: gives a pointer event to the box gesture in `slot`, or starts one. `None`
/// when the event is the Select tool's own.
pub fn route(slot: &mut Option<BoxDrag>, cx: &Context<'_>, ev: &PointerEvent) -> Option<Output> {
    if slot.is_none() {
        return match ev.kind {
            PointerKind::Down => {
                *slot = Some(BoxDrag::press(cx, ev)?);
                Some(Output::handled())
            }
            // A double click on a knob is two presses on it, not a node edit.
            PointerKind::DoubleClick => grab_at(cx, ev.pos).map(|_| Output::handled()),
            _ => None,
        };
    }
    match ev.kind {
        PointerKind::Drag => slot.as_mut().map(|d| d.drag(cx, ev)),
        PointerKind::Move => Some(Output::handled()),
        PointerKind::Up => slot.take().map(|d| d.up(cx, ev)),
        // A press without its release: the Select tool cancels the old gesture first.
        PointerKind::Down | PointerKind::DoubleClick => None,
    }
}

/// Ends the box gesture in `slot`, undoing its previews.
pub fn cancel(slot: &mut Option<BoxDrag>) -> Output {
    slot.take().map_or_else(Output::default, BoxDrag::cancel)
}

// ── The Rotate and Scale tools ─────────────────────────────────────────────────────────────────

/// What a freehand drag does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FreeKind {
    Rotate,
    Scale,
}

#[derive(Clone, Debug)]
struct FreeDrag {
    down: Point,
    /// The fixed point, taken at the press.
    origin: Option<Point>,
    selection: Selection,
    started: bool,
    last_pos: Point,
    last_sent: Option<Invocation>,
}

/// The Rotate and Scale tools' shared state machine: a click places the reference point
/// (`transform.setOrigin` with `point`), a drag transforms the selection about it, or about the
/// centre of the selection's box while no point is placed.
#[derive(Clone, Debug)]
pub struct Freehand {
    kind: FreeKind,
    /// The point this tool last placed, used while the shell does not report one.
    clicked: Option<Point>,
    drag: Option<FreeDrag>,
}

impl Freehand {
    pub fn new(kind: FreeKind) -> Self {
        Self { kind, clicked: None, drag: None }
    }

    /// The point a drag would transform about now.
    pub fn origin(&self, cx: &Context<'_>) -> Option<Point> {
        cx.settings.reference.or(self.clicked).or_else(|| selection_box(cx.scene, cx.selection).map(center))
    }

    fn invocation(&self, cx: &Context<'_>, drag: &FreeDrag, origin: Point, pos: Point, mods: Modifiers) -> Invocation {
        match self.kind {
            FreeKind::Rotate => {
                let mut angle = rotation_angle(origin, drag.down, pos);
                if mods.shift {
                    angle = snap_angle(angle, 90.0);
                }
                rotate_invocation(cx.scene, &drag.selection, angle, origin)
            }
            FreeKind::Scale => {
                let min = cx.view.units(cx.settings.drag_threshold);
                let factors = scale_factors(origin, drag.down, pos, (true, true), mods.shift, min);
                scale_invocation(cx.scene, &drag.selection, factors, origin)
            }
        }
    }

    fn update(&mut self, cx: &Context<'_>, pos: Point, mods: Modifiers) -> Option<Invocation> {
        let drag = self.drag.as_ref()?;
        let origin = drag.origin?;
        if !finite(pos) {
            return None;
        }
        let inv = self.invocation(cx, drag, origin, pos, mods);
        let drag = self.drag.as_mut()?;
        drag.last_pos = pos;
        if drag.last_sent.as_ref() == Some(&inv) {
            return None;
        }
        drag.last_sent = Some(inv.clone());
        Some(inv)
    }

    fn label(&self) -> &'static str {
        match self.kind {
            FreeKind::Rotate => "Rotate",
            FreeKind::Scale => "Scale",
        }
    }

    pub fn pointer(&mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        if !finite(ev.pos) {
            return Output::default();
        }
        match ev.kind {
            PointerKind::Move => Output::default(),
            PointerKind::Down | PointerKind::DoubleClick => {
                let mut out = self.cancel();
                out.handled = true;
                if ev.kind == PointerKind::Down {
                    let origin = self.origin(cx);
                    self.drag =
                        Some(FreeDrag { down: ev.pos, origin, selection: cx.selection.clone(), started: false, last_pos: ev.pos, last_sent: None });
                }
                out
            }
            PointerKind::Drag => {
                let mut out = Output::handled();
                let Some(drag) = self.drag.as_mut() else { return Output::default() };
                if !drag.started {
                    let moved = dist(cx.view.to_screen(drag.down), cx.view.to_screen(ev.pos));
                    if moved < cx.settings.drag_threshold || cx.scene.locked || drag.origin.is_none() {
                        return out;
                    }
                    drag.started = true;
                    out.push(Action::Begin { label: self.label().into() });
                }
                if let Some(inv) = self.update(cx, ev.pos, ev.mods) {
                    out.push(Action::Preview { invocation: inv });
                }
                out
            }
            PointerKind::Up => {
                let mut out = Output::handled();
                let Some(started) = self.drag.as_ref().map(|d| d.started) else { return Output::default() };
                if started {
                    if let Some(inv) = self.update(cx, ev.pos, ev.mods) {
                        out.push(Action::Preview { invocation: inv });
                    }
                    out.push(Action::Commit);
                } else if usable(ev.pos) {
                    self.clicked = Some(ev.pos);
                    out.push(Action::Run { invocation: Invocation::new("transform.setOrigin", json!({"point": [ev.pos.x, ev.pos.y]})) });
                }
                self.drag = None;
                out
            }
        }
    }

    pub fn modifiers(&mut self, cx: &Context<'_>, mods: Modifiers) -> Output {
        let mut out = Output::default();
        if let Some(pos) = self.drag.as_ref().filter(|d| d.started).map(|d| d.last_pos)
            && let Some(inv) = self.update(cx, pos, mods)
        {
            out.push(Action::Preview { invocation: inv });
        }
        out
    }

    pub fn cancel(&mut self) -> Output {
        match self.drag.take() {
            Some(d) if d.started => Output::with(Action::Cancel),
            _ => Output::default(),
        }
    }

    pub fn busy(&self) -> bool {
        self.drag.is_some()
    }

    /// While dragging: a line from the fixed point to the pointer.
    pub fn overlays(&self) -> Vec<Overlay> {
        match &self.drag {
            Some(FreeDrag { started: true, origin: Some(origin), last_pos, .. }) => {
                vec![Overlay::Line { kind: OverlayKind::Hover, from: *origin, to: *last_pos }]
            }
            _ => Vec::new(),
        }
    }
}
