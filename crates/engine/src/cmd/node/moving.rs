//! `selection.move`: moving and duplicating what a target object names (decision 0004 and
//! `docs/spec/03a` § 7, § 11).
//!
//! | Mode | Nodes | Handles of moved nodes | Selected handles alone |
//! |---|---|---|---|
//! | `normal` | move | move with them | move; the opposite handle of a smooth node turns to stay in line (along a straight side, the handle slides on its line) |
//! | `pointsOnly` | move | stay | keep their angle: only the length changes |
//! | `nudge` | move | the handles of each segment with one moved end are scaled per axis between the fixed end and the moved one | move |
//! | `mirror` | move | move with them | move; the opposite handle of a smooth node takes the same length at the opposite angle (along a straight side, as `normal`) |
//!
//! In every mode but `pointsOnly`, the handle of a smooth node with a line on its other side turns
//! about the node onto that line and keeps its length, when the node, the handle and the line's
//! other end did not all move by the same offset ([`super::ops::keep_tangents`]).
//!
//! The only handle of a quadratic segment belongs to both end nodes, and both smooth ends stay
//! smooth ([`keep_both_smooth`]). Where a smooth node lies between two such handles, the node
//! slides onto the line between them, so a move does not run on round the path.
//!
//! `duplicate` copies the corner, cap and segment components and the brushes that sit on what it
//! copies: on a whole path all of them, on part of one those whose node keeps the segments they
//! need (a corner both, a cap the one after it, the others the one before it).
//!
//! Components follow the rule of `component.move`: a locked one stays, one aligned automatically
//! stays, and one aligned only horizontally moves only up and down. When the targets name nothing
//! else that moves, the command fails with a message naming the components.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use fontcraft_geom::{Point, Vec2, round_point, round_to_grid};
use fontcraft_model::attach::{is_attachment, is_cap};
use fontcraft_model::corner::is_corner;
use fontcraft_model::guide_follow::{place_x, placed_x};
use fontcraft_model::{Component, Font, Guide, Hint, LayerParts, LayerPartsMut, LayerSide, MAX_LAYER_HINTS, Node, NodeKind, Path, Shape, Transform};

use super::ops::{EPS, handle_owner, keep_tangents, next, on, prev};
use crate::params::{GuideRef, GuideScope, ResolvedTargets, Targets};
use crate::{EngineError, Result};

/// How far off the line of its node a handle may lie and still count as in line, in font units.
const IN_LINE: f64 = 0.25;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Normal,
    PointsOnly,
    Nudge,
    Mirror,
}

/// The target's points grouped by shape, with each segment's two end nodes added, leaving out
/// shapes that are moved whole.
pub(super) fn points_by_shape(t: &Targets, parts: LayerParts) -> BTreeMap<usize, BTreeSet<usize>> {
    let mut out = super::group(&t.points);
    for (s, ends) in super::group(&t.segments) {
        let Some(path) = parts.path(s) else { continue };
        for seg in path.segments().iter().filter(|g| ends.contains(&g.end)) {
            out.entry(s).or_default().extend([seg.start, seg.end]);
        }
    }
    out.retain(|s, _| !t.shapes.contains(s));
    out
}

fn at(v: &[Point], i: usize) -> Point {
    v.get(i).copied().unwrap_or_default()
}

fn mark(new: &mut [Point], moved: &mut [bool], i: usize, by: Vec2) {
    if let (Some(p), Some(m)) = (new.get_mut(i), moved.get_mut(i))
        && !*m
    {
        *p += by;
        *m = true;
    }
}

