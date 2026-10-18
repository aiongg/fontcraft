//! Drag aids of the Select tool: Tunni line and Tunni point drags, the lever, sliding a node along
//! its curve, and the points that follow a move (Power Nudge and declared servant nodes).
//!
//! | Gesture | Result |
//! |---|---|
//! | drag a Tunni line, or `Shift`+drag a Tunni point (Tunni lines showing) | both handles of the segment change by the same share of their distance to the Tunni point (`tunni.dragLineSymmetric`); other segments whose two handles are selected change with it |
//! | `Alt`+drag a Tunni line | only the handle nearer the press changes length (`tunni.dragPointAsymmetric`) |
//! | drag a Tunni point | only the handle whose direction is nearer the drag's changes length |
//! | click a Tunni line or point | selects the segment's two handles; `Shift` toggles them |
//! | `Shift`+`Alt`+drag a node | the node slides along its curve and the curve keeps its shape; an open end slides past the end and extends the curve (`nodes.slideAlongCurve`) |
//! | `Cmd` held during a move, a Tunni drag or a slide | the drag follows 1/ratio of the pointer's movement over the first [`LEVER_RAMP`] screen points, then moves as fast as the pointer (`drag.leverPrecision`, `editing.leverRatio`) |
//! | a move with Power Nudge on | unselected smooth nodes next to a moved node follow in proportion, up to a corner node, an extreme or a locked node (`path.togglePowerNudge`) |
//! | a move next to a declared servant | the servant follows on its axis, with Power Nudge on or off (`path.setServantNode`) |
//!
//! Every drag is one interaction made of `selection.move` steps in `pointsOnly` mode, so the
//! engine sees ordinary moves: grid rounding, locks and undo work as for any other move.
//!
//! **Followers.** From each moved on-curve node the walk goes both ways along the contour over
//! unselected on-curve nodes that may follow (declared servants; with Power Nudge also smooth
//! nodes that are not extremes). It stops at the first node that may not follow, which stays
//! fixed. A follower moves by the drag times its position between the fixed node and the moved
//! node on each axis (by length along the contour where the two share that coordinate); when
//! the walk ends at another moved node or at an open end, followers move by the whole drag. A
//! follower's own handles move with it.

use std::collections::BTreeMap;

use fontcraft_geom::{CubicBez, ParamCurve, Point, Vec2};
use fontcraft_model::{Node, Path, fc_keys};
use serde_json::{Map, Value, json};

use crate::event::{Modifiers, PointerEvent, PointerKind};
use crate::geometry::{dist, finite, usable};
use crate::hit::{Hit, hit_test};
use crate::output::{Action, Invocation, Output};
use crate::scene::Scene;
use crate::segment::{Piece, Segment, segments};
use crate::selection::{Item, Selection};
use crate::tool::{Context, ToolSettings};

/// The lever ratio while `editing.leverRatio` is not set.
pub const DEFAULT_LEVER_RATIO: f64 = 6.0;
/// Largest lever ratio.
pub const MAX_LEVER_RATIO: f64 = 100.0;
/// How far, in screen points, the pointer moves at the lever's slow rate before the drag follows
/// it one to one.
pub const LEVER_RAMP: f64 = 50.0;
/// The user setting that holds the lever ratio.
pub const LEVER_RATIO_SETTING: &str = "editing.leverRatio";
/// The user setting that holds the Power Nudge switch.
pub const POWER_NUDGE_SETTING: &str = "editing.powerNudge";
/// Most on-curve nodes one walk passes over before it gives up (and moves no follower).
pub const MAX_SERVANT_RUN: usize = 1_000;
/// Most points that follow one move.
pub const MAX_FOLLOWERS: usize = 10_000;
/// Most segments one Tunni drag changes together.
pub const MAX_TUNNI_SEGMENTS: usize = 10_000;

/// A dragged handle never gets shorter than this, in font units.
const MIN_HANDLE: f64 = 1.0;
/// A dragged handle never gets longer than this share of its node's distance to the Tunni point.
const MAX_TENSION: f64 = 4.0;
/// Lengths below this, in font units, count as zero.
const EPS: f64 = 1e-9;
/// Coordinates closer than this count as equal when deciding extremes and proportions.
const SAME: f64 = 1e-6;
/// Evenly spaced samples before the nearest point of a slide is refined.
const SLIDE_SAMPLES: usize = 64;
/// Ternary-search steps that refine it.
const SLIDE_REFINE: usize = 40;
/// How far past an open end a node may slide, in the end segment's parameter.
const SLIDE_EXTEND: f64 = 1.0;
/// A sliding node stops this far, in the segment's parameter, before the next node.
const SLIDE_MARGIN: f64 = 0.02;

impl ToolSettings {
    /// These settings with the drag aids' user settings, read through `get` (a setting id to its
    /// value). Values out of range are ignored.
    pub fn with_drag_aids(mut self, get: impl Fn(&str) -> Option<Value>) -> Self {
        if let Some(r) = get(LEVER_RATIO_SETTING).and_then(|v| v.as_f64())
            && r.is_finite()
            && (1.0..=MAX_LEVER_RATIO).contains(&r)
        {
            self.lever_ratio = r;
        }
        if let Some(on) = get(POWER_NUDGE_SETTING).and_then(|v| v.as_bool()) {
            self.power_nudge = on;
        }
        self
    }
}

// ---------------------------------------------------------------------------------------------
// The lever
// ---------------------------------------------------------------------------------------------

/// The lever during one drag: where the pointer and the dragged position were when the lever
/// last went on or off, so pressing or letting go of the modifier never makes the drag jump.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lever {
    /// The pointer and the dragged position when the lever last went on or off.
    pointer: Point,
    out: Point,
    /// The pointer and the dragged position at the last event.
    last_pointer: Point,
    last: Point,
    /// Whether the lever is on; `None` before the first event.
    on: Option<bool>,
}

impl Lever {
    /// A lever for a drag that started at `down`.
    pub fn new(down: Point) -> Self {
        Self { pointer: down, out: down, last_pointer: down, last: down, on: None }
    }

    /// Where the drag stands when the pointer is at `pos`: the pointer itself without the lever,
    /// slowed down while `Cmd` is held.
    pub fn apply(&mut self, cx: &Context<'_>, pos: Point, mods: Modifiers) -> Point {
        if !finite(pos) {
            return self.last;
        }
        let ratio = cx.settings.lever_ratio;
        let on = mods.cmd && ratio.is_finite() && ratio > 1.0;
        if self.on.is_some_and(|was| was != on) {
            // The movement since the last event counts at the new rate.
            self.pointer = self.last_pointer;
            self.out = self.last;
        }
        self.on = Some(on);
        let d = pos - self.pointer;
        let moved = if on { slowed(d, ratio, cx.view.units(LEVER_RAMP)) } else { d };
        let out = self.out + moved;
        if finite(out) {
            self.last = out;
            self.last_pointer = pos;
        }
        self.last
    }
}

/// `d` shortened by `ratio` over its first `ramp` font units, and one to one beyond.
fn slowed(d: Vec2, ratio: f64, ramp: f64) -> Vec2 {
    let len = d.hypot();
    if !(len > EPS && ratio.is_finite() && ratio >= 1.0) {
        return Vec2::ZERO;
    }
    let out = if len <= ramp { len / ratio } else { ramp / ratio + (len - ramp) };
    d * (out / len)
}

// ---------------------------------------------------------------------------------------------
// Tunni lines
// ---------------------------------------------------------------------------------------------

/// A cubic segment that has a Tunni point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tunni {
    pub shape: usize,
    /// The segment's points: start node, first handle, second handle, end node.
    pub nodes: [usize; 4],
    pub curve: CubicBez,
    /// Where the lines through the two handles meet, ahead of both nodes.
    pub point: Point,
}

impl Tunni {
    fn of(seg: &Segment) -> Option<Tunni> {
        let ([h1, h2], [Piece::Cubic(c)]) = (seg.handles.as_slice(), seg.pieces.as_slice()) else { return None };
        let (s, _, u1, _) = tunni_frame(*c)?;
        let point = c.p0 + u1 * s;
        usable(point).then_some(Tunni { shape: seg.shape, nodes: [seg.start, *h1, *h2, seg.end], curve: *c, point })
    }

    fn handles(&self) -> [(usize, usize); 2] {
        [(self.shape, self.nodes[1]), (self.shape, self.nodes[2])]
    }

    fn locked(&self, scene: &Scene) -> bool {
        let Some(path) = scene.path(self.shape) else { return true };
        scene.locked || path.info.locked || self.nodes.iter().any(|&i| path.nodes.get(i).is_none_or(|n| n.locked))
    }
}

/// The Tunni point of `c`, where the lines through its handles meet ahead of both nodes.
pub fn tunni_point(c: CubicBez) -> Option<Point> {
    let (s, _, u1, _) = tunni_frame(c)?;
    Some(c.p0 + u1 * s)
}

/// The distances from each node of `c` to its Tunni point, and the two handle directions.
fn tunni_frame(c: CubicBez) -> Option<(f64, f64, Vec2, Vec2)> {
    let u1 = unit(c.p1 - c.p0)?;
    let u2 = unit(c.p2 - c.p3)?;
    let chord = c.p3 - c.p0;
    let cross = u1.cross(u2);
    if cross.abs() <= EPS {
        return None;
    }
    let s = chord.cross(u2) / cross;
    let r = -u1.cross(chord) / cross;
    (s > EPS && r > EPS && s.is_finite() && r.is_finite()).then_some((s, r, u1, u2))
}