/// Moves the nodes `selected` of `path` by `d` as `mode` says.
pub(super) fn move_path(path: &mut Path, selected: &BTreeSet<usize>, d: Vec2, mode: Mode, step: f64) {
    let old: Vec<Point> = path.nodes.iter().map(Node::point).collect();
    let mut new = old.clone();
    let mut moved = vec![false; old.len()];
    let path_ref: &Path = path;
    match mode {
        Mode::Normal | Mode::Mirror => {
            for &i in selected {
                mark(&mut new, &mut moved, i, d);
            }
            for &i in selected.iter().filter(|&&i| on(path_ref, i)) {
                for j in [prev(path_ref, i), next(path_ref, i)].into_iter().flatten() {
                    if !on(path_ref, j) {
                        mark(&mut new, &mut moved, j, d);
                    }
                }
            }
            let is_moved = |moved: &[bool], i: usize| moved.get(i).copied().unwrap_or(false);
            let off_curve: Vec<usize> = (0..old.len()).filter(|&j| !on(path_ref, j)).collect();
            let handles: Vec<usize> = off_curve.iter().copied().filter(|&j| is_moved(&moved, j) && only_handle(path_ref, j).is_none()).collect();
            for j in handles {
                if mode == Mode::Mirror {
                    mirror(path_ref, j, &old, &mut new, &mut moved);
                } else {
                    keep_smooth(path_ref, j, &old, &mut new, &mut moved);
                }
            }
            for j in off_curve {
                let Some(ends) = only_handle(path_ref, j) else { continue };
                let near = ends.iter().flat_map(|&(k, o)| [Some(k), o]).flatten().chain([j]);
                if near.into_iter().any(|i| is_moved(&moved, i)) {
                    keep_both_smooth(path_ref, j, ends, selected, &old, &mut new, &mut moved);
                }
            }
        }
        Mode::PointsOnly => {
            for &i in selected {
                let mut by = d;
                if let Some((k, _)) = handle_owner(path_ref, i)
                    && !selected.contains(&k)
                {
                    let v = at(&old, i) - at(&old, k);
                    let len = v.hypot();
                    if len > EPS {
                        let u = v / len;
                        by = u * d.dot(u);
                    }
                }
                mark(&mut new, &mut moved, i, by);
            }
        }
        Mode::Nudge => {
            for &i in selected {
                mark(&mut new, &mut moved, i, d);
            }
            for seg in path_ref.segments() {
                let (ms, me) = (selected.contains(&seg.start), selected.contains(&seg.end));
                if ms && me {
                    for &h in &seg.handles {
                        mark(&mut new, &mut moved, h, d);
                    }
                    continue;
                }
                if !(ms || me) {
                    continue;
                }
                let (m, f) = if ms { (seg.start, seg.end) } else { (seg.end, seg.start) };
                let (pm, pf) = (at(&old, m), at(&old, f));
                for &h in seg.handles.iter().filter(|h| !selected.contains(*h)) {
                    let attached = prev(path_ref, h) == Some(m) || next(path_ref, h) == Some(m);
                    let ph = at(&old, h);
                    let axis = |h: f64, m: f64, f: f64, d: f64| {
                        let span = m - f;
                        if span.abs() > EPS {
                            f + (h - f) * (span + d) / span
                        } else if attached {
                            h + d
                        } else {
                            h
                        }
                    };
                    let to = Point::new(axis(ph.x, pm.x, pf.x, d.x), axis(ph.y, pm.y, pf.y, d.y));
                    if let (Some(p), Some(mv)) = (new.get_mut(h), moved.get_mut(h)) {
                        *p = to;
                        *mv = true;
                    }
                }
            }
        }
    }
    for ((node, p), m) in path.nodes.iter_mut().zip(&new).zip(&moved) {
        if *m {
            let r = round_point(*p, step);
            (node.x, node.y) = (r.x, r.y);
        }
    }
    if mode != Mode::PointsOnly {
        keep_tangents(path, &old, step, |_| false);
    }
}

/// After handle `j` moved: when its node is smooth and stays, the handle on the other side turns
/// to stay in line with it, keeping its length; when that side is straight, `j` slides along the
/// line instead.
fn keep_smooth(path: &Path, j: usize, old: &[Point], new: &mut [Point], moved: &mut [bool]) {
    let Some((k, Some(o))) = handle_owner(path, j) else { return };
    let is_moved = |i: usize| moved.get(i).copied().unwrap_or(false);
    if is_moved(k) || is_moved(o) || !path.nodes.get(k).is_some_and(|n| n.smooth) {
        return;
    }
    let pk = at(old, k);
    if on(path, o) {
        let u = pk - at(old, o);
        let len = u.hypot();
        if len <= EPS {
            return;
        }
        let u = u / len;
        if let Some(p) = new.get_mut(j) {
            *p = pk + u * (*p - pk).dot(u).max(0.0);
        }
    } else {
        turn(path, j, k, o, old, new, moved);
    }
}

/// Keeps the smooth node `k`, between the handles `j`, which moved, and `o`, in line with them:
/// `o` turns about the node, keeping its length; where `o` is the only handle of its segment and
/// the segment's other end is smooth, the node slides onto the line between the two instead,
/// dividing it as before.
fn turn(path: &Path, j: usize, k: usize, o: usize, old: &[Point], new: &mut [Point], moved: &mut [bool]) {
    let smooth = |i: usize| path.nodes.get(i).is_some_and(|n| n.smooth);
    let shared = only_handle(path, o).is_some_and(|ends| ends.iter().any(|&(e, _)| e != k && smooth(e)));
    let (at_j, at_o) = (at(new, j), at(new, o));
    if shared {
        if moved.get(k).copied().unwrap_or(true) {
            return;
        }
        let (a, b) = ((at(old, k) - at(old, j)).hypot(), (at(old, k) - at(old, o)).hypot());
        if a + b > EPS
            && let (Some(p), Some(m)) = (new.get_mut(k), moved.get_mut(k))
        {
            *p = at_j + (at_o - at_j) * (a / (a + b));
            *m = true;
        }
        return;
    }
    let pk = at(new, k);
    let len_o = (at_o - pk).hypot();
    let dir = pk - at_j;
    let l = dir.hypot();
    if l <= EPS {
        return;
    }
    if let (Some(p), Some(m)) = (new.get_mut(o), moved.get_mut(o)) {
        *p = pk + dir * (len_o / l);
        *m = true;
    }
}

/// An end of a segment: the on-curve node, and its neighbour on the other side (a handle, or the
/// other end of a straight segment).
type End = (usize, Option<usize>);

/// The two ends of the quadratic segment that `j` is the only handle of.
fn only_handle(path: &Path, j: usize) -> Option<[End; 2]> {
    if on(path, j) {
        return None;
    }
    let (a, b) = (prev(path, j)?, next(path, j)?);
    let ends = a != b && on(path, a) && on(path, b) && path.nodes.get(b).is_some_and(|n| n.kind == NodeKind::QCurve);
    ends.then(|| [(a, prev(path, a)), (b, next(path, b))])
}

/// After a move near `j`, the only handle of a quadratic segment, whose ends are `ends`: a smooth
/// end next to a line keeps `j` on that line (two such ends put it where their lines cross), and
/// at a smooth end between two handles the other handle follows as in [`turn`]. An end whose
/// node, handle and neighbour all moved alike, and a handle or node named by the move, stay.
fn keep_both_smooth(path: &Path, j: usize, ends: [End; 2], selected: &BTreeSet<usize>, old: &[Point], new: &mut [Point], moved: &mut [bool]) {
    const SAME: f64 = 1e-6;
    let offset = |new: &[Point], i: usize| at(new, i) - at(old, i);
    let smooth = |k: usize| path.nodes.get(k).is_some_and(|n| n.on_curve() && n.smooth);
    let bent = |new: &[Point], k: usize, o: usize| {
        let d = offset(new, j);
        (d - offset(new, k)).hypot() > SAME || (d - offset(new, o)).hypot() > SAME
    };
    let (straight, curved): (Vec<_>, Vec<_>) =
        ends.iter().filter_map(|&(k, o)| Some((k, o?))).filter(|&(k, _)| smooth(k)).partition(|&(_, o)| on(path, o));
    // The straight sides: the end's place and the direction its handle points in.
    let line = |&(k, o): &(usize, usize)| {
        let u = at(new, k) - at(new, o);
        let len = u.hypot();
        (len > EPS).then(|| (at(new, k), u / len))
    };
    let lines: Vec<(Point, Vec2)> =
        if straight.iter().any(|&(k, o)| bent(new, k, o)) { straight.iter().filter_map(line).collect() } else { Vec::new() };
    let place = match *lines.as_slice() {
        [(p, u)] => Some(p + u * (at(new, j) - p).dot(u).max(0.0)),
        [(p, u), (q, v)] => {
            // p + s·u = q + r·v, ahead of both ends.
            let cross = u.cross(v);
            let (s, r) = ((q - p).cross(v) / cross, (q - p).cross(u) / cross);
            let ahead = cross.abs() > EPS && s.is_finite() && r.is_finite() && s > 0.0 && r > 0.0;
            Some(if ahead { p + u * s } else { at(old, j) })
        }
        _ => None,
    };
    if let Some(to) = place
        && let (Some(p), Some(m)) = (new.get_mut(j), moved.get_mut(j))
    {
        *p = to;
        *m = true;
    }
    for (k, o) in curved {
        if selected.contains(&o) || !bent(new, k, o) {
            continue;
        }
        // In line already, up to rounding.
        let (a, b) = (at(new, k) - at(new, j), at(new, o) - at(new, k));
        let chord = a.hypot();
        if chord > EPS && a.dot(b) > 0.0 && (a.cross(b) / chord).abs() < IN_LINE {
            continue;
        }
        turn(path, j, k, o, old, new, moved);
    }
}