fn unit(v: Vec2) -> Option<Vec2> {
    let len = v.hypot();
    (len > EPS && len.is_finite()).then(|| v / len)
}

/// Every cubic segment of the scene's editable paths that has a Tunni point.
pub fn tunni_segments(scene: &Scene) -> Vec<Tunni> {
    let mut out = Vec::new();
    for (shape, path) in scene.paths() {
        out.extend(segments(shape, path).iter().filter_map(Tunni::of));
    }
    out
}

/// Which part of a Tunni line was pressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Line,
    Point,
}

/// The Tunni line or point under `pos`, the point first.
fn tunni_hit(cx: &Context<'_>, pos: Point) -> Option<(Tunni, Part)> {
    let tol = cx.settings.hit_tolerance;
    if !tol.is_finite() || tol < 0.0 {
        return None;
    }
    let at = cx.view.to_screen(pos);
    let mut best: Option<(f64, u8, Tunni, Part)> = None;
    for t in tunni_segments(cx.scene) {
        let point = dist(cx.view.to_screen(t.point), at);
        let line = segment_distance(at, cx.view.to_screen(t.curve.p1), cx.view.to_screen(t.curve.p2));
        for (d, rank, part) in [(point, 0, Part::Point), (line, 1, Part::Line)] {
            if d <= tol && best.is_none_or(|b| (d, rank) < (b.0, b.1)) {
                best = Some((d, rank, t, part));
            }
        }
    }
    best.map(|b| (b.2, b.3))
}