/// After handle `j` moved: when its node is smooth and stays, the opposite handle becomes the
/// mirror image of `j` through the node. A straight other side cannot turn, and the only handle of
/// a segment belongs to another node too, so there `j` behaves as in [`keep_smooth`]. A handle moved onto its node has no direction to mirror: the opposite
/// one stays.
fn mirror(path: &Path, j: usize, old: &[Point], new: &mut [Point], moved: &mut [bool]) {
    let Some((k, Some(o))) = handle_owner(path, j) else { return };
    let is_moved = |i: usize| moved.get(i).copied().unwrap_or(false);
    if is_moved(k) || is_moved(o) || !path.nodes.get(k).is_some_and(|n| n.smooth) {
        return;
    }
    if on(path, o) || only_handle(path, o).is_some() {
        keep_smooth(path, j, old, new, moved);
        return;
    }
    let pk = at(old, k);
    let arm = at(new, j) - pk;
    if arm.hypot() <= EPS {
        return;
    }
    let to = pk - arm;
    if let (Some(p), Some(m)) = (new.get_mut(o), moved.get_mut(o)) {
        *p = to;
        *m = true;
    }
}

fn translate_path(path: &mut Path, d: Vec2, step: f64) {
    for n in &mut path.nodes {
        let r = round_point(n.point() + d, step);
        (n.x, n.y) = (r.x, r.y);
    }
}

fn translate_component(c: &mut Component, d: Vec2, step: f64) {
    match &mut c.transform {
        Transform::Decomposed { pos, .. } => {
            pos.0 = round_to_grid(pos.0 + d.x, step);
            pos.1 = round_to_grid(pos.1 + d.y, step);
        }
        Transform::Matrix(m) => {
            if let Some(x) = m.get_mut(4) {
                *x = round_to_grid(*x + d.x, step);
            }
            if let Some(y) = m.get_mut(5) {
                *y = round_to_grid(*y + d.y, step);
            }
        }
    }
}

fn translate_shape(shape: &mut Shape, d: Vec2, step: f64) {
    match shape {
        Shape::Path(p) => translate_path(p, d, step),
        Shape::Component(c) => translate_component(c, d, step),
        Shape::Image(_) | Shape::Raw(_) => {}
    }
}

fn shift(x: &mut f64, y: &mut f64, d: Vec2, step: f64) {
    *x = round_to_grid(*x + d.x, step);
    *y = round_to_grid(*y + d.y, step);
}

/// Moves the place of a guide in a layer `width` wide.
fn shift_guide(g: &mut Guide, width: f64, d: Vec2, step: f64) {
    let mut x = placed_x(g, width);
    shift(&mut x, &mut g.y, d, step);
    place_x(g, x, width);
}

/// Why a component the targets name does not move freely.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Held {
    Locked,
    Aligned,
    /// Aligned horizontally: it moves only up and down.
    AlignedHorizontally,
}

impl Held {
    /// The offset the component takes of `d`.
    fn offset(self, d: Vec2) -> Vec2 {
        match self {
            Held::Locked | Held::Aligned => Vec2::ZERO,
            Held::AlignedHorizontally => Vec2::new(0.0, d.y),
        }
    }

    fn message(self, shape: usize) -> String {
        match self {
            Held::Locked => format!("component {shape} is locked: unlock it to move it"),
            Held::Aligned => format!("component {shape} is aligned automatically: disable its automatic alignment to move it"),
            Held::AlignedHorizontally => format!("component {shape} is aligned horizontally: it moves only up and down"),
        }
    }
}

/// The components among the target's shapes that do not move freely, by shape index. Alignment
/// applies to the foreground only; a background component is held only by its lock.
fn held_components(font: &Font, t: &Targets, r: &ResolvedTargets<'_>) -> BTreeMap<usize, Held> {
    let mut out = BTreeMap::new();
    for &s in &t.shapes {
        let Some(Shape::Component(c)) = r.parts.shapes.get(s) else { continue };
        let held = if c.info.locked {
            Some(Held::Locked)
        } else if t.side == LayerSide::Foreground {
            match crate::cmd::component::free_axes(font, r.layer, s) {
                (true, true) => None,
                (false, true) => Some(Held::AlignedHorizontally),
                _ => Some(Held::Aligned),
            }
        } else {
            None
        };
        if let Some(h) = held {
            out.insert(s, h);
        }
    }
    out
}

/// Fails when `d` moves something but none of what the targets name moves because every named
/// thing is a held component; the message names each one.
fn check_something_moves(t: &Targets, points: &BTreeMap<usize, BTreeSet<usize>>, held: &BTreeMap<usize, Held>, d: Vec2) -> Result<()> {
    if held.is_empty() || d == Vec2::ZERO {
        return Ok(());
    }
    let others = !points.is_empty() || !t.anchors.is_empty() || !t.guides.is_empty() || !t.annotations.is_empty();
    let free_shape = t.shapes.iter().any(|s| held.get(s).is_none_or(|h| h.offset(d) != Vec2::ZERO));
    if others || free_shape {
        return Ok(());
    }
    let message = held.iter().map(|(&s, h)| h.message(s)).collect::<Vec<_>>().join("; ");
    Err(t.bad(message))
}

/// Moves (or, with `duplicate`, copies and moves the copies of) what `t` names in `font` by
/// `d`. Returns the selection afterwards: the same targets, or the copies. A move leaves locked
/// and automatically aligned components where they are (see the module notes); `alone` says the
/// target lists are all the command moves, so that it fails when none of them can move.
pub(super) fn apply(font: &mut Font, t: &Targets, d: Vec2, mode: Mode, duplicate: bool, step: f64, alone: bool) -> Result<Targets> {
    let (master, width, points, held) = {
        let r = t.resolve(font)?;
        let held = if duplicate { BTreeMap::new() } else { held_components(font, t, &r) };
        (r.master.id.clone(), r.layer.width, points_by_shape(t, r.parts), held)
    };
    if alone {
        check_something_moves(t, &points, &held, d)?;
    }
    let mut out = t.clone();
    let layer_part = t.edit_layer(font, |layer| {
        let mut parts = layer.parts_mut(t.side);
        if duplicate {
            let copies = duplicate_in(&mut parts, t, &points, d, step);
            if t.side == LayerSide::Foreground {
                copy_attachments(&mut layer.info.hints, &copies.hosts);
            }
            Ok(Some(copies))
        } else {
            move_in(&mut parts, t, &points, &held, d, mode, step).map(|()| None)
        }
    })?;
    let global: Vec<usize> = t.guides.iter().filter(|g| g.scope == GuideScope::Global).map(|g| g.index).collect();
    let mut new_global = Vec::new();
    if !global.is_empty() {
        let masters = Arc::make_mut(&mut font.masters);
        let m = masters.iter_mut().find(|m| m.id == master).ok_or_else(|| EngineError::Internal(format!("master `{master}` went missing")))?;
        for i in global {
            let Some(mut g) = m.guides.get(i).cloned() else { continue };
            shift_guide(&mut g, width, d, step);
            if duplicate {
                new_global.push(GuideRef { scope: GuideScope::Global, index: m.guides.len() });
                m.guides.push(g);
            } else if let Some(slot) = m.guides.get_mut(i) {
                *slot = g;
            }
        }
    }
    if let Some(copies) = layer_part {
        out.segments.clear();
        out.points = copies.points;
        out.shapes = copies.shapes;
        out.anchors = copies.anchors;
        out.guides = copies.guides;
        out.guides.extend(new_global);
        out.annotations = copies.annotations;
    }
    Ok(out)
}