/// The distance from `p` to the straight segment from `a` to `b`.
fn segment_distance(p: Point, a: Point, b: Point) -> f64 {
    let ab = b - a;
    let len2 = ab.hypot2();
    let t = if len2 > EPS { ((p - a).dot(ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
    dist(p, a + ab * t)
}

/// What a Tunni drag changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Handles {
    /// Both handles, by the same share of their distance to the Tunni point.
    Both,
    /// One handle: 0 the first, 1 the second.
    One(usize),
    /// A Tunni point dragged without `Shift`: the drag's direction decides the handle.
    Undecided,
}

#[derive(Clone, Debug)]
struct TunniDrag {
    tunni: Tunni,
    handles: Handles,
    /// Other segments that change with it: their two handles are selected.
    others: Vec<Tunni>,
    down: Point,
    shift: bool,
    lever: Lever,
    begun: bool,
    last: Option<Value>,
}

impl TunniDrag {
    fn press(cx: &Context<'_>, ev: &PointerEvent) -> Option<TunniDrag> {
        let (tunni, part) = tunni_hit(cx, ev.pos)?;
        if tunni.locked(cx.scene) {
            return None;
        }
        let handles = match part {
            Part::Line if ev.mods.alt => {
                let (a, b) = (dist(ev.pos, tunni.curve.p1), dist(ev.pos, tunni.curve.p2));
                Handles::One(usize::from(b < a))
            }
            Part::Line => Handles::Both,
            Part::Point if ev.mods.shift => Handles::Both,
            Part::Point => Handles::Undecided,
        };
        let mut others = Vec::new();
        if handles == Handles::Both {
            let selected = |t: &Tunni| t.handles().iter().all(|h| cx.selection.points.contains(h));
            others = tunni_segments(cx.scene)
                .into_iter()
                .filter(|t| t.handles() != tunni.handles() && selected(t) && !t.locked(cx.scene))
                .take(MAX_TUNNI_SEGMENTS)
                .collect();
        }
        Some(TunniDrag { tunni, handles, others, down: ev.pos, shift: ev.mods.shift, lever: Lever::new(ev.pos), begun: false, last: None })
    }

    /// The preview for the pointer at `pos`, when it differs from the last one.
    fn update(&mut self, cx: &Context<'_>, pos: Point, mods: Modifiers) -> Option<Invocation> {
        let p = self.lever.apply(cx, pos, mods);
        let delta = p - self.down;
        if self.handles == Handles::Undecided {
            let c = self.tunni.curve;
            let (u1, u2) = (unit(c.p1 - c.p0)?, unit(c.p2 - c.p3)?);
            self.handles = Handles::One(usize::from(delta.dot(u2).abs() > delta.dot(u1).abs()));
        }
        let mut moves = Vec::new();
        match self.handles {
            Handles::Both => {
                let share = symmetric_share(self.tunni, delta)?;
                for t in std::iter::once(&self.tunni).chain(&self.others) {
                    if let Some(c) = with_added_tension(t.curve, share) {
                        moves.push(((t.shape, t.nodes[1]), t.curve.p1, c.p1));
                        moves.push(((t.shape, t.nodes[2]), t.curve.p2, c.p2));
                    }
                }
            }
            Handles::One(k) => {
                let t = self.tunni;
                let (s, r, u1, u2) = tunni_frame(t.curve)?;
                let (node, handle, u, reach, index) =
                    if k == 0 { (t.curve.p0, t.curve.p1, u1, s, t.nodes[1]) } else { (t.curve.p3, t.curve.p2, u2, r, t.nodes[2]) };
                let len = ((handle - node).hypot() + delta.dot(u)).clamp(MIN_HANDLE, (MAX_TENSION * reach).max(MIN_HANDLE));
                let to = node + u * len;
                if !usable(to) {
                    return None;
                }
                moves.push(((t.shape, index), handle, to));
            }
            Handles::Undecided => return None,
        }
        let inv = moves_invocation(cx.scene, "Tunni Line", &moves)?;
        if self.last.as_ref() == Some(&inv.params) {
            return None;
        }
        self.last = Some(inv.params.clone());
        Some(inv)
    }

    /// A click without a drag: selects the segment's handles (`Shift`: toggles them).
    fn click(&self, cx: &Context<'_>) -> Output {
        let items = self.tunni.handles().map(|(s, n)| Item::Point(s, n));
        let selection = if self.shift {
            let mut s = cx.selection.clone();
            for item in items {
                s.toggle(item);
            }
            s
        } else {
            Selection::from_items(items)
        };
        Output::with(Action::Select { selection })
    }
}

/// The share of their distance to the Tunni point that both handles gain when the Tunni line is
/// dragged by `delta`: the drag across the chord over the Tunni point's height above it.
fn symmetric_share(t: Tunni, delta: Vec2) -> Option<f64> {
    let chord = unit(t.curve.p3 - t.curve.p0)?;
    let height = chord.cross(t.point - t.curve.p0);
    if height.abs() <= EPS {
        return None;
    }
    let share = chord.cross(delta) / height;
    share.is_finite().then_some(share)
}

/// `c` with both handles moved along their own lines by `share` of their node's distance to the
/// Tunni point, each kept between [`MIN_HANDLE`] and [`MAX_TENSION`].
fn with_added_tension(c: CubicBez, share: f64) -> Option<CubicBez> {
    let (s, r, u1, u2) = tunni_frame(c)?;
    let len = |handle: Vec2, reach: f64| (handle.hypot() + share * reach).clamp(MIN_HANDLE, (MAX_TENSION * reach).max(MIN_HANDLE));
    let out = CubicBez::new(c.p0, c.p0 + u1 * len(c.p1 - c.p0, s), c.p3 + u2 * len(c.p2 - c.p3, r), c.p3);
    (usable(out.p1) && usable(out.p2)).then_some(out)
}

// ---------------------------------------------------------------------------------------------
// Sliding a node along its curve
// ---------------------------------------------------------------------------------------------

/// A segment seen from one of its end nodes, running away from it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Curve {
    Line(Point, Point),
    Cubic(CubicBez),
}

impl Curve {
    fn eval(self, t: f64) -> Point {
        match self {
            Curve::Line(a, b) => a + (b - a) * t,
            Curve::Cubic(c) => c.eval(t),
        }
    }
}

/// One side of the sliding node: the segment running away from it and its handles, near first.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Side {
    curve: Curve,
    handles: Option<[usize; 2]>,
}

impl Side {
    /// `seg` seen from its node `node`, when it is a line or a cubic.
    fn of(seg: &Segment, node: usize) -> Option<Side> {
        let forward = seg.start == node;
        match (seg.handles.as_slice(), seg.pieces.as_slice()) {
            ([], [Piece::Line(l)]) => Some(Side { curve: if forward { Curve::Line(l.p0, l.p1) } else { Curve::Line(l.p1, l.p0) }, handles: None }),
            ([h1, h2], [Piece::Cubic(c)]) => Some(if forward {
                Side { curve: Curve::Cubic(*c), handles: Some([*h1, *h2]) }
            } else {
                Side { curve: Curve::Cubic(CubicBez::new(c.p3, c.p2, c.p1, c.p0)), handles: Some([*h2, *h1]) }
            }),
            _ => None,
        }
    }
}