fn move_in(
    parts: &mut LayerPartsMut,
    t: &Targets,
    points: &BTreeMap<usize, BTreeSet<usize>>,
    held: &BTreeMap<usize, Held>,
    d: Vec2,
    mode: Mode,
    step: f64,
) -> Result<()> {
    for (&s, set) in points {
        if let Some(path) = parts.path_mut(s) {
            move_path(path, set, d, mode, step);
        }
    }
    for &s in &t.shapes {
        let by = match held.get(&s).map(|h| h.offset(d)) {
            Some(by) if by == Vec2::ZERO => continue,
            Some(by) => by,
            None => d,
        };
        if let Some(shape) = parts.shapes.get_mut(s) {
            translate_shape(shape, by, step);
        }
    }
    for a in parts.anchors.iter_mut().filter(|a| t.anchors.contains(&a.name)) {
        shift(&mut a.x, &mut a.y, d, step);
    }
    for g in t.guides.iter().filter(|g| g.scope == GuideScope::Local) {
        if let Some(guide) = parts.guides.get_mut(g.index) {
            shift_guide(guide, parts.width, d, step);
        }
    }
    for &i in &t.annotations {
        if let Some(a) = parts.annotations.get_mut(i) {
            shift(&mut a.x, &mut a.y, d, step);
        }
    }
    Ok(())
}

/// The new lists of a duplicate: what the shell selects afterwards.
struct Copies {
    points: Vec<(usize, usize)>,
    shapes: Vec<usize>,
    anchors: Vec<String>,
    guides: Vec<GuideRef>,
    annotations: Vec<usize>,
    hosts: Vec<Host>,
}

/// A copied path, for the hints that sit on it.
struct Host {
    /// The shape index of the original and of the copy.
    from: usize,
    to: usize,
    /// For a copy of part of a path: whether the original is closed, its node count, and the
    /// original node of each copied one.
    part: Option<(bool, usize, Vec<usize>)>,
}

impl Host {
    /// `hint`, an attachment with its node counted in file order, on the copy of part of a path.
    fn part_hint(&self, hint: &Hint) -> Option<Hint> {
        let (closed, len, nodes) = self.part.as_ref()?;
        crate::cmd::shared::move_hint_origin(hint, &|shape, node| {
            if shape != self.from {
                return None;
            }
            // On a closed path the file's node k is the model's node k + 1, the start node last.
            let node = if *closed { node.checked_add(1)?.checked_rem(*len)? } else { node };
            let at = nodes.iter().position(|&n| n == node)?;
            let (first, last) = (at == 0, at + 1 == nodes.len());
            let kept = if is_corner(hint) {
                !first && !last
            } else if is_cap(hint) {
                !last
            } else {
                !first
            };
            kept.then_some((self.to, at))
        })
    }
}

/// Adds copies of the corner, cap and segment components and brushes of `hints` that sit on a
/// copied path, as far as the layer has room for hints.
fn copy_attachments(hints: &mut Vec<Hint>, hosts: &[Host]) {
    let whole: BTreeMap<usize, usize> = hosts.iter().filter(|h| h.part.is_none()).map(|h| (h.from, h.to)).collect();
    let mut copies = Vec::new();
    for hint in hints.iter().filter(|h| is_corner(h) || is_attachment(h)) {
        match crate::cmd::shared::remap_hint(hint, &|shape| whole.get(&shape).copied()) {
            Some(copy) if copy != *hint => copies.push(copy),
            Some(_) => {}
            None => copies.extend(hosts.iter().filter_map(|h| h.part_hint(hint))),
        }
    }
    let room = MAX_LAYER_HINTS.saturating_sub(hints.len());
    hints.extend(copies.into_iter().take(room));
}