/// Where each point goes when the node `node` of `path` (the shape at `shape`) slides as near as
/// it can to `target`: `(address, from, to)` for every point that moves. `None` when the node
/// cannot slide.
pub fn slide_moves(shape: usize, path: &Path, node: usize, target: Point) -> Option<Vec<((usize, usize), Point, Point)>> {
    let n = path.nodes.get(node)?;
    if !n.on_curve() || !finite(target) {
        return None;
    }
    let segs = segments(shape, path);
    let next = segs.iter().find(|s| s.start == node && s.end != node);
    let prev = segs.iter().find(|s| s.end == node && s.start != node);
    let sides = [next.and_then(|s| Side::of(s, node)), prev.and_then(|s| Side::of(s, node))];
    // An open end may slide past the end only where nothing joins it.
    let joined = [prev.is_some(), next.is_some()];
    let mut best: Option<(f64, usize, f64)> = None;
    for (k, side) in sides.iter().enumerate() {
        let Some(side) = side else { continue };
        let lo = if joined.get(k).copied().unwrap_or(true) { 0.0 } else { -SLIDE_EXTEND };
        let (d, t) = nearest_param(side.curve, target, lo, 1.0 - SLIDE_MARGIN);
        if d.is_finite() && best.is_none_or(|b| d < b.0) {
            best = Some((d, k, t));
        }
    }
    let (_, k, t) = best?;
    let into = sides.get(k).copied().flatten()?;
    let other = sides.get(1 - k).copied().flatten();
    let from = n.point();
    let to = into.curve.eval(t);
    let mut moves = vec![((shape, node), from, to)];
    if let (Curve::Cubic(c), Some([near, far])) = (into.curve, into.handles) {
        let rest = c.subsegment(t..1.0);
        moves.push(((shape, near), c.p1, rest.p1));
        moves.push(((shape, far), c.p2, rest.p2));
    }
    if let Some(Side { curve: Curve::Cubic(o), handles: Some([near, far]) }) = other {
        // The other segment and the slid-over piece become one curve. The handle lengths at the
        // node give where the piece would have been cut from that curve; the merge is exact when
        // the two segments were cut from one cubic.
        let merged = match into.curve {
            Curve::Cubic(c) if n.smooth => merge(o, c.subsegment(0.0..t)),
            _ => None,
        };
        match merged {
            Some((on, of)) => {
                moves.push(((shape, near), o.p1, on));
                moves.push(((shape, far), o.p2, of));
            }
            None => moves.push(((shape, near), o.p1, o.p1 + (to - from))),
        }
    }
    moves.iter().all(|m| usable(m.2)).then_some(moves)
}

/// The two handles, near the node first, of the curve that runs from the far end of `o` through
/// the node (where `o` and `piece` start) to the end of `piece`.
fn merge(o: CubicBez, piece: CubicBez) -> Option<(Point, Point)> {
    let a = (o.p1 - o.p0).hypot();
    let b = (piece.p1 - piece.p0).hypot();
    let u = a / (a + b);
    if !(u.is_finite() && u > SAME && u < 1.0 - SAME) {
        return None;
    }
    let far = o.p3 + (o.p2 - o.p3) / u;
    let near = piece.p3 + (piece.p2 - piece.p3) / (1.0 - u);
    (usable(far) && usable(near)).then_some((near, far))
}

/// The parameter in `lo..=hi` of the point of `curve` nearest `p`, and its distance. Sampled and
/// then refined, so the work is fixed whatever the input.
fn nearest_param(curve: Curve, p: Point, lo: f64, hi: f64) -> (f64, f64) {
    let at = |t: f64| dist(curve.eval(t), p);
    let step = (hi - lo) / SLIDE_SAMPLES as f64;
    let mut best = (f64::INFINITY, lo);
    for i in 0..=SLIDE_SAMPLES {
        let t = lo + step * i as f64;
        let d = at(t);
        if d < best.0 {
            best = (d, t);
        }
    }
    let (mut a, mut b) = ((best.1 - step).max(lo), (best.1 + step).min(hi));
    for _ in 0..SLIDE_REFINE {
        let m1 = a + (b - a) / 3.0;
        let m2 = b - (b - a) / 3.0;
        if at(m1) < at(m2) {
            b = m2;
        } else {
            a = m1;
        }
    }
    let t = (a + b) / 2.0;
    let d = at(t);
    if d < best.0 { (d, t) } else { best }
}

#[derive(Clone, Debug)]
struct SlideDrag {
    shape: usize,
    node: usize,
    /// The path as it was at the press: previews change what the scene shows.
    path: Path,
    down: Point,
    lever: Lever,
    begun: bool,
    last: Option<Value>,
}

impl SlideDrag {
    fn update(&mut self, cx: &Context<'_>, pos: Point, mods: Modifiers) -> Option<Invocation> {
        let p = self.lever.apply(cx, pos, mods);
        let from = self.path.nodes.get(self.node)?.point();
        let target = from + (p - self.down);
        let moves = slide_moves(self.shape, &self.path, self.node, target)?;
        let inv = moves_invocation(cx.scene, "Slide", &moves)?;
        if self.last.as_ref() == Some(&inv.params) {
            return None;
        }
        self.last = Some(inv.params.clone());
        Some(inv)
    }
}

// ---------------------------------------------------------------------------------------------
// The drags, routed by the Select tool
// ---------------------------------------------------------------------------------------------

/// A drag aid's gesture in progress.
#[derive(Clone, Debug)]
pub struct AidDrag(Gesture);

#[derive(Clone, Debug)]
enum Gesture {
    Tunni(Box<TunniDrag>),
    Slide(Box<SlideDrag>),
}

impl AidDrag {
    fn begun(&self) -> bool {
        match &self.0 {
            Gesture::Tunni(t) => t.begun,
            Gesture::Slide(s) => s.begun,
        }
    }

    fn label(&self) -> &'static str {
        match &self.0 {
            Gesture::Tunni(_) => "Tunni Line",
            Gesture::Slide(_) => "Slide",
        }
    }

    fn down(&self) -> Point {
        match &self.0 {
            Gesture::Tunni(t) => t.down,
            Gesture::Slide(s) => s.down,
        }
    }

    fn update(&mut self, cx: &Context<'_>, pos: Point, mods: Modifiers) -> Option<Invocation> {
        match &mut self.0 {
            Gesture::Tunni(t) => t.update(cx, pos, mods),
            Gesture::Slide(s) => s.update(cx, pos, mods),
        }
    }

    fn set_begun(&mut self) {
        match &mut self.0 {
            Gesture::Tunni(t) => t.begun = true,
            Gesture::Slide(s) => s.begun = true,
        }
    }

    /// A press that never became a drag.
    fn click(&self, cx: &Context<'_>) -> Output {
        match &self.0 {
            Gesture::Tunni(t) => t.click(cx),
            Gesture::Slide(s) => {
                // As the Select tool's Shift-click: the node toggles in the selection.
                let mut selection = cx.selection.clone();
                selection.toggle(Item::Point(s.shape, s.node));
                Output::with(Action::Select { selection })
            }
        }
    }
}

/// A press that starts a drag aid, if `ev` is one.
fn press(cx: &Context<'_>, ev: &PointerEvent) -> Option<AidDrag> {
    let scene = cx.scene;
    let hit = hit_test(scene, &cx.view, ev.pos, cx.settings.hit_tolerance);
    if ev.mods.shift
        && ev.mods.alt
        && let Some(h) = &hit
        && let Hit::Node { shape, node } = h.hit
        && !h.locked
        && let Some(path) = scene.path(shape)
    {
        let slide = SlideDrag { shape, node, path: path.clone(), down: ev.pos, lever: Lever::new(ev.pos), begun: false, last: None };
        return Some(AidDrag(Gesture::Slide(Box::new(slide))));
    }
    if !cx.settings.tunni_lines || matches!(hit.map(|h| h.hit), Some(Hit::Node { .. } | Hit::Handle { .. } | Hit::Anchor { .. })) {
        return None;
    }
    TunniDrag::press(cx, ev).map(|t| AidDrag(Gesture::Tunni(Box::new(t))))
}

/// Gives a pointer event to the drag aids first. `None` leaves the event to the tool; a press a
/// drag aid takes, and every later event of its gesture, give its output.
pub fn route(state: &mut Option<AidDrag>, cx: &Context<'_>, ev: &PointerEvent) -> Option<Output> {
    if !finite(ev.pos) {
        return None;
    }
    match ev.kind {
        PointerKind::Down => {
            let mut out = cancel(state);
            *state = press(cx, ev);
            if state.is_none() && out.actions.is_empty() {
                return None;
            }
            out.handled = true;
            Some(out)
        }
        PointerKind::Drag => {
            let aid = state.as_mut()?;
            let mut out = Output::handled();
            if !aid.begun() {
                let moved = dist(cx.view.to_screen(aid.down()), cx.view.to_screen(ev.pos));
                if moved < cx.settings.drag_threshold {
                    return Some(out);
                }
                aid.set_begun();
                out.push(Action::Begin { label: aid.label().into() });
            }
            if let Some(invocation) = aid.update(cx, ev.pos, ev.mods) {
                out.push(Action::Preview { invocation });
            }
            Some(out)
        }
        PointerKind::Up => {
            let mut aid = state.take()?;
            if !aid.begun() {
                return Some(aid.click(cx));
            }
            let mut out = Output::handled();
            if let Some(invocation) = aid.update(cx, ev.pos, ev.mods) {
                out.push(Action::Preview { invocation });
            }
            out.push(Action::Commit);
            Some(out)
        }
        PointerKind::Move | PointerKind::DoubleClick => {
            if state.is_some() {
                return Some(Output::handled());
            }
            None
        }
    }
}

/// Abandons a drag aid's gesture in progress.
pub fn cancel(state: &mut Option<AidDrag>) -> Output {
    match state.take() {
        Some(aid) if aid.begun() => Output::with(Action::Cancel),
        _ => Output::default(),
    }
}