fn duplicate_in(parts: &mut LayerPartsMut, t: &Targets, points: &BTreeMap<usize, BTreeSet<usize>>, d: Vec2, step: f64) -> Copies {
    let first = parts.shapes.len();
    let mut new_shapes: Vec<Shape> = Vec::new();
    let mut hosts: Vec<Host> = Vec::new();
    for &s in &t.shapes {
        if let Some(shape) = parts.shapes.get(s) {
            let mut copy = shape.clone();
            if let Shape::Path(p) = &mut copy
                && p.info.identifier.is_some()
            {
                p.info.identifier = None;
            }
            hosts.push(Host { from: s, to: first + new_shapes.len(), part: None });
            new_shapes.push(copy);
        }
    }
    for (&s, set) in points {
        let Some(Shape::Path(path)) = parts.shapes.get(s) else { continue };
        if (0..path.nodes.len()).all(|i| set.contains(&i)) {
            let mut whole = path.clone();
            if whole.info.identifier.is_some() {
                whole.info.identifier = None;
            }
            hosts.push(Host { from: s, to: first + new_shapes.len(), part: None });
            new_shapes.push(Shape::Path(whole));
        } else {
            for (copy, nodes) in super::ops::selected_segments(path, set) {
                let part = nodes.map(|nodes| (path.closed, path.nodes.len(), nodes));
                hosts.push(Host { from: s, to: first + new_shapes.len(), part });
                new_shapes.push(Shape::Path(copy));
            }
        }
    }
    let mut copies = Copies { points: Vec::new(), shapes: Vec::new(), anchors: Vec::new(), guides: Vec::new(), annotations: Vec::new(), hosts };
    for (k, mut shape) in new_shapes.into_iter().enumerate() {
        translate_shape(&mut shape, d, step);
        let index = first + k;
        match &shape {
            Shape::Path(p) => copies.points.extend((0..p.nodes.len()).map(|n| (index, n))),
            _ => copies.shapes.push(index),
        }
        parts.shapes.push(shape);
    }
    for name in &t.anchors {
        let Some(a) = parts.anchors.iter().find(|a| a.name == *name) else { continue };
        // The anchor rule: underscores appended until the name is free (`top_`).
        let Some(fresh) = crate::cmd::anchor::unique_anchor_name(parts.anchors, name) else { continue };
        let mut copy = a.clone();
        copy.name = fresh.clone();
        shift(&mut copy.x, &mut copy.y, d, step);
        copies.anchors.push(fresh);
        parts.anchors.push(copy);
    }
    for g in t.guides.iter().filter(|g| g.scope == GuideScope::Local) {
        let Some(mut copy) = parts.guides.get(g.index).cloned() else { continue };
        shift_guide(&mut copy, parts.width, d, step);
        copies.guides.push(GuideRef { scope: GuideScope::Local, index: parts.guides.len() });
        parts.guides.push(copy);
    }
    for &i in &t.annotations {
        let Some(mut copy) = parts.annotations.get(i).cloned() else { continue };
        shift(&mut copy.x, &mut copy.y, d, step);
        copies.annotations.push(parts.annotations.len());
        parts.annotations.push(copy);
    }
    copies
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fontcraft_model::{Alignment, Component, Shape};
    use serde_json::{Value, json};

    use crate::Session;

    fn run(s: &mut Session, id: &str, p: Value) -> Value {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
    }

    fn xy(s: &Session) -> Vec<(f64, f64)> {
        let font = s.font().unwrap();
        let master = font.default_master().unwrap().id.clone();
        match font.glyph("A").unwrap().layer(&master).unwrap().shapes.last() {
            Some(fontcraft_model::Shape::Path(p)) => p.nodes.iter().map(|n| (n.x, n.y)).collect(),
            other => panic!("not a path: {other:?}"),
        }
    }

    /// Two curves meeting at node 3 (200,200), whose handles (100,200) and (300,200) are level.
    fn session(smooth: bool) -> Session {
        let mut s = Session::with_document();
        let nodes = json!([
            {"x": 0, "y": 0},
            {"x": 0, "y": 100, "type": "offcurve"}, {"x": 100, "y": 200, "type": "offcurve"}, {"x": 200, "y": 200, "type": "curve", "smooth": smooth},
            {"x": 300, "y": 200, "type": "offcurve"}, {"x": 400, "y": 100, "type": "offcurve"}, {"x": 400, "y": 0, "type": "curve"},
        ]);
        run(&mut s, "path.addPath", json!({"glyph": "A", "closed": false, "nodes": nodes}));
        s
    }

    #[test]
    fn mirror_makes_a_smooth_node_symmetric_and_leaves_a_corner_alone() {
        let mut s = session(true);
        let before = xy(&s);
        run(&mut s, "selection.move", json!({"glyph": "A", "points": [[0, 4]], "dx": 50, "dy": 30, "mode": "mirror"}));
        let after = xy(&s);
        assert_eq!(after[4], (350.0, 230.0));
        assert_eq!(after[2], (50.0, 170.0), "the opposite handle mirrors the moved one through the node");
        assert_eq!(after[3], (200.0, 200.0), "the node stays");
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(xy(&s), before, "one undo step");

        // A plain move turns the opposite handle but keeps its length.
        run(&mut s, "selection.move", json!({"glyph": "A", "points": [[0, 4]], "dx": 50, "dy": 30}));
        let (x, y) = xy(&s)[2];
        assert!(((200.0 - x).hypot(200.0 - y) - 100.0).abs() <= 1.0, "length kept: ({x}, {y})");

        let mut s = session(false);
        run(&mut s, "selection.move", json!({"glyph": "A", "points": [[0, 4]], "dx": 50, "dy": 30, "mode": "mirror"}));
        let after = xy(&s);
        assert_eq!((after[2], after[4]), ((100.0, 200.0), (350.0, 230.0)), "a corner's other handle stays");

        let e = s.execute("selection.move", &json!({"glyph": "A", "points": [[0, 4]], "dx": 1, "dy": 1, "mode": "mirrored"})).unwrap_err();
        assert!(e.to_string().contains("`mode`"), "{e}");
    }

    const SQUARE: &str = r#"[{"x": 0, "y": 0}, {"x": 100, "y": 0}, {"x": 100, "y": 100}, {"x": 0, "y": 100}]"#;

    /// A document whose glyph `A` holds one component of `B` (a square) with the given lock and
    /// alignment, and nothing else.
    fn component_session(locked: bool, alignment: Alignment) -> Session {
        let mut s = Session::with_document();
        let square: Value = serde_json::from_str(SQUARE).unwrap();
        run(&mut s, "path.addPath", json!({"glyph": "B", "nodes": square}));
        s.edit("setup", |font| {
            let mut g = font.glyph_mut("A")?;
            let mut c = Component::new("B", 0.0, 0.0);
            c.info.locked = locked;
            c.info.alignment = alignment;
            g.layers.first_mut().unwrap().shapes.push(Shape::Component(c));
            Ok(())
        })
        .unwrap();
        s
    }

    fn component_xy(s: &Session) -> (f64, f64) {
        let font = s.font().unwrap();
        let master = font.default_master().unwrap().id.clone();
        match font.glyph("A").unwrap().layer(&master).unwrap().shapes.first() {
            Some(Shape::Component(c)) => {
                let a = c.transform.affine().as_coeffs();
                (a[4], a[5])
            }
            other => panic!("not a component: {other:?}"),
        }
    }

    fn refused(s: &mut Session, p: Value) -> String {
        let before = s.doc().unwrap().font.clone();
        let e = s.execute("selection.move", &p).unwrap_err().to_string();
        assert!(Arc::ptr_eq(&before, &s.doc().unwrap().font), "a refused move changes nothing: {e}");
        e
    }

    #[test]
    fn arrow_keys_leave_a_locked_component_and_move_the_rest() {
        let mut s = component_session(true, Alignment::Disabled);
        let e = refused(&mut s, json!({"glyph": "A", "shapes": [0], "dx": 10, "dy": 0}));
        assert!(e.contains("component 0 is locked"), "{e}");

        // With a path beside it, the path moves and the component stays.
        let square: Value = serde_json::from_str(SQUARE).unwrap();
        run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": square}));
        let at = component_xy(&s);
        run(&mut s, "selection.move", json!({"glyph": "A", "shapes": [0, 1], "dx": 10, "dy": 0}));
        assert_eq!(component_xy(&s), at, "the locked component stays");
        assert_eq!(xy(&s)[0], (10.0, 0.0), "the path moves");

        // A copy is made all the same: the original stays where it is.
        run(&mut s, "selection.move", json!({"glyph": "A", "shapes": [0], "dx": 10, "dy": 0, "duplicate": true}));
        assert_eq!(component_xy(&s), at);
    }

    #[test]
    fn arrow_keys_move_an_aligned_component_only_on_its_free_axis() {
        // Alone on the layer with the default alignment: aligned in both directions.
        let mut s = component_session(false, Alignment::Default);
        let e = refused(&mut s, json!({"glyph": "A", "shapes": [0], "dx": 10, "dy": 10}));
        assert!(e.contains("aligned automatically"), "{e}");

        let mut s = component_session(false, Alignment::Horizontal);
        let (x, y) = component_xy(&s);
        run(&mut s, "selection.move", json!({"glyph": "A", "shapes": [0], "dx": 10, "dy": 20}));
        assert_eq!(component_xy(&s), (x, y + 20.0), "only y changes");
        let e = refused(&mut s, json!({"glyph": "A", "shapes": [0], "dx": 10, "dy": 0}));
        assert!(e.contains("moves only up and down"), "{e}");
        // selection.setPosition follows the same rule.
        run(&mut s, "selection.setPosition", json!({"glyph": "A", "shapes": [0], "x": 500, "y": 300}));
        assert_eq!(component_xy(&s), (x, 300.0));
    }
}