/// One `selection.move` step in `pointsOnly` mode for each group of points that move by the same
/// offset, as a `command.batch` labelled `label`.
fn moves_invocation(scene: &Scene, label: &str, moves: &[((usize, usize), Point, Point)]) -> Option<Invocation> {
    let offsets: Vec<((usize, usize), Vec2)> = moves.iter().map(|&(at, from, to)| (at, to - from)).collect();
    if offsets.is_empty() || offsets.iter().any(|(_, d)| !(d.x.is_finite() && d.y.is_finite())) {
        return None;
    }
    Some(Invocation::new("command.batch", json!({ "label": label, "steps": point_steps(scene, &offsets) })))
}

/// `selection.move` steps in `pointsOnly` mode, one per distinct offset.
fn point_steps(scene: &Scene, offsets: &[((usize, usize), Vec2)]) -> Vec<Value> {
    let mut groups: BTreeMap<(u64, u64), (Vec2, Vec<Value>)> = BTreeMap::new();
    for &((s, n), d) in offsets {
        groups.entry((d.x.to_bits(), d.y.to_bits())).or_insert_with(|| (d, Vec::new())).1.push(json!([s, n]));
    }
    groups
        .into_values()
        .map(|(d, points)| {
            let mut params: Map<String, Value> = scene.target();
            params.insert("points".into(), Value::Array(points));
            params.insert("dx".into(), json!(d.x));
            params.insert("dy".into(), json!(d.y));
            params.insert("mode".into(), json!("pointsOnly"));
            json!({ "command": "selection.move", "params": Value::Object(params) })
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Followers: Power Nudge and servant nodes
// ---------------------------------------------------------------------------------------------

/// The axes a follower moves on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
    Both,
}

impl Axis {
    /// The axis a stored servant link names (`fc_keys::SERVANT`).
    pub fn parse(name: &str) -> Option<Axis> {
        match name {
            "x" => Some(Axis::X),
            "y" => Some(Axis::Y),
            "both" => Some(Axis::Both),
            _ => None,
        }
    }
}

/// The axis `node` is declared a servant on, if it is one.
pub fn declared_servant(node: &Node) -> Option<Axis> {
    let link = node.info.user_data.get(fc_keys::SERVANT)?.as_dict()?;
    Axis::parse(link.get("axis")?.as_str()?)
}

/// How a walk from a moved node ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum End {
    /// At a node that stays where it is.
    Fixed(usize),
    /// At another moved node, or at an open end: the followers move by the whole drag.
    Whole,
    /// Back at the start, or too long: no follower moves.
    Nothing,
}

/// The points that follow a move, each with its share of the drag on x and y. Taken from the
/// scene as it is when the drag starts: previews move what the scene shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Followers(Vec<((usize, usize), f64, f64)>);

impl Followers {
    /// The points that follow when `selection` moves: declared servants always, and with `power`
    /// the smooth nodes that are not extremes. Selected points are left out; they move with the
    /// selection.
    pub fn new(scene: &Scene, selection: &Selection, power: bool) -> Self {
        let mut out: BTreeMap<(usize, usize), (f64, f64)> = BTreeMap::new();
        if selection.points.is_empty() || scene.locked {
            return Self::default();
        }
        let shapes: std::collections::BTreeSet<usize> = selection.points.iter().map(|p| p.0).collect();
        for shape in shapes {
            let Some(path) = scene.path(shape) else { continue };
            if !path.info.locked {
                path_followers(shape, path, selection, power, &mut out);
            }
            if out.len() >= MAX_FOLLOWERS {
                break;
            }
        }
        Self(out.into_iter().take(MAX_FOLLOWERS).map(|(at, (wx, wy))| (at, wx, wy)).collect())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Each follower's offset when the selection moves by `(dx, dy)`.
    pub fn offsets(&self, dx: f64, dy: f64) -> Vec<((usize, usize), Vec2)> {
        if !(dx.is_finite() && dy.is_finite()) {
            return Vec::new();
        }
        self.0.iter().map(|&(at, wx, wy)| (at, Vec2::new(dx * wx, dy * wy))).collect()
    }

    /// `inv`, a move of the selection by `(dx, dy)`, together with the moves of the followers as
    /// one `command.batch`; `inv` alone when nothing follows.
    pub fn attach(&self, scene: &Scene, inv: Invocation, dx: f64, dy: f64) -> Invocation {
        let list = self.offsets(dx, dy);
        if list.is_empty() {
            return inv;
        }
        let mut steps = vec![json!({ "command": inv.id, "params": inv.params })];
        steps.extend(point_steps(scene, &list));
        Invocation::new("command.batch", json!({ "label": "Move", "steps": steps }))
    }
}

fn path_followers(shape: usize, path: &Path, selection: &Selection, power: bool, out: &mut BTreeMap<(usize, usize), (f64, f64)>) {
    let segs = segments(shape, path);
    let Some(first) = segs.first() else { return };
    let ons: Vec<usize> =
        if path.closed { segs.iter().map(|s| s.start).collect() } else { std::iter::once(first.start).chain(segs.iter().map(|s| s.end)).collect() };
    // Each handle belongs to the nearer on-curve node of its segment.
    let mut owned: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for s in &segs {
        let k = s.handles.len();
        for (i, &h) in s.handles.iter().enumerate() {
            owned.entry(if 2 * i < k { s.start } else { s.end }).or_default().push(h);
        }
    }
    let selected = |i: usize| selection.points.contains(&(shape, i));
    let point = |i: usize| path.nodes.get(i).map(Node::point);
    let extreme = |i: usize, p: Point| {
        owned.get(&i).is_some_and(|hs| hs.iter().filter_map(|&h| point(h)).any(|h| (h.x - p.x).abs() < SAME || (h.y - p.y).abs() < SAME))
    };
    let count = ons.len();
    for (k, &a) in ons.iter().enumerate() {
        if !selected(a) {
            continue;
        }
        let Some(a_point) = point(a) else { continue };
        for forward in [false, true] {
            let mut run: Vec<(usize, Axis)> = Vec::new();
            let mut end = End::Nothing;
            let mut j = k;
            for _ in 0..MAX_SERVANT_RUN.min(count) {
                let next = match (forward, path.closed) {
                    (true, _) if j + 1 < count => Some(j + 1),
                    (true, true) => Some(0),
                    (false, _) if j > 0 => Some(j - 1),
                    (false, true) => count.checked_sub(1),
                    _ => None,
                };
                let Some(nj) = next else {
                    end = End::Whole;
                    break;
                };
                j = nj;
                if j == k {
                    break;
                }
                let Some(&idx) = ons.get(j) else { break };
                if selected(idx) {
                    end = End::Whole;
                    break;
                }
                let Some(node) = path.nodes.get(idx) else { break };
                let axis = if node.locked {
                    None
                } else {
                    declared_servant(node).or_else(|| (power && node.smooth && !extreme(idx, node.point())).then_some(Axis::Both))
                };
                match axis {
                    Some(axis) => run.push((idx, axis)),
                    None => {
                        end = End::Fixed(idx);
                        break;
                    }
                }
            }
            if run.is_empty() || end == End::Nothing {
                continue;
            }
            let weights = match end {
                End::Fixed(f) => match point(f) {
                    Some(f_point) => run_weights(f_point, a_point, &run, &point),
                    None => continue,
                },
                _ => vec![(1.0, 1.0); run.len()],
            };
            for (&(idx, axis), (wx, wy)) in run.iter().zip(weights) {
                let weight = match axis {
                    Axis::X => (wx, 0.0),
                    Axis::Y => (0.0, wy),
                    Axis::Both => (wx, wy),
                };
                let handles = owned.get(&idx).map(Vec::as_slice).unwrap_or_default();
                for &p in std::iter::once(&idx).chain(handles) {
                    if !selected(p) && path.nodes.get(p).is_some_and(|n| !n.locked) && out.len() < MAX_FOLLOWERS {
                        out.entry((shape, p)).or_insert(weight);
                    }
                }
            }
        }
    }
}

/// Each follower's share of the drag on x and y: its position between the fixed node `f` and the
/// moved node `a`, or its share of the length along the contour where the two share that
/// coordinate. `run` lists the followers from `a` outwards.
fn run_weights(f: Point, a: Point, run: &[(usize, Axis)], point: &impl Fn(usize) -> Option<Point>) -> Vec<(f64, f64)> {
    // Lengths along the polygon from the fixed node through the followers to the moved node.
    let chain: Vec<Point> = std::iter::once(f).chain(run.iter().rev().filter_map(|&(i, _)| point(i))).chain(std::iter::once(a)).collect();
    let mut along = Vec::with_capacity(chain.len());
    let mut total = 0.0;
    let mut prev = f;
    for &p in &chain {
        total += dist(prev, p);
        along.push(total);
        prev = p;
    }
    let share = |value: f64, from: f64, to: f64, k: usize| {
        let w = if (to - from).abs() > SAME {
            (value - from) / (to - from)
        } else if total > SAME {
            along.get(k).copied().unwrap_or(total) / total
        } else {
            1.0
        };
        if w.is_finite() { w.clamp(0.0, 1.0) } else { 0.0 }
    };
    let n = run.len();
    run.iter()
        .enumerate()
        .map(|(i, &(idx, _))| {
            // Position of this follower in `chain` (which runs from the fixed end).
            let k = n - i;
            let p = point(idx).unwrap_or(a);
            (share(p.x, f.x, a.x, k), share(p.y, f.y, a.y, k))
        })
        .collect()
}

#[cfg(test)]
#[path = "drag_aids_tests.rs"]
mod tests;
